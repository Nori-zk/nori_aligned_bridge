// NOTE: Mixed parameter types across methods — some take &StateHash (mina_p2p_messages type),
// others take &str. This matches what callers naturally have: get_mina_proof_of_state receives
// a StateHash from EthRPC.get_bridge_tip_hash, while get_mina_proof_of_account receives strings
// from record fields. Unifying would force conversions at every call site.

use futures::future::join_all;
use log::info;
use mina_p2p_messages::v2::{
    LedgerHash, MinaBaseProofStableV2, MinaStateProtocolStateValueStableV2, StateHash,
};
use mina_state_verifier::verify_mina_state;
use reqwest::Url;
use std::env;

use crate::error::Error;
use crate::mina::{
    get_mina_proof_of_account, query_candidate_chain_0, query_frontier, query_state, query_state_proof_candidate_chain,
};
use crate::proof::account_proof::{MinaAccountProof, MinaAccountPubInputs};
use crate::proof::state_proof::{MinaStatePubInputs, MinaStateProof};
use crate::utils::constants::BRIDGE_TRANSITION_FRONTIER_LEN;

/// Extra blocks to request beyond the calculated max_length when querying bestChain,
/// to account for the tip advancing between the frontier query and the bestChain query.
const BEST_CHAIN_QUERY_BUFFER: usize = 4;

/// Maximum number of recent blocks the Mina daemon node supports querying state info for.
const MINA_DAEMON_MAX_QUERYABLE_BLOCKS: usize = 290;

