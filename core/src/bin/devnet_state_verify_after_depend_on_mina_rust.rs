//! End-to-end smoke test: fetch live Mina Devnet data and run local Pickles verification.
//!
//! This binary exercises the full `get_mina_proof_of_state` pipeline against a live
//! Mina Devnet daemon to verify that the upgraded mina-rust + proof-systems dependencies
//! can successfully:
//!   1. Query the transition frontier via GraphQL
//!   2. Fetch 16 blocks of protocol states and proofs
//!   3. Assemble MinaStateProof + MinaStatePubInputs
//!   4. Serialize via bincode
//!   5. Run local Pickles verification (verify_mina_state)
//!
//! # Environment
//!
//! Reads from `services/.env.local` (or `.env.local`). Required vars:
//!   MINA_RPC_NETWORK_URL  — e.g. https://devnet-plain-1.gcp.o1test.net/graphql
//!   MINA_NETWORK          — "devnet"
//!
//! # Usage
//!
//!   cargo run -p mina_bridge_core --bin devnet_state_verify

use std::time::Instant;

use mina_bridge_core::rpcs::mina_daemon::MinaDaemonRPC;
use mina_bridge_core::utils::constants::BRIDGE_TRANSITION_FRONTIER_LEN;

#[tokio::main]
async fn main() {
    // Load env files (same order as the existing tests)
    dotenv::from_filename("services/.env.local").ok();
    env_logger::init();

    println!("=== Mina Devnet State Verification Smoke Test ===\n");

    // ── Step 1: Connect to the Mina daemon ─────────────────────────────
    let rpc = MinaDaemonRPC::from_env().unwrap_or_else(|e| {
        eprintln!("ERROR: Failed to create MinaDaemonRPC: {e}");
        eprintln!("Make sure MINA_RPC_NETWORK_URL and MINA_NETWORK are set.");
        std::process::exit(1);
    });
    println!("[OK] MinaDaemonRPC created from env\n");

    // ── Step 2: Query frontier (16 blocks) ─────────────────────────────
    println!("Querying frontier with max_length={}...", BRIDGE_TRANSITION_FRONTIER_LEN);
    let frontier_start = Instant::now();
    let frontier = rpc
        .query_frontier(BRIDGE_TRANSITION_FRONTIER_LEN)
        .await
        .unwrap_or_else(|e| {
            eprintln!("ERROR: query_frontier failed: {e}");
            std::process::exit(1);
        });
    println!(
        "[OK] Frontier returned {} blocks in {:.1}s\n",
        frontier.len(),
        frontier_start.elapsed().as_secs_f64()
    );

    // Display the frontier
    println!("Frontier blocks (oldest → newest):");
    for (i, (sh, h)) in frontier.iter().enumerate() {
        let marker = if i == 0 { " ← F_g (oldest)" } else if i == frontier.len() - 1 { " ← tip (newest)" } else { "" };
        println!("  [{:2}] height={} hash={}{}", i, h, sh, marker);
    }
    println!();

    // ── Step 3: Pick F_g = frontier[0] ─────────────────────────────────
    let (fg_state_hash, fg_height) = frontier.first().expect("frontier is non-empty").clone();
    let (tip_state_hash, tip_height) = frontier.last().expect("frontier is non-empty").clone();
    println!("F_g:  height={}, state_hash={}", fg_height, fg_state_hash);
    println!("Tip:  height={}, state_hash={}", tip_height, tip_state_hash);
    println!();

    // ── Step 4: get_mina_proof_of_state ────────────────────────────────
    // This calls less_insane_get_mina_proof_of_state which internally:
    //   - queries 16 blocks by height
    //   - fetches 16 protocol states
    //   - assembles proof + pub inputs
    //   - serializes via bincode
    //   - runs verify_mina_state (Pickles verification)
    println!("Calling get_mina_proof_of_state(height={}, hash={})...", fg_height, fg_state_hash);
    println!("  This will fetch 16 blocks, assemble the proof, and run Pickles verification.");
    println!("  Expected to take 1-5 minutes depending on network and CPU.\n");

    let proof_start = Instant::now();
    let result = rpc
        .get_mina_proof_of_state(fg_height, &fg_state_hash.to_string())
        .await;
    let proof_elapsed = proof_start.elapsed();

    match result {
        Ok((proof, pub_inputs)) => {
            println!("\n=== VERIFICATION PASSED ===\n");
            println!("  Total time:                {:.1}s", proof_elapsed.as_secs_f64());
            println!("  candidate_chain_states:    {} blocks", proof.candidate_chain_states.len());
            println!("  bridge_tip_state_hash:     {}", pub_inputs.bridge_tip_state_hash);
            println!("  is_devnet:                 {}", pub_inputs.is_state_proof_from_devnet);
            println!("  candidate tip state_hash:  {}", pub_inputs.candidate_chain_state_hashes.last().unwrap());
            println!();

            // Sanity checks
            assert_eq!(
                pub_inputs.bridge_tip_state_hash.to_string(),
                fg_state_hash.to_string(),
                "bridge_tip_state_hash should match F_g"
            );
            assert_eq!(
                pub_inputs.candidate_chain_state_hashes.len(),
                BRIDGE_TRANSITION_FRONTIER_LEN,
                "should have exactly 16 state hashes"
            );
            println!("[OK] All assertions passed.");
            println!("\nThe upgraded mina-rust + proof-systems dependencies work correctly");
            println!("with live Mina Devnet data.");
        }
        Err(e) => {
            eprintln!("\n=== VERIFICATION FAILED ===\n");
            eprintln!("  Error: {e}");
            eprintln!("  Time before failure: {:.1}s", proof_elapsed.as_secs_f64());
            eprintln!();
            eprintln!("This may indicate:");
            eprintln!("  - A breaking change in the upgraded dependencies");
            eprintln!("  - The Mina Devnet daemon is unreachable or returning unexpected data");
            eprintln!("  - A bincode serialization incompatibility");
            eprintln!("  - A Pickles verification failure due to verifier index mismatch");
            std::process::exit(1);
        }
    }
}
