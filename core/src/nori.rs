use alloy::{
    primitives::{Address, U256},
    providers::ProviderBuilder,
    sol_types::sol,
};
use log::{error, info};
use std::{process, str::FromStr};

use crate::{
    sdk::{
        get_bridged_chain_tip_state_hash, update_bridge_chain, validate_account,
        AccountVerificationData,
    },
    utils::wallet::get_wallet,
};

// Generate NoriTokenBridge contract bindings
sol!(
    #[allow(clippy::too_many_arguments)]
    #[sol(rpc)]
    NoriTokenBridge,
    "abi/NoriTokenBridge.json"
);

/// Unlocks Nori tokens on the bridge
#[allow(clippy::too_many_arguments)]
pub async fn unlock_nori_token(
    mina_rpc_url: &str,
    eth_network: &aligned_sdk::common::types::Network,
    batcher_addr: &str,
    eth_rpc_url: &str,
    proof_generator_addr: &str,
    batcher_eth_addr: &str,
    keystore_path: Option<&str>,
    private_key: Option<&str>,
    state_settlement_addr: &str,
    account_validation_addr: &str,
    nori_token_bridge_eth_addr: &str,
    nori_token_storage_zkapp_addr: &str,
    nori_token_controller_token_id: &str,
    to_unlock_amount: u128,
) {
    let wallet_data = get_wallet(eth_network, keystore_path, private_key).unwrap_or_else(|err| {
        error!("{}", err);
        process::exit(1);
    });

    let state_verification_result = update_bridge_chain(
        mina_rpc_url,
        eth_network,
        state_settlement_addr,
        batcher_addr,
        eth_rpc_url,
        proof_generator_addr,
        wallet_data.clone(),
        batcher_eth_addr,
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
        get_bridged_chain_tip_state_hash(state_settlement_addr, eth_rpc_url)
            .await
            .unwrap_or_else(|err| {
                error!("{}", err);
                process::exit(1);
            });

    info!("tip state hash: {}", &tip_state_hash);

    let AccountVerificationData {
        proof_commitment,
        proving_system_aux_data_commitment,
        proof_generator_addr: proof_generator_addr_bytes,
        batch_merkle_root,
        merkle_proof,
        verification_data_batch_index,
        pub_input,
    } = validate_account(
        nori_token_storage_zkapp_addr,
        nori_token_controller_token_id,
        &tip_state_hash,
        mina_rpc_url,
        eth_network,
        account_validation_addr,
        batcher_addr,
        eth_rpc_url,
        proof_generator_addr,
        batcher_eth_addr,
        wallet_data.clone(),
        false,
    )
    .await
    .unwrap_or_else(|err| {
        error!("{}", err);
        process::exit(1);
    });

    info!("Creating contract instance");
    let provider = ProviderBuilder::new()
        .wallet(wallet_data.wallet)
        .connect_http(
            reqwest::Url::parse(eth_rpc_url)
                .map_err(|err| err.to_string())
                .unwrap(),
        );

    let contract = NoriTokenBridge::new(
        Address::from_str(nori_token_bridge_eth_addr).unwrap(),
        provider,
    );

    let to_unlock_amount = U256::from(to_unlock_amount);
    info!("Unlock params:");
    info!("toUnlockAmount: {}", to_unlock_amount);
    info!("proof_commitment: {}", hex::encode(proof_commitment));
    info!(
        "proving_system_aux_data_commitment: {}",
        hex::encode(proving_system_aux_data_commitment)
    );
    info!(
        "proof_generator_addr: {}",
        hex::encode(proof_generator_addr_bytes)
    );
    info!("batch_merkle_root: {}", hex::encode(batch_merkle_root));
    info!("merkle_proof: {}", hex::encode(&merkle_proof));
    info!(
        "verification_data_batch_index: {}",
        verification_data_batch_index
    );
    info!("pub_input: {}", hex::encode(&pub_input));
    info!("batcher_eth_addr: {}", &batcher_eth_addr);

    let call = contract.unlockTokens(
        to_unlock_amount,
        proof_commitment.into(),
        proving_system_aux_data_commitment.into(),
        proof_generator_addr_bytes.into(),
        batch_merkle_root.into(),
        merkle_proof.into(),
        U256::from(verification_data_batch_index),
        pub_input.into(),
        Address::from_str(batcher_eth_addr).unwrap(),
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
                receipt.transaction_hash,
                receipt.gas_used
            );
        }
        Err(err) => error!("NoriTokenBridge transaction failed!: {err}"),
    }
}
