use aligned_sdk::common::types::Network;
use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol_types::sol,
};
use clap::{Parser, Subcommand};
use log::{debug, error, info};
use mina_bridge_core::{
    sdk::{
        get_bridged_chain_tip_state_hash, update_bridge_chain, validate_account,
        AccountVerificationData,
    },
    utils::{env::EnvironmentVariables, wallet, wallet_alloy},
};
use std::{process, str::FromStr, time::SystemTime};

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    SudokuValidity,
    "abi/SudokuValidity.json"
);

sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    NoriTokenBridge,
    "abi/NoriTokenBridge.json"
);

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    DeployContract,
    ValidateSolution,
    UnlockNoriToken,
    Transfer {
        #[arg(short, long)]
        private_key: String,
        #[arg(short, long)]
        to: String,
        #[arg(short, long)]
        amount: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let now = SystemTime::now();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let EnvironmentVariables {
        rpc_url,
        network,
        state_settlement_addr,
        account_validation_addr,
        batcher_addr,
        batcher_eth_addr,
        eth_rpc_url,
        proof_generator_addr,
        keystore_path,
        private_key,
        sudoku_zkapp_addr,
        sudoku_token_id,
        sudoku_validity_devnet_addr,
        nori_token_storage_zkapp_addr,
        nori_token_controller_token_id,
        nori_token_bridge_devnet_addr,
    } = EnvironmentVariables::new().unwrap_or_else(|err| {
        error!("{}", err);
        process::exit(1);
    });

    let state_settlement_addr = state_settlement_addr.unwrap_or_else(|| {
        error!("Error getting State settlement contract address");
        process::exit(1);
    });
    let account_validation_addr = account_validation_addr.unwrap_or_else(|| {
        error!("Error getting Account validation contract address");
        process::exit(1);
    });

    let sudoku_address = match network {
        Network::Devnet => sudoku_validity_devnet_addr.to_string(),
        Network::Holesky => std::env::var("SUDOKU_VALIDITY_HOLESKY_ADDRESS").unwrap_or_else(|_| {
            error!("Error getting Sudoku vality contract address");
            process::exit(1);
        }),
        _ => todo!(),
    };

    let wallet_alloy =
        wallet_alloy::get_wallet(&network, keystore_path.as_deref(), private_key.as_deref())
            .unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });

    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet_alloy)
        .on_http(
            reqwest::Url::parse(&eth_rpc_url)
                .map_err(|err| err.to_string())
                .unwrap(),
        );

    match cli.command {
        Command::DeployContract => {
            // TODO(xqft): we might as well use the Chain type from Alloy, it isn't right to add
            // aligned-sdk as a dependency only for this type.

            let contract = SudokuValidity::deploy(
                &provider,
                Address::from_str(&state_settlement_addr).unwrap(),
                Address::from_str(&account_validation_addr).unwrap(),
            )
            .await
            .map_err(|err| err.to_string())
            .unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });

            info!(
                "SudokuValidity contract successfuly deployed with address {}",
                contract.address()
            );
        }
        Command::ValidateSolution => {
            // We could check if the specific block containing the tx is already verified, before
            // updating the bridge's chain.
            // let is_state_verified = is_state_verified(&state_hash, &state_settlement_addr, &eth_rpc_url)
            //     .await
            //     .unwrap_or_else(|err| {
            //         error!("{}", err);
            //         process::exit(1);
            //     });

            // if !is_state_verified {
            //     info!("State that includes the zkApp tx isn't verified. Bridging latest chain...");

            let wallet =
                wallet::get_wallet(&network, keystore_path.as_deref(), private_key.as_deref())
                    .unwrap_or_else(|err| {
                        error!("{}", err);
                        process::exit(1);
                    });

            let state_verification_result = update_bridge_chain(
                &rpc_url,
                &network,
                &state_settlement_addr,
                &batcher_addr,
                &eth_rpc_url,
                &proof_generator_addr,
                wallet.clone(),
                &batcher_eth_addr,
                true,
                false,
            )
            .await;

            match state_verification_result {
                Err(err) if err == "Latest chain is already verified" => {
                    info!("Bridge chain is up to date, won't verify new states.")
                }
                Err(err) => {
                    error!("{}", err);
                    process::exit(1);
                }
                _ => {}
            }
            // }

            let tip_state_hash =
                get_bridged_chain_tip_state_hash(&state_settlement_addr, &eth_rpc_url)
                    .await
                    .unwrap_or_else(|err| {
                        error!("{}", err);
                        process::exit(1);
                    });

            info!("tip state hash: {}", &tip_state_hash);

            let AccountVerificationData {
                proof_commitment,
                proving_system_aux_data_commitment,
                proof_generator_addr,
                batch_merkle_root,
                merkle_proof,
                verification_data_batch_index,
                pub_input,
            } = validate_account(
                &sudoku_zkapp_addr,
                &sudoku_token_id,
                &tip_state_hash,
                &rpc_url,
                &network,
                &account_validation_addr,
                &batcher_addr,
                &eth_rpc_url,
                &proof_generator_addr,
                &batcher_eth_addr,
                wallet,
                false,
            )
            .await
            .unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });

            info!("Creating contract instance");
            let contract =
                SudokuValidity::new(Address::from_str(&sudoku_address).unwrap(), provider);

            let call = contract.validateSolution(
                proof_commitment.into(),
                proving_system_aux_data_commitment.into(),
                proof_generator_addr.into(),
                batch_merkle_root.into(),
                merkle_proof.into(),
                U256::from(verification_data_batch_index),
                pub_input.into(),
                Address::from_str(&batcher_eth_addr).unwrap(),
            );

            info!("Sending transaction to SudokuValidity contract...");
            let tx = call.send().await;

            match tx {
                Ok(tx) => {
                    let receipt = tx.get_receipt().await.unwrap_or_else(|err| {
                        error!("{}", err);
                        process::exit(1);
                    });
                    let new_timestamp: U256 = contract
                        .getLatestSolutionTimestamp()
                        .call()
                        .await
                        .unwrap_or_else(|err| {
                            error!("{}", err);
                            process::exit(1);
                        })
                        ._0;

                    info!(
                        "SudokuValidity contract was updated! transaction hash: {}, gas cost: {}, new timestamp: {}",
                        receipt.transaction_hash, receipt.gas_used, new_timestamp
                    );
                }
                Err(err) => error!("SudokuValidity transaction failed!: {err}"),
            }
        }
        Command::UnlockNoriToken => {
            let wallet =
                wallet::get_wallet(&network, keystore_path.as_deref(), private_key.as_deref())
                    .unwrap_or_else(|err| {
                        error!("{}", err);
                        process::exit(1);
                    });

            let state_verification_result = update_bridge_chain(
                &rpc_url,
                &network,
                &state_settlement_addr,
                &batcher_addr,
                &eth_rpc_url,
                &proof_generator_addr,
                wallet.clone(),
                &batcher_eth_addr,
                true,
                false,
            )
            .await;

            match state_verification_result {
                Err(err) if err == "Latest chain is already verified" => {
                    info!("Bridge chain is up to date, won't verify new states.")
                }
                Err(err) => {
                    error!("{}", err);
                    process::exit(1);
                }
                _ => {}
            }

            let tip_state_hash =
                get_bridged_chain_tip_state_hash(&state_settlement_addr, &eth_rpc_url)
                    .await
                    .unwrap_or_else(|err| {
                        error!("{}", err);
                        process::exit(1);
                    });

            info!("tip state hash: {}", &tip_state_hash);

            let AccountVerificationData {
                proof_commitment,
                proving_system_aux_data_commitment,
                proof_generator_addr,
                batch_merkle_root,
                merkle_proof,
                verification_data_batch_index,
                pub_input,
            } = validate_account(
                &nori_token_storage_zkapp_addr,
                &nori_token_controller_token_id,
                &tip_state_hash,
                &rpc_url,
                &network,
                &account_validation_addr,
                &batcher_addr,
                &eth_rpc_url,
                &proof_generator_addr,
                &batcher_eth_addr,
                wallet,
                false,
            )
            .await
            .unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });
            
            info!("Creating contract instance");
            let contract = NoriTokenBridge::new(Address::from_str(&nori_token_bridge_devnet_addr).unwrap(), provider);

            let toUnlockAmount = U256::from(1_000_000_000_000_000_000u64); // 1 ETH
            info!("Unlock params:");
            info!("toUnlockAmount: {}", toUnlockAmount);
            info!("proof_commitment: {}", hex::encode(proof_commitment));
            info!("proving_system_aux_data_commitment: {}", hex::encode(proving_system_aux_data_commitment));
            info!("proof_generator_addr: {}", hex::encode(proof_generator_addr));
            info!("batch_merkle_root: {}", hex::encode(batch_merkle_root));
            info!("merkle_proof: {}", hex::encode(&merkle_proof));
            info!("verification_data_batch_index: {}", verification_data_batch_index);
            info!("pub_input: {}", hex::encode(&pub_input));
            info!("batcher_eth_addr: {}", &batcher_eth_addr);

            let call = contract.unlockTokens(
                toUnlockAmount,
                proof_commitment.into(),
                proving_system_aux_data_commitment.into(),
                proof_generator_addr.into(),
                batch_merkle_root.into(),
                merkle_proof.into(),
                U256::from(verification_data_batch_index),
                pub_input.into(),
                Address::from_str(&batcher_eth_addr).unwrap(),
            );

            info!("Sending transaction to NoriTokenBridge contract...");
            let tx = call.send().await;

            match tx {
                Ok(tx) => {
                    let receipt = tx.get_receipt().await.unwrap_or_else(|err| {
                        error!("{}", err);
                        process::exit(1);
                    });

                    info!(
                        "NoriTokenBridge contract was updated! transaction hash: {}, gas cost: {}",
                        receipt.transaction_hash, receipt.gas_used
                    );
                }
                Err(err) => error!("NoriTokenBridge transaction failed!: {err}"),
            }
        }
        Command::Transfer { private_key, to, amount } => {
            let signer: PrivateKeySigner = private_key.parse().expect("Invalid private key");
            let wallet = EthereumWallet::from(signer);
            
            let provider = ProviderBuilder::new()
                .with_recommended_fillers()
                .wallet(wallet)
                .on_http(
                    reqwest::Url::parse(&eth_rpc_url)
                        .map_err(|err| err.to_string())
                        .unwrap(),
                );
            
            let to_addr = Address::from_str(&to).expect("Invalid to address");
            let amount_val = U256::from_str_radix(&amount, 10).unwrap_or_else(|_| U256::from_str(&amount).expect("Invalid amount"));

            let tx = TransactionRequest::default()
                .to(to_addr)
                .value(amount_val);
            
            info!("Sending {} wei to {}", amount_val, to_addr);
            
            let tx_hash = provider.send_transaction(tx).await
                .unwrap_or_else(|err| {
                    error!("Failed to send transaction: {}", err);
                    process::exit(1);
                })
                .watch()
                .await
                .unwrap_or_else(|err| {
                    error!("Failed to watch transaction: {}", err);
                    process::exit(1);
                });
                
            info!("Transaction confirmed: {}", tx_hash);
        }
    }

    if let Ok(elapsed) = now.elapsed() {
        info!("Time spent: {} s", elapsed.as_secs());
    }
}
