use aligned_sdk::common::types::{AlignedVerificationData, Network, VerificationDataCommitment};
use alloy::{
    network::{Ethereum, EthereumWallet},
    primitives::{Address, Bytes, FixedBytes, U256},
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::client::RpcClient,
    sol,
    transports::{http::Http, BoxTransport},
};
use alloy::hex::ToHexExt;
use log::info;
use mina_p2p_messages::v2::StateHash;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::{
    proof::{account_proof::MinaAccountPubInputs, state_proof::MinaStatePubInputs},
    sol::serialization::SolSerialize,
    utils::{
        constants::BRIDGE_TRANSITION_FRONTIER_LEN,
        wallet::WalletData,
    },
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

// Define constant values that will be used for gas limits and calculations
const MAX_GAS_LIMIT_VALUE: u64 = 1_000_000; // Maximum allowed gas for a transaction
const MAX_GAS_PRICE_GWEI: u64 = 300; // Maximum allowed gas price in Gwei
const GAS_ESTIMATE_MARGIN: u64 = 110; // Safety margin (110 means 110%, or +10%)

/// Wrapper of Mina Ledger hash for Ethereum
#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct SolStateHash(#[serde_as(as = "SolSerialize")] pub StateHash);

/// Arguments of the Mina State Settlement Example Ethereum Contract constructor:
///
/// - `aligned_service_addr`: Address of the Aligned Service Manager Ethereum Contract
/// - `root_state_hash`: Root state hash of the Mina transition frontier
pub struct MinaStateSettlementExampleConstructorArgs {
    aligned_service_addr: alloy::primitives::Address,
    root_state_hash: alloy::primitives::FixedBytes<32>,
}

/// Arguments of the Mina Account Validation Example Ethereum Contract constructor:
///
/// - `aligned_service_addr`: Address of the Aligned Service Manager Ethereum Contract
pub struct MinaAccountValidationExampleConstructorArgs {
    aligned_service_addr: alloy::primitives::Address,
}

impl MinaStateSettlementExampleConstructorArgs {
    /// Creates the arguments of the Mina State Settlement Example Ethereum Contract constructor.
    /// Receives `aligned_service_addr` as a string slice and `root_state_hash` as a vector of bytes
    /// and converts them to Ethereum friendly types.
    pub fn new(aligned_service_addr: &str, root_state_hash: Vec<u8>) -> Result<Self, String> {
        let aligned_service_addr =
            alloy::primitives::Address::parse_checksummed(aligned_service_addr, None)
                .map_err(|err| err.to_string())?;
        let root_state_hash = alloy::primitives::FixedBytes(
            root_state_hash
                .try_into()
                .map_err(|_| "Could not convert root state hash into fixed array".to_string())?,
        );
        Ok(Self {
            aligned_service_addr,
            root_state_hash,
        })
    }
}

impl MinaAccountValidationExampleConstructorArgs {
    /// Creates the arguments of the Mina Account Validation Example Ethereum Contract constructor.
    /// Receives `aligned_service_addr` as a string slice and converts them to Ethereum friendly types.
    pub fn new(aligned_service_addr: &str) -> Result<Self, String> {
        let aligned_service_addr =
            alloy::primitives::Address::parse_checksummed(aligned_service_addr, None)
                .map_err(|err| err.to_string())?;
        Ok(Self {
            aligned_service_addr,
        })
    }
}

// Main function that validates gas parameters
// Takes provider (connection to Ethereum) and estimated_gas as parameters
async fn validate_gas_params<P>(
    provider: &P,
    estimated_gas: U256,
) -> Result<U256, String> 
where
    P: Provider,
{
    // Query the current network gas price
    let current_gas_price = provider
        .get_gas_price()
        .await
        .map_err(|err| err.to_string())?;

    let current_gas_price = U256::from(current_gas_price);

    // Convert gas price from Wei to Gwei by dividing by 1_000_000_000
    let gas_price_gwei = current_gas_price
        .checked_div(U256::from(1_000_000_000))
        .ok_or("Gas price calculation overflow")?;

    // Check if the current gas price is above our maximum allowed price
    if gas_price_gwei > U256::from(MAX_GAS_PRICE_GWEI) {
        return Err(format!(
            "Gas price too high: {} gwei (max: {} gwei)",
            gas_price_gwei, MAX_GAS_PRICE_GWEI
        ));
    }

    // Calculate gas limit with safety margin:
    // 1. Multiply estimated gas by 110 (for 10% extra)
    // 2. Divide by 100 to get the final value
    let gas_with_margin = estimated_gas
        .checked_mul(U256::from(GAS_ESTIMATE_MARGIN))
        .and_then(|v| v.checked_div(U256::from(100)))
        .ok_or("Gas margin calculation overflow")?;

    // Check if our gas limit with margin is above maximum allowed gas
    if gas_with_margin > U256::from(MAX_GAS_LIMIT_VALUE) {
        return Err(format!(
            "Estimated gas too high: {} (max: {})",
            gas_with_margin, MAX_GAS_LIMIT_VALUE
        ));
    }

    // If all checks pass, return the gas limit with safety margin
    Ok(gas_with_margin)
}

/// Wrapper of the `updateChain` function of the Mina State Settlement Example Ethereum Contract with address
/// `contract_addr`.
/// Adapts arguments to be Ethereum friendly and sends the corresponding transaction to run `updateChain` on
/// Ethereum.
///
/// See [updateChain](https://github.com/lambdaclass/mina_bridge/blob/7f2fa1f0eac39499ff2ed3ed2d989ea7314805e3/contract/src/MinaStateSettlementExample.sol#L78)
/// for more info.
pub async fn update_chain(
    verification_data: AlignedVerificationData,
    pub_input: &MinaStatePubInputs,
    _network: &Network,
    eth_rpc_url: &str,
    wallet: WalletData,
    contract_addr: &str,
    batcher_payment_service: &str,
) -> Result<(), String> {
    let bridge_eth_addr = contract_addr.parse::<Address>().map_err(|e| e.to_string())?;
    let batcher_payment_service_addr = batcher_payment_service.parse::<Address>().map_err(|e| e.to_string())?;

    let serialized_pub_input = bincode::serialize(pub_input)
        .map_err(|err| format!("Failed to serialize public inputs: {err}"))?;

    info!("Creating contract instance");
    let url = reqwest::Url::parse(eth_rpc_url).map_err(|e| e.to_string())?;
    let http = Http::new(url);
    let boxed = BoxTransport::new(http);
    let client = RpcClient::new(boxed, true);
    let root = RootProvider::new(client);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet.wallet)
        .on_provider(root);
    
    let contract = MinaStateSettlementExample::new(bridge_eth_addr, provider.clone());

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

    info!("Updating contract");

    let proof_commitment = FixedBytes::from(proof_commitment);
    let proving_system_aux_data_commitment = FixedBytes::from(proving_system_aux_data_commitment);
    let batch_merkle_root = FixedBytes::from(batch_merkle_root);
    // proof_generator_addr is likely [u8; 20]. Contract expects FixedBytes<20>.
    let proof_generator_addr = FixedBytes::from(proof_generator_addr);

    let update_call = contract.updateChain(
        proof_commitment,
        proving_system_aux_data_commitment,
        proof_generator_addr,
        batch_merkle_root,
        merkle_proof,
        U256::from(index_in_batch),
        serialized_pub_input.into(),
        batcher_payment_service_addr,
    );
    // update call reverts if batch is not valid or proof isn't included in it.

    let estimated_gas = update_call
        .estimate_gas()
        .await
        .map_err(|err| err.to_string())?;

    info!("Estimated gas cost: {}", estimated_gas);

    // Validate gas parameters and get safe gas limit
    let gas_limit = validate_gas_params(&provider, U256::from(estimated_gas)).await?;
    let update_call = update_call.gas(gas_limit.to::<u128>());

    let pending_tx = update_call
        .send()
        .await
        .map_err(|err| err.to_string())?;
    
    info!(
        "Transaction {} was submitted and is now pending",
        pending_tx.tx_hash().encode_hex()
    );

    let receipt = pending_tx
        .get_receipt()
        .await
        .map_err(|err| err.to_string())?;

    info!(
        "Transaction mined! final gas cost: {}",
        receipt.gas_used
    );

    info!("Checking that the state hashes were stored correctly..");

    // TODO(xqft): do the same for ledger hashes
    info!("Getting network state hashes");
    let new_network_state_hashes = get_bridge_chain_state_hashes(contract_addr, eth_rpc_url)
        .await
        .map_err(|err| err.to_string())?;

    if new_network_state_hashes != pub_input.candidate_chain_state_hashes {
        return Err("Stored network state hashes don't match the candidate's".to_string());
    }

    let tip_state_hash = new_network_state_hashes
        .last()
        .ok_or("Failed to get tip state hash".to_string())?
        .clone();
    info!("Successfuly updated smart contract to verified network of tip {tip_state_hash}");

    Ok(())
}

