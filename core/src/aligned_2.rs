//! Aligned proof submission and verification primitives.
//!
//! Each function does exactly one thing. Resist the urge to combine them — the value
//! of keeping submission and verification separate is that each can be composed, retried,
//! and scaled independently. Less responsibility per call means fewer reasons to change
//! and fewer failure modes to reason about.

use std::time::Duration;

use aligned_sdk::common::types::{AlignedVerificationData, Network, VerificationData, Wallet};

use log::{error, info};

/// Standard batcher max proof size (4 MiB), matches public batcher deployments.
const MAX_PROOF_SIZE: usize = 4 * 1024 * 1024;

/// CBOR framing/metadata overhead added by the batcher when serializing VerificationData.
const CBOR_OVERHEAD_BYTES: usize = 1024;

/// Returns an upper-bound estimate of the CBOR-encoded size of `data`.
/// Returns `Err` if the estimate exceeds `MAX_PROOF_SIZE`.
fn validate_cbor_size(data: &VerificationData) -> Result<usize, String> {
    let mut size = data.proof.len();
    if let Some(ref v) = data.pub_input {
        size += v.len();
    }
    if let Some(ref v) = data.verification_key {
        size += v.len();
    }
    if let Some(ref v) = data.vm_program_code {
        size += v.len();
    }
    size += CBOR_OVERHEAD_BYTES;

    if size > MAX_PROOF_SIZE {
        return Err(format!(
            "Proof payload too large: estimated {} bytes exceeds max {} bytes",
            size, MAX_PROOF_SIZE
        ));
    }

    Ok(size)
}

/// Submits a proof to the Aligned batcher. Decoupled from verification — returns immediately
/// without waiting for on-chain confirmation. Also validates the proof payload size before
/// submission, which the underlying SDK does not do.
/// Note: guard against concurrent submissions (no more than one in-flight at a time).
pub async fn submit(
    network: &Network,
    verification_data: &VerificationData,
    max_fee: ethers::types::U256,
    wallet: Wallet<ethers::core::k256::ecdsa::SigningKey>,
    nonce: ethers::types::U256,
    proof_name: &str,
) -> Result<AlignedVerificationData, String> {
    let cbor_size = validate_cbor_size(verification_data)?;
    info!("Submitting {} into Aligned (cbor ~{} bytes)...", proof_name, cbor_size);

    let result = aligned_sdk::verification_layer::submit(
        network.to_owned(),
        verification_data,
        max_fee,
        wallet,
        nonce,
    )
    .await
    .map_err(|e| e.to_string());

    match &result {
        Ok(data) => info!(
            "Proof submitted. Batch merkle root: {:?}",
            hex::encode(&data.batch_merkle_root)
        ),
        Err(e) => error!("Proof submission failed: {}", e),
    }

    result
}

/// Single-shot check of whether a proof is verified on-chain. Decoupled from submission —
/// the caller drives when and how often this is called. Returns Ok(true) if verified,
/// Ok(false) if not yet, Err on timeout or RPC failure.
pub async fn check_verification(
    eth_rpc_url: &str,
    network: &Network,
    aligned_verification_data: &AlignedVerificationData,
    timeout: Duration,
) -> Result<bool, String> {
    tokio::time::timeout(
        timeout,
        aligned_sdk::verification_layer::is_proof_verified(
            aligned_verification_data,
            network.to_owned(),
            eth_rpc_url,
        ),
    )
    .await
    .map_err(|_| "is_proof_verified timed out".to_string())?
    .map_err(|e| e.to_string())
}
