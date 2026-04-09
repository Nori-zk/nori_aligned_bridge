#!/bin/bash
#
# Counts ALL transactions associated with the Nori bridge burn account on Mina devnet.
#
# Strategy:
#   1. Query archive for all burn events — gives every block height where the
#      contract emitted an event involving the account.
#   2. For each event block height, also check adjacent blocks (mint tx may land
#      in a different block than the burn tx).
#   3. Query the daemon block-by-block for each candidate height, counting every
#      user command and zkApp command that touches the account.
#   4. Deduplicate by tx hash so nothing is double-counted.
#
# Requirements: curl, jq
# Usage: bash count_account_transactions.sh

set -euo pipefail

DAEMON="https://devnet-plain-1.gcp.o1test.net/graphql"
ARCHIVE="https://devnet-archive-node-api.gcp.o1test.net/"
PUBLIC_KEY="B62qrpX8gianx6Yifq2r2F1UvX175917h5i2tgvsPwHz5KEcxnFpy8f" # ADMIN
CONTRACT="B62qpLUxa2RuHYFNXHbuFgZMxCWp4kSNQE8prNXeazZD1dzFiYYHvJi" # TOKEN

echo "Account:  $PUBLIC_KEY"
echo "Contract: $CONTRACT"
echo ""

# Step 1: Get the full range of blocks to scan.
# Use the daemon's bestChain to get (stateHash, height) for every block it can serve.
# This is the maximum queryable window (~290 blocks).

echo "Fetching daemon bestChain for full block index..."
CHAIN_RESPONSE=$(curl -s -X POST "$DAEMON" -H "Content-Type: application/json" \
  -d '{"query":"{ bestChain(maxLength: 290) { stateHash protocolState { consensusState { blockHeight } } } }"}')

if echo "$CHAIN_RESPONSE" | jq -e '.errors' > /dev/null 2>&1; then
  echo "ERROR: daemon bestChain query failed:"
  echo "$CHAIN_RESPONSE" | jq '.errors'
  exit 1
fi

DAEMON_BLOCK_COUNT=$(echo "$CHAIN_RESPONSE" | jq '.data.bestChain | length')
DAEMON_MIN_HEIGHT=$(echo "$CHAIN_RESPONSE" | jq '.data.bestChain[0].protocolState.consensusState.blockHeight')
DAEMON_MAX_HEIGHT=$(echo "$CHAIN_RESPONSE" | jq '.data.bestChain[-1].protocolState.consensusState.blockHeight')
echo "Daemon serves $DAEMON_BLOCK_COUNT blocks: heights $DAEMON_MIN_HEIGHT to $DAEMON_MAX_HEIGHT"

# Build a lookup of stateHash by height from the daemon chain.
# We need stateHashes to query individual blocks.
declare -A HEIGHT_TO_HASH
while IFS=$'\t' read -r h sh; do
  HEIGHT_TO_HASH[$h]="$sh"
done < <(echo "$CHAIN_RESPONSE" | jq -r '.data.bestChain[] | [.protocolState.consensusState.blockHeight, .stateHash] | @tsv')

# Step 2: Get all burn event block heights from the archive (full history).

echo ""
echo "Fetching all burn events from archive (full history)..."
ARCHIVE_RESPONSE=$(curl -s -X POST "$ARCHIVE" -H "Content-Type: application/json" \
  -d "{\"query\":\"{ events(input: { address: \\\"$CONTRACT\\\", status: ALL }) { blockInfo { height stateHash } } }\"}")

if echo "$ARCHIVE_RESPONSE" | jq -e '.errors' > /dev/null 2>&1; then
  echo "ERROR: archive query failed:"
  echo "$ARCHIVE_RESPONSE" | jq '.errors'
  exit 1
fi

ARCHIVE_EVENT_COUNT=$(echo "$ARCHIVE_RESPONSE" | jq '.data.events | length')
echo "Archive burn events (all-time): $ARCHIVE_EVENT_COUNT"

# Collect unique event block heights from archive.
EVENT_HEIGHTS=$(echo "$ARCHIVE_RESPONSE" | jq -r '[.data.events[].blockInfo.height] | unique | .[]')

# Step 3: Build the set of ALL candidate block heights to scan.
# For each event height, include height-1 and height+1 to catch mint txs
# that landed in an adjacent block.
declare -A CANDIDATE_HEIGHTS
for h in $EVENT_HEIGHTS; do
  for offset in -1 0 1; do
    candidate=$((h + offset))
    CANDIDATE_HEIGHTS[$candidate]=1
  done
done

# Also add every daemon block — catches any transactions not associated with
# a burn event (e.g. standalone mints, funding txs, fee payer only txs).
for h in "${!HEIGHT_TO_HASH[@]}"; do
  CANDIDATE_HEIGHTS[$h]=1
