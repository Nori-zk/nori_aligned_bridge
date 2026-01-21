use std::str::FromStr;
use std::time::{Duration, Instant};
use std::{process, string};

use aligned_sdk::{
    common::types::{
        AlignedVerificationData, FeeEstimationType, Network, ProvingSystemId, VerificationData,
        Wallet,
    },
    verification_layer::estimate_fee,
};

use alloy::primitives::Address;
use ethers::signers::Signer;
use ethers::types::U256;
use futures::TryFutureExt;
use log::{error, info, warn};

use crate::utils::constants::{HOODI_CHAIN_ID, SEPOLIA_CHAIN_ID};
use crate::{
    proof::MinaProof,
    utils::{constants::ANVIL_CHAIN_ID, wallet::WalletData},
};

/// Submission mode for Aligned verification layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubmissionMode {
    /// Use `submit_and_wait_verification` (default, blocking wait via websocket).
    #[default]
    SubmitAndWait,
    /// Use `submit` followed by polling `is_proof_verified`.
    SubmitWithPolling,
}

impl std::str::FromStr for SubmissionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "submit_and_wait" | "default" | "" => Ok(SubmissionMode::SubmitAndWait),
            "submit_with_polling" | "polling" => Ok(SubmissionMode::SubmitWithPolling),
            _ => Err(format!(
                "Unknown submission mode: '{}'. Valid values: 'submit_and_wait', 'submit_with_polling'",
                s
            )),
        }
    }
}

/// Configuration for the polling-based submission mode.
#[derive(Debug, Clone)]
pub struct PollingConfig {
    /// Interval between polling attempts.
    pub poll_interval: Duration,
    /// Maximum time to wait for verification before timing out.
    pub timeout: Duration,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(10),
            timeout: Duration::from_secs(600), // 10 minutes default
        }
    }
}

