//! Validated names and the event sequence (CONTRACTS §2). F-1c's manifests and I-4's scopes use
//! them; their grammar is fixed here, with its own unit tests (no `todo!()` survives F-1a).

/// A module id: `^[a-z][a-z0-9_]{0,31}$` (the manifest `id`; the role suffix `cortex_m_<id>`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(String);

/// A scope: `<resource>:<action>`, each part `[a-z][a-z_]*` (S1 §5.2).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopeName(String);

/// A per-project event sequence number; never negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Seq(i64);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NameError {
    #[error("invalid module id: {0:?}")]
    ModuleId(String),
    #[error("invalid scope name: {0:?}")]
    ScopeName(String),
    #[error("negative sequence: {0}")]
    NegativeSeq(i64),
}

/// `[a-z]` then up to `max_rest` of the characters `rest` allows.
fn ident(s: &str, max_rest: usize, rest: fn(u8) -> bool) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_lowercase()
        && b.len() - 1 <= max_rest
        && b[1..].iter().all(|c| rest(*c))
}

fn lower_digit_underscore(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_'
}

fn lower_underscore(c: u8) -> bool {
    c.is_ascii_lowercase() || c == b'_'
}

impl ModuleId {
    pub fn parse(s: &str) -> Result<Self, NameError> {
        if ident(s, 31, lower_digit_underscore) {
            Ok(Self(s.to_owned()))
        } else {
            Err(NameError::ModuleId(s.chars().take(64).collect()))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ScopeName {
    pub fn parse(s: &str) -> Result<Self, NameError> {
        let ok = match s.split_once(':') {
            Some((resource, action)) => {
                ident(resource, usize::MAX, lower_underscore)
                    && ident(action, usize::MAX, lower_underscore)
            }
            None => false,
        };
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err(NameError::ScopeName(s.chars().take(64).collect()))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Seq {
    pub fn new(n: i64) -> Result<Self, NameError> {
        if n >= 0 {
            Ok(Self(n))
        } else {
            Err(NameError::NegativeSeq(n))
        }
    }
    pub fn get(self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_id_grammar() {
        for ok in [
            "a",
            "audit",
            "test_a",
            "m1",
            &format!("a{}", "b".repeat(31)),
        ] {
            assert!(ModuleId::parse(ok).is_ok(), "{ok:?} should parse");
        }
        for bad in [
            "",
            "A",
            "1a",
            "_a",
            "a-b",
            "a b",
            "é",
            &format!("a{}", "b".repeat(32)),
        ] {
            assert!(ModuleId::parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn scope_name_grammar() {
        for ok in ["roster:read", "token:issue", "self:renew", "a_b:c_d"] {
            assert!(ScopeName::parse(ok).is_ok(), "{ok:?} should parse");
        }
        for bad in [
            "",
            "roster",
            ":read",
            "roster:",
            "Roster:read",
            "roster:read:x",
            "r1:read",
            "roster:re-ad",
        ] {
            assert!(ScopeName::parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn seq_is_never_negative() {
        assert_eq!(Seq::new(0).unwrap().get(), 0);
        assert_eq!(Seq::new(i64::MAX).unwrap().get(), i64::MAX);
        assert_eq!(Seq::new(-1), Err(NameError::NegativeSeq(-1)));
    }
}