done

CANDIDATE_COUNT=${#CANDIDATE_HEIGHTS[@]}
echo ""
echo "Candidate block heights to scan: $CANDIDATE_COUNT"

# Step 4: Query each candidate block and collect all tx hashes involving the account.

echo "Scanning blocks for transactions involving $PUBLIC_KEY..."
echo ""

ALL_TX_HASHES_FILE=$(mktemp)
SCANNED=0
DAEMON_SERVED=0
OFF_FRONTIER=0

for h in $(echo "${!CANDIDATE_HEIGHTS[@]}" | tr ' ' '\n' | sort -n); do
  STATE_HASH="${HEIGHT_TO_HASH[$h]:-}"

  if [ -z "$STATE_HASH" ]; then
    OFF_FRONTIER=$((OFF_FRONTIER + 1))
    continue
  fi

  BLOCK_RESPONSE=$(curl -s -X POST "$DAEMON" -H "Content-Type: application/json" \
    -d "{\"query\":\"{ block(stateHash: \\\"$STATE_HASH\\\") { transactions { userCommands { hash source { publicKey } receiver { publicKey } } zkappCommands { hash zkappCommand { feePayer { body { publicKey } } accountUpdates { body { publicKey } } } } } } }\"}")

  BLOCK_DATA=$(echo "$BLOCK_RESPONSE" | jq '.data.block')
  if [ "$BLOCK_DATA" = "null" ]; then
    OFF_FRONTIER=$((OFF_FRONTIER + 1))
    continue
  fi

  # Extract tx hashes for user commands involving the account
  echo "$BLOCK_RESPONSE" | jq -r --arg acct "$PUBLIC_KEY" --arg h "$h" '
    .data.block.transactions.userCommands[] |
    select(.source.publicKey == $acct or .receiver.publicKey == $acct) |
    "\(.hash)\tuser_cmd\t\($h)"' >> "$ALL_TX_HASHES_FILE" 2>/dev/null || true

  # Extract tx hashes for zkApp commands involving the account
  echo "$BLOCK_RESPONSE" | jq -r --arg acct "$PUBLIC_KEY" --arg h "$h" '
    .data.block.transactions.zkappCommands[] |
    select(.zkappCommand.feePayer.body.publicKey == $acct or
           (.zkappCommand.accountUpdates[].body.publicKey == $acct)) |
    "\(.hash)\tzkapp_cmd\t\($h)"' >> "$ALL_TX_HASHES_FILE" 2>/dev/null || true

  DAEMON_SERVED=$((DAEMON_SERVED + 1))
  SCANNED=$((SCANNED + 1))

  # Progress every 50 blocks
  if [ $((SCANNED % 50)) -eq 0 ]; then
    echo "  ...scanned $SCANNED blocks"
  fi
done

echo "  ...scanned $SCANNED blocks (done)"
echo ""

# Step 5: Deduplicate by tx hash and report.

UNIQUE_TX_COUNT=$(sort -u -t$'\t' -k1,1 "$ALL_TX_HASHES_FILE" | wc -l | tr -d ' ')
UNIQUE_USER_CMDS=$(grep 'user_cmd' "$ALL_TX_HASHES_FILE" | sort -u -t$'\t' -k1,1 | wc -l | tr -d ' ' || echo 0)
UNIQUE_ZKAPP_CMDS=$(grep 'zkapp_cmd' "$ALL_TX_HASHES_FILE" | sort -u -t$'\t' -k1,1 | wc -l | tr -d ' ' || echo 0)

echo "================================================================"
echo "Results"
echo "================================================================"
echo ""
echo "Blocks scanned by daemon:      $DAEMON_SERVED"
echo "Blocks off daemon frontier:    $OFF_FRONTIER"
echo ""
echo "Unique user commands:          $UNIQUE_USER_CMDS"
echo "Unique zkApp commands:         $UNIQUE_ZKAPP_CMDS"
echo "---"
echo "TOTAL unique transactions:     $UNIQUE_TX_COUNT"
echo ""

if [ "$OFF_FRONTIER" -gt 0 ]; then
  echo "WARNING: $OFF_FRONTIER block(s) are off the daemon frontier and could not be scanned."
  echo "The true total may be higher."
fi

# List all unique tx hashes
echo ""
echo "================================================================"
echo "All transaction hashes"
echo "================================================================"
sort -u -t$'\t' -k1,1 "$ALL_TX_HASHES_FILE" | while IFS=$'\t' read -r hash type height; do
  echo "  height=$height type=$type hash=$hash"
done

rm -f "$ALL_TX_HASHES_FILE"
