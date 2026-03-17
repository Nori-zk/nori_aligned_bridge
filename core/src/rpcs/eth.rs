use aligned_sdk::common::types::AlignedVerificationData;
use alloy::primitives::{Address, TxHash, U256};
use alloy::rpc::types::TransactionReceipt;
use mina_p2p_messages::v2::StateHash;
use reqwest::Url;

use crate::error::Error;
use crate::eth_2;
use crate::proof::{account_proof::MinaAccountPubInputs, state_proof::MinaStatePubInputs};
use crate::rpcs::errors::EthError;
use crate::utils::constants::BRIDGE_TRANSITION_FRONTIER_LEN;
use crate::utils::wallet::WalletData;

pub struct EthRPC {
    eth_rpc_url: Url,
    nori_mina_state_settlement_address: Address,
    nori_token_bridge_address: Address,
    batcher_payment_service: Address,
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
        let batcher_payment_service = std::env::var("ALIGNED_BATCHER_PAYMENT_SERVICE")
            .map_err(|e| Error(format!("ALIGNED_BATCHER_PAYMENT_SERVICE: {e}")))?
            .trim()
            .parse::<Address>()
            .map_err(|e| Error(format!("invalid ALIGNED_BATCHER_PAYMENT_SERVICE: {e}")))?;
        Ok(Self {
            eth_rpc_url,
            nori_mina_state_settlement_address,
            nori_token_bridge_address,
            batcher_payment_service,
            wallet,
        })
    }

    /// Sends an `updateChain` transaction to `MinaStateSettlement.sol`.
    /// Returns the tx hash immediately without waiting for confirmation.
    pub async fn send_update_chain(
        &self,
        verification_data: AlignedVerificationData,
        pub_input: &MinaStatePubInputs,
    ) -> Result<TxHash, EthError> {
        eth_2::send_update_chain(
            verification_data,
            pub_input,
            &self.eth_rpc_url,
            self.wallet.clone(),
            self.nori_mina_state_settlement_address,
            self.batcher_payment_service,
        )
        .await
    }

    /// Sends an `unlockTokens` transaction to `NoriTokenBridge.sol`.
    /// Returns the tx hash immediately without waiting for confirmation.
    pub async fn send_unlock_tokens(
        &self,
        verification_data: AlignedVerificationData,
        pub_input: &MinaAccountPubInputs,
        to_unlock_amount: U256,
    ) -> Result<TxHash, EthError> {
        eth_2::send_unlock_tokens(
            verification_data,
            pub_input,
            to_unlock_amount,
            &self.eth_rpc_url,
            self.wallet.clone(),
            self.nori_token_bridge_address,
            self.batcher_payment_service,
        )
        .await
    }

    /// Checks whether a transaction has been mined.
    /// Returns `Ok(Some(receipt))` if mined, `Ok(None)` if still pending.
    pub async fn get_tx_receipt(
        &self,
        tx_hash: TxHash,
    ) -> Result<Option<TransactionReceipt>, EthError> {
        eth_2::get_tx_receipt(&self.eth_rpc_url, tx_hash).await
    }

    /// Reads the chain state hashes from `MinaStateSettlement.sol`.
    pub async fn get_chain_state_hashes(
        &self,
    ) -> Result<[StateHash; BRIDGE_TRANSITION_FRONTIER_LEN], EthError> {
        eth_2::get_chain_state_hashes(
            &self.eth_rpc_url,
            self.nori_mina_state_settlement_address,
        )
        .await
    }
}
