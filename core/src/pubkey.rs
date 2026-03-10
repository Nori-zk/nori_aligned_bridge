use mina_curves::pasta::Fp;
use mina_signer::CompressedPubKey;
use std::fmt;

/// Parsed compressed Mina public key: x-coordinate and y-parity.
pub struct MinaCompressedPubKey {
    pub x: Fp,
    pub is_odd: bool,
}

/// Base58Check-encoded Mina public key address (B62q...).
pub struct MinaPubKeyBase58(String);

impl fmt::Display for MinaPubKeyBase58 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<MinaCompressedPubKey> for MinaPubKeyBase58 {
    fn from(pubkey: MinaCompressedPubKey) -> Self {
        let compressed = CompressedPubKey {
            x: pubkey.x,
            is_odd: pubkey.is_odd,
        };
        MinaPubKeyBase58(compressed.into_address())
    }
}
