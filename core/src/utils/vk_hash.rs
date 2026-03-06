//! Verification key hash computation matching NoriTokenBridge.sol:
//!   uint256(keccak256(abi.encode(account.zkapp.verificationKey)))
//!
//! Use to verify off-chain that a given Account's zkApp verification key
//! matches the contract's accepted value (e.g. `noriStorageZkappAcctVk`).

use alloy_sol_types::{SolType, SolValue};
use sha3::{Digest, Keccak256};

use crate::{mina, sol::account::MinaAccountValidationExample};

/// Computes the same verification key hash as the contract: keccak256(abi.encode(verificationKey)).
///
/// # Arguments
/// * `encoded_account` - ABI-encoded Account bytes (same as `encodedAccount` in the contract,
///   or `pubInput[40..]` when pubInput is ledger_hash (32) + 8 bytes + encoded Account).
///
/// # Returns
/// The 32-byte hash, or an error if decoding fails.
pub fn vk_hash_from_encoded_zkapp_acct(
    encoded_account: &[u8],
) -> Result<[u8; 32], String> {
    let account =
        <MinaAccountValidationExample::Account as SolType>::abi_decode(encoded_account, false)
            .map_err(|e| format!("ABI decode Account failed: {}", e))?;

    let vk_encoded = account.zkapp.verificationKey.abi_encode();
    let hash = Keccak256::digest(&vk_encoded);
    Ok(hash.into())
}

/// Same as `vk_hash_from_encoded_zkapp_acct` but returns the hash as a hex string with `0x` prefix.
pub fn vk_hash_hex_from_encoded_zkapp_acct(
    encoded_account: &[u8],
) -> Result<String, String> {
    let hash = vk_hash_from_encoded_zkapp_acct(encoded_account)?;
    Ok(format!("0x{}", hex::encode(hash)))
}

/// Fetches the Mina account from the node and computes its zkApp verification key hash.
///
/// Same encoding path as `get_mina_proof_of_account`: query account → convert to Solidity Account → abi_encode → hash.
///
/// # Arguments
/// * `rpc_url` - Mina GraphQL RPC URL
/// * `state_hash` - State hash to query the account at (e.g. tip state hash)
/// * `public_key` - Mina account public key (address)
/// * `token_id` - Token ID for the account
pub async fn vk_hash_from_mina_account(
    rpc_url: &str,
    state_hash: &str,
    public_key: &str,
    token_id: &str,
) -> Result<[u8; 32], String> {
    println!("Querying account from Mina RPC");
    let (account, _ledger_hash, _merkle_path) =
        mina::query_account(rpc_url, state_hash, public_key, token_id).await?;
    let encoded_account = MinaAccountValidationExample::Account::try_from(&account)?.abi_encode();
    vk_hash_from_encoded_zkapp_acct(&encoded_account)
}

/// Same as `vk_hash_from_mina_account` but returns the hash as a hex string with `0x` prefix.
pub async fn vk_hash_hex_from_mina_account(
    rpc_url: &str,
    state_hash: &str,
    public_key: &str,
    token_id: &str,
) -> Result<String, String> {
    let hash = vk_hash_from_mina_account(rpc_url, state_hash, public_key, token_id).await?;
    Ok(format!("0x{}", hex::encode(hash)))
}
