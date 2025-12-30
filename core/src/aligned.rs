use std::process;
use std::str::FromStr;

use aligned_sdk::{
    common::types::{
        AlignedVerificationData, FeeEstimationType, Network, ProvingSystemId, VerificationData,
        Wallet,
    },
    verification_layer::estimate_fee,
};

use alloy::primitives::Address;
use ethers::signers::Signer;
use futures::TryFutureExt;
use log::{error, info};

use crate::{
    proof::MinaProof,
    utils::{
        constants::{ANVIL_CHAIN_ID, HOLESKY_CHAIN_ID},
        wallet::WalletData,
    },
};

/// Submits a Mina Proof to Aligned's batcher and waits until the batch is verified.
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
        Network::Holesky => HOLESKY_CHAIN_ID,
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

    let max_fee = estimate_fee(eth_rpc_url, FeeEstimationType::Instant)
        .map_err(|err| err.to_string())
        .await?;
    let nonce =
        aligned_sdk::verification_layer::get_nonce_from_batcher(network.clone(), wallet.address())
            .await
            .map_err(|_| "Error while retrieving nonce from aligned batcher".to_string())?;

    info!("Max fee: {max_fee} gas");
    info!("Nonce: {nonce}");

    info!("Submitting {proof_name} into Aligned and waiting for the batch to be verified...");

    aligned_sdk::verification_layer::submit_and_wait_verification(
        eth_rpc_url,
        network.to_owned(),
        &verification_data,
        max_fee,
        wallet,
        nonce,
    )
    .await
    .map_err(|e| e.to_string())
}
