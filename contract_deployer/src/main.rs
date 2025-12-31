use aligned_sdk::common::types::Network;
use alloy::primitives::Address;
use chrono::Local;
use clap::{Parser, Subcommand};
use log::{debug, error, info};
use mina_bridge_core::{
    eth::{
        deploy_mina_account_validation_example_contract, deploy_mina_bridge_example_contract,
        deploy_nori_token_bridge_contract, MinaAccountValidationExampleConstructorArgs,
        MinaStateSettlementExampleConstructorArgs, SolStateHash,
    },
    mina::query_root,
    utils::{
        constants::{ALIGNED_SM_DEVNET_ETH_ADDR, BRIDGE_TRANSITION_FRONTIER_LEN},
        env::EnvironmentVariables,
        wallet::get_wallet,
    },
};
use rust_decimal::{prelude::ToPrimitive, Decimal};
use std::{fs, process, str::FromStr};

#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Deploy Mina example contracts and NoriTokenBridge
    DeployAllContracts {
        /// Initial balance (in ether, supports decimals). Example: 1.25
        #[arg(value_name = "NORI_TOKEN_BRIDGE_INITIAL_BALANCE")]
        initial_balance: Option<String>,
    },
    /// Deploy only NoriTokenBridge using existing state/account contracts
    DeployNoriBridge {
        /// Initial balance (in ether, supports decimals). Example: 1.25
        #[arg(value_name = "NORI_TOKEN_BRIDGE_INITIAL_BALANCE")]
        initial_balance: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    match cli.command {
        Command::DeployAllContracts { initial_balance } => {
            debug!("Received initial balance arg: {:?}", initial_balance);
            info!("Reading env. variables");
            let EnvironmentVariables {
                rpc_url,
                eth_rpc_url,
                network,
                private_key,
                keystore_path,
                ..
            } = EnvironmentVariables::new().unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });

            let root_hash = query_root(&rpc_url, BRIDGE_TRANSITION_FRONTIER_LEN)
                .await
                .unwrap_or_else(|err| {
                    error!("Failed to query root state hash: {err}");
                    process::exit(1);
                });
            info!(
                "Queried root state hash {root_hash} for chain of length {BRIDGE_TRANSITION_FRONTIER_LEN}"
            );
            let root_hash = bincode::serialize(&SolStateHash(root_hash)).unwrap_or_else(|err| {
                error!("Failed to serialize root state hash: {err}");
                process::exit(1);
            });

            let aligned_sm_addr = match network {
                Network::Devnet => Ok(ALIGNED_SM_DEVNET_ETH_ADDR.to_owned()),
                Network::Holesky => std::env::var("ALIGNED_SERVICE_MANAGER_ADDR")
                    .map_err(|err| format!("Error getting Aligned SM contract address: {err}")),
                _ => Err("Unimplemented Ethereum contract on selected chain".to_owned()),
            }
            .unwrap_or_else(|err| {
                error!("{err}");
                process::exit(1);
            });

            let bridge_constructor_args =
                MinaStateSettlementExampleConstructorArgs::new(&aligned_sm_addr, root_hash)
                    .unwrap_or_else(|err| {
                        error!("Failed to make constructor args for bridge contract call: {err}");
                        process::exit(1);
                    });
            let account_constructor_args = MinaAccountValidationExampleConstructorArgs::new(
                &aligned_sm_addr,
            )
            .unwrap_or_else(|err| {
                error!("Failed to make constructor args for account contract call: {err}");
                process::exit(1);
            });

            let wallet_data =
                get_wallet(&network, keystore_path.as_deref(), private_key.as_deref())
                    .unwrap_or_else(|err| {
                        error!("Failed to get wallet: {err}");
                        process::exit(1);
                    });

            // Contract for Devnet state proofs
            let is_state_proof_from_devnet = match network {
                Network::Devnet => true,
                Network::Holesky => false,
                _ => {
                    error!(
                        "Unrecognized chain, possible values for ETH_CHAIN are \"devnet\" and \"holesky\"."
                    );
                    process::exit(1);
                }
            };

            let state_settlement_addr = deploy_mina_bridge_example_contract(
                &eth_rpc_url,
                &bridge_constructor_args,
                &wallet_data.wallet,
                is_state_proof_from_devnet,
            )
            .await
            .unwrap_or_else(|err| {
                error!("Failed to deploy contract: {err}");
                process::exit(1);
            });

            let account_validation_addr = deploy_mina_account_validation_example_contract(
                &eth_rpc_url,
                account_constructor_args,
                &wallet_data.wallet,
            )
            .await
            .unwrap_or_else(|err| {
                error!("Failed to deploy contract: {err}");
                process::exit(1);
            });

            let initial_balance_wei = parse_initial_balance(initial_balance.as_deref())
                .unwrap_or_else(|err| {
                    error!("Invalid initial balance: {err}");
                    process::exit(1);
                });

            let nori_token_bridge_addr = deploy_nori_token_bridge_contract(
                &eth_rpc_url,
                state_settlement_addr,
                account_validation_addr,
                &wallet_data.wallet,
                initial_balance_wei,
            )
            .await
            .unwrap_or_else(|err| {
                error!("Failed to deploy NoriTokenBridge: {err}");
                process::exit(1);
            });

            // log in local filesystem
            let ts = Local::now().format("%Y%m%d%H%M%S");
            let filename = format!(".generated.contract.addresses.{}", ts);
            let generated_addresses = format!(
                "STATE_SETTLEMENT_ETH_ADDR={}\nACCOUNT_VALIDATION_ETH_ADDR={}\nNORI_TOKEN_BRIDGE_ETH_ADDRESS={}\n",
                state_settlement_addr, account_validation_addr, nori_token_bridge_addr
            );

            fs::write(&filename, generated_addresses).unwrap_or_else(|err| {
                error!("Failed to write {filename}: {err}");
                process::exit(1);
            });

            info!(
                "deployed contract addresses are saved into {} file",
                filename
            );
        }

        Command::DeployNoriBridge { initial_balance } => {
            debug!("Received initial balance arg: {:?}", initial_balance);
            info!("Reading env. variables");
            let env_vars = EnvironmentVariables::new().unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });

            let wallet_data = get_wallet(
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

            let initial_balance_wei = parse_initial_balance(initial_balance.as_deref())
                .unwrap_or_else(|err| {
                    error!("Invalid initial balance: {err}");
                    process::exit(1);
                });

            info!("Deploying Nori Token Bridge...");
            info!("State Settlement Address: {}", state_settlement_addr);
            info!("Account Validation Address: {}", account_validation_addr);

            let nori_token_bridge_addr = deploy_nori_token_bridge_contract(
                &env_vars.eth_rpc_url,
                state_settlement_addr,
                account_validation_addr,
                &wallet_data.wallet,
                initial_balance_wei,
            )
            .await
            .unwrap_or_else(|err| {
                error!("Failed to deploy NoriTokenBridge: {err}");
                process::exit(1);
            });

            // log in local filesystem
            let ts = Local::now().format("%Y%m%d%H%M%S");
            let filename = format!(".generated.contract.addresses.{}", ts);
            let generated_addresses = format!(
                "STATE_SETTLEMENT_ETH_ADDR={}\nACCOUNT_VALIDATION_ETH_ADDR={}\nNORI_TOKEN_BRIDGE_ETH_ADDRESS={}\n",
                state_settlement_addr, account_validation_addr, nori_token_bridge_addr
            );

            fs::write(&filename, generated_addresses).unwrap_or_else(|err| {
                error!("Failed to write {filename}: {err}");
                process::exit(1);
            });

            info!(
                "deployed contract addresses are saved into {} file",
                filename
            );
        }
    }
}

fn parse_initial_balance(raw: Option<&str>) -> Result<Option<u128>, String> {
    let Some(raw) = raw else { return Ok(None); };
    let dec = Decimal::from_str(raw).map_err(|e| format!("failed to parse decimal: {e}"))?;
    if dec.is_sign_negative() {
        return Err("initial balance must be non-negative".to_string());
    }
    // Scale to wei (18 decimals) and round to nearest integer
    let wei = (dec * Decimal::from(10u128.pow(18))).round();
    wei.to_u128()
        .ok_or_else(|| "initial balance too large".to_string())
        .map(Some)
}