impl PollingConfig {
    /// Creates a new PollingConfig from environment variables.
    /// Uses defaults if env vars are not set.
    pub fn from_env() -> Self {
        let poll_interval = std::env::var("ALIGNED_POLL_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(10));

        let timeout = std::env::var("ALIGNED_POLL_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(600));

        Self {
            poll_interval,
            timeout,
        }
    }
}

/// Returns the submission mode from the environment variable `ALIGNED_SUBMISSION_MODE`.
/// Defaults to `SubmitAndWait` if not set or invalid.
pub fn get_submission_mode_from_env() -> SubmissionMode {
    std::env::var("ALIGNED_SUBMISSION_MODE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default()
}

/// Submits a Mina Proof to Aligned's batcher and waits until the batch is verified.
/// Uses the submission mode from environment variable `ALIGNED_SUBMISSION_MODE`.
#[allow(clippy::too_many_arguments)]
pub async fn submit(
    proof: MinaProof,
    network: &Network,
    proof_generator_addr: &str,
    _batcher_addr: &str,
    eth_rpc_url: &str,
    wallet: WalletData,
    save_proof: bool,
) -> Result<AlignedVerificationData, String> {
    let submission_mode = get_submission_mode_from_env();
    submit_with_mode(
        proof,
        network,
        proof_generator_addr,
        _batcher_addr,
        eth_rpc_url,
        wallet,
        save_proof,
        submission_mode,
    )
    .await
}

/// Submits a Mina Proof to Aligned's batcher and waits until the batch is verified.
/// Allows explicit specification of the submission mode.
#[allow(clippy::too_many_arguments)]
pub async fn submit_with_mode(
    proof: MinaProof,
    network: &Network,
    proof_generator_addr: &str,
    _batcher_addr: &str,
    eth_rpc_url: &str,
    wallet: WalletData,
    save_proof: bool,
    submission_mode: SubmissionMode,
) -> Result<AlignedVerificationData, String> {
    let (proof, pub_input, proving_system, proof_name, file_name) = match proof {
        MinaProof::State((proof, pub_input)) => {
            let proof = bincode::serialize(&proof)
                .map_err(|err| format!("Failed to serialize state proof: {err}"))?;
            let pub_input = bincode::serialize(&pub_input)
                .map_err(|err| format!("Failed to serialize public inputs: {err}"))?;
            (
                proof,
                pub_input,
                ProvingSystemId::Mina,
                "Mina Proof of State",
                "mina_state",
            )
        }
        MinaProof::Account((proof, pub_input)) => {
            let proof = bincode::serialize(&proof)
                .map_err(|err| format!("Failed to serialize state proof: {err}"))?;
            let pub_input = bincode::serialize(&pub_input)
                .map_err(|err| format!("Failed to serialize public inputs: {err}"))?;
            (
                proof,
                pub_input,
                ProvingSystemId::MinaAccount,
                "Mina Proof of Account",
                "mina_account",
            )
        }
    };

    if save_proof {
        std::fs::write(format!("./{file_name}.pub"), &pub_input).unwrap_or_else(|err| {
            error!("{}", err);
            process::exit(1);
        });
        std::fs::write(format!("./{file_name}.proof"), &proof).unwrap_or_else(|err| {
            error!("{}", err);
            process::exit(1);
        });
    }

    let proof_generator_addr =
        Address::from_str(proof_generator_addr).map_err(|err| err.to_string())?;

    let chain_id = match network {
        Network::Devnet => ANVIL_CHAIN_ID,
        Network::Sepolia => SEPOLIA_CHAIN_ID,
        Network::Hoodi => HOODI_CHAIN_ID,
        _ => ANVIL_CHAIN_ID,
    };

    let wallet = Wallet::from_bytes(&wallet.private_key_bytes)
        .map_err(|err| format!("Failed to create wallet from bytes: {err}"))?
        .with_chain_id(chain_id);

    // Aligned SDK expects ethers types for VerificationData.
    // Since we don't have ethers imported as a dependency, we rely on aligned-sdk types.
    // However, aligned-sdk uses ethers types internally.
    // The proof_generator_addr in VerificationData is ethers::types::Address.
    // We need to convert alloy::Address to ethers::Address.
    // We can do this by converting to bytes and then to ethers address.

    let proof_generator_addr_bytes: [u8; 20] = proof_generator_addr.into_array();
    let proof_generator_addr_ethers = ethers::types::H160::from(proof_generator_addr_bytes);

    let verification_data = VerificationData {
        proving_system,
        proof,
        pub_input: Some(pub_input),
        // Use this instead of `None` to force Aligned to include the commitment to the proving system ID (valid for Aligned 0.7.0)
        verification_key: Some(vec![]),
        vm_program_code: None,
        proof_generator_addr: proof_generator_addr_ethers,
    };

    let batcher_fee_estimation_type =
        std::env::var("BATCHER_FEE_ESTM_TYPE").unwrap_or("0".to_string());

    let max_batcher_fee_env = std::env::var("BATCHER_MAX_FEE")
        .unwrap_or_else(|_| "0".to_string())
        .parse::<U256>()
        .unwrap();
    let max_fee = match batcher_fee_estimation_type.as_str() {
        "0" => {
            let fee = estimate_fee(eth_rpc_url, FeeEstimationType::Default)
                .map_err(|err| err.to_string())
                .await?;
            fee
        }
        "1" => {
            let fee = estimate_fee(eth_rpc_url, FeeEstimationType::Instant)
                .map_err(|err| err.to_string())
                .await?;
            fee
        }
        "2" => {
            if max_batcher_fee_env == U256::from(0) {
                panic!("BATCHER_MAX_FEE cannot be 0 when BATCHER_FEE_ESTM_TYPE is 2(Custom)");
            }
            max_batcher_fee_env
        },
        _ => panic!(
            "Invalid batcher fee estimation type: {}",
            batcher_fee_estimation_type
        ),
    };

    let nonce =
        aligned_sdk::verification_layer::get_nonce_from_batcher(network.clone(), wallet.address())
            .await
            .map_err(|_| "Error while retrieving nonce from aligned batcher".to_string())?;

    info!("Max fee: {max_fee} gas");
    info!("Nonce: {nonce}");

    match submission_mode {
        SubmissionMode::SubmitAndWait => {
            submit_and_wait_verification_with_timing(
                eth_rpc_url,
                network,
                &verification_data,
                max_fee,
                wallet,
                nonce,
                &proof_name,
            )
            .await
        }
        SubmissionMode::SubmitWithPolling => {
            let polling_config = PollingConfig::from_env();
            submit_with_polling(
                eth_rpc_url,
                network,
                &verification_data,
                max_fee,
                wallet,
                nonce,
                &proof_name,
                &polling_config,
            )
            .await
        }
    }
}

/// Submits a proof using `submit_and_wait_verification` with timing logs.
/// NOTE: Please Try Check if the previous submission was successful before submitting again.
async fn submit_and_wait_verification_with_timing(
    eth_rpc_url: &str,
    network: &Network,
    verification_data: &VerificationData,
    max_fee: ethers::types::U256,
    wallet: Wallet<ethers::core::k256::ecdsa::SigningKey>,
    nonce: ethers::types::U256,
    proof_name: &str,
) -> Result<AlignedVerificationData, String> {
    info!(
        "Submitting {} into Aligned (mode: submit_and_wait) and waiting for verification...",
        proof_name
    );

    let start = Instant::now();

    let result = aligned_sdk::verification_layer::submit_and_wait_verification(
        eth_rpc_url,
        network.to_owned(),
        verification_data,
        max_fee,
        wallet,
        nonce,
    )
    .await
    .map_err(|e| e.to_string());

    let elapsed = start.elapsed();
    let elapsed_ms = elapsed.as_millis();
    let elapsed_secs = elapsed.as_secs_f64();

    match &result {
        Ok(data) => {
            info!(
                "Proof verification completed successfully. Network: {:?}, Batch merkle root: {:?}, Elapsed: {:.2}s ({} ms)",
                network,
                hex::encode(&data.batch_merkle_root),
                elapsed_secs,
                elapsed_ms
            );
        }
        Err(e) => {
            error!(
                "Proof verification failed after {:.2}s ({} ms): {}",
                elapsed_secs, elapsed_ms, e
            );
        }
    }

    result
}

/// Submits a proof using `submit` followed by polling `is_proof_verified`.
/// NOTE: Please Try to Check if the previous submission was successful before submitting again.
async fn submit_with_polling(
    eth_rpc_url: &str,
    network: &Network,
    verification_data: &VerificationData,
    max_fee: ethers::types::U256,
    wallet: Wallet<ethers::core::k256::ecdsa::SigningKey>,
    nonce: ethers::types::U256,
    proof_name: &str,
    polling_config: &PollingConfig,
) -> Result<AlignedVerificationData, String> {
    info!(
        "Submitting {} into Aligned (mode: submit_with_polling)...",
        proof_name
    );
    info!(
        "Polling config: interval={}s, timeout={}s",
        polling_config.poll_interval.as_secs(),
        polling_config.timeout.as_secs()
    );

    let total_start = Instant::now();
    let submit_start = Instant::now();

    // Submit the proof
    let aligned_verification_data = aligned_sdk::verification_layer::submit(
        network.to_owned(),
        verification_data,
        max_fee,
        wallet,
        nonce,
    )
    .await
    .map_err(|e| e.to_string())?;

    let submit_elapsed = submit_start.elapsed();
    info!(
        "Proof submitted successfully. Submit time: {:.2}s ({} ms). Batch merkle root: {:?}",
        submit_elapsed.as_secs_f64(),
        submit_elapsed.as_millis(),
        hex::encode(&aligned_verification_data.batch_merkle_root)
    );

    // Poll for verification
    let poll_start = Instant::now();
    let mut poll_count = 0u32;

    loop {
        let total_elapsed = total_start.elapsed();
        if total_elapsed >= polling_config.timeout {
            let msg = format!(
                "Verification polling timed out after {:.2}s ({} polls). Submit time: {:.2}s, Poll time: {:.2}s",
                total_elapsed.as_secs_f64(),
                poll_count,
                submit_elapsed.as_secs_f64(),
                poll_start.elapsed().as_secs_f64()
            );
            error!("{}", msg);
            return Err(msg);
        }

        poll_count += 1;

        match aligned_sdk::verification_layer::is_proof_verified(
            &aligned_verification_data,
            network.to_owned(),
            eth_rpc_url,
        )
        .await
        {
            Ok(true) => {
                let poll_elapsed = poll_start.elapsed();
                let total_elapsed = total_start.elapsed();
                info!(
                    "Proof verification completed successfully after {} polls. Network: {:?}, Batch merkle root: {:?}",
                    poll_count,
                    network,
                    hex::encode(&aligned_verification_data.batch_merkle_root)
                );
                info!(
                    "Timing breakdown: Submit: {:.2}s, Verification wait: {:.2}s, Total: {:.2}s ({} ms)",
                    submit_elapsed.as_secs_f64(),
                    poll_elapsed.as_secs_f64(),
                    total_elapsed.as_secs_f64(),
                    total_elapsed.as_millis()
                );
                return Ok(aligned_verification_data);
            }
            Ok(false) => {
                info!(
                    "Poll {}: Proof not yet verified, waiting {}s before next poll...",
                    poll_count,
                    polling_config.poll_interval.as_secs()
                );
            }
            Err(e) => {
                warn!(
                    "Poll {}: Error checking verification status: {}. Retrying...",
                    poll_count, e
                );
            }
        }

        tokio::time::sleep(polling_config.poll_interval).await;
    }
}
