//! Mina daemon GraphQL primitives and proof assembly for the bridge service workers.
//!
//! Self-contained subset of `mina.rs` containing only the queries and proof assembly
//! functions that `rpcs/mina_daemon.rs` actually needs. `rpcs/mina_daemon.rs` imports
//! from this module, not from `mina.rs`.
//!
//! # What was left behind in `mina.rs` and why
//!
//! - `get_mina_proof_of_state`: uses the old architecture where the bridge tip is read
//!   from the Ethereum contract via `get_bridge_tip_hash`. The service workers target an
//!   arbitrary group finalization block height (F_g) set by Worker 2, not a bridge tip
//!   fetched from a contract. The replacement lives in this module.
//! - `query_candidate_chain_0`: hardcodes `max_length` to `BRIDGE_TRANSITION_FRONTIER_LEN`
//!   (16) and assumes the tip IS the proof window -- only valid when proving the live tip,
//!   not an arbitrary F_g that may be hundreds of blocks behind.
//! - `query_candidate_chain`: identical to `query_candidate_chain_0` but derives ledger
//!   hashes from full protocol states instead of the GraphQL response -- same limitation.
//! - `query_root`: returns the root state hash of the frontier -- not used by any worker.

use std::str::FromStr;

use alloy_sol_types::SolValue;
use base64::prelude::*;
use futures::future::join_all;
use graphql_client::{reqwest::post_graphql, GraphQLQuery};
use kimchi::mina_curves::pasta::Fp;
use log::info;
use mina_p2p_messages::{
    binprot::BinProtRead,
    hash::MinaHash,
    v2::{
        LedgerHash, MinaBaseAccountBinableArgStableV2 as MinaAccount, MinaBaseProofStableV2,
        MinaBaseZkappAccountStableV2, MinaStateProtocolStateValueStableV2, StateHash,
    },
};
use mina_state_verifier::verify_mina_state;

use crate::{
    proof::account_proof::{MerkleNode, MinaAccountProof, MinaAccountPubInputs},
    proof::state_proof::{MinaStatePubInputs, MinaStateProof},
    rpcs::errors::MinaDaemonError,
    sol::account::MinaAccountValidationExample,
    utils::constants::{BRIDGE_TRANSITION_FRONTIER_LEN, MINA_DAEMON_MAX_QUERYABLE_BLOCKS},
};

/// Extra blocks to request beyond the calculated max_length when querying bestChain,
/// to account for the tip advancing between the frontier query and the bestChain query.
const BEST_CHAIN_QUERY_BUFFER: usize = 4;

// --- GraphQL type aliases required by graphql_client codegen ---

type StateHashAsDecimal = String;
type PrecomputedBlockProof = String;
type FieldElem = String;
type Length = String;
type TokenId = String;
type PublicKey = String;

// --- GraphQL query definitions ---

/// Queries a single protocol state by its state hash string.
///
/// GraphQL: `protocolState(encoding: BASE64, stateHash: $stateHash)`
///
/// Returns the full protocol state as a base64-encoded binprot blob. The caller is
/// responsible for decoding and deserializing.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/mina_schema.json",
    query_path = "graphql/state_query.graphql"
)]
struct StateQuery;

/// Queries the best chain (transition frontier) up to `maxLength` blocks.
///
/// GraphQL: `bestChain(maxLength: $maxLength)` returning `stateHashField`, `stateHash`,
/// `protocolStateProof.base64`, and `protocolState.blockchainState.snarkedLedgerHash`.
///
/// # Limitation
///
/// `bestChain` always returns the most recent `maxLength` blocks counting backwards from
/// the current tip. It cannot target a specific block height or hash range.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/mina_schema.json",
    query_path = "graphql/best_chain_query.graphql"
)]
struct BestChainQuery;

/// Queries an account's state, merkle proof, and ledger hash at a given block.
///
/// GraphQL: `encodedSnarkedLedgerAccountMembership(stateHash: $stateHash, accountInfos: $accountInfos)`
/// plus `block(stateHash: $stateHash).protocolState.blockchainState.snarkedLedgerHash`.
///
/// Returns the account as a base64-encoded binprot blob, a merkle path of left/right field
/// element siblings, and the snarked ledger hash at the given block.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/mina_schema.json",
    query_path = "graphql/account_query.graphql"
)]
struct AccountQuery;

/// Lightweight query for the state hash and block height of each block in the transition
/// frontier.
///
/// GraphQL: `bestChain(maxLength: $maxLength)` returning `stateHash` and
/// `protocolState.consensusState.blockHeight`.
///
/// Used by `MinaTransitionFrontierMonitor` to snapshot the frontier, and by
/// `AlignedProofSubmitter` to determine the live tip height.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/mina_schema.json",
    query_path = "graphql/frontier_query.graphql"
)]
struct FrontierQuery;

