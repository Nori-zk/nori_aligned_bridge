// NOTE: Mixed parameter types across methods -- some take &StateHash (mina_p2p_messages type),
// others take &str. This matches what callers naturally have: get_mina_proof_of_state receives
// a StateHash from EthRPC.get_bridge_tip_hash, while get_mina_proof_of_account receives strings
// from record fields. Unifying would force conversions at every call site.

use mina_p2p_messages::v2::{MinaStateProtocolStateValueStableV2, StateHash};
use reqwest::Url;
use std::env;

use crate::error::Error;
use crate::mina_daemon;
use crate::proof::account_proof::{MinaAccountProof, MinaAccountPubInputs};
use crate::proof::state_proof::{MinaStatePubInputs, MinaStateProof};
use super::errors::MinaDaemonError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MinaNetwork {
    Mainnet,
    Devnet,
}

pub struct MinaDaemonRPC {
    rpc_url: Url,
    network: MinaNetwork,
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
        Ok(Self { rpc_url, network })
    }

    fn is_devnet(&self) -> bool {
        self.network == MinaNetwork::Devnet
    }

    pub async fn query_frontier(&self, max_length: usize) -> Result<Vec<(StateHash, u64)>, MinaDaemonError> {
        mina_daemon::query_frontier(self.rpc_url.as_str(), max_length)
            .await
    }

    pub async fn query_state(
        &self,
        state_hash: &StateHash,
    ) -> Result<MinaStateProtocolStateValueStableV2, MinaDaemonError> {
        mina_daemon::query_state(self.rpc_url.as_str(), state_hash)
            .await
    }

    /// Best-effort query for the canonical state hash at `height`.
    ///
    /// See [`mina_daemon::best_effort_canonical_state_hash_at_height`] for full
    /// documentation on what "best-effort" means, why this is not a guarantee,
    /// and what the error variants signify.
    pub async fn best_effort_canonical_state_hash_at_height(
        &self,
        height: u64,
    ) -> Result<String, MinaDaemonError> {
        mina_daemon::best_effort_canonical_state_hash_at_height(self.rpc_url.as_str(), height)
            .await
    }

    pub async fn get_mina_proof_of_state(
        &self,
        group_finalization_block_height: u64,
        group_finalization_state_hash: &str,
    ) -> Result<(MinaStateProof, MinaStatePubInputs), MinaDaemonError> {
        mina_daemon::less_insane_get_mina_proof_of_state(
            self.rpc_url.as_str(),
            self.is_devnet(),
            group_finalization_block_height,
            group_finalization_state_hash,
        )
        .await
    }

    /// Old bestChain-based state proof generation. Uses `query_state_proof_candidate_chain`
    /// instead of per-height `block(height:)` queries.
    #[allow(deprecated)]
    pub async fn get_mina_proof_of_state_old(
        &self,
        group_finalization_block_height: u64,
        group_finalization_state_hash: &str,
    ) -> Result<(MinaStateProof, MinaStatePubInputs), MinaDaemonError> {
        mina_daemon::get_mina_proof_of_state(
            self.rpc_url.as_str(),
            self.is_devnet(),
            group_finalization_block_height,
            group_finalization_state_hash,
        )
        .await
    }

    pub async fn get_mina_proof_of_account(
        &self,
        public_key: &str,
        token_id: &str,
        state_hash: &str,
    ) -> Result<(MinaAccountProof, MinaAccountPubInputs), MinaDaemonError> {
        mina_daemon::get_mina_proof_of_account(
            public_key,
            token_id,
            state_hash,
            self.rpc_url.as_str(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Queries the daemon frontier for the tip, then generates a state proof
    /// for that block. Requires MINA_RPC_NETWORK_URL and MINA_NETWORK env vars
    /// (load .env.local or services/.env.local before running).
    ///
    /// Queries the frontier tip, then fetches the protocol state for that block.
    /// Smoke test that the daemon is reachable and returns valid data for a
    /// single query_state call.
    ///
    /// cargo test -p mina_bridge_core --lib rpcs::mina_daemon::tests::test_query_state -- --nocapture
    #[tokio::test]
    async fn test_query_state() {
        dotenv::from_filename("services/.env.local").ok();
        dotenv::from_filename(".env.local").ok();

        let rpc = MinaDaemonRPC::from_env().expect("MinaDaemonRPC::from_env");

        let frontier = rpc.query_frontier(1).await.expect("query_frontier");
        let (tip_state_hash, tip_height) = frontier.into_iter().next().expect("empty frontier");
        println!("Tip: height={tip_height}, state_hash={tip_state_hash}");

        let state = rpc.query_state(&tip_state_hash).await.expect("query_state");
        println!("Protocol state retrieved for tip {tip_state_hash}");
        // Sanity: the state's previous_state_hash should be a non-empty string.
        let prev = state.previous_state_hash.to_string();
        assert!(!prev.is_empty(), "previous_state_hash should be non-empty");
    }

    /// Queries the daemon frontier for the tip, then generates a full state
    /// proof for the 16-block window starting at that block.
    ///
    /// cargo test -p mina_bridge_core --lib rpcs::mina_daemon::tests::test_get_mina_proof_of_state -- --nocapture
    #[tokio::test]
    async fn test_get_mina_proof_of_state() {
        dotenv::from_filename("services/.env.local").ok();
        dotenv::from_filename(".env.local").ok();
        env_logger::try_init().ok();

        let rpc = MinaDaemonRPC::from_env().expect("MinaDaemonRPC::from_env");

        use crate::utils::constants::BRIDGE_TRANSITION_FRONTIER_LEN;

        // Query the full frontier so we can pick F_g = tip - 15 (the oldest
        // block in a 16-block proof window ending at the tip).
        let frontier = rpc
            .query_frontier(BRIDGE_TRANSITION_FRONTIER_LEN)
            .await
            .expect("query_frontier");
        let (fg_state_hash, fg_height) = frontier.first().expect("empty frontier").clone();
        let (tip_state_hash, tip_height) = frontier.last().expect("empty frontier").clone();
        println!("Tip: height={tip_height}, state_hash={tip_state_hash}");
        println!("F_g: height={fg_height}, state_hash={fg_state_hash}");

        // First, verify each query_state works sequentially.
        for (sh, h) in &frontier {
            println!("Sequential query_state for height {h}...");
            rpc.query_state(sh).await.expect("sequential query_state");
        }
        println!("All 16 sequential query_state calls passed.");

        // Generate a state proof for the 16-block window [F_g, tip].
        let (proof, pub_inputs) = rpc
            .get_mina_proof_of_state(fg_height, &fg_state_hash.to_string())
            .await
            .expect("get_mina_proof_of_state");

        println!(
            "State proof generated: {} chain states, bridge_tip_state_hash={}",
            proof.candidate_chain_states.len(),
            pub_inputs.bridge_tip_state_hash,
        );
        assert_eq!(pub_inputs.bridge_tip_state_hash.to_string(), fg_state_hash.to_string());
    }

    /// Same as test_get_mina_proof_of_state but uses the old bestChain-based
    /// `mina_daemon::get_mina_proof_of_state` (not less_insane).
    ///
    /// cargo test -p mina_bridge_core --lib rpcs::mina_daemon::tests::test_old_get_mina_proof_of_state -- --nocapture
    #[tokio::test]
    async fn test_old_get_mina_proof_of_state() {
        dotenv::from_filename("services/.env.local").ok();
        dotenv::from_filename(".env.local").ok();
        env_logger::try_init().ok();

        let rpc = MinaDaemonRPC::from_env().expect("MinaDaemonRPC::from_env");

        use crate::utils::constants::BRIDGE_TRANSITION_FRONTIER_LEN;

        let frontier = rpc
            .query_frontier(BRIDGE_TRANSITION_FRONTIER_LEN)
            .await
            .expect("query_frontier");
        let (fg_state_hash, fg_height) = frontier.first().expect("empty frontier").clone();
        let (tip_state_hash, tip_height) = frontier.last().expect("empty frontier").clone();
        println!("Tip: height={tip_height}, state_hash={tip_state_hash}");
        println!("F_g: height={fg_height}, state_hash={fg_state_hash}");

        // Call the old bestChain-based function via the RPC wrapper.
        let (proof, pub_inputs) = rpc
            .get_mina_proof_of_state_old(fg_height, &fg_state_hash.to_string())
            .await
            .expect("old get_mina_proof_of_state");

        println!(
            "Old path proof generated: {} chain states, bridge_tip_state_hash={}",
            proof.candidate_chain_states.len(),
            pub_inputs.bridge_tip_state_hash,
        );
        assert_eq!(pub_inputs.bridge_tip_state_hash.to_string(), fg_state_hash.to_string());
    }
}