/// Wrapper of the `getTipStateHash` function of the Mina State Settlement Example Ethereum Contract with address
/// `contract_addr`.
/// Calls `getTipStateHash` on Ethereum.
///
/// See [getTipStateHash](https://github.com/lambdaclass/mina_bridge/blob/7f2fa1f0eac39499ff2ed3ed2d989ea7314805e3/contract/src/MinaStateSettlementExample.sol#L44)
/// for more info.
pub async fn get_bridge_tip_hash(
    contract_addr: &str,
    eth_rpc_url: &str,
) -> Result<SolStateHash, String> {
    let bridge_eth_addr = contract_addr.parse::<Address>().map_err(|e| e.to_string())?;

    info!("Creating contract instance");
    let url = reqwest::Url::parse(eth_rpc_url).map_err(|e| e.to_string())?;
    let http = Http::new(url);
    let boxed = BoxTransport::new(http);
    let client = RpcClient::new(boxed, true);
    let provider: RootProvider<_, Ethereum> = RootProvider::new(client);
    let contract = MinaStateSettlementExample::new(bridge_eth_addr, provider);

    let state_hash_return = contract
        .getTipStateHash()
        .call()
        .await
        .map_err(|err| err.to_string())?;

    let state_hash_bytes = state_hash_return._0;

    let state_hash: SolStateHash = bincode::deserialize(state_hash_bytes.as_slice())
        .map_err(|err| format!("Failed to deserialize bridge tip state hash: {err}"))?;
    info!("Retrieved bridge tip state hash: {}", state_hash.0,);

    Ok(state_hash)
}

