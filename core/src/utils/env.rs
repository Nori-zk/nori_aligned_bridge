use aligned_sdk::common::types::Network;
extern crate dotenv;
use dotenv::dotenv;
use log::info;

/// Struct that is created by reading environment variables or, for some fields, from defined constants if the
/// corresponding environment variable is not defined.
///
/// - `mina_rpc_url`: Mina node RPC URL to get the Mina state
/// - `eth_network`: Enum variant to specify the Ethereum network to update the Mina state
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
/// - `nori_token_bridge_eth_addr`: Address of the NoriTokenBridge contract on Devnet
pub struct EnvironmentVariables {
    pub mina_rpc_url: String,
    pub eth_network: Network,
    pub state_settlement_addr: Option<String>,
    pub account_validation_addr: Option<String>,
    pub batcher_addr: String,
    pub batcher_eth_addr: String,
    pub eth_rpc_url: String,
    pub proof_generator_addr: String,
    pub keystore_path: Option<String>,
    pub private_key: Option<String>,
    pub nori_token_bridge_eth_addr: String,
    pub batcher_fee_estimation_type: String,
    pub batcher_max_fee: Option<String>,
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
    /// - `ETH_CHAIN` is not set to a valid Ethereum network (`"devnet"` or `"hoodi"`)
    /// - Both `KEYSTORE_PATH` and `PRIVATE_KEY` are set
    pub fn new() -> Result<EnvironmentVariables, String> {
        load_env();

        let mina_rpc_url = std::env::var("MINA_RPC_URL")
            .map_err(|err| format!("Couldn't get MINA_RPC_URL env. variable: {err}"))?;
        let eth_network = match std::env::var("ETH_CHAIN")
            .map_err(|err| format!("Couldn't get ETH_CHAIN env. variable: {err}"))?
            .as_str()
        {
            "devnet" => {
                info!("Selected Anvil devnet chain.");
                Network::Devnet
            }
            "hoodi" => {
                info!("Selected hoodi chain.");
                Network::Hoodi
            }
            "sepolia" => {
                info!("Selected sepolia chain.");
                Network::Sepolia
            }
            _ => return Err(
                "Unrecognized chain, possible values for ETH_CHAIN are \"devnet\", \"sepolia\" and \"hoodi\"."
                    .to_owned(),
            ),
        };

        let state_settlement_addr = std::env::var("STATE_SETTLEMENT_ETH_ADDR").ok();
        let account_validation_addr = std::env::var("ACCOUNT_VALIDATION_ETH_ADDR").ok();

        let batcher_addr = std::env::var("BATCHER_ADDR").map_err(|err| {
            format!("Couldn't get BATCHER_ADDR env. variable: {err}")
        })?;
        let batcher_eth_addr = std::env::var("BATCHER_ETH_ADDR").map_err(|err| {
            format!("Couldn't get BATCHER_ETH_ADDR env. variable: {err}")
        })?;
        let eth_rpc_url = std::env::var("ETH_RPC_URL").map_err(|err| {
            format!("Couldn't get ETH_RPC_URL env. variable: {err}")
        })?;
        let proof_generator_addr = std::env::var("PROOF_GENERATOR_ADDR").map_err(|err| {
            format!("Couldn't get PROOF_GENERATOR_ADDR env. variable: {err}")
        })?;

        let keystore_path = std::env::var("KEYSTORE_PATH").ok();
        let private_key = std::env::var("PRIVATE_KEY").ok();

        if keystore_path.is_some() && private_key.is_some() {
            return Err(
                "Both keystore and private key env. variables are defined. Choose only one."
                    .to_string(),
            );
        } else if keystore_path.is_none() && private_key.is_none() {
            return Err("Neither keystore nor private key env. variables are defined.".to_string());
        }

        let nori_token_bridge_eth_addr =
            std::env::var("NORI_TOKEN_BRIDGE_ETH_ADDRESS").map_err(|err| {
                format!("Couldn't get NORI_TOKEN_BRIDGE_ETH_ADDRESS env. variable: {err}")
            })?;

        let batcher_fee_estimation_type = match std::env::var("BATCHER_FEE_ESTM_TYPE") {
            Ok(value) => value,
            Err(_) => "0".to_string(),
        };
        let batcher_max_fee = std::env::var("BATCHER_MAX_FEE").ok();
        if batcher_fee_estimation_type == "2" && batcher_max_fee.is_none() {
            panic!("BATCHER_MAX_FEE is not set when BATCHER_FEE_ESTM_TYPE is 2");
        }

        Ok(EnvironmentVariables {
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
            batcher_fee_estimation_type,
            batcher_max_fee,
        })
    }
}

fn load_env() {
    // load .env
    dotenv()
        .map_err(|err| format!("Couldn't load .env file: {}", err))
        .unwrap();

    let app_env = std::env::var("ETH_CHAIN").unwrap_or_else(|_| "devnet".to_string());
    let env_file = format!(".env.{}", app_env);

    // then load the corresponding environment file
    dotenv::from_filename(&env_file).expect(&format!("Failed to load {}", env_file));
}
