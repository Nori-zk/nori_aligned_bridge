use mina_bridge_core::utils::token_id::token_id_decimal_to_base58;
use std::env;

fn main() {
    let decimal = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: token_id_to_base58 <decimal>");
        std::process::exit(1);
    });
    match token_id_decimal_to_base58(&decimal) {
        Ok(base58) => println!("{base58}"),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
