// ETHEREUM NETWORKS related constants
/// Chain ID of Aligned Devnet network (Anvil local)
pub const ANVIL_CHAIN_ID: u64 = 31337;
/// Chain ID of Holesky testnet
pub const HOLESKY_CHAIN_ID: u64 = 17000;
/// Chain ID of Hoodi testnet
pub const HOODI_CHAIN_ID: u64 = 560048;
/// Chain ID of Sepolia testnet
pub const SEPOLIA_CHAIN_ID: u64 = 11155111;
/// Chain ID of Ethereum mainnet
pub const MAINNET_CHAIN_ID: u64 = 1;

// MINA related constants
/// Size of a Mina state hash in bytes (32 bytes = 256 bits).
pub const MINA_HASH_SIZE: usize = 32;

// Fixed by the Pickles circuit. Block F_g is the oldest block in the proof batch, exactly
// BRIDGE_TRANSITION_FRONTIER_LEN blocks deep from the tip.
pub const BRIDGE_TRANSITION_FRONTIER_LEN: usize = 16;

/// Maximum number of recent blocks the Mina daemon node supports querying state info for.
pub const MINA_DAEMON_MAX_QUERYABLE_BLOCKS: usize = 290;