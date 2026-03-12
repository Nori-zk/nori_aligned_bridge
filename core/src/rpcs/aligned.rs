use std::path::PathBuf;
use std::time::Duration;

use aligned_sdk::common::types::{
    AlignedVerificationData, FeeEstimationType, Network, ProvingSystemId, VerificationData, Wallet,
};
use alloy::primitives::Address;
use alloy::signers::local::PrivateKeySigner;
use ethers::core::k256::ecdsa::SigningKey;
use ethers::signers::Signer;
use ethers::types::{H160, U256};

use super::error::Error;
use crate::aligned_2;
use crate::proof::MinaProof;
use crate::utils::constants::{
    ANVIL_CHAIN_ID, HOLESKY_CHAIN_ID, HOODI_CHAIN_ID, MAINNET_CHAIN_ID, SEPOLIA_CHAIN_ID,
};

/// Parses an `ALIGNED_NETWORK` env string into an aligned SDK `Network`.
fn parse_aligned_network(s: &str) -> Result<Network, Error> {
    match s.trim() {
        "devnet" => Ok(Network::Devnet),
        "holesky" => Ok(Network::Holesky),
        "holesky_stage" => Ok(Network::HoleskyStage),
        "hoodi" => Ok(Network::Hoodi),
        "mainnet" => Ok(Network::Mainnet),
        "mainnet_stage" => Ok(Network::MainnetStage),
        "sepolia" => Ok(Network::Sepolia),
        other => Err(Error(format!(
            "invalid ALIGNED_NETWORK: '{other}', expected one of: \
             devnet, holesky, holesky_stage, hoodi, mainnet, mainnet_stage, sepolia"
        ))),
    }
}

/// Returns the Ethereum chain ID for a known aligned `Network`.
/// `Custom` is not handled here — it must be resolved via `ETH_CHAIN_ID` in `from_env`.
fn chain_id_for_known_network(network: &Network) -> Option<u64> {
    match network {
        Network::Devnet => Some(ANVIL_CHAIN_ID),
        Network::Holesky | Network::HoleskyStage => Some(HOLESKY_CHAIN_ID),
        Network::Hoodi => Some(HOODI_CHAIN_ID),
        Network::Mainnet | Network::MainnetStage => Some(MAINNET_CHAIN_ID),
        Network::Sepolia => Some(SEPOLIA_CHAIN_ID),
        Network::Custom(..) => None,
    }
}

/// Exclusive credential for signing Ethereum transactions.
/// Validated at construction time — a `PrivateKey` variant holds parsed key bytes,
/// a `KeystorePath` variant holds a path verified to exist on disk.
pub enum EthSigner {
    /// Validated private key bytes (32 bytes, parsed from hex at construction).
    PrivateKey(Vec<u8>),
    /// Path to a keystore file, verified to exist at construction.
    KeystorePath(PathBuf),
}

impl EthSigner {
    /// Reads `ETH_PRIVATE_KEY` and `ETH_KEYSTORE_PATH` from env.
    /// Exactly one must be set. The private key is parsed immediately;
    /// the keystore path is checked for existence.
    fn from_env() -> Result<Self, Error> {
        let private_key = std::env::var("ETH_PRIVATE_KEY").ok();
        let keystore_path = std::env::var("ETH_KEYSTORE_PATH").ok();
        match (private_key, keystore_path) {
            (Some(_), Some(_)) => Err(Error(
                "both ETH_PRIVATE_KEY and ETH_KEYSTORE_PATH are set; choose one".to_string(),
            )),
            (None, None) => Err(Error(
                "neither ETH_PRIVATE_KEY nor ETH_KEYSTORE_PATH is set".to_string(),
            )),
            (Some(pk), None) => {
                let signer: PrivateKeySigner = pk.parse().map_err(|e| {
                    Error(format!("invalid ETH_PRIVATE_KEY: {e}"))
                })?;
                Ok(EthSigner::PrivateKey(signer.to_bytes().to_vec()))
            }
            (None, Some(ks)) => {
                let path = PathBuf::from(&ks);
                if !path.exists() {
                    return Err(Error(format!(
                        "ETH_KEYSTORE_PATH does not exist: {ks}"
                    )));
                }
                Ok(EthSigner::KeystorePath(path))
            }
        }
    }

