use state_proof::{MinaStateProof, MinaStatePubInputs};

/// Mina Proof of State definition.
pub mod state_proof;

// TODO(xqft): we should fix this lint instead
#[allow(clippy::large_enum_variant)]
pub enum MinaProof {
    State((MinaStateProof, MinaStatePubInputs)),
}
