// Ethereum contract deployment primitives.
//
// Deploys the three Nori bridge contracts to an Ethereum chain (typically a
// local anvil instance) and configures the NoriTokenBridge with references to
// the other two. Returns deployed addresses -- does not write config files or
// set environment variables.

use alloy::{
    primitives::{Address, FixedBytes},
    providers::ProviderBuilder,
};
use log::info;
use reqwest::Url;

use crate::eth_2::{MinaAccountValidationExample, MinaStateSettlementExample, NoriTokenBridge};
use crate::rpcs::errors::{classify_contract_call_error, EthError};
use crate::utils::wallet::WalletData;

/// Addresses of all three deployed contracts.
pub struct DeployedContracts {
    pub mina_state_settlement: Address,
    pub mina_account_validation: Address,
    pub nori_token_bridge: Address,
}

/// Deploys MinaStateSettlementExample.
///
/// Constructor: `(address _alignedServiceAddr, bytes32 _tipStateHash, bool _devnetFlag)`
///
/// `root_state_hash_bytes` is the bincode-serialized `SolStateHash` of the Mina
/// transition frontier root, obtained from the Mina daemon. The caller is
/// responsible for querying the daemon and serializing.
pub async fn deploy_mina_state_settlement(
    eth_rpc_url: &Url,
    wallet: WalletData,
    aligned_service_manager_addr: Address,
    root_state_hash_bytes: [u8; 32],
    is_devnet: bool,
) -> Result<Address, EthError> {
    let provider = ProviderBuilder::new()
        .wallet(wallet.wallet)
        .connect_http(eth_rpc_url.clone());

    let contract = MinaStateSettlementExample::deploy(
        &provider,
        aligned_service_manager_addr,
        FixedBytes::from(root_state_hash_bytes),
        is_devnet,
    )
    .await
    .map_err(classify_contract_call_error)?;

    let addr = *contract.address();
    info!("MinaStateSettlementExample deployed at {addr}");
    Ok(addr)
}

/// Deploys MinaAccountValidationExample.
///
/// Constructor: `(address _alignedServiceAddr)`
pub async fn deploy_mina_account_validation(
    eth_rpc_url: &Url,
    wallet: WalletData,
    aligned_service_manager_addr: Address,
) -> Result<Address, EthError> {
    let provider = ProviderBuilder::new()
        .wallet(wallet.wallet)
        .connect_http(eth_rpc_url.clone());

    let contract = MinaAccountValidationExample::deploy(
        &provider,
        aligned_service_manager_addr,
    )
    .await
    .map_err(classify_contract_call_error)?;

    let addr = *contract.address();
    info!("MinaAccountValidationExample deployed at {addr}");
    Ok(addr)
}

/// Deploys NoriTokenBridge.
///
/// Constructor: `() payable` -- accepts optional initial ETH balance in wei.
/// After deployment, call [`configure_nori_token_bridge`] to wire it to the
/// settlement and account validation contracts.
pub async fn deploy_nori_token_bridge(
    eth_rpc_url: &Url,
    wallet: WalletData,
    initial_balance_wei: Option<u128>,
) -> Result<Address, EthError> {
    let provider = ProviderBuilder::new()
        .wallet(wallet.wallet)
        .connect_http(eth_rpc_url.clone());

    let mut builder = NoriTokenBridge::deploy_builder(&provider);

    if let Some(balance) = initial_balance_wei {
        builder = builder.value(alloy::primitives::U256::from(balance));
    }

    let addr = builder
        .deploy()
        .await
        .map_err(classify_contract_call_error)?;

    info!("NoriTokenBridge deployed at {addr}");
    Ok(addr)
}

/// Calls `setAlignedContracts` on a deployed NoriTokenBridge to wire it to the
/// settlement and account validation contracts. Must be called by the same
/// wallet that deployed the bridge (the bridge operator).
pub async fn configure_nori_token_bridge(
    eth_rpc_url: &Url,
    wallet: WalletData,
    nori_token_bridge_addr: Address,
    mina_state_settlement_addr: Address,
    mina_account_validation_addr: Address,
) -> Result<(), EthError> {
    let provider = ProviderBuilder::new()
        .wallet(wallet.wallet)
        .connect_http(eth_rpc_url.clone());

    let contract = NoriTokenBridge::new(nori_token_bridge_addr, provider);
    let call = contract.setAlignedContracts(
        mina_state_settlement_addr,
        mina_account_validation_addr,
    );

    let pending_tx = call.send().await.map_err(classify_contract_call_error)?;
    let tx_hash = *pending_tx.tx_hash();

    info!(
        "NoriTokenBridge.setAlignedContracts tx sent: {tx_hash:?} \
         (settlement={mina_state_settlement_addr}, account_validation={mina_account_validation_addr})"
    );

    Ok(())
}

/// Deploys all three contracts and configures the bridge. Convenience wrapper
/// around the individual deploy functions.
///
/// Deploy order:
///   1. MinaStateSettlementExample (needs aligned SM address + Mina root hash + devnet flag)
///   2. MinaAccountValidationExample (needs aligned SM address)
///   3. NoriTokenBridge (no constructor args, optional initial balance)
///   4. NoriTokenBridge.setAlignedContracts (wires bridge to 1 and 2)
pub async fn deploy_all(
    eth_rpc_url: &Url,
    wallet: WalletData,
    aligned_service_manager_addr: Address,
    root_state_hash_bytes: [u8; 32],
    is_devnet: bool,
    initial_bridge_balance_wei: Option<u128>,
) -> Result<DeployedContracts, EthError> {
    let mina_state_settlement = deploy_mina_state_settlement(
        eth_rpc_url,
        wallet.clone(),
        aligned_service_manager_addr,
        root_state_hash_bytes,
        is_devnet,
    )
    .await?;

    let mina_account_validation = deploy_mina_account_validation(
        eth_rpc_url,
        wallet.clone(),
        aligned_service_manager_addr,
    )
    .await?;

    let nori_token_bridge = deploy_nori_token_bridge(
        eth_rpc_url,
        wallet.clone(),
        initial_bridge_balance_wei,
    )
    .await?;

    configure_nori_token_bridge(
        eth_rpc_url,
        wallet,
        nori_token_bridge,
        mina_state_settlement,
        mina_account_validation,
    )
    .await?;

    Ok(DeployedContracts {
        mina_state_settlement,
        mina_account_validation,
        nori_token_bridge,
    })
}