    /// Builds an ethers wallet from the validated credential, bound to the given chain ID.
    fn to_wallet(&self, chain_id: u64) -> Result<Wallet<SigningKey>, Error> {
        let key_bytes = match self {
            EthSigner::PrivateKey(bytes) => bytes.clone(),
            EthSigner::KeystorePath(path) => {
                let password = rpassword::prompt_password("Enter keystore password:")
                    .map_err(|e| Error(format!("failed to read keystore password: {e}")))?;
                let signer =
                    alloy::signers::local::LocalSigner::decrypt_keystore(path, password)
                        .map_err(|e| Error(format!("failed to decrypt keystore: {e}")))?;
                signer.to_bytes().to_vec()
            }
        };

        Wallet::from_bytes(&key_bytes)
            .map_err(|e| Error(format!("failed to create ethers wallet: {e}")))?
            .with_chain_id(chain_id)
            .try_into()
            .map_err(|_| Error("failed to set chain_id on wallet".to_string()))
    }
}

pub struct AlignedRPC {
    aligned_network: Network,
    aligned_proof_generator_addr: Address,
    eth_rpc_url: String,
    eth_chain_id: u64,
    eth_signer: EthSigner,
}

impl AlignedRPC {
    pub fn from_env() -> Result<Self, Error> {
        let aligned_network = parse_aligned_network(
            &std::env::var("ALIGNED_NETWORK")
                .map_err(|e| Error(format!("ALIGNED_NETWORK: {e}")))?,
        )?;

        let aligned_proof_generator_addr = std::env::var("ALIGNED_PROOF_GENERATOR_ADDR")
            .map_err(|e| Error(format!("ALIGNED_PROOF_GENERATOR_ADDR: {e}")))?
            .trim()
            .parse::<Address>()
            .map_err(|e| Error(format!("invalid ALIGNED_PROOF_GENERATOR_ADDR: {e}")))?;

        let eth_rpc_url = std::env::var("ETH_RPC_URL")
            .map_err(|e| Error(format!("ETH_RPC_URL: {e}")))?
            .trim()
            .to_string();

        let eth_chain_id = match chain_id_for_known_network(&aligned_network) {
            Some(id) => id,
            None => {
                let s = std::env::var("ETH_CHAIN_ID")
                    .map_err(|e| Error(format!("ETH_CHAIN_ID required for custom network: {e}")))?;
                s.trim()
                    .parse::<u64>()
                    .map_err(|e| Error(format!("invalid ETH_CHAIN_ID: {e}")))?
            }
        };

        let eth_signer = EthSigner::from_env()?;

        Ok(Self {
            aligned_network,
            aligned_proof_generator_addr,
            eth_rpc_url,
            eth_chain_id,
            eth_signer,
        })
    }