pub type MinaCandidateChainData = (
    Vec<MinaStateProtocolStateValueStableV2>,
    [StateHash; BRIDGE_TRANSITION_FRONTIER_LEN],
    [LedgerHash; BRIDGE_TRANSITION_FRONTIER_LEN],
    MinaBaseProofStableV2,
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MinaNetwork {
    Mainnet,
    Devnet,
}

pub struct MinaDaemonRPC {
    rpc_url: Url,
    network: MinaNetwork,
    nori_mina_token_bridge_token_id: String,
}

impl MinaDaemonRPC {
    pub fn from_env() -> Result<Self, Error> {
        let rpc_url = env::var("MINA_RPC_NETWORK_URL")
            .map_err(|e| Error(format!("MINA_RPC_NETWORK_URL: {e}")))?
            .trim()
            .parse::<Url>()
            .map_err(|e| Error(format!("invalid MINA_RPC_NETWORK_URL: {e}")))?;
        let network_str = env::var("MINA_NETWORK")
            .map_err(|e| Error(format!("MINA_NETWORK: {e}")))?;
        let network = match network_str.trim() {
            "mainnet" => MinaNetwork::Mainnet,
            "devnet" => MinaNetwork::Devnet,
            other => return Err(Error(format!(
                "invalid MINA_NETWORK: '{other}', expected 'mainnet' or 'devnet'"
            ))),
        };
        let nori_mina_token_bridge_token_id = env::var("NORI_MINA_TOKEN_BRIDGE_TOKEN_ID")
            .map_err(|e| Error(format!("NORI_MINA_TOKEN_BRIDGE_TOKEN_ID: {e}")))?
            .trim()
            .to_string();
        Ok(Self { rpc_url, network, nori_mina_token_bridge_token_id })
    }

    pub async fn query_candidate_chain(
        &self,
    ) -> Result<MinaCandidateChainData, Error> {
        query_candidate_chain_0(self.rpc_url.as_str())
            .await
            .map_err(Error)
    }

    pub async fn query_frontier(&self, max_length: usize) -> Result<Vec<(StateHash, u64)>, Error> {
        query_frontier(self.rpc_url.as_str(), max_length)
            .await
            .map_err(Error)
    }

    pub async fn query_state(
        &self,
        state_hash: &StateHash,
    ) -> Result<MinaStateProtocolStateValueStableV2, Error> {
        query_state(self.rpc_url.as_str(), state_hash)
            .await
            .map_err(Error)
    }

    pub async fn get_mina_proof_of_state(
        &self,
        group_finalization_block_height: u64,
        group_finalization_state_hash: &str,
    ) -> Result<(MinaStateProof, MinaStatePubInputs), Error> {
        let (_, live_tip_height) = self.query_frontier(1).await?
            .into_iter().next()
            .ok_or_else(|| Error("Empty frontier response".to_string()))?;

        // HACK: The Mina daemon API does not support querying bestChain by height range,
        // nor does it include the block height in the response.
        // We will request group_finalization_distance_from_tip + BRIDGE_TRANSITION_FRONTIER_LEN
        // + BEST_CHAIN_QUERY_BUFFER blocks from bestChain. We request more than the 16 we need
        // because the chain can shift at any time! The tip may advance between the frontier
        // query and the bestChain query. By fetching extra blocks we ensure the state hash at
        // group_finalization_block_height is still in the returned set, and we locate the true
        // 16-block window by searching for it by state hash rather than trusting the index.
        // This is fundamentally a race condition that cannot be eliminated without a better API.

        // The alternative is 16 block(height:) queries + 16 query_state calls (32 round trips),
        // which avoids the race condition entirely since block(height:) is deterministic (note the
        // MINA_DAEMON_MAX_QUERYABLE_BLOCKS is assumed to still apply to this parallel method), but
        // is far more expensive than a single bestChain call + 16 query_state calls (17 round trips)
        // and I cannot be bothered with this rubbish.
        //
        // This method gets the call down to 18 round trips: 1 frontier, 1 bestChain, and 16
        // query_state (because query_state_proof_candidate_chain does not return protocol states).
        // Better than 32, but only just.

        // Bail early if the total number of blocks we would need to request from the daemon
        // (distance to F_g + the 16-block proof window + buffer) exceeds its queryable limit.
        let group_finalization_distance_from_tip = live_tip_height as usize - group_finalization_block_height as usize;
        if group_finalization_distance_from_tip + BRIDGE_TRANSITION_FRONTIER_LEN + BEST_CHAIN_QUERY_BUFFER > MINA_DAEMON_MAX_QUERYABLE_BLOCKS {
            return Err(Error(format!(
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
        ) = query_state_proof_candidate_chain(self.rpc_url.as_str(), best_chain_max_length)
            .await
            .map_err(Error)?;

        // Locate group_finalization_state_hash in the returned set to find the true index
        // of F_g, then take the 16-block window starting from that index.
        let fg_index = chain_state_hashes
            .iter()
            .position(|h| h.to_string() == group_finalization_state_hash)
            .ok_or_else(|| Error(format!(
                "group_finalization_state_hash {group_finalization_state_hash} not found in \
                 bestChain response of {best_chain_max_length} blocks"
            )))?;
        let fg_end = fg_index + BRIDGE_TRANSITION_FRONTIER_LEN;
        if fg_end > chain_state_hashes.len() {
            return Err(Error(format!(
                "Not enough blocks after F_g: need {BRIDGE_TRANSITION_FRONTIER_LEN} but only {} available",
                chain_state_hashes.len() - fg_index
            )));
        }

        // Slice the 16-block window: F_g is the oldest, F_g + 15 is the tip we prove.
        let candidate_chain_state_hashes: [StateHash; BRIDGE_TRANSITION_FRONTIER_LEN] =
            chain_state_hashes[fg_index..fg_end].to_vec().try_into()
                .map_err(|_| Error("Failed to convert state hashes to fixed array".to_string()))?;
        let candidate_chain_ledger_hashes: [LedgerHash; BRIDGE_TRANSITION_FRONTIER_LEN] =
            chain_ledger_hashes[fg_index..fg_end].to_vec().try_into()
                .map_err(|_| Error("Failed to convert ledger hashes to fixed array".to_string()))?;
        let candidate_tip_proof = chain_proofs[fg_end - 1].clone();

        // Fetch full protocol states for the 16-block window (needed for consensus checks).
        let candidate_chain_states: Vec<MinaStateProtocolStateValueStableV2> = join_all(
            candidate_chain_state_hashes
                .iter()
                .map(|state_hash| query_state(self.rpc_url.as_str(), state_hash)),
        )
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(Error)?;

        // F_g's protocol state is the first in the window — no separate query needed.
        let group_finalization_state = candidate_chain_states[0].clone();

        let candidate_tip_state_hash = candidate_chain_state_hashes
            .last()
            .ok_or_else(|| Error("Missing candidate tip state hash".to_string()))?;
        info!("Queried Mina candidate chain with tip {candidate_tip_state_hash} and F_g at {group_finalization_state_hash}");

        let mina_state_proof = MinaStateProof {
            candidate_tip_proof,
            candidate_chain_states,
            bridge_tip_state: group_finalization_state,
        };
        let mina_state_pub_inputs = MinaStatePubInputs {
            is_state_proof_from_devnet: self.network == MinaNetwork::Devnet,
            bridge_tip_state_hash: candidate_chain_state_hashes[0].clone(),
            candidate_chain_state_hashes,
            candidate_chain_ledger_hashes,
        };

        let proof_bytes = bincode::serialize(&mina_state_proof)
            .map_err(|e| Error(format!("Failed to serialize state proof: {e}")))?;
        let pub_input_bytes = bincode::serialize(&mina_state_pub_inputs)
            .map_err(|e| Error(format!("Failed to serialize public inputs: {e}")))?;
        // FIXME i really dont like have a private copy of this here
        // we should import this from some 3rd party repo and not have
        // a copy pasted version of our own
        if !verify_mina_state(&proof_bytes, &pub_input_bytes) {
            return Err(Error("Mina state proof verification failed".to_string()));
        }
        info!("Mina state proof verification passed");

        Ok((mina_state_proof, mina_state_pub_inputs))
    }

    pub async fn get_mina_proof_of_account(
        &self,
        public_key: &str,
        state_hash: &str,
    ) -> Result<(MinaAccountProof, MinaAccountPubInputs), Error> {
        get_mina_proof_of_account(public_key, &self.nori_mina_token_bridge_token_id, state_hash, self.rpc_url.as_str())
            .await
            .map_err(Error)
    }
}