/// Queries a single block by height, returning the state hash, snarked ledger hash, and
/// protocol state proof.
///
/// GraphQL: `block(height: $height)` returning `stateHashField`, `stateHash`,
/// `protocolStateProof.base64`, and `protocolState.blockchainState.snarkedLedgerHash`.
///
/// Unlike `BestChainQuery`, `block(height:)` is deterministic -- it always returns the
/// canonical block at the given height regardless of where the tip is. The caller can
/// target an exact block height without overfetching or worrying about the tip advancing
/// between calls.
///
/// Used by `less_insane_get_mina_proof_of_state`.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/mina_schema.json",
    query_path = "graphql/state_proof_block_query.graphql"
)]
struct StateProofBlockQuery;

/// Lightweight query for a single block's state hash by height.
///
/// GraphQL: `block(height: $height)` returning only `stateHash`.
///
/// Used by `query_block_state_hash_at_height`.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/mina_schema.json",
    query_path = "graphql/block_at_height_query.graphql"
)]
struct BlockAtHeightQuery;

// --- Primitive query functions ---

/// Queries a single protocol state from the Mina daemon by state hash.
///
/// # Arguments
///
/// * `rpc_url` - The Mina daemon GraphQL endpoint URL.
/// * `state_hash` - The state hash to query, as a `mina_p2p_messages` `StateHash`.
///
/// # Returns
///
/// The deserialized protocol state value on success, or a `MinaDaemonError` describing
/// the failure (network error, missing data, base64 decode failure, or binprot
/// deserialization failure).
pub async fn query_state(
    rpc_url: &str,
    state_hash: &StateHash,
) -> Result<MinaStateProtocolStateValueStableV2, MinaDaemonError> {
    let variables = state_query::Variables {
        state_hash: state_hash.to_string(),
    };
    info!("Querying state {}", variables.state_hash);
    let client = reqwest::Client::new();
    let response = post_graphql::<StateQuery, _>(&client, rpc_url, variables).await?;
    if let Some(errors) = response.errors {
        if !errors.is_empty() {
            let msgs: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
            return Err(MinaDaemonError::GraphQLError(msgs.join("; ")));
        }
    }
    let data = response.data.ok_or_else(|| MinaDaemonError::MalformedResponse("missing response data".into()))?;
    let proof = BASE64_STANDARD
        .decode(data.protocol_state)
        .map_err(|err| MinaDaemonError::MalformedResponse(format!("Couldn't decode state from base64: {err}")))
        .and_then(|binprot| {
            MinaStateProtocolStateValueStableV2::binprot_read(&mut binprot.as_slice())
                .map_err(|err| MinaDaemonError::MalformedResponse(format!("Couldn't read state binprot: {err}")))
        })?;
    Ok(proof)
}

/// Queries the transition frontier for up to `max_length` blocks, returning each block's
/// state hash and block height.
///
/// # Arguments
///
/// * `rpc_url` - The Mina daemon GraphQL endpoint URL.
/// * `max_length` - The number of blocks to request from the frontier. The daemon must
///   return exactly this many blocks or the call fails.
///
/// # Returns
///
/// A `Vec` of `(StateHash, block_height)` tuples ordered oldest-to-newest (the order
/// returned by the daemon's `bestChain` query), or a `MinaDaemonError`.
pub async fn query_frontier(
    rpc_url: &str,
    max_length: usize,
) -> Result<Vec<(StateHash, u64)>, MinaDaemonError> {
    let client = reqwest::Client::new();
    let variables = frontier_query::Variables {
        max_length: max_length
            .try_into()
            .map_err(|_| MinaDaemonError::BadRequest("Frontier length conversion failure".into()))?,
    };
    let response = post_graphql::<FrontierQuery, _>(&client, rpc_url, variables).await?;
    if let Some(errors) = response.errors {
        if !errors.is_empty() {
            let msgs: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
            return Err(MinaDaemonError::GraphQLError(msgs.join("; ")));
        }
    }
    let data = response.data.ok_or_else(|| MinaDaemonError::MalformedResponse("missing response data".into()))?;
    let best_chain = data
        .best_chain
        .ok_or_else(|| MinaDaemonError::MalformedResponse("Missing best chain field".into()))?;
    if best_chain.len() != max_length {
        return Err(MinaDaemonError::MalformedResponse(format!(
            "Expected {} blocks from frontier query, got {}",
            max_length,
            best_chain.len()
        )));
    }
    best_chain
        .into_iter()
        .map(|block| {
            let state_hash = block.state_hash;
            let block_height: u64 = block
                .protocol_state
                .consensus_state
                .block_height
                .parse()
                .map_err(|_| MinaDaemonError::MalformedResponse("Block height conversion failure".into()))?;
            Ok((state_hash, block_height))
        })
        .collect()
}

