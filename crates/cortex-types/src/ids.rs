//! IDs: newtypes over a UUIDv7; Display and serde are lowercase hyphenated text.
//! (CONTRACTS §2; S1 §2.1; S2 §3.2; TESTS F1-04.)
//!
//! Each ID is its own type, so a `WorkerId` cannot be passed where a `ProjectId` is expected.
//! Each has inherent `new`, `timestamp` and `parse`; the `Id` trait offers the same three for
//! generic code. `new` makes no monotonicity promise across processes or clock steps; the
//! monotonic generator is F-1A6 (BOUND A6B).
use crate::time::{Clock, Timestamp};

/// What every ID offers: a UUIDv7 from the clock, its millisecond timestamp, and a parser that
/// accepts only a UUIDv7 in lowercase hyphenated text.
pub trait Id: Sized {
    fn new(clock: &dyn Clock) -> Self;
    fn timestamp(&self) -> Timestamp;
    fn parse(s: &str) -> Result<Self, IdError>;
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdError {
    #[error("not a UUID in lowercase hyphenated form: {0:?}")]
    NotUuid(String),
    /// Version is not 7, or the variant is not the RFC variant (F1A-R2-01).
    #[error("not an RFC-variant UUIDv7 (version {0})")]
    NotV7(usize),
}

/// A UUIDv7 whose 48-bit Unix-millisecond prefix is the clock's time. The remaining bits are
/// random (uuid's `NoContext`).
fn new_v7(clock: &dyn Clock) -> uuid::Uuid {
    let micros = clock.now().unix_micros();
    let millis = u64::try_from(micros.div_euclid(1000)).expect("a UUIDv7 needs a time after 1970");
    uuid::Uuid::new_v7(uuid::Timestamp::from_unix(
        uuid::NoContext,
        millis / 1000,
        u32::try_from((millis % 1000) * 1_000_000).expect("under one second"),
    ))
}

/// The UUIDv7's millisecond prefix, as a `Timestamp`.
fn timestamp_of(u: &uuid::Uuid) -> Timestamp {
    let b = u.as_bytes();
    let millis = b[..6]
        .iter()
        .fold(0_i64, |acc, x| (acc << 8) | i64::from(*x));
    Timestamp::from_unix_micros(millis * 1000)
}

/// Only the canonical text: 36 characters, lowercase, hyphenated, version 7 AND the RFC variant
/// (mike's F1A-R2-01: the version nibble alone admits NCS, Microsoft and future variants).
fn parse_v7(s: &str) -> Result<uuid::Uuid, IdError> {
    let canonical = s.len() == 36
        && s.bytes()
            .all(|c| c == b'-' || c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
    let u = match uuid::Uuid::try_parse(s) {
        Ok(u) if canonical && u.hyphenated().to_string() == s => u,
        _ => return Err(IdError::NotUuid(s.chars().take(64).collect())),
    };
    match (u.get_version_num(), u.get_variant()) {
        (7, uuid::Variant::RFC4122) => Ok(u),
        (v, _) => Err(IdError::NotV7(v)),
    }
}

macro_rules! id_type {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(uuid::Uuid);

        impl $name {
            /// A UUIDv7 from the clock's time.
            pub fn new(clock: &dyn Clock) -> Self {
                Self(new_v7(clock))
            }
            /// The UUIDv7's 48-bit millisecond prefix.
            pub fn timestamp(&self) -> Timestamp {
                timestamp_of(&self.0)
            }
            /// Accepts only a UUIDv7 in lowercase hyphenated text.
            pub fn parse(s: &str) -> Result<Self, IdError> {
                parse_v7(s).map(Self)
            }
        }

        impl Id for $name {
            fn new(clock: &dyn Clock) -> Self {
                $name::new(clock)
            }
            fn timestamp(&self) -> Timestamp {
                $name::timestamp(self)
            }
            fn parse(s: &str) -> Result<Self, IdError> {
                $name::parse(s)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0.hyphenated(), f)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <std::borrow::Cow<'de, str> as serde::Deserialize>::deserialize(d)?;
                $name::parse(&s).map_err(serde::de::Error::custom)
            }
        }
    )+};
}

id_type!(
    /// One Cortex installation.
    InstallationId,
    /// A project.
    ProjectId,
    /// A team within a project.
    TeamId,
    /// A worker.
    WorkerId,
    /// A token (its `token_id`, never the token).
    TokenId,
    /// A record.
    RecordId,
    /// An event.
    EventId,
    /// A request, for correlation.
    RequestId,
    /// A handoff (Wave 0 S0: the `HandoffsApi` port's views; socket contract r4 ask 5).
    HandoffId,
);
