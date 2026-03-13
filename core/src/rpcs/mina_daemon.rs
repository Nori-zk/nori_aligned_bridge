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

    fn is_devnet(&self) -> bool {
        self.network == MinaNetwork::Devnet
    }

    pub async fn query_frontier(&self, max_length: usize) -> Result<Vec<(StateHash, u64)>, Error> {
        mina_daemon::query_frontier(self.rpc_url.as_str(), max_length)
            .await
            .map_err(Error)
    }

    pub async fn query_state(
        &self,
        state_hash: &StateHash,
    ) -> Result<MinaStateProtocolStateValueStableV2, Error> {
        mina_daemon::query_state(self.rpc_url.as_str(), state_hash)
            .await
            .map_err(Error)
    }

    pub async fn get_mina_proof_of_state(
        &self,
        group_finalization_block_height: u64,
        group_finalization_state_hash: &str,
    ) -> Result<(MinaStateProof, MinaStatePubInputs), Error> {
        mina_daemon::less_insane_get_mina_proof_of_state(
            self.rpc_url.as_str(),
            self.is_devnet(),
            group_finalization_block_height,
            group_finalization_state_hash,
        )
        .await
        .map_err(Error)
    }

    pub async fn get_mina_proof_of_account(
        &self,
        public_key: &str,
        state_hash: &str,
    ) -> Result<(MinaAccountProof, MinaAccountPubInputs), Error> {
        mina_daemon::get_mina_proof_of_account(
            public_key,
            &self.nori_mina_token_bridge_token_id,
            state_hash,
            self.rpc_url.as_str(),
        )
        .await
        .map_err(Error)
    }
}