/// Queries the daemon for the state hash of the block at `height`.
///
/// # What this does
///
/// Sends `block(height: N)` to the Mina daemon, which returns the state hash of
/// whichever block the daemon currently considers canonical at that height. This is a
/// lightweight query that fetches only the state hash (no proofs, no ledger hashes).
///
/// # THIS IS A BEST-EFFORT CANONICALITY CHECK -- READ CAREFULLY
///
/// The Mina protocol does not have deterministic finality. There is no on-chain flag,
/// no RPC method, and no oracle that can guarantee a block is permanently canonical.
/// Mina uses Ouroboros proof-of-stake consensus where finality is probabilistic: the
/// deeper a block is buried under subsequent blocks, the less likely it is to be
/// reorged, but the probability never reaches zero.
///
/// What this function actually returns is the daemon's CURRENT consensus view of which
/// block is canonical at the given height. The daemon maintains a transition frontier
/// of approximately `MINA_DAEMON_MAX_QUERYABLE_BLOCKS` (290) recent blocks, and within
/// that window it has resolved all forks using consensus rules. The block it returns is
/// the one on the winning fork RIGHT NOW.
///
/// According to o1labs, Mina has never experienced a reorg deeper than approximately 11
/// blocks. The bridge classifier calls this function only after a burn's detection block
/// is at least `BRIDGE_TRANSITION_FRONTIER_LEN - 1` (15) blocks deep. At that depth, a
/// reorg is extremely unlikely based on all observed network history, but it is not
/// impossible.
///
/// If the daemon reorgs after this function returns, the state hash it returned may no
/// longer be canonical. The caller is responsible for understanding that this answer
/// can, in theory, become stale.
///
/// # Errors
///
/// Returns `MinaDaemonError::BlockNotInFrontier` when the daemon responds with "Could
/// not find block in transition frontier". This means the daemon cannot answer for this
/// height -- it does NOT mean the block is non-canonical. Possible causes: the block is
/// older than ~290 blocks from the tip, the daemon just restarted with an incomplete
/// frontier, or the height has not been reached yet.
///
/// Returns other `MinaDaemonError` variants for infrastructure failures: the daemon is
/// unreachable, returned malformed data, or something unexpected happened.
///
/// All errors are about the daemon's ability to answer, never about the block's
/// canonicality.
pub async fn best_effort_canonical_state_hash_at_height(
    rpc_url: &str,
    height: u64,
) -> Result<String, MinaDaemonError> {
    let client = reqwest::Client::new();
    let variables = block_at_height_query::Variables {
        height: height as i64,
    };
    let response = post_graphql::<BlockAtHeightQuery, _>(&client, rpc_url, variables).await?;
    if let Some(errors) = &response.errors {
        if !errors.is_empty() {
            let msgs: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
            let joined = msgs.join("; ");
            if joined.contains("Could not find block in transition frontier") {
                return Err(MinaDaemonError::BlockNotInFrontier(format!(
                    "height {height}: {joined}"
                )));
            }
            return Err(MinaDaemonError::GraphQLError(joined));
        }
    }
    let state_hash = response
        .data
        .ok_or_else(|| MinaDaemonError::MalformedResponse(
            format!("block(height:{height}): no errors but response data is null"),
        ))?
        .block
        .state_hash;
    Ok(state_hash.to_string())
}

/// Queries the best chain with a caller-specified `max_length` and returns state hashes,
/// snarked ledger hashes, and the protocol state proof for each block.
///
/// Returns `Vec`s rather than fixed-size arrays so that the caller may request more than
/// `BRIDGE_TRANSITION_FRONTIER_LEN` blocks.
///
/// # Arguments
///
/// * `rpc_url` - The Mina daemon GraphQL endpoint URL.
/// * `max_length` - The number of blocks to request from the best chain.
///
/// # Returns
///
/// A tuple of `(state_hashes, ledger_hashes, proofs)` on success, or a `MinaDaemonError`.
/// Each proof is base64-decoded and binprot-deserialized into `MinaBaseProofStableV2`.
///
/// # Limitation
///
/// Inherits `BestChainQuery`'s limitation: the result set always counts backwards from
/// the current tip. The caller cannot target a specific block height or hash range, such
/// as an arbitrary group finalization block height (F_g).
#[deprecated(note = "Cannot target F_g; use per-height block queries instead")]
pub async fn query_state_proof_candidate_chain(
    rpc_url: &str,
    max_length: usize,
) -> Result<
    (
        Vec<StateHash>,
        Vec<LedgerHash>,
        Vec<MinaBaseProofStableV2>,
    ),
    MinaDaemonError,