/// Wrapper of the `getChainStateHashes` function of the Mina State Settlement Example Ethereum Contract with address
/// `contract_addr`.
/// Calls `getChainStateHashes` on Ethereum.
///
/// See [getChainStateHashes](https://github.com/lambdaclass/mina_bridge/blob/7f2fa1f0eac39499ff2ed3ed2d989ea7314805e3/contract/src/MinaStateSettlementExample.sol#L54)
/// for more info.
pub async fn get_bridge_chain_state_hashes(
    contract_addr: &str,
    eth_rpc_url: &str,
) -> Result<[StateHash; BRIDGE_TRANSITION_FRONTIER_LEN], String> {
    let bridge_eth_addr = contract_addr.parse::<Address>().map_err(|e| e.to_string())?;

    info!("Creating contract instance");
    let url = reqwest::Url::parse(eth_rpc_url).map_err(|e| e.to_string())?;
    let http = Http::new(url);
    let boxed = BoxTransport::new(http);
    let client = RpcClient::new(boxed, true);
    let provider: RootProvider<_, Ethereum> = RootProvider::new(client);
    let contract = MinaStateSettlementExample::new(bridge_eth_addr, provider);

    let hashes_return = contract
        .getChainStateHashes()
        .call()
        .await
        .map_err(|err| format!("Could not call contract for state hashes: {err}"))?;
    
    let hashes = hashes_return._0;

    hashes
        .into_iter()
        .map(|hash| {
            bincode::deserialize::<SolStateHash>(hash.as_slice())
                .map_err(|err| format!("Failed to deserialize network state hashes: {err}"))
                .map(|hash| hash.0)
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|hashes| {
            hashes
                .try_into()
                .map_err(|_| "Failed to convert network state hashes vec into array".to_string())
        })
}

/// Wrapper of the `validateAccount` function of the Mina Account Validation Example Ethereum Contract with address
/// `contract_addr`.
/// Adapts arguments to be Ethereum friendly and sends the corresponding transaction to run `validateAccount` on
/// Ethereum.
///
/// See [validateAccount](https://github.com/lambdaclass/mina_bridge/blob/7f2fa1f0eac39499ff2ed3ed2d989ea7314805e3/contract/src/MinaAccountValidationExample.sol#L32)
/// for more info.
pub async fn validate_account(
    verification_data: AlignedVerificationData,
    pub_input: &MinaAccountPubInputs,
    eth_rpc_url: &str,
    contract_addr: &str,
    batcher_payment_service: &str,
) -> Result<(), String> {
    let bridge_eth_addr = contract_addr.parse::<Address>().map_err(|e| e.to_string())?;
    let batcher_payment_service_addr = batcher_payment_service.parse::<Address>().map_err(|e| e.to_string())?;

    info!("Creating contract instance");

    let url = reqwest::Url::parse(eth_rpc_url).map_err(|e| e.to_string())?;
    let http = Http::new(url);
    let boxed = BoxTransport::new(http);
    let client = RpcClient::new(boxed, true);
    let provider: RootProvider<_, Ethereum> = RootProvider::new(client);
    let contract = MinaAccountValidationExample::new(bridge_eth_addr, provider.clone());

    let serialized_pub_input = bincode::serialize(pub_input)
        .map_err(|err| format!("Failed to serialize public inputs: {err}"))?;

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

    info!("Validating account");

    let proof_commitment = FixedBytes::from(proof_commitment);
    let proving_system_aux_data_commitment = FixedBytes::from(proving_system_aux_data_commitment);
    let batch_merkle_root = FixedBytes::from(batch_merkle_root);
    let proof_generator_addr = FixedBytes::from(proof_generator_addr);

    let aligned_args = MinaAccountValidationExample::AlignedArgs {
        proofCommitment: proof_commitment,
        provingSystemAuxDataCommitment: proving_system_aux_data_commitment,
        proofGeneratorAddr: proof_generator_addr,
        batchMerkleRoot: batch_merkle_root,
        merkleProof: merkle_proof,
        verificationDataBatchIndex: U256::from(index_in_batch),
        pubInput: serialized_pub_input.into(),
        batcherPaymentService: batcher_payment_service_addr,
    };

    let call = contract.validateAccount(aligned_args);
    let estimated_gas = call.estimate_gas().await.map_err(|err| err.to_string())?;

    info!("Estimated account verification gas cost: {estimated_gas}");

    let gas_limit = validate_gas_params(&provider, U256::from(estimated_gas)).await?;

    call.gas(gas_limit.to::<u128>()).call().await.map_err(|err| err.to_string())?;

    Ok(())
}

/// Deploys the Mina State Settlement Example Contract on Ethereum
pub async fn deploy_mina_bridge_example_contract(
    eth_rpc_url: &str,
    constructor_args: &MinaStateSettlementExampleConstructorArgs,
    wallet: &EthereumWallet,
    is_state_proof_from_devnet: bool,
) -> Result<alloy::primitives::Address, String> {
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(reqwest::Url::parse(eth_rpc_url).map_err(|err| err.to_string())?);

    let MinaStateSettlementExampleConstructorArgs {
        aligned_service_addr,
        root_state_hash,
    } = constructor_args;
    let contract = MinaStateSettlementExample::deploy(
        &provider,
        *aligned_service_addr,
        *root_state_hash,
        is_state_proof_from_devnet,
    )
    .await
    .map_err(|err| err.to_string())?;
    let address = contract.address();

    let network = if is_state_proof_from_devnet {
        "Devnet"
    } else {
        "Mainnet"
    };

    info!(
        "Mina {} Bridge example contract successfuly deployed with address {}",
        network, address
    );
    info!(
        "Set STATE_SETTLEMENT_ETH_ADDR={} if using Mina {}",
        address, network
    );

    Ok(*address)
}

/// Deploys the Mina Account Validation Example Contract on Ethereum
pub async fn deploy_mina_account_validation_example_contract(
    eth_rpc_url: &str,
    constructor_args: MinaAccountValidationExampleConstructorArgs,
    wallet: &EthereumWallet,
) -> Result<alloy::primitives::Address, String> {
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(reqwest::Url::parse(eth_rpc_url).map_err(|err| err.to_string())?);

    let MinaAccountValidationExampleConstructorArgs {
        aligned_service_addr,
    } = constructor_args;
    let contract = MinaAccountValidationExample::deploy(&provider, aligned_service_addr)
        .await
        .map_err(|err| err.to_string())?;
    let address = contract.address();

    info!(
        "Mina Account Validation example contract successfuly deployed with address {}",
        address
    );
    info!("Set ACCOUNT_VALIDATION_ETH_ADDR={}", address);

    Ok(*address)
}

/// Deploys the Nori Token Bridge Contract on Ethereum
pub async fn deploy_nori_token_bridge_contract(
    eth_rpc_url: &str,
    state_settlement_addr: alloy::primitives::Address,
    account_validation_addr: alloy::primitives::Address,
    wallet: &EthereumWallet,
    initial_balance: Option<u128>,
) -> Result<alloy::primitives::Address, String> {
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(reqwest::Url::parse(eth_rpc_url).map_err(|err| err.to_string())?);

    let builder =
        NoriTokenBridge::deploy_builder(&provider, state_settlement_addr, account_validation_addr);

    let builder = if let Some(balance) = initial_balance {
        builder.value(alloy::primitives::U256::from(balance))
    } else {
        builder
    };

    let address = builder.deploy().await.map_err(|err| err.to_string())?;

    info!(
        "Nori Token Bridge contract successfuly deployed with address {}",
        address
    );
    info!("Set NORI_TOKEN_BRIDGE_ETH_ADDR={}", address);

    Ok(address)
}

