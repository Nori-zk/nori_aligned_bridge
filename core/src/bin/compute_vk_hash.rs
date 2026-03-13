use mina_bridge_core::utils::vk_hash;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 4 {
        return Err(
            "Usage: compute_vk_hash <state_hash> <public_key> <token_id>".to_string(),
        );
    }
    let rpc_url = "https://api.minascan.io/node/devnet/v1/graphql";
    let state_hash = &args[1];
    let public_key = &args[2];
    let token_id = &args[3];
    
    println!("state_hash: {}", state_hash);
    println!("public_key: {}", public_key);
    println!("token_id: {}", token_id);
    let hash_hex = run_async(vk_hash::vk_hash_hex_from_mina_account(
        rpc_url, state_hash, public_key, token_id,
    ))?;
    println!("verificationKeyHash (uint256): {}", hash_hex);
    println!("Compare with NoriTokenBridge.noriStorageZkappAcctVk");
    return Ok(());
}

fn run_async<F, O>(f: F) -> Result<O, String>
where
    F: std::future::Future<Output = Result<O, String>>,
{
    tokio::runtime::Runtime::new()
        .map_err(|e| e.to_string())?
        .block_on(f)
}