    /// Builds `VerificationData` from a `MinaProof` and submits it to the Aligned batcher.
    /// Returns the `AlignedVerificationData` receipt on success.
    pub async fn submit(
        &self,
        proof: MinaProof,
    ) -> Result<AlignedVerificationData, Error> {
        let (proof_bytes, pub_input, proving_system, proof_name) = match proof {
            MinaProof::State((proof, pub_input)) => {
                let proof_bytes = bincode::serialize(&proof)
                    .map_err(|e| Error(format!("failed to serialize state proof: {e}")))?;
                let pub_input_bytes = bincode::serialize(&pub_input)
                    .map_err(|e| Error(format!("failed to serialize state public inputs: {e}")))?;
                (proof_bytes, pub_input_bytes, ProvingSystemId::Mina, "Mina Proof of State")
            }
            MinaProof::Account((proof, pub_input)) => {
                let proof_bytes = bincode::serialize(&proof)
                    .map_err(|e| Error(format!("failed to serialize account proof: {e}")))?;
                let pub_input_bytes = bincode::serialize(&pub_input)
                    .map_err(|e| Error(format!("failed to serialize account public inputs: {e}")))?;
                (proof_bytes, pub_input_bytes, ProvingSystemId::MinaAccount, "Mina Proof of Account")
            }
        };

        let proof_generator_addr_bytes: [u8; 20] =
            self.aligned_proof_generator_addr.into_array();
        let proof_generator_addr_ethers =
            H160::from(proof_generator_addr_bytes);

        let verification_data = VerificationData {
            proving_system,
            proof: proof_bytes,
            pub_input: Some(pub_input),
            // Force Aligned to include the commitment to the proving system ID (valid for Aligned 0.7.0)
            verification_key: Some(vec![]),
            vm_program_code: None,
            proof_generator_addr: proof_generator_addr_ethers,
        };

        let max_fee = self.estimate_max_fee().await?;
        let wallet = self.ethers_wallet()?;

        let nonce = aligned_sdk::verification_layer::get_nonce_from_batcher(
            self.aligned_network.clone(),
            wallet.address(),
        )
        .await
        .map_err(|_| Error("failed to retrieve nonce from aligned batcher".to_string()))?;

        aligned_2::submit(
            &self.aligned_network,
            &verification_data,
            max_fee,
            wallet,
            nonce,
            proof_name,
        )
        .await
        .map_err(|e| Error(e))
    }

    /// Single-shot check of whether a previously submitted proof has been verified on-chain.
    pub async fn check_verification(
        &self,
        aligned_verification_data: &AlignedVerificationData,
        timeout: Duration,
    ) -> Result<bool, Error> {
        aligned_2::check_verification(
            &self.eth_rpc_url,
            &self.aligned_network,
            aligned_verification_data,
            timeout,
        )
        .await
        .map_err(|e| Error(e))
    }

    /// Estimates the max batcher fee from env config.
    /// ALIGNED_BATCHER_FEE_TYPE: "0" = default, "1" = instant, "2" = custom (requires ALIGNED_BATCHER_MAX_FEE).
    async fn estimate_max_fee(&self) -> Result<U256, Error> {
        let fee_type = std::env::var("ALIGNED_BATCHER_FEE_TYPE").unwrap_or("0".to_string());
        match fee_type.as_str() {
            "0" => aligned_sdk::verification_layer::estimate_fee(
                &self.eth_rpc_url,
                FeeEstimationType::Default,
            )
            .await
            .map_err(|e| Error(e.to_string())),
            "1" => aligned_sdk::verification_layer::estimate_fee(
                &self.eth_rpc_url,
                FeeEstimationType::Instant,
            )
            .await
            .map_err(|e| Error(e.to_string())),
            "2" => {
                let fee_str = std::env::var("ALIGNED_BATCHER_MAX_FEE")
                    .map_err(|e| Error(format!("ALIGNED_BATCHER_MAX_FEE: {e}")))?;
                let fee = fee_str
                    .parse::<U256>()
                    .map_err(|e| Error(format!("invalid ALIGNED_BATCHER_MAX_FEE: {e}")))?;
                if fee == U256::from(0) {
                    return Err(Error(
                        "ALIGNED_BATCHER_MAX_FEE cannot be 0 when ALIGNED_BATCHER_FEE_TYPE is 2"
                            .to_string(),
                    ));
                }
                Ok(fee)
            }
            other => Err(Error(format!(
                "invalid ALIGNED_BATCHER_FEE_TYPE: '{other}', expected '0', '1', or '2'"
            ))),
        }
    }

    /// Builds an ethers wallet from the stored signer credential.
    fn ethers_wallet(&self) -> Result<Wallet<SigningKey>, Error> {
        self.eth_signer.to_wallet(self.eth_chain_id)
    }
}
