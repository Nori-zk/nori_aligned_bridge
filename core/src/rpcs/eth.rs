use alloy::primitives::Address;
use reqwest::Url;

use crate::error::Error;
use crate::eth::{get_bridge_tip_hash, SolStateHash};
use crate::utils::wallet::WalletData;

pub struct EthRPC {
    eth_rpc_url: Url,
    nori_mina_state_settlement_address: Address,
    nori_token_bridge_address: Address,
    wallet: WalletData,
}

impl EthRPC {
    pub fn from_env(wallet: WalletData) -> Result<Self, Error> {
        let eth_rpc_url = std::env::var("ETH_RPC_URL")
            .map_err(|e| Error(format!("ETH_RPC_URL: {e}")))?
            .trim()
            .parse::<Url>()
            .map_err(|e| Error(format!("invalid ETH_RPC_URL: {e}")))?;
        let nori_mina_state_settlement_address =
            std::env::var("NORI_ETH_MINA_STATE_SETTLEMENT_ADDRESS")
                .map_err(|e| Error(format!("NORI_ETH_MINA_STATE_SETTLEMENT_ADDRESS: {e}")))?
                .trim()
                .parse::<Address>()
                .map_err(|e| {
                    Error(format!(
                        "invalid NORI_ETH_MINA_STATE_SETTLEMENT_ADDRESS: {e}"
                    ))
                })?;
        let nori_token_bridge_address = std::env::var("NORI_ETH_TOKEN_BRIDGE_ADDRESS")
            .map_err(|e| Error(format!("NORI_ETH_TOKEN_BRIDGE_ADDRESS: {e}")))?
            .trim()
            .parse::<Address>()
            .map_err(|e| Error(format!("invalid NORI_ETH_TOKEN_BRIDGE_ADDRESS: {e}")))?;
        Ok(Self {
            eth_rpc_url,
            nori_mina_state_settlement_address,
            nori_token_bridge_address,
            wallet,
        })
    }

    #[deprecated(note = "old architecture: continuous bridge-tip chaining not needed. New design proves state at group_finalization_block_height directly via block(height:) queries.")]
    pub async fn get_bridge_tip_hash(&self) -> Result<SolStateHash, Error> {
        get_bridge_tip_hash(
            &self.nori_mina_state_settlement_address.to_string(),
            self.eth_rpc_url.as_str(),
        )
        .await
        .map_err(Error)
    }
}
