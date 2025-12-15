use aligned_sdk::common::types::Network;
extern crate dotenv;
use dotenv::dotenv;
use log::info;

use super::constants::{
    ANVIL_BATCHER_ADDR, ANVIL_BATCHER_ETH_ADDR, ANVIL_ETH_RPC_URL, PROOF_GENERATOR_ADDR,
};

/// Struct that is created by reading environment variables or, for some fields, from defined constants if the
/// corresponding environment variable is not defined.
///
/// - `rpc_url`: Mina node RPC URL to get the Mina state
/// - `network`: Enum variant to specify the Ethereum network to update the Mina state
/// - `state_settlement_addr`: Address of the Mina State Settlement Example Contract
/// - `account_validation_addr`: Address of the Mina Account Validation Example Contract
/// - `batcher_addr`: Address of the Aligned Batcher Service
/// - `batcher_eth_addr`: Address of the Aligned Batcher Payment Service
/// - `eth_rpc_url`: Ethereum node RPC URL to send the transaction to update the Mina state
/// - `proof_generator_addr`: Address of the Aligned Proof Generator
/// - `keystore_path`: Path to the keystore used to sign Ethereum transactions.
///   `None` if `private_key` is defined.
/// - `private_key`: Private key of the Ethereum wallet used to sign Ethereum transactions.
///   `None` if `keystore_path` is defined.
/// - `sudoku_zkapp_addr`: Address of the Sudoku zkApp
/// - `sudoku_token_id`: Token ID of the Sudoku zkApp
/// - `sudoku_validity_devnet_addr`: Address of the Sudoku validity contract on Devnet
/// - `nori_token_storage_zkapp_addr`: Address of the NoriTokenStorage zkApp
/// - `nori_token_controller_token_id`: Token ID of the NoriTokenController zkApp
/// - `nori_token_bridge_devnet_addr`: Address of the NoriTokenBridge contract on Devnet
pub struct EnvironmentVariables {
    pub rpc_url: String,
    pub network: Network,
    pub state_settlement_addr: Option<String>,
    pub account_validation_addr: Option<String>,
    pub batcher_addr: String,
    pub batcher_eth_addr: String,
    pub eth_rpc_url: String,
    pub proof_generator_addr: String,
    pub keystore_path: Option<String>,
    pub private_key: Option<String>,

    pub sudoku_zkapp_addr: String,
    pub sudoku_token_id: String,
    pub sudoku_validity_devnet_addr: String,

    pub nori_token_storage_zkapp_addr: String,
    pub nori_token_controller_token_id: String,
    pub nori_token_bridge_devnet_addr: String,
}

fn load_var_or(key: &str, default: &str, network: &Network) -> Result<String, String> {
    // Default value is only valid for Anvil devnet setup.
    match std::env::var(key) {
        Ok(value) => Ok(value),
        Err(_) if matches!(network, Network::Devnet) => {
            info!("Using default {} for devnet: {}", key, default);
            Ok(default.to_string())
        }
        Err(err) => Err(format!(
            "Chain selected is not Devnet but couldn't read {}: {}",
            key, err
        )),
    }
}

impl EnvironmentVariables {
    /// Creates the `EnvironmentVariables` struct from environment variables or, for some fields, from defined
    /// constants if the corresponding environment variable is not defined.
    ///
    /// Returns `Err` if:
    ///
    /// - `MINA_RPC_URL` or `ETH_CHAIN` environemnt variables are not defined
    /// - `ETH_CHAIN` is not set to a valid Ethereum network (`"devnet"` or `"holesky"`)
    /// - Both `KEYSTORE_PATH` and `PRIVATE_KEY` are set
    pub fn new() -> Result<EnvironmentVariables, String> {
        dotenv().map_err(|err| format!("Couldn't load .env file: {}", err))?;

        let rpc_url = std::env::var("MINA_RPC_URL")
            .map_err(|err| format!("Couldn't get MINA_RPC_URL env. variable: {err}"))?;
        let network = match std::env::var("ETH_CHAIN")
            .map_err(|err| format!("Couldn't get ETH_CHAIN env. variable: {err}"))?
            .as_str()
        {
            "devnet" => {
                info!("Selected Anvil devnet chain.");
                Network::Devnet
            }
            "holesky" => {
                info!("Selected Holesky chain.");
                Network::Holesky
            }
            _ => return Err(
                "Unrecognized chain, possible values for ETH_CHAIN are \"devnet\" and \"holesky\"."
                    .to_owned(),
            ),
        };

        let state_settlement_addr = std::env::var("STATE_SETTLEMENT_ETH_ADDR").ok();
        let account_validation_addr = std::env::var("ACCOUNT_VALIDATION_ETH_ADDR").ok();

        let batcher_addr = load_var_or("BATCHER_ADDR", ANVIL_BATCHER_ADDR, &network)?;
        let batcher_eth_addr = load_var_or("BATCHER_ETH_ADDR", ANVIL_BATCHER_ETH_ADDR, &network)?;
        let eth_rpc_url = load_var_or("ETH_RPC_URL", ANVIL_ETH_RPC_URL, &network)?;
        let proof_generator_addr =
            load_var_or("PROOF_GENERATOR_ADDR", PROOF_GENERATOR_ADDR, &network)?;

        let keystore_path = std::env::var("KEYSTORE_PATH").ok();
        let private_key = std::env::var("PRIVATE_KEY").ok();
        info!("KEYSTORE_PATH: {:?}", keystore_path);
        info!("PRIVATE_KEY: {:?}", private_key);

        if keystore_path.is_some() && private_key.is_some() {
            return Err(
                "Both keystore and private key env. variables are defined. Choose only one."
                    .to_string(),
            );
        }

        let sudoku_zkapp_addr = std::env::var("SUDOKU_ZKAPP_ADDRESS")
            .map_err(|err| format!("Couldn't get SUDOKU_ZKAPP_ADDRESS env. variable: {err}"))?;
        let sudoku_token_id = std::env::var("SUDOKU_TOKEN_ID")
            .map_err(|err| format!("Couldn't get SUDOKU_TOKEN_ID env. variable: {err}"))?;
        let sudoku_validity_devnet_addr = std::env::var("SUDOKU_VALIDITY_DEVNET_ADDRESS")
            .map_err(|err| format!("Couldn't get SUDOKU_VALIDITY_DEVNET_ADDRESS env. variable: {err}"))?;

        let nori_token_storage_zkapp_addr = std::env::var("NORI_TOKEN_STORAGE_ZKAPP_ADDRESS")
            .map_err(|err| format!("Couldn't get NORI_TOKEN_STORAGE_ZKAPP_ADDRESS env. variable: {err}"))?;
        let nori_token_controller_token_id = std::env::var("NORI_TOKEN_CONTROLLER_TOKEN_ID")
            .map_err(|err| format!("Couldn't get NORI_TOKEN_CONTROLLER_TOKEN_ID env. variable: {err}"))?;
        let nori_token_bridge_devnet_addr = std::env::var("NORI_TOKEN_BRIDGE_DEVNET_ADDRESS")
            .map_err(|err| format!("Couldn't get NORI_TOKEN_BRIDGE_DEVNET_ADDRESS env. variable: {err}"))?;

        Ok(EnvironmentVariables {
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
        })
    }
}
