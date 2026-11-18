//! The one source of randomness of the issue path (cox W1-32): the operating system, through `getrandom`, and
//! nothing else. A failure is an [`EntropyError`] that refuses the issue (fail closed, 500, nothing stored); there
//! is NO fallback generator. A cell injects a source to make it fail or to force the stagger value.

use crate::token::EntropyError;

/// Fills a buffer with random bytes, or fails.
pub trait Entropy: Send + Sync {
    fn fill(&self, buf: &mut [u8]) -> Result<(), EntropyError>;

    /// A value in `[0, 1)` for the expiry stagger (S1 A3), from eight fresh bytes: 53 bits, so every double in
    /// the grid is equally likely.
    fn unit(&self) -> Result<f64, EntropyError> {
        let mut b = [0_u8; 8];
        self.fill(&mut b)?;
        let bits = u64::from_be_bytes(b) >> 11;
        #[allow(clippy::cast_precision_loss)]
        Ok(bits as f64 / (1_u64 << 53) as f64)
    }
}

/// The operating system's randomness.
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn fill(&self, buf: &mut [u8]) -> Result<(), EntropyError> {
        crate::token::os_fill(buf)
    }
}
