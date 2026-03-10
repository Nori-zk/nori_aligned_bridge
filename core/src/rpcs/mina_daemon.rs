use super::error::Error;
use crate::{mina::{query_candidate_chain_0, query_frontier}, utils::constants::BRIDGE_TRANSITION_FRONTIER_LEN};
use mina_p2p_messages::v2::{
    LedgerHash, MinaBaseProofStableV2, MinaStateProtocolStateValueStableV2, StateHash,
};
use reqwest::Url;
use std::env;

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

    pub async fn query_candidate_chain(
        &self,
    ) -> Result<
        (
            Vec<MinaStateProtocolStateValueStableV2>,
            [StateHash; BRIDGE_TRANSITION_FRONTIER_LEN],
            [LedgerHash; BRIDGE_TRANSITION_FRONTIER_LEN],
            MinaBaseProofStableV2,
        ),
        Error,
    > {
        query_candidate_chain_0(self.rpc_url.as_str())
            .await
            .map_err(Error)
    }

    pub async fn query_frontier(&self, max_length: usize) -> Result<Vec<(StateHash, u32)>, Error> {
        query_frontier(self.rpc_url.as_str(), max_length)
            .await
            .map_err(Error)
    }
}
