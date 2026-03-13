use aligned_sdk::common::types::Network;
use alloy::{
    network::EthereumWallet,
    signers::local::{LocalSigner, PrivateKeySigner},
};
use log::info;
use std::env;
use zeroize::Zeroizing;

use crate::error::Error;

#[derive(Clone)]
pub struct WalletData {
    pub wallet: EthereumWallet,
    pub private_key_bytes: Vec<u8>,
}

impl WalletData {
    /// Loads wallet from env. Exactly one of `ETH_KEYSTORE_PATH` or `ETH_PRIVATE_KEY` must be set.
    pub fn from_env() -> Result<Self, Error> {
        let keystore_path = env::var("ETH_KEYSTORE_PATH").ok();
        let private_key = env::var("ETH_PRIVATE_KEY").ok();

        match (keystore_path.as_deref(), private_key.as_deref()) {
            (Some(_), Some(_)) => {
                Err(Error("Both ETH_KEYSTORE_PATH and ETH_PRIVATE_KEY are set. Choose only one.".to_string()))
            }
            (Some(path), None) => {
                let password = Zeroizing::new(
                    rpassword::prompt_password("Please enter your keystore password:")
                        .map_err(|err| Error(err.to_string()))?,
                );
                let signer = LocalSigner::decrypt_keystore(path, password)
                    .map_err(|err| Error(format!("invalid ETH_KEYSTORE_PATH: {err}")))?;
                let bytes = signer.to_bytes().to_vec();
                Ok(WalletData {
                    wallet: EthereumWallet::new(signer),
                    private_key_bytes: bytes,
                })
            }
            (None, Some(key)) => {
                let signer: PrivateKeySigner = key
                    .parse()
                    .map_err(|_| Error("invalid ETH_PRIVATE_KEY".to_string()))?;
                let bytes = signer.to_bytes().to_vec();
                Ok(WalletData {
                    wallet: EthereumWallet::new(signer),
                    private_key_bytes: bytes,
                })
            }
            (None, None) => {
                Err(Error("Neither ETH_KEYSTORE_PATH nor ETH_PRIVATE_KEY is set.".to_string()))
            }
        }
    }
}

/// Returns the `Wallet` struct defined in the `alloy` crate and the private key bytes.
/// This wallet is used to sign Ethereum example contract deployments.
/// The private key bytes are used to reconstruct the wallet in `ethers` for aligned-sdk compatibility.
///
/// If `keystore_path` is defined it stops execution, prompts on the TTY and then reads the password from TTY.
///
/// Returns `Err` if:
/// - `keystore_path` is not a valid path to a keystore
/// - `keystore_path` is defined and the password read from the TTY is not valid
/// - `private_key` is not a valid Ethereum private key
/// - Both `keystore_path` and `private_key` are defined
pub fn get_wallet(
    network: &Network,
    keystore_path: Option<&str>,
    private_key: Option<&str>,
) -> Result<WalletData, String> {
    if keystore_path.is_some() && private_key.is_some() {
        return Err(
            "Both keystore and private key env. variables are defined. Choose only one."
                .to_string(),
        );
    }

    if let Some(keystore_path) = keystore_path {
        let password = Zeroizing::new(
            rpassword::prompt_password("Please enter your keystore password:")
                .map_err(|err| err.to_string())?,
        );
        let signer = LocalSigner::decrypt_keystore(keystore_path, password)
            .map_err(|err| err.to_string())?;
        let bytes = signer.to_bytes().to_vec();
        Ok(WalletData {
            wallet: EthereumWallet::new(signer),
            private_key_bytes: bytes,
        })
    } else if let Some(private_key) = private_key {
        let signer: PrivateKeySigner = private_key
            .parse()
            .map_err(|_| "Failed to get signer".to_string())?;
        let bytes = signer.to_bytes().to_vec();
        Ok(WalletData {
            wallet: EthereumWallet::new(signer),
            private_key_bytes: bytes,
        })
    } else {
        return Err(
            "couldn't find KEYSTORE_PATH or PRIVATE_KEY."
                .to_string(),
        );
    }
}
