// Ethereum primitives for bridge workers.
//
// Unlike eth.rs (which inline-awaits receipts), these functions decouple
// send from confirmation. The worker records the tx hash to MongoDB between
// send and receipt check, enabling crash recovery.

use aligned_sdk::common::types::{AlignedVerificationData, VerificationDataCommitment};
use alloy::{
    primitives::{Address, Bytes, FixedBytes, TxHash, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionReceipt,
    sol,
};
use log::info;
use mina_p2p_messages::v2::StateHash;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::{
    proof::{account_proof::MinaAccountPubInputs, state_proof::MinaStatePubInputs},
    rpcs::errors::{classify_contract_call_error, classify_receipt_failure, classify_transport_error, EthError},
    sol::serialization::SolSerialize,
    utils::{constants::BRIDGE_TRANSITION_FRONTIER_LEN, wallet::WalletData},
};

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    MinaStateSettlementExample,
    "abi/MinaStateSettlementExample.json"
);

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    MinaAccountValidationExample,
    "abi/MinaAccountValidationExample.json"
);

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    NoriTokenBridge,
    "abi/NoriTokenBridge.json"
);

sol! {
    #[sol(rpc)]
    interface IAlignedServiceManager {
        function batchesState(bytes32 batchIdentifier) external view returns (uint32 taskCreatedBlock, bool responded, uint256 respondToTaskFeeLimit);
    }
}

/// Wrapper of Mina Ledger hash for Ethereum
#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct SolStateHash(#[serde_as(as = "SolSerialize")] pub StateHash);

// Gas constants -- same as eth.rs.
const MAX_GAS_LIMIT_VALUE: u64 = 1_000_000;
const MAX_GAS_PRICE_GWEI: u64 = 300;
const GAS_ESTIMATE_MARGIN: u64 = 110;

/// Validates gas parameters against safety limits.
/// Returns the gas limit with a 10% safety margin.
async fn validate_gas_params<P: Provider>(provider: &P, estimated_gas: u64) -> Result<u64, EthError> {
    let current_gas_price = provider
        .get_gas_price()
        .await
        .map_err(classify_transport_error)?;

    let gas_price_gwei = current_gas_price / 1_000_000_000;

    if gas_price_gwei > MAX_GAS_PRICE_GWEI as u128 {
        return Err(EthError::GasSafetyLimit(format!(
            "gas price too high: {} gwei (max: {} gwei)",
            gas_price_gwei, MAX_GAS_PRICE_GWEI
        )));
    }

    let gas_with_margin = estimated_gas
        .checked_mul(GAS_ESTIMATE_MARGIN)
        .and_then(|v| v.checked_div(100))
        .ok_or_else(|| EthError::GasSafetyLimit("gas margin calculation overflow".into()))?;

    if gas_with_margin > MAX_GAS_LIMIT_VALUE {
        return Err(EthError::GasSafetyLimit(format!(
            "estimated gas too high: {} (max: {})",
            gas_with_margin, MAX_GAS_LIMIT_VALUE
        )));
    }

    Ok(gas_with_margin)
}

/// Sends an `updateChain` transaction to `MinaStateSettlement.sol`.
///
/// Estimates gas, validates gas params, sends the transaction, and returns
/// the tx hash immediately without waiting for confirmation.
pub async fn send_update_chain(
    verification_data: AlignedVerificationData,
    pub_input: &MinaStatePubInputs,
    eth_rpc_url: &Url,
    wallet: WalletData,
    contract_addr: Address,
    batcher_payment_service: Address,
) -> Result<TxHash, EthError> {
    let serialized_pub_input = bincode::serialize(pub_input)
        .map_err(|err| EthError::SerializationError(format!("failed to serialize public inputs: {err}")))?;

    let provider = ProviderBuilder::new()
        .wallet(wallet.wallet)
        .connect_http(eth_rpc_url.clone());

    let contract = MinaStateSettlementExample::new(contract_addr, provider.clone());

    let AlignedVerificationData {
        verification_data_commitment,
        batch_merkle_root,
        batch_inclusion_proof,
        index_in_batch,
    } = verification_data;

    let merkle_proof: Bytes = batch_inclusion_proof
        .merkle_path
        .into_iter()
        .flatten()
        .collect::<Vec<u8>>()
        .into();

    let VerificationDataCommitment {
        proof_commitment,
        proving_system_aux_data_commitment,
        proof_generator_addr,
        ..
    } = verification_data_commitment;

    let update_call = contract.updateChain(
        FixedBytes::from(proof_commitment),
        FixedBytes::from(proving_system_aux_data_commitment),
        FixedBytes::from(proof_generator_addr),
        FixedBytes::from(batch_merkle_root),
        merkle_proof,
        U256::from(index_in_batch),
        serialized_pub_input.into(),
        batcher_payment_service,
    );

    let estimated_gas = update_call
        .estimate_gas()
        .await
        .map_err(classify_contract_call_error)?;

    info!("updateChain estimated gas: {}", estimated_gas);

    let gas_limit = validate_gas_params(&provider, estimated_gas).await?;
    let update_call = update_call.gas(gas_limit);

    let pending_tx = update_call.send().await.map_err(classify_contract_call_error)?;
    let tx_hash = *pending_tx.tx_hash();

    info!("updateChain tx sent: {:?}", tx_hash);

    Ok(tx_hash)
}