> {
    info!("Querying candidate chain for state proof construction with max_length={max_length}");
    let client = reqwest::Client::new();
    let variables = best_chain_query::Variables {
        max_length: max_length
            .try_into()
            .map_err(|_| MinaDaemonError::BadRequest("max_length conversion failure".into()))?,
    };
    let response = post_graphql::<BestChainQuery, _>(&client, rpc_url, variables)
        .await?
        .data
        .ok_or_else(|| MinaDaemonError::MalformedResponse("Missing candidate query response data".into()))?;
    let best_chain = response
        .best_chain
        .ok_or_else(|| MinaDaemonError::MalformedResponse("Missing best chain field".into()))?;

    let chain_state_hashes: Vec<StateHash> = best_chain
        .iter()
        .map(|block| block.state_hash.clone())
        .collect();
    let chain_ledger_hashes: Vec<LedgerHash> = best_chain
        .iter()
        .map(|block| {
            block
                .protocol_state
                .blockchain_state
                .snarked_ledger_hash
                .clone()
        })
        .collect();
    let chain_proofs: Vec<MinaBaseProofStableV2> = best_chain
        .iter()
        .map(|block| {
            block
                .protocol_state_proof
                .base64
                .clone()
                .ok_or_else(|| MinaDaemonError::MalformedResponse("Missing protocol state proof base64".into()))
                .and_then(|base64| {
                    BASE64_URL_SAFE
                        .decode(base64)
                        .map_err(|err| MinaDaemonError::MalformedResponse(format!("Couldn't decode state proof from base64: {err}")))
                })
                .and_then(|binprot| {
                    MinaBaseProofStableV2::binprot_read(&mut binprot.as_slice())
                        .map_err(|err| MinaDaemonError::MalformedResponse(format!("Couldn't read state proof binprot: {err}")))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    info!("Queried {} blocks for state proof candidate chain", chain_state_hashes.len());

    Ok((
        chain_state_hashes,
        chain_ledger_hashes,
        chain_proofs,
    ))
}

/// Formats a zkApp account's fields as a human-readable string for debug logging.
fn format_zkapp_readable(zkapp: &MinaBaseZkappAccountStableV2) -> String {
    let mut output = String::new();

    // app_state - 8 field elements
    output.push_str("  app_state:\n");
    for (i, state) in zkapp.app_state.0.0.iter().enumerate() {
        output.push_str(&format!("    [{}]: 0x{}\n", i, hex::encode(state.as_ref())));
    }

    // verification_key
    output.push_str(&format!("  verification_key: {}\n",
        if zkapp.verification_key.is_some() { "Some(...)" } else { "None" }));

    // zkapp_version
    output.push_str(&format!("  zkapp_version: {:?}\n", zkapp.zkapp_version));

    // action_state - 5 field elements
    output.push_str("  action_state:\n");
    for (i, state) in zkapp.action_state.iter().enumerate() {
        output.push_str(&format!("    [{}]: 0x{}\n", i, hex::encode(state.as_ref())));
    }

    // last_action_slot
    output.push_str(&format!("  last_action_slot: {:?}\n", zkapp.last_action_slot));

    // proved_state
    output.push_str(&format!("  proved_state: {}\n", zkapp.proved_state));

    // zkapp_uri
    let uri: Result<String, _> = (&zkapp.zkapp_uri).try_into();
    output.push_str(&format!("  zkapp_uri: {:?}\n", uri.unwrap_or_default()));

    output
}

/// Queries the account state, merkle proof, and ledger hash for a given account at a
/// given block from the Mina daemon.
///
/// # Arguments
///
/// * `rpc_url` - The Mina daemon GraphQL endpoint URL.
/// * `state_hash` - The block state hash at which to query the account (as a string).
/// * `public_key` - The Mina public key of the account to query.
/// * `token_id` - The token ID to query the account under.
///
/// # Returns
///
/// A tuple of `(account, ledger_hash, merkle_path)` on success:
/// - `account`: the deserialized Mina account.
/// - `ledger_hash`: the snarked ledger hash at the given block, as an `Fp` field element.
/// - `merkle_path`: the merkle path from the ledger root to the account leaf.
async fn query_account(
    rpc_url: &str,
    state_hash: &str,
    public_key: &str,
    token_id: &str
) -> Result<(MinaAccount, Fp, Vec<MerkleNode>), MinaDaemonError> {
    info!(
        "Querying account[public_key:{public_key}, token_id:{token_id}], its merkle proof and ledger hash for state {state_hash}"
    );
    let client = reqwest::Client::new();


    let variables = account_query::Variables {
        state_hash: state_hash.to_owned(),
        account_infos: vec![account_query::AccountInput {
            public_key: public_key.to_owned(),
            token: Some(token_id.to_owned()),
        }],
    };


    let request_body = AccountQuery::build_query(variables);
    info!("Sending request to {}: {:?}", rpc_url, serde_json::to_string(&request_body).unwrap());

    let response = client
        .post(rpc_url)
        .json(&request_body)
        .send()
        .await?;

    let response_text = response.text().await.map_err(|e: reqwest::Error| {
        // .text() can fail due to body decoding (charset, content-encoding)
        // or due to the connection dropping mid-stream.
        if e.is_decode() {
            MinaDaemonError::MalformedResponse(format!("failed to read response body: {e}"))
        } else {
            MinaDaemonError::RpcUnreachable(format!("failed to read response body: {e}"))
        }
    })?;
    info!("Raw response body: {}", response_text);

    /*
    let response_text = include_str!("../accoun_query_response.json");
    info!("Raw response body: {}", response_text);
    */
    let response: graphql_client::Response<account_query::ResponseData> =
        serde_json::from_str(&response_text).map_err(|e| MinaDaemonError::MalformedResponse(format!("Failed to parse JSON: {}", e)))?;

    if let Some(errors) = response.errors {
        if !errors.is_empty() {
            let msgs: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
            return Err(MinaDaemonError::GraphQLError(msgs.join("; ")));
        }
    }
    let response = response
        .data
        .ok_or_else(|| MinaDaemonError::MalformedResponse("missing response data".into()))?;

    let membership = response
        .encoded_snarked_ledger_account_membership
        .first()
        .ok_or_else(|| MinaDaemonError::AccountNotFound("Failed to retrieve membership query field".into()))?;

    let account_bytes = BASE64_STANDARD
        .decode(&membership.account)
        .map_err(|err| MinaDaemonError::MalformedResponse(format!("Failed to decode account from base64: {err}")))?;

    info!("Decoded account bytes length: {}", account_bytes.len());
    info!("Decoded account bytes (hex): {}", hex::encode(&account_bytes));

    let account = MinaAccount::binprot_read(&mut account_bytes.as_slice())
        .map_err(|err| MinaDaemonError::MalformedResponse(format!("Failed to deserialize account binprot: {err}")))?;

    info!("=== Decoded MinaAccount ===");
    info!("Public Key: {:?}", account.public_key);
    info!("Token ID: {:?}", account.token_id);
    info!("Token Symbol: {:?}", account.token_symbol);
    info!("Balance: {:?}", account.balance);
    info!("Nonce: {:?}", account.nonce);
    info!("Delegate: {:?}", account.delegate);
    info!("Has zkApp: {}", account.zkapp.is_some());
    if let Some(ref zkapp) = account.zkapp {
        info!("=== zkApp Details (Human Readable) ===\n{}", format_zkapp_readable(zkapp));
        info!("=== End zkApp Details ===");
    }
    info!("=== End of MinaAccount ===");

    let ledger_hash = response
        .block
        .protocol_state
        .blockchain_state
        .snarked_ledger_hash
        .to_fp()
        .map_err(|_| MinaDaemonError::MalformedResponse("Failed to convert snarked_ledger_hash to field element".into()))?;

    let merkle_path = membership
        .merkle_path
        .iter()
        .map(|node| -> Result<MerkleNode, ()> {
            match (node.left.as_ref(), node.right.as_ref()) {
                (Some(fp_str), None) => Ok(MerkleNode::Left(Fp::from_str(fp_str)?)),
                (None, Some(fp_str)) => Ok(MerkleNode::Right(Fp::from_str(fp_str)?)),
                _ => unreachable!(),
            }
        })
        .collect::<Result<Vec<_>, ()>>()
        .map_err(|_| MinaDaemonError::MalformedResponse("Error deserializing merkle path nodes".into()))?;

    Ok((account, ledger_hash, merkle_path))
}

/// Queries the account membership proof for a given public key at a given block and
/// assembles a `MinaAccountProof` and `MinaAccountPubInputs` suitable for submission
/// to Aligned Layer.
///
/// # Arguments
///
/// * `public_key` - The Mina public key of the account to prove.
/// * `token_id` - The token ID to query the account under.
/// * `state_hash` - The block state hash at which to prove the account (as a string).
/// * `rpc_url` - The Mina daemon GraphQL endpoint URL.
///
/// # Returns
///
/// A tuple of `(MinaAccountProof, MinaAccountPubInputs)` on success. The proof contains
/// the merkle path and deserialized account. The public inputs contain the ledger hash
/// and ABI-encoded account for on-chain verification.
pub async fn get_mina_proof_of_account(
    public_key: &str,
    token_id: &str,
    state_hash: &str,
    rpc_url: &str,
) -> Result<(MinaAccountProof, MinaAccountPubInputs), MinaDaemonError> {
    let (account, ledger_hash, merkle_path) =
        query_account(rpc_url, state_hash, public_key, token_id).await?;

    let encoded_account = MinaAccountValidationExample::Account::try_from(&account)
        .map_err(|e| MinaDaemonError::SerializationError(e))?
        .abi_encode();

    info!(
        "Retrieved proof of account for ledger {}",
        LedgerHash::from_fp(ledger_hash)
    );

    Ok((
        MinaAccountProof {
            merkle_path,
            account,
        },
        MinaAccountPubInputs {
            ledger_hash,
            encoded_account,
        },
    ))
}

// --- Proof assembly functions ---

/// Replacement for `get_mina_proof_of_state` that deterministically targets the exact
/// 16-block window starting at an arbitrary group finalization block height (F_g).
///
/// Uses `StateProofBlockQuery` (`block(height:)`) which is deterministic -- it always
/// returns the canonical block at the requested height regardless of where the tip is.
/// Each of the 16 heights in the proof window `[F_g, F_g + 15]` is queried individually,
/// so there is no race condition and no need to overfetch.
///
/// Costs 32 round trips (16 `block(height:)` + 16 `query_state`) vs 18 for
/// `get_mina_proof_of_state`, but avoids the race condition and wasteful overfetching
/// that the bestChain approach suffers from. See `get_mina_proof_of_state` for details.
///
/// # Limitation
///
/// Still subject to `MINA_DAEMON_MAX_QUERYABLE_BLOCKS` (290) -- if F_g is older than
/// 290 blocks from the tip, the daemon cannot serve the request.
pub async fn less_insane_get_mina_proof_of_state(
    rpc_url: &str,
    is_devnet: bool,
    group_finalization_block_height: u64,
    group_finalization_state_hash: &str,
) -> Result<(MinaStateProof, MinaStatePubInputs), MinaDaemonError> {
    let client = reqwest::Client::new();

    // We need 32 round trips total: 16 block(height:) + 16 query_state. Each pair
    // (block + state) is sequential, but all 16 pairs run concurrently. Concurrency
    // is capped at 4 to avoid overwhelming the Mina daemon -- at higher concurrency
    // the daemon returns empty response bodies for some requests.
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let block_futures = (0..BRIDGE_TRANSITION_FRONTIER_LEN).map(|i| {
        let semaphore = semaphore.clone();
        let height = group_finalization_block_height + i as u64;
        let client = client.clone();
        let rpc_url = rpc_url.to_owned();
        async move {
            let _permit = semaphore.acquire().await
                .map_err(|_| MinaDaemonError::BadRequest("semaphore closed".into()))?;
            // First call in the pair: block(height:) for state hash, ledger hash, proof.
            let variables = state_proof_block_query::Variables {
                height: height as i64,
            };
            let response = post_graphql::<StateProofBlockQuery, _>(&client, &rpc_url, variables).await?;
            if let Some(errors) = response.errors {
                if !errors.is_empty() {
                    let msgs: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
                    return Err(MinaDaemonError::GraphQLError(msgs.join("; ")));
                }
            }
            let block = response
                .data
                .ok_or_else(|| MinaDaemonError::MalformedResponse(format!("block(height:{height}): missing response data")))?
                .block;

            let state_hash: StateHash = block.state_hash.clone();
            let ledger_hash: LedgerHash = block
                .protocol_state
                .blockchain_state
                .snarked_ledger_hash
                .clone();
            let proof = block
                .protocol_state_proof
                .base64
                .ok_or_else(|| MinaDaemonError::MalformedResponse(format!("block(height:{height}): missing protocol state proof base64")))
                .and_then(|b64| {
                    BASE64_URL_SAFE
                        .decode(b64)
                        .map_err(|err| MinaDaemonError::MalformedResponse(format!("block(height:{height}): base64 decode: {err}")))
                })
                .and_then(|binprot| {
                    MinaBaseProofStableV2::binprot_read(&mut binprot.as_slice())
                        .map_err(|err| MinaDaemonError::MalformedResponse(format!("block(height:{height}): binprot read: {err}")))
                })?;

            // Second call in the pair: query_state for the full protocol state, using
            // the state hash we just got from block(height:). This chains sequentially
            // within this future but runs concurrently with the other 15 pairs.
            let protocol_state = query_state(&rpc_url, &state_hash).await?;

            Ok::<_, MinaDaemonError>((state_hash, ledger_hash, proof, protocol_state))
        }
    });

    let results: Vec<(StateHash, LedgerHash, MinaBaseProofStableV2, MinaStateProtocolStateValueStableV2)> =
        join_all(block_futures)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

    // Verify F_g's state hash matches what the caller expects.
    let (ref fg_state_hash, _, _, _) = results[0];
    if fg_state_hash.to_string() != group_finalization_state_hash {
        return Err(MinaDaemonError::StateHashMismatch(format!(
            "State hash mismatch at F_g (height {group_finalization_block_height}): \
             expected {group_finalization_state_hash}, got {fg_state_hash}"
        )));
    }

    // Unzip into the fixed-size arrays and vecs needed by the proof structs.
    let mut state_hashes_vec = Vec::with_capacity(BRIDGE_TRANSITION_FRONTIER_LEN);
    let mut ledger_hashes_vec = Vec::with_capacity(BRIDGE_TRANSITION_FRONTIER_LEN);
    let mut proofs_vec = Vec::with_capacity(BRIDGE_TRANSITION_FRONTIER_LEN);
    let mut states_vec = Vec::with_capacity(BRIDGE_TRANSITION_FRONTIER_LEN);
    for (sh, lh, proof, state) in results {
        state_hashes_vec.push(sh);
        ledger_hashes_vec.push(lh);
        proofs_vec.push(proof);
        states_vec.push(state);
    }

    let candidate_chain_state_hashes: [StateHash; BRIDGE_TRANSITION_FRONTIER_LEN] =
        state_hashes_vec.try_into()
            .map_err(|_| MinaDaemonError::MalformedResponse("Failed to convert state hashes to fixed array".into()))?;
    let candidate_chain_ledger_hashes: [LedgerHash; BRIDGE_TRANSITION_FRONTIER_LEN] =
        ledger_hashes_vec.try_into()
            .map_err(|_| MinaDaemonError::MalformedResponse("Failed to convert ledger hashes to fixed array".into()))?;
    let candidate_tip_proof = proofs_vec.pop()
        .ok_or_else(|| MinaDaemonError::MalformedResponse("Empty proofs vec".into()))?;
    let group_finalization_state = states_vec[0].clone();

    let candidate_tip_state_hash = candidate_chain_state_hashes
        .last()
        .ok_or_else(|| MinaDaemonError::MalformedResponse("Missing candidate tip state hash".into()))?;
    info!("Queried Mina candidate chain with tip {candidate_tip_state_hash} and F_g at {group_finalization_state_hash}");

    let mina_state_proof = MinaStateProof {
        candidate_tip_proof,
        candidate_chain_states: states_vec,
        bridge_tip_state: group_finalization_state,
    };
    let mina_state_pub_inputs = MinaStatePubInputs {
        is_state_proof_from_devnet: is_devnet,
        bridge_tip_state_hash: candidate_chain_state_hashes[0].clone(),
        candidate_chain_state_hashes,
        candidate_chain_ledger_hashes,
    };

    info!("Serializing state proof and public inputs for local verification...");
    let proof_bytes = bincode::serialize(&mina_state_proof)
        .map_err(|e| MinaDaemonError::SerializationError(format!("Failed to serialize state proof: {e}")))?;
    let pub_input_bytes = bincode::serialize(&mina_state_pub_inputs)
        .map_err(|e| MinaDaemonError::SerializationError(format!("Failed to serialize public inputs: {e}")))?;
    // FIXME i really dont like have a private copy of this here
    // we should import this from some 3rd party repo and not have
    // a copy pasted version of our own
    info!("Running local Pickles verification (this can take minutes)...");
    let start = std::time::Instant::now();
    // TODO: re-enable local Pickles verification once it is stable
    if !verify_mina_state(&proof_bytes, &pub_input_bytes) {
         return Err(MinaDaemonError::LocalVerificationFailed("Mina state proof verification failed".into()));
    }
    info!("Mina state proof verification passed in {:.1}s", start.elapsed().as_secs_f64());

    Ok((mina_state_proof, mina_state_pub_inputs))
}

/// Assembles a `MinaStateProof` and `MinaStatePubInputs` for the 16-block window starting
/// at an arbitrary group finalization block height (F_g).
///
/// # How it works
///
/// 1. Queries the frontier for the live tip height (1 round trip).
/// 2. Requests `distance_to_F_g + 16 + BEST_CHAIN_QUERY_BUFFER` blocks from
///    `query_state_proof_candidate_chain` (1 round trip).
/// 3. Searches the entire result set by state hash to locate F_g -- we cannot trust the
///    index because the tip may have shifted since step 1. Slices the 16-block window
///    `[F_g, F_g + 15]`.
/// 4. Fetches full protocol states for those 16 blocks via `query_state` in parallel
///    (16 round trips via `join_all`).
///
/// Total: 18 round trips (1 frontier + 1 bestChain + 16 query_state).
///
/// # Why this is bad
///
/// Because `BestChainQuery` cannot target a specific block height or hash range (see its
/// docstring), when F_g is far behind the tip we must request up to
/// `MINA_DAEMON_MAX_QUERYABLE_BLOCKS` (290) blocks just to ensure F_g is included, then
/// throw away all but 16. It is actually worse than this because the tip can advance at
/// any time between the frontier query (step 1) and the bestChain query (step 2), eating
/// into the budget. We must add `BEST_CHAIN_QUERY_BUFFER` extra blocks to absorb this
/// race, meaning we request distance + 16 + buffer blocks total. When the tip advances the
/// requirement for having the `BEST_CHAIN_QUERY_BUFFER` means F_g can fall off the end of
/// the result prematurely
///
/// Because the bestChain response does not include block heights, we have to do a linear
/// state hash search through the entire result set to find F_g -- an unacceptable consequence
/// of the API not supporting targeted queries.
///
/// `less_insane_get_mina_proof_of_state` avoids all of these problems by using per-height
/// `block(height:)` queries instead, at the cost of more round trips (32 vs 18).
///
/// This approach is inherited from `query_candidate_chain_0` in `mina.rs`, which hardcodes
/// `max_length` to 16 and assumes the tip IS the proof window. That assumption was valid in
/// the old architecture where the bridge tip was read from the Ethereum contract. It breaks
/// when targeting an arbitrary F_g that may be hundreds of blocks behind the tip.
#[allow(deprecated)]
pub async fn get_mina_proof_of_state(
    rpc_url: &str,
    is_devnet: bool,
    group_finalization_block_height: u64,
    group_finalization_state_hash: &str,
) -> Result<(MinaStateProof, MinaStatePubInputs), MinaDaemonError> {
    let (_, live_tip_height) = query_frontier(rpc_url, 1)
        .await?
        .into_iter().next()
        .ok_or_else(|| MinaDaemonError::MalformedResponse("Empty frontier response".into()))?;

    // Bail early if the total number of blocks we would need to request from the daemon
    // (distance to F_g + the 16-block proof window + buffer) exceeds its queryable limit.
    let group_finalization_distance_from_tip = live_tip_height as usize - group_finalization_block_height as usize;
    if group_finalization_distance_from_tip + BRIDGE_TRANSITION_FRONTIER_LEN + BEST_CHAIN_QUERY_BUFFER > MINA_DAEMON_MAX_QUERYABLE_BLOCKS {
        return Err(MinaDaemonError::BlockTooOld(format!(
            "group_finalization_block_height {group_finalization_block_height} is too old: \
             {group_finalization_distance_from_tip} blocks behind tip {live_tip_height}, exceeds daemon limit of {MINA_DAEMON_MAX_QUERYABLE_BLOCKS}"
        )));
    }

    let best_chain_max_length = group_finalization_distance_from_tip
        + BRIDGE_TRANSITION_FRONTIER_LEN
        + BEST_CHAIN_QUERY_BUFFER;

    // Fetch the oversized chain from the daemon.
    let (
        chain_state_hashes,
        chain_ledger_hashes,
        chain_proofs,
    ) = query_state_proof_candidate_chain(rpc_url, best_chain_max_length)
        .await?;

    // Locate group_finalization_state_hash in the returned set to find the true index
    // of F_g, then take the 16-block window starting from that index.
    let fg_index = chain_state_hashes
        .iter()
        .position(|h| h.to_string() == group_finalization_state_hash)
        .ok_or_else(|| MinaDaemonError::StateHashMismatch(format!(
            "group_finalization_state_hash {group_finalization_state_hash} not found in \
             bestChain response of {best_chain_max_length} blocks"
        )))?;
    let fg_end = fg_index + BRIDGE_TRANSITION_FRONTIER_LEN;
    if fg_end > chain_state_hashes.len() {
        return Err(MinaDaemonError::MalformedResponse(format!(
            "Not enough blocks after F_g: need {BRIDGE_TRANSITION_FRONTIER_LEN} but only {} available",
            chain_state_hashes.len() - fg_index
        )));
    }

    // Slice the 16-block window: F_g is the oldest, F_g + 15 is the tip we prove.
    let candidate_chain_state_hashes: [StateHash; BRIDGE_TRANSITION_FRONTIER_LEN] =
        chain_state_hashes[fg_index..fg_end].to_vec().try_into()
            .map_err(|_| MinaDaemonError::MalformedResponse("Failed to convert state hashes to fixed array".into()))?;
    let candidate_chain_ledger_hashes: [LedgerHash; BRIDGE_TRANSITION_FRONTIER_LEN] =
        chain_ledger_hashes[fg_index..fg_end].to_vec().try_into()
            .map_err(|_| MinaDaemonError::MalformedResponse("Failed to convert ledger hashes to fixed array".into()))?;
    let candidate_tip_proof = chain_proofs[fg_end - 1].clone();

    // Fetch full protocol states for the 16-block window (needed for consensus checks).
    // Concurrency capped at 4 to avoid reqwest HTTP/2 multiplexing bug where the daemon
    // returns empty response bodies under high concurrency.
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let candidate_chain_states: Vec<MinaStateProtocolStateValueStableV2> = join_all(
        candidate_chain_state_hashes
            .iter()
            .map(|state_hash| {
                let semaphore = semaphore.clone();
                let rpc_url = rpc_url.to_owned();
                async move {
                    let _permit = semaphore.acquire().await
                        .map_err(|_| MinaDaemonError::BadRequest("semaphore closed".into()))?;
                    query_state(&rpc_url, state_hash).await
                }
            }),
    )
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;

    // F_g's protocol state is the first in the window -- no separate query needed.
    let group_finalization_state = candidate_chain_states[0].clone();

    let candidate_tip_state_hash = candidate_chain_state_hashes
        .last()
        .ok_or_else(|| MinaDaemonError::MalformedResponse("Missing candidate tip state hash".into()))?;
    info!("Queried Mina candidate chain with tip {candidate_tip_state_hash} and F_g at {group_finalization_state_hash}");

    let mina_state_proof = MinaStateProof {
        candidate_tip_proof,
        candidate_chain_states,
        bridge_tip_state: group_finalization_state,
    };
    let mina_state_pub_inputs = MinaStatePubInputs {
        is_state_proof_from_devnet: is_devnet,
        bridge_tip_state_hash: candidate_chain_state_hashes[0].clone(),
        candidate_chain_state_hashes,
        candidate_chain_ledger_hashes,
    };

    info!("Serializing state proof and public inputs for local verification...");
    let proof_bytes = bincode::serialize(&mina_state_proof)
        .map_err(|e| MinaDaemonError::SerializationError(format!("Failed to serialize state proof: {e}")))?;
    let pub_input_bytes = bincode::serialize(&mina_state_pub_inputs)
        .map_err(|e| MinaDaemonError::SerializationError(format!("Failed to serialize public inputs: {e}")))?;
    // FIXME i really dont like have a private copy of this here
    // we should import this from some 3rd party repo and not have
    // a copy pasted version of our own
    info!("Running local Pickles verification (this can take minutes)...");
    let start = std::time::Instant::now();
    // TODO: re-enable local Pickles verification once it is stable
    if !verify_mina_state(&proof_bytes, &pub_input_bytes) {
        return Err(MinaDaemonError::LocalVerificationFailed("Mina state proof verification failed".into()));
    }
    info!("Mina state proof verification passed in {:.1}s", start.elapsed().as_secs_f64());

    Ok((mina_state_proof, mina_state_pub_inputs))
}
