//! Aligned proof submission and verification primitives.
//!
//! Each function does exactly one thing. Resist the urge to combine them — the value
//! of keeping submission and verification separate is that each can be composed, retried,
//! and scaled independently. Less responsibility per call means fewer reasons to change
//! and fewer failure modes to reason about.

use std::time::Duration;

use aligned_sdk::common::types::{AlignedVerificationData, Network, VerificationData, Wallet};
use alloy::primitives::{Address, FixedBytes};
use log::{error, info};
use reqwest::Url;

use crate::eth_2;
use crate::rpcs::errors::{classify_submit_error, classify_verification_error, AlignedError};

/// Standard batcher max proof size (4 MiB), matches public batcher deployments.
const MAX_PROOF_SIZE: usize = 4 * 1024 * 1024;

/// CBOR framing/metadata overhead added by the batcher when serializing VerificationData.
const CBOR_OVERHEAD_BYTES: usize = 1024;

/// Returns an upper-bound estimate of the CBOR-encoded size of `data`.
/// Returns `Err` if the estimate exceeds `MAX_PROOF_SIZE`.
fn validate_cbor_size(data: &VerificationData) -> Result<usize, AlignedError> {
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
        return Err(AlignedError::PayloadTooLarge(format!(
            "estimated {} bytes exceeds max {} bytes",
            size, MAX_PROOF_SIZE
        )));
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
) -> Result<AlignedVerificationData, AlignedError> {
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
    .map_err(classify_submit_error);

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
///
/// Returns a bare bool which is ambiguous: `false` can mean "not yet" or "definitively
/// not verified". Use `check_verification` instead, which reads `batchesState.responded`
/// to disambiguate.
#[deprecated(note = "use check_verification which disambiguates pending vs failed via batchesState.responded")]
pub async fn check_verification_old(
    eth_rpc_url: &str,
    network: &Network,
    aligned_verification_data: &AlignedVerificationData,
    timeout: Duration,
) -> Result<bool, AlignedError> {
    tokio::time::timeout(
        timeout,
        aligned_sdk::verification_layer::is_proof_verified(
            aligned_verification_data,
            network.to_owned(),
            eth_rpc_url,
        ),
    )
    .await
    .map_err(|_| AlignedError::RpcUnreachable("is_proof_verified timed out".into()))?
    .map_err(classify_verification_error)
}

/// Result of checking whether an Aligned batch proof has been verified on-chain.
///
/// The Aligned SDK's `is_proof_verified` returns a bare `bool`, collapsing three distinct
/// situations into a single `false`. This enum disambiguates them by combining the SDK call
/// with a direct read of `batchesState(batchIdentifier).responded` on the
/// AlignedLayerServiceManager contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchVerificationStatus {
    /// `verifyBatchInclusion` returned `true`. The proof is verified on-chain.
    Verified,
    /// The Aligned aggregator has not yet responded to this batch. The proof may still
    /// be verified in the future. Keep polling.
    Pending,
    /// The Aligned aggregator responded but `verifyBatchInclusion` returned `false`.
    /// The merkle inclusion proof failed. This is a definitive, non-recoverable failure.
    Failed,
}

/// Single-shot check of whether a proof is verified on-chain, with deterministic
/// disambiguation of "pending" vs "definitively failed".
///
/// 1. Calls `verifyBatchInclusion` via the SDK's `is_proof_verified`.
///    - `Ok(true)` -> `Verified`
///    - `Err(...)` -> propagated as `AlignedError`
///    - `Ok(false)` -> ambiguous, proceed to step 2
/// 2. Reads `batchesState(batchIdentifier).responded` from the AlignedLayerServiceManager.
///    - `responded == false` -> `Pending` (aggregator hasn't responded yet)
///    - `responded == true` -> `Failed` (aggregator responded, merkle proof failed)
pub async fn check_verification(
    eth_rpc_url: &Url,
    aligned_service_manager_addr: Address,
    network: &Network,
    aligned_verification_data: &AlignedVerificationData,
    timeout: Duration,
) -> Result<BatchVerificationStatus, AlignedError> {
    #[allow(deprecated)]
    let verified = check_verification_old(
        eth_rpc_url.as_str(),
        network,
        aligned_verification_data,
        timeout,
    )
    .await?;

    if verified {
        return Ok(BatchVerificationStatus::Verified);
    }

    // verifyBatchInclusion returned false — disambiguate by reading batchesState.
    let batch_merkle_root = FixedBytes::<32>::from(
        aligned_verification_data.batch_merkle_root,
    );
    let proof_generator_addr = FixedBytes::<20>::from(
        aligned_verification_data.verification_data_commitment.proof_generator_addr,
    );

    let responded = eth_2::is_batch_responded(
        eth_rpc_url,
        aligned_service_manager_addr,
        batch_merkle_root,
        proof_generator_addr,
    )
    .await
    .map_err(AlignedError::BatcherContractCallFailed)?;

    if responded {
        Ok(BatchVerificationStatus::Failed)
    } else {
        Ok(BatchVerificationStatus::Pending)
    }
}
