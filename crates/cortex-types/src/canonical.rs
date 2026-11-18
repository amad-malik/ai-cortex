//! RFC 8785 (JCS) canonical JSON (CONTRACTS §2; S1 §6's audit hash; S2's record hashes).
//!
//! Selected at F-1a GREEN by CONTRACTS §2's gate: `serde_json_canonicalizer` 0.4.1 (MIT; its
//! number formatter `ryu-js` is Apache-2.0 OR BSL-1.0) reproduces nemo's four JCS vectors byte for
//! byte and passes `cargo deny`. `serde_jcs` 0.1.0 was rejected: it fails the UTF-16 key-order
//! vector. No hand-written serialiser (nemo's F1-02).

#[derive(Debug, thiserror::Error)]
pub enum CanonicalError {
    #[error("not representable in RFC 8785 canonical JSON: {0}")]
    Unrepresentable(String),
}

pub fn canonical_json(v: &serde_json::Value) -> Result<Vec<u8>, CanonicalError> {
    serde_json_canonicalizer::to_vec(v).map_err(|e| CanonicalError::Unrepresentable(e.to_string()))
}
