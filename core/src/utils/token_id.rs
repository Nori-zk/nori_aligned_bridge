use ark_serialize::CanonicalSerialize;
use mina_curves::pasta::Fp;
use mina_p2p_messages::{
    bigint,
    v2::MinaBaseAccountIdDigestStableV1,
};
use std::str::FromStr;

/// Converts a decimal field element string (from o1js `deriveTokenId().toString()`)
/// to the base58check format expected by the Mina daemon's GraphQL API.
pub fn token_id_decimal_to_base58(decimal: &str) -> Result<String, String> {
    let fp = Fp::from_str(decimal)
        .map_err(|_| format!("invalid field element: {decimal}"))?;
    let mut bytes = Vec::with_capacity(32);
    fp.serialize_compressed(&mut bytes)
        .map_err(|e| format!("field element serialization: {e}"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "field element did not serialize to 32 bytes".to_string())?;
    let bigint = bigint::BigInt::from_bytes(bytes);
    let token_hash: mina_p2p_messages::v2::TokenIdKeyHash =
        MinaBaseAccountIdDigestStableV1(bigint).into();
    Ok(token_hash.to_string())
}
