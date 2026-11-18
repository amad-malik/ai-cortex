//! Problem `type` codes (CONTRACTS §2 and §5). r2: mike's F1A-R0-02, kai's ruling in row 82054725.
//!
//! `ProblemType` is a validated wire code, not a closed enum, so that a module's registered code
//! (S1 §5.4's token codes such as `token_expired`, I-2's `target_retired`) reaches `Problem`'s
//! `type` field with its own text, and is never replaced by `invalid_request`.
//! - The 13 platform codes are constants, in CONTRACTS §5's order.
//! - A module code is made only by the holder of the one `ProblemCodeMinter`. F-1d's registry
//!   takes it at startup, and checks that no two modules register the same code. F-1a owns the
//!   type and the code grammar; registration and collision checks between modules are F-1d's.
//!
//! The minter: an atomic flag hands it out once per process; it is neither `Clone` nor `Copy`,
//! and dropping it does not make it available again.

/// A problem `type` code: `^[a-z][a-z0-9_]{0,63}$`. Its field is private: a value comes from a
/// platform constant or from the minter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProblemType(&'static str);

impl ProblemType {
    pub const TOKEN_MISSING: Self = Self("token_missing");
    pub const TOKEN_INVALID: Self = Self("token_invalid");
    pub const MISSING_SCOPE: Self = Self("missing_scope");
    pub const TARGET_NOT_ALLOWED: Self = Self("target_not_allowed");
    pub const NOT_FOUND: Self = Self("not_found");
    pub const RESTORE_IN_PROGRESS: Self = Self("restore_in_progress");
    pub const MAINTENANCE: Self = Self("maintenance");
    pub const CAPABILITY_UNAVAILABLE: Self = Self("capability_unavailable");
    pub const IDEMPOTENCY_KEY_REQUIRED: Self = Self("idempotency_key_required");
    pub const IDEMPOTENCY_KEY_REUSED: Self = Self("idempotency_key_reused");
    pub const ALREADY_ISSUED: Self = Self("already_issued");
    pub const BODY_TOO_LARGE: Self = Self("body_too_large");
    pub const INVALID_REQUEST: Self = Self("invalid_request");

    /// The platform codes, in CONTRACTS §5's order.
    pub const PLATFORM: [Self; 13] = [
        Self::TOKEN_MISSING,
        Self::TOKEN_INVALID,
        Self::MISSING_SCOPE,
        Self::TARGET_NOT_ALLOWED,
        Self::NOT_FOUND,
        Self::RESTORE_IN_PROGRESS,
        Self::MAINTENANCE,
        Self::CAPABILITY_UNAVAILABLE,
        Self::IDEMPOTENCY_KEY_REQUIRED,
        Self::IDEMPOTENCY_KEY_REUSED,
        Self::ALREADY_ISSUED,
        Self::BODY_TOO_LARGE,
        Self::INVALID_REQUEST,
    ];

    /// The wire code, for example `missing_scope` or `target_retired`.
    pub fn code(self) -> &'static str {
        self.0
    }

    /// True for the 13 platform codes; false for a minted module code.
    pub fn is_platform(self) -> bool {
        Self::PLATFORM.contains(&self)
    }
}

/// The one capability that makes module problem codes. Not `Clone`; its field is private.
pub struct ProblemCodeMinter {
    _sealed: (),
}

impl ProblemCodeMinter {
    /// Hands out the minter once per process; every later call returns `None`. F-1d's registry
    /// takes it at startup, so no other code can make a module code.
    pub fn take() -> Option<Self> {
        static TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        TAKEN
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .ok()
            .map(|_| Self { _sealed: () })
    }

    /// A module code: `^[a-z][a-z0-9_]{0,63}$`, and not a platform code. Two modules registering
    /// the same code is F-1d's registry check, not this one.
    pub fn mint(&self, code: &'static str) -> Result<ProblemType, ProblemCodeError> {
        let _ = &self._sealed;
        let b = code.as_bytes();
        let grammar = !b.is_empty()
            && b.len() <= 64
            && b[0].is_ascii_lowercase()
            && b[1..]
                .iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_');
        if !grammar {
            return Err(ProblemCodeError::BadCode(code.chars().take(80).collect()));
        }
        if let Some(p) = ProblemType::PLATFORM.iter().find(|p| p.0 == code) {
            return Err(ProblemCodeError::PlatformCode(p.0));
        }
        Ok(ProblemType(code))
    }
}

/// Dropping the minter does not free it: the process-wide flag stays set, so `take` never hands
/// out a second minter, before or after the first one's holder is gone.
impl Drop for ProblemCodeMinter {
    fn drop(&mut self) {}
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProblemCodeError {
    #[error("not a problem code: {0:?}")]
    BadCode(String),
    #[error("`{0}` is a platform code")]
    PlatformCode(&'static str),
}
