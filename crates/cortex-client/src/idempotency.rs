//! The Idempotency-Key (R-11; the `IdempotencyKey` parameter of the S3 twin): a UUIDv7 per logical operation, reused
//! on every retry of it. 8 to 128 printable ASCII characters, `0x20` to `0x7E`.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("an Idempotency-Key is 8 to 128 characters")]
    Length,
    #[error("an Idempotency-Key is printable ASCII (0x20 to 0x7E)")]
    Byte,
}

/// A key that passed the chain's rule. Not a secret: it is shown in errors so a caller can repeat a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// A fresh UUIDv7 (36 characters). Nothing of the token or the body goes into it.
    pub fn generate() -> IdempotencyKey {
        IdempotencyKey(uuid::Uuid::now_v7().hyphenated().to_string())
    }

    /// Validate a caller-supplied key (`Client`'s `with_key`).
    pub fn parse(text: &str) -> Result<IdempotencyKey, KeyError> {
        if !(8..=128).contains(&text.len()) {
            return Err(KeyError::Length);
        }
        if !text.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return Err(KeyError::Byte);
        }
        Ok(IdempotencyKey(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
