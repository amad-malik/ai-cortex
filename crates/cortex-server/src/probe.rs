//! The server's side of readiness (piece 11 `serve`, mira; contract r0.4 section 2 and W1-38 B): what `/v2/health` and `/v2/health/ready`
//! read, and what startup checks before the listener opens.
//!
//! The database is read through the [`Ledger`] trait (one read: can the pool reach the database, and what does the migration ledger say
//! for each compiled and enabled set), so every decision here is testable without a database. The real [`Ledger`] over `cortex_db`'s
//! `verify` is [`crate::ledger::DbLedger`].
//!
//! - [`verdict`]: the states of all the sets -> one answer. A changed checksum (or a malformed set) STOPS startup; an unapplied step, a
//!   database that is ahead of this build, or a set that could not be checked is NOT READY, never a stop.
//! - [`startup_check`]: the one-line refusals and the fixed not-ready reasons for startup.
//! - [`ServerProbe`]: the [`Probe`] the open health routes use. The open routes are unauthenticated, so the database is asked at most
//!   once per [`PROBE_TTL_MICROS`] however often they are called (one refresh at a time), as ONE snapshot per document.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cortex_db::migrate::LedgerState;
use cortex_module::{Health, Registry};
use cortex_types::Clock;
use tokio::sync::Mutex;

use crate::health::PieceHealth;
use crate::health_routes::{Probe, ProbeSnapshot};
use crate::serve::{Refused, refused};

/// How long one database answer is reused, on the injected clock.
pub const PROBE_TTL_MICROS: i64 = 1_000_000;

/// How long one refresh (the whole database read) may take before the database counts as not reachable.
pub const REFRESH_TIMEOUT: Duration = Duration::from_secs(5);

/// Why one set could not be checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerFault {
    /// The set's own versions do not increase (a build defect).
    OutOfOrder,
    /// The database failed during the read.
    Database,
}

/// One read of the database: the pool, then the ledger for each set (`platform` first, then the enabled pieces in migration order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerRead {
    Unreachable,
    /// The pool's module registry holds no `platform` (a wiring defect no waiting fixes).
    PlatformNotRegistered,
    Answered(Vec<(String, Result<LedgerState, LedgerFault>)>),
}

#[async_trait]
pub trait Ledger: Send + Sync + 'static {
    async fn read(&self) -> LedgerRead;
}

/// What all the sets add up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Current,
    /// Some step is not applied yet.
    Unapplied,
    /// The ledger holds steps this build does not know.
    DbAhead,
    /// Nothing to check, or a set could not be checked: fail closed.
    Unchecked,
    /// A changed checksum or a malformed set: startup stops.
    Stop {
        module: String,
        version: Option<u32>,
    },
}

/// The states of all the sets, in migration order, as one answer.
///
/// - The FIRST changed checksum or malformed set in that order stops startup (so the line names the earliest module that went wrong, not
///   the lowest version).
/// - Otherwise a database that is ahead of this build, then an unapplied step, then a set that could not be checked; all NOT READY.
/// - No sets at all is `Unchecked`: nothing was verified, so nothing is current.
pub fn verdict(states: &[(String, Result<LedgerState, LedgerFault>)]) -> Verdict {
    let mut ahead = false;
    let mut unapplied = false;
    let mut unchecked = states.is_empty();
    for (module, state) in states {
        match state {
            Ok(LedgerState::ChecksumChanged { version }) => {
                return Verdict::Stop {
                    module: module.clone(),
                    version: Some(*version),
                };
            }
            Err(LedgerFault::OutOfOrder) => {
                return Verdict::Stop {
                    module: module.clone(),
                    version: None,
                };
            }
            Ok(LedgerState::DbAhead) => ahead = true,
            Ok(LedgerState::Unapplied { .. }) => unapplied = true,
            Err(LedgerFault::Database) => unchecked = true,
            Ok(LedgerState::Current) => {}
        }
    }
    if ahead {
        Verdict::DbAhead
    } else if unapplied {
        Verdict::Unapplied
    } else if unchecked {
        Verdict::Unchecked
    } else {
        Verdict::Current
    }
}

/// What startup does when the check does not stop it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    Current,
    /// The server starts NOT READY, for this fixed reason.
    NotReady(&'static str),
}