/// Sends an `unlockTokens` transaction to `NoriTokenBridge.sol`.
///
/// Estimates gas, validates gas params, sends the transaction, and returns
/// the tx hash immediately without waiting for confirmation.
pub async fn send_unlock_tokens(
    verification_data: AlignedVerificationData,
    pub_input: &MinaAccountPubInputs,
    to_unlock_amount: U256,
    eth_rpc_url: &Url,
    wallet: WalletData,
    nori_token_bridge_addr: Address,
    batcher_payment_service: Address,
) -> Result<TxHash, EthError> {
    let serialized_pub_input = bincode::serialize(pub_input)
        .map_err(|err| EthError::SerializationError(format!("failed to serialize public inputs: {err}")))?;

    let provider = ProviderBuilder::new()
        .wallet(wallet.wallet)
        .connect_http(eth_rpc_url.clone());

    let contract = NoriTokenBridge::new(nori_token_bridge_addr, provider.clone());

    let AlignedVerificationData {
        verification_data_commitment,
        batch_merkle_root,
        batch_inclusion_proof,
        index_in_batch,
    } = verification_data;

    let merkle_proof: Bytes = batch_inclusion_proof
        .merkle_path
        .into_iter()
        .flatten()
        .collect::<Vec<u8>>()
        .into();

    let VerificationDataCommitment {
        proof_commitment,
        proving_system_aux_data_commitment,
        proof_generator_addr,
        ..
    } = verification_data_commitment;

    let call = contract.unlockTokens(
        to_unlock_amount,
        FixedBytes::from(proof_commitment),
        FixedBytes::from(proving_system_aux_data_commitment),
        FixedBytes::from(proof_generator_addr),
        FixedBytes::from(batch_merkle_root),
        merkle_proof,
        U256::from(index_in_batch),
        serialized_pub_input.into(),
        batcher_payment_service,
    );

    let estimated_gas = call
        .estimate_gas()
        .await
        .map_err(classify_contract_call_error)?;

    info!("unlockTokens estimated gas: {}", estimated_gas);

    let gas_limit = validate_gas_params(&provider, estimated_gas).await?;
    let call = call.gas(gas_limit);

    let pending_tx = call.send().await.map_err(classify_contract_call_error)?;
    let tx_hash = *pending_tx.tx_hash();

    info!("unlockTokens tx sent: {:?}", tx_hash);

    Ok(tx_hash)
}

/// Checks whether a transaction has been mined.
///
/// Returns `Ok(Some(receipt))` if mined, `Ok(None)` if still pending,
/// or `Err` on RPC failure.
pub async fn get_tx_receipt(
    eth_rpc_url: &Url,
    tx_hash: TxHash,
) -> Result<Option<TransactionReceipt>, EthError> {
    let provider = ProviderBuilder::new().connect_http(eth_rpc_url.clone());

    provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(classify_transport_error)
}

/// Reads the chain state hashes from `MinaStateSettlement.sol`.
///
/// Pure view call, no wallet needed.
pub async fn get_chain_state_hashes(
    eth_rpc_url: &Url,
    contract_addr: Address,
) -> Result<[StateHash; BRIDGE_TRANSITION_FRONTIER_LEN], EthError> {
    let provider = ProviderBuilder::new().connect_http(eth_rpc_url.clone());
    let contract = MinaStateSettlementExample::new(contract_addr, provider);

    let hashes = contract
        .getChainStateHashes()
        .call()
        .await
        .map_err(classify_contract_call_error)?;

    let state_hashes: Vec<StateHash> = hashes
        .into_iter()
        .map(|hash: FixedBytes<32>| {
            bincode::deserialize::<SolStateHash>(hash.as_slice())
                .map_err(|err| EthError::SerializationError(format!("failed to deserialize state hash: {err}")))
                .map(|h| h.0)
        })
        .collect::<Result<Vec<_>, _>>()?;

    state_hashes
        .try_into()
        .map_err(|_| EthError::SerializationError("wrong number of state hashes from contract".into()))
}

/// Checks a mined receipt for success or failure.
///
/// Call this after `get_tx_receipt` returns `Some(receipt)`.
/// Returns `Ok(receipt)` if the transaction succeeded, or the appropriate
/// `EthError` if it failed.
///
/// `gas_limit` is the gas limit that was set on the transaction (from
/// `validate_gas_params`). The receipt does not carry this value, so the
/// caller must provide it.
pub fn check_receipt_status(receipt: TransactionReceipt, gas_limit: u64) -> Result<TransactionReceipt, EthError> {
    if receipt.status() {
        Ok(receipt)
    } else {
        Err(classify_receipt_failure(receipt.gas_used, gas_limit))
    }
}

/// Reads `batchesState(batchIdentifier).responded` from the AlignedLayerServiceManager contract.
///
/// The batch identifier is `keccak256(abi.encodePacked(batchMerkleRoot, senderAddress))` when
/// `senderAddress` is non-zero, or just `batchMerkleRoot` when zero. This matches the key
/// construction in `AlignedLayerServiceManager.verifyBatchInclusion`.
///
/// Returns `Ok(true)` if the Aligned aggregator has responded to this batch, `Ok(false)` if not.
pub async fn is_batch_responded(
    eth_rpc_url: &Url,
    aligned_service_manager_addr: Address,
    batch_merkle_root: FixedBytes<32>,
    proof_generator_addr: FixedBytes<20>,
) -> Result<bool, EthError> {
    let batch_identifier = if proof_generator_addr == FixedBytes::<20>::ZERO {
        batch_merkle_root
    } else {
        let mut packed = Vec::with_capacity(52);
        packed.extend_from_slice(batch_merkle_root.as_slice());
        packed.extend_from_slice(proof_generator_addr.as_slice());
        FixedBytes::<32>::from(alloy::primitives::keccak256(&packed))
    };

    let provider = ProviderBuilder::new().connect_http(eth_rpc_url.clone());
    let contract = IAlignedServiceManager::new(aligned_service_manager_addr, provider);

    let result = contract
        .batchesState(batch_identifier)
        .call()
        .await
        .map_err(classify_contract_call_error)?;

    Ok(result.responded)
}
