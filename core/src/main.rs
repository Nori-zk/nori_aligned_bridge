use clap::{Parser, Subcommand};
use log::{error, info};
use mina_bridge_core::{
    aligned, eth, mina, nori,
    proof::MinaProof,
    utils::{env::EnvironmentVariables, wallet::get_wallet},
};
use std::{process, time::SystemTime};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    SubmitState {
        #[arg(short, long)]
        devnet: bool,
        /// Write the proof into .proof and .pub files
        #[arg(short, long)]
        save_proof: bool,
    },
    SubmitAccount {
        /// Write the proof into .proof and .pub files
        #[arg(short, long)]
        save_proof: bool,
        /// Public key string of the account to verify
        public_key: String,
        token_id: String,

        /// Hash of the state to verify the account for
        state_hash: String,
    },
    UnlockNoriToken {
        /// Public key string of the account to unlock
        public_key: String,
        /// Token id string of the fungible token to unlock
        token_id: String,
        /// Amount of Nori tokens to unlock (in ether, supports decimals)
        #[arg(short, long)]
        to_unlock_amount: f64,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let now = SystemTime::now();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let EnvironmentVariables {
        mina_rpc_url,
        eth_network,
        state_settlement_addr,
        account_validation_addr,
        batcher_addr,
        batcher_eth_addr,
        eth_rpc_url,
        proof_generator_addr,
        keystore_path,
        private_key,
        nori_token_bridge_eth_addr,
        ..
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

    let wallet_data = get_wallet(
        &eth_network,
        keystore_path.as_deref(),
        private_key.as_deref(),
    )
    .unwrap_or_else(|err| {
        error!("{}", err);
        process::exit(1);
    });

    match cli.command {
        Command::SubmitState { devnet, save_proof } => {
            let (proof, pub_input) = mina::get_mina_proof_of_state(
                &mina_rpc_url,
                &eth_rpc_url,
                &state_settlement_addr,
                devnet,
            )
            .await
            .unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });

            let verification_data = aligned::submit(
                MinaProof::State((proof, pub_input.clone())),
                &eth_network,
                &proof_generator_addr,
                &batcher_addr,
                &eth_rpc_url,
                wallet_data.clone(),
                save_proof,
            )
            .await
            .unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });

            eth::update_chain(
                verification_data,
                &pub_input,
                &eth_network,
                &eth_rpc_url,
                wallet_data,
                &state_settlement_addr,
                &batcher_eth_addr,
            )
            .await
            .unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });
        }
        Command::SubmitAccount {
            save_proof,
            public_key,
            token_id,
            state_hash,
        } => {
            let (proof, pub_input) =
                mina::get_mina_proof_of_account(&public_key, &token_id, &state_hash, &mina_rpc_url)
                    .await
                    .unwrap_or_else(|err| {
                        error!("{}", err);
                        process::exit(1);
                    });

            let verification_data = aligned::submit(
                MinaProof::Account((proof, pub_input.clone())),
                &eth_network,
                &proof_generator_addr,
                &batcher_addr,
                &eth_rpc_url,
                wallet_data.clone(),
                save_proof,
            )
            .await
            .unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });

            if let Err(err) = eth::validate_account(
                verification_data,
                &pub_input,
                &eth_rpc_url,
                &account_validation_addr,
                &batcher_eth_addr,
            )
            .await
            {
                error!("Mina account {public_key} was not validated: {err}",);
            } else {
                info!("Mina account {public_key} was validated!");
            };
        }
        Command::UnlockNoriToken {
            to_unlock_amount,
            public_key,
            token_id,
        } => {
            // Convert floating ETH amount to 18-decimal base units
            if !to_unlock_amount.is_finite() || to_unlock_amount.is_sign_negative() {
                error!("to_unlock_amount must be a non-negative finite number");
                process::exit(1);
            }
            let wei_f = to_unlock_amount * 1e18_f64;
            if wei_f > (u128::MAX as f64) {
                error!("to_unlock_amount is too large");
                process::exit(1);
            }
            let to_unlock_amount_wei = wei_f.round() as u128;

            nori::unlock_nori_token(
                &mina_rpc_url,
                &eth_network,
                &batcher_addr,
                &eth_rpc_url,
                &proof_generator_addr,
                &batcher_eth_addr,
                keystore_path.as_deref(),
                private_key.as_deref(),
                &state_settlement_addr,
                &account_validation_addr,
                &nori_token_bridge_eth_addr,
                &public_key,
                &token_id,
                to_unlock_amount_wei,
            )
            .await;
        }
    }

    if let Ok(elapsed) = now.elapsed() {
        info!("Time spent: {} s", elapsed.as_secs());
    }
}