/// Startup's decision from one read. The refusals are ONE line each and name a module and a version at most (no SQL, no path, no
/// connection detail); the not-ready reasons are fixed texts.
pub fn startup_check(read: &LedgerRead) -> Result<Startup, Refused> {
    let states = match read {
        LedgerRead::Answered(states) => states,
        LedgerRead::Unreachable => return Err(refused("the database could not be reached")),
        LedgerRead::PlatformNotRegistered => {
            return Err(refused(
                "the database module registry does not hold platform",
            ));
        }
    };
    match verdict(states) {
        Verdict::Current => Ok(Startup::Current),
        Verdict::Unapplied => Ok(Startup::NotReady("migration steps are not applied yet")),
        Verdict::DbAhead => Ok(Startup::NotReady(
            "the database holds migration steps this build does not know",
        )),
        Verdict::Unchecked => Ok(Startup::NotReady(
            "the migration ledger could not be checked",
        )),
        Verdict::Stop {
            module,
            version: Some(version),
        } => Err(refused(&format!(
            "migration {version} of {module} was changed after it was applied"
        ))),
        Verdict::Stop {
            module,
            version: None,
        } => Err(refused(&format!(
            "the migration steps of {module} are not in order"
        ))),
    }
}

/// The [`Probe`] of the running server: the registry's own piece health, and the database through a [`Ledger`], observed together as ONE
/// [`ProbeSnapshot`], reused for [`PROBE_TTL_MICROS`] and refreshed by one caller at a time.
///
/// - The freshness of an answer counts from when it ARRIVED, not from when the refresh began: callers that queued behind a slow
///   refresh share its answer instead of each starting another one.
/// - The whole database read is bounded by the refresh timeout ([`REFRESH_TIMEOUT`]): a database that does not answer is "not
///   reachable". (The statement timeout inside [`crate::ledger::DbLedger`] bounds one statement, not pool acquisition or the whole read.)
pub struct ServerProbe {
    ledger: Arc<dyn Ledger>,
    registry: Arc<Registry>,
    /// Every COMPILED piece id, in migration order; the registry holds only the enabled ones.
    compiled: Vec<&'static str>,
    clock: Arc<dyn Clock>,
    refresh_timeout: Duration,
    cache: Mutex<Option<(i64, ProbeSnapshot)>>,
}

impl ServerProbe {
    pub fn new(
        ledger: Arc<dyn Ledger>,
        registry: Arc<Registry>,
        compiled: Vec<&'static str>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::with_refresh_timeout(ledger, registry, compiled, clock, REFRESH_TIMEOUT)
    }

    pub fn with_refresh_timeout(
        ledger: Arc<dyn Ledger>,
        registry: Arc<Registry>,
        compiled: Vec<&'static str>,
        clock: Arc<dyn Clock>,
        refresh_timeout: Duration,
    ) -> Self {
        ServerProbe {
            ledger,
            registry,
            compiled,
            clock,
            refresh_timeout,
            cache: Mutex::new(None),
        }
    }

    async fn refresh(&self) -> ProbeSnapshot {
        let read = tokio::time::timeout(self.refresh_timeout, self.ledger.read())
            .await
            .unwrap_or(LedgerRead::Unreachable);
        let (reachable, current) = match &read {
            LedgerRead::Unreachable => (false, false),
            LedgerRead::PlatformNotRegistered => (true, false),
            LedgerRead::Answered(states) => (true, verdict(states) == Verdict::Current),
        };
        let healths = self.registry.health().await;
        let pieces = self
            .compiled
            .iter()
            .map(|id| {
                let found = healths.iter().find(|h| h.piece == *id);
                PieceHealth {
                    name: (*id).to_owned(),
                    enabled: found.is_some(),
                    ready: found.is_some_and(|h| h.health == Health::Ready),
                }
            })
            .collect();
        ProbeSnapshot {
            reachable,
            current,
            pieces,
        }
    }
}

#[async_trait]
impl Probe for ServerProbe {
    /// The cached snapshot while it is younger than the TTL on the injected clock, else a new one. The lock is held across the refresh,
    /// so callers that overlap share one read. The new answer is stamped when it ARRIVES. A clock that went backwards keeps no answer.
    async fn snapshot(&self) -> ProbeSnapshot {
        let mut cache = self.cache.lock().await;
        let now = self.clock.now().unix_micros();
        if let Some((at, snap)) = cache.as_ref()
            && now >= *at
            && now - *at < PROBE_TTL_MICROS
        {
            return snap.clone();
        }
        let snap = self.refresh().await;
        *cache = Some((self.clock.now().unix_micros(), snap.clone()));
        snap
    }
}
