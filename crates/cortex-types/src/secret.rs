//! `Secret`: zeroised on drop; no Debug, no Display, no Serialize; `expose()` is explicit.
//! (CONTRACTS §2; S1 §2.1, T5; TESTS F1-03.)
//!
//! It implements no formatting trait and no serde trait, so a `Secret` cannot reach a log, an
//! error or a response by formatting: F1-03's compile-fail fixtures pin that.
pub struct Secret(String);

impl Secret {
    /// Wraps a secret value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The only way to read the value.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl zeroize::Zeroize for Secret {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(self);
    }
}

impl zeroize::ZeroizeOnDrop for Secret {}
