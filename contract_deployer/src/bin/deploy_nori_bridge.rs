use alloy::primitives::Address;
use log::{error, info};
use mina_bridge_core::{
    eth::deploy_nori_token_bridge_contract,
    utils::{env::EnvironmentVariables, wallet_alloy::get_wallet},
};
use std::process;
use std::str::FromStr;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("Reading env. variables");
    // We use EnvironmentVariables to load common config like network, rpc, wallet.
    // Note: This requires all env vars defined in EnvironmentVariables::new() to be present.
    let env_vars = EnvironmentVariables::new().unwrap_or_else(|err| {
        error!("{}", err);
        process::exit(1);
    });

    let wallet = get_wallet(
        &env_vars.network,
        env_vars.keystore_path.as_deref(),
        env_vars.private_key.as_deref(),
    )
    .unwrap_or_else(|err| {
        error!("Failed to get wallet: {err}");
        process::exit(1);
    });

    let state_settlement_addr = env_vars
        .state_settlement_addr
        .as_ref()
        .ok_or("STATE_SETTLEMENT_ETH_ADDR is not set".to_string())
        .and_then(|addr| Address::from_str(addr).map_err(|e| e.to_string()))
        .unwrap_or_else(|err| {
            error!("{}", err);
            process::exit(1);
        });

    let account_validation_addr = env_vars
        .account_validation_addr
        .as_ref()
        .ok_or("ACCOUNT_VALIDATION_ETH_ADDR is not set".to_string())
        .and_then(|addr| Address::from_str(addr).map_err(|e| e.to_string()))
        .unwrap_or_else(|err| {
            error!("{}", err);
            process::exit(1);
        });

    info!("Deploying Nori Token Bridge...");
    info!("State Settlement Address: {}", state_settlement_addr);
    info!("Account Validation Address: {}", account_validation_addr);

    deploy_nori_token_bridge_contract(
        &env_vars.eth_rpc_url,
        state_settlement_addr,
        account_validation_addr,
        &wallet,
        Some(10 * (10_u128.pow(18)) as u128),
    )
    .await
    .unwrap_or_else(|err| {
        error!("Failed to deploy NoriTokenBridge: {err}");
        process::exit(1);
    });
}

