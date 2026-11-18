//! Shared value types for every Cortex v2 crate (lane A, F-1a; CONTRACTS §2).
//!
//! The value types every crate shares. `Secret` cannot be formatted; each ID is its own type;
//! `ProblemType` is a validated code whose module codes come only from the one minter;
//! `canonical_json` is RFC 8785 (TESTS F1-03, F1-04; mike's F1A-R0-02).

mod canonical;
pub mod datetime;
mod ids;
mod names;
mod problem;
mod secret;
mod sha;
mod time;

pub use canonical::{CanonicalError, canonical_json};
pub use ids::{
    EventId, HandoffId, Id, IdError, InstallationId, ProjectId, RecordId, RequestId, TeamId,
    TokenId, WorkerId,
};
pub use names::{ModuleId, NameError, ScopeName, Seq};
pub use problem::{ProblemCodeError, ProblemCodeMinter, ProblemType};
pub use secret::Secret;
pub use sha::Sha256;
#[cfg(feature = "testkit")]
pub use time::TestClock;
pub use time::{Clock, SystemClock, Timestamp};
