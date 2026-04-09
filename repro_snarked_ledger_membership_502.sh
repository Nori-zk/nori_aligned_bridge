#!/bin/bash
#
# Reproduces a Mina daemon bug where encodedSnarkedLedgerAccountMembership
# returns 502 Bad Gateway for a valid account.
#
# The account exists in the staged ledger with a non-zero token balance, but the
# daemon crashes when asked for its snarked ledger membership proof. It should
# return a GraphQL error (e.g. "account not found in snarked ledger"), not 502.
#
# Tests against multiple independent nodes to confirm this is a daemon bug, not
# an infrastructure issue.
#
# Requirements: curl, jq
# Usage: bash repro_snarked_ledger_membership_502.sh

set -euo pipefail

NODES=(
  "https://api.minascan.io/node/devnet/v1/graphql"
  "https://mina-node.devnet.nori.it.com/graphql"
  "https://devnet-plain-1.gcp.o1test.net/graphql"
)
PUBLIC_KEY="B62qrpX8gianx6Yifq2r2F1UvX175917h5i2tgvsPwHz5KEcxnFpy8f"
TOKEN="x46WDgG2zDwsVzWmYJ3jb1PBjRvawvirVjxUFdaADnYz2xhuAi" # cargo run --bin token_id_to_base58 -- 1676738181413133854148376313277099836893922609484575143243904052352543641682

echo "PublicKey: $PUBLIC_KEY"
echo "Token:     $TOKEN"
echo ""

for NODE in "${NODES[@]}"; do
  echo "================================================================"
  echo "Node: $NODE"
  echo "================================================================"
  echo ""

  # Step 1: Get the current chain tip. Proves the node is reachable.
  echo "--- Step 1: Get current chain tip ---"
  TIP_RESPONSE=$(curl -s -X POST "$NODE" -H "Content-Type: application/json" \
    -d '{"query":"{ bestChain(maxLength: 1) { stateHash protocolState { consensusState { blockHeight } } } }"}')
  echo "$TIP_RESPONSE" | jq .
  STATE_HASH=$(echo "$TIP_RESPONSE" | jq -r '.data.bestChain[0].stateHash')

  if [ "$STATE_HASH" = "null" ] || [ -z "$STATE_HASH" ]; then
    echo "SKIP: Could not get tip state hash. Node may be down."
    echo ""
    continue
  fi
  echo ""

  # Step 2: Prove the account exists in the staged ledger with a non-zero balance.
  echo "--- Step 2: Prove account exists in staged ledger ---"
  curl -s -X POST "$NODE" -H "Content-Type: application/json" \
    -d "{\"query\":\"{ account(publicKey: \\\"$PUBLIC_KEY\\\", token: \\\"$TOKEN\\\") { tokenId balance { total } nonce } }\"}" | jq .
  echo ""

  # Step 3: Request the snarked ledger membership proof. This 502s.
  echo "--- Step 3: encodedSnarkedLedgerAccountMembership ---"
  echo "stateHash: $STATE_HASH"
  PAYLOAD=$(jq -n \
    --arg sh "$STATE_HASH" \
    --arg pk "$PUBLIC_KEY" \
    --arg tk "$TOKEN" \
    '{query: "query($sh: String!, $ai: [AccountInput!]!) { encodedSnarkedLedgerAccountMembership(stateHash: $sh, accountInfos: $ai) { account } }", variables: {sh: $sh, ai: [{publicKey: $pk, token: $tk}]}}')
  HTTP_CODE=$(curl -s -o /tmp/repro_membership_response.txt -w "%{http_code}" \
    -X POST "$NODE" -H "Content-Type: application/json" \
    -d "$PAYLOAD")
  cat /tmp/repro_membership_response.txt
  echo ""
  echo "HTTP_STATUS: $HTTP_CODE"

  if [ "$HTTP_CODE" = "502" ]; then
    echo "BUG CONFIRMED on $NODE"
  fi
  echo ""
done
