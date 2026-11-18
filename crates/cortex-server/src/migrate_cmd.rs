//! `cortex-server migrate` (contract r0.5 sections 5 and 6; piece 11, cox): the owner role applies the migration sets of `platform` and of
//! every ENABLED piece, in the registry's migration order, forward only. The order and the sets come from the ASSEMBLED registry, so
//! `migrate` and `serve` can never disagree about which pieces exist; the decision (what is refused, what is printed) is
//! [`crate::migrate::run`], the real applier is [`crate::migrator::DbMigrator`], the owner connection is [`crate::dbowner::owner_config`].
//!
//! Every refusal is ONE line of fixed text that names a key or a module at most: no SQL, no password, no user name, no path.

use std::ffi::OsString;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use cortex_config::{Config, Env};
use cortex_db::migrate::platform_migrations;
use cortex_db::{ConnectFailure, DbEndpoint, MigrationSet, OwnerConfig, OwnerConn};
use cortex_module::Registry;
use cortex_types::Secret;

use crate::assembly::{assemble, duplicate_enabled, startup_line};
use crate::bootstrap::Outcome;
use crate::cli::{EXIT_FAILURE, EXIT_OK};
use crate::dbowner::{OwnerConfigError, owner_config};
use crate::migrate::run;
use crate::migrator::DbMigrator;

fn refused(what: &str) -> Outcome {
    Outcome {
        code: EXIT_FAILURE,
        stderr: Some(format!("migrate refused: {what}")),
    }
}

/// How long `migrate` waits for a database that is not ready yet (DEFECT S, cox 7615a247). ONE monotonic deadline, `bound` after the start, covers the
/// host-name lookup, every connection attempt and handshake, and every sleep: each is cut at the REMAINING time and a failure never extends it. The
/// sleep between attempts starts at `first`, grows by `first` after each attempt up to `cap`. When the deadline passes the answer is the refusal
/// `migrate_with` always had. A `bound` of zero is the old behaviour exactly: one attempt, no lookup of its own, no timeout, no retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbWait {
    pub bound: Duration,
    pub first: Duration,
    pub cap: Duration,
    /// A progress line every `note_every` of ELAPSED time (and one at the first failed attempt), even while one attempt is still pending.
    pub note_every: Duration,
}

impl DbWait {
    /// No wait at all: the first failed connect is the answer (what `migrate_with` has always done).
    pub const NONE: DbWait = DbWait {
        bound: Duration::ZERO,
        first: Duration::ZERO,
        cap: Duration::ZERO,
        note_every: Duration::ZERO,
    };
}

/// Apply every migration of `platform` and of the registry's enabled pieces under the OWNER role, WITHOUT waiting for the database: the first
/// failed connect is the answer. `registry` is the assembled one.
pub async fn migrate_with(cfg: &Config, registry: &Registry) -> Outcome {
    migrate_with_wait(cfg, registry, DbWait::NONE, &mut |_| {}).await
}

/// The one line a connection that never came has always had; the install tests and the upgrade engine read it.
const CONNECT_REFUSED: &str = "migrate failed: could not connect to the database as the owner";
/// The first progress line. It carries no host, port, user or SQLSTATE.
const WAITING: &str = "migrate: waiting for the database (it is not ready yet)";

/// The environment variable that sets the wait of the PRODUCTION entry, in whole seconds (a process-level input: `migrate_with` reads no environment).
pub const WAIT_ENV: &str = "CORTEX_MIGRATE_DB_WAIT_SECS";
/// The wait when the variable is unset.
pub const DEFAULT_WAIT_SECS: u64 = 300;
/// The largest value the variable may name.
const MAX_WAIT_SECS: u64 = 3600;
/// How long the runtime may take to shut down after the answer is known (an uncancellable resolver thread must not hold the process).
const TEARDOWN: Duration = Duration::from_secs(2);
const KNOB_REFUSED: &str =
    "migrate refused: CORTEX_MIGRATE_DB_WAIT_SECS is not a whole number of seconds from 0 to 3600";

impl DbWait {
    /// The production policy for `secs` of wait: steps of 1, 2, 3 seconds and then 3 seconds, a progress line every 30 seconds.
    pub fn production(secs: u64) -> DbWait {
        DbWait {
            bound: Duration::from_secs(secs),
            first: Duration::from_secs(1),
            cap: Duration::from_secs(3),
            note_every: Duration::from_secs(30),
        }
    }
}

/// The value of [`WAIT_ENV`] as the process saw it: unset gives the default, a whole number of ASCII decimal seconds from 0 to 3600 is taken, and
/// anything else (empty, a sign, a space, a fraction, a non-ASCII digit, an overflow, over 3600, not Unicode) is `None`, without echoing it.
pub fn parse_wait(value: Option<OsString>) -> Option<u64> {
    let Some(raw) = value else {
        return Some(DEFAULT_WAIT_SECS);
    };
    let text = raw.to_str()?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u64>().ok().filter(|n| *n <= MAX_WAIT_SECS)
}

/// What one attempt came to.
enum Attempt {
    Connected(OwnerConn),
    NotReady,
    Refused,
}

/// One attempt. A TCP host that is a NAME is resolved here, by our own lookup, so that a resolver failure is identified positively (and is "not
/// ready": the network or its DNS may not be up yet) and is not guessed from an I/O error kind; each address is then tried in order. Anything
/// else (an address, a socket directory) goes straight to the classified connect.
async fn attempt(owner: &OwnerConfig) -> Attempt {
    fn from(r: Result<OwnerConn, ConnectFailure>) -> Attempt {
        match r {
            Ok(c) => Attempt::Connected(c),
            Err(ConnectFailure::NotReady) => Attempt::NotReady,
            Err(ConnectFailure::Refused) => Attempt::Refused,
        }
    }
    let DbEndpoint::Tcp { host, port } = &owner.endpoint else {
        return from(OwnerConn::connect_classified(owner).await);
    };
    if host.parse::<IpAddr>().is_ok() {
        return from(OwnerConn::connect_classified(owner).await);
    }
    let Ok(found) = tokio::net::lookup_host((host.as_str(), *port)).await else {
        return Attempt::NotReady;
    };
    for addr in found {
        let by_address = OwnerConfig {
            endpoint: DbEndpoint::Tcp {
                host: addr.ip().to_string(),
                port: addr.port(),
            },
            database: owner.database.clone(),
            owner_user: owner.owner_user.clone(),
            owner_password: Secret::new(owner.owner_password.expose().to_owned()),
        };
        match OwnerConn::connect_classified(&by_address).await {
            Err(ConnectFailure::NotReady) => continue,
            other => return from(other),
        }
    }
    Attempt::NotReady
}

/// Connect as the owner within `wait`; `None` is the refusal. Progress lines follow ELAPSED time, also while one attempt is pending.
async fn connect_owner(
    owner: &OwnerConfig,
    wait: DbWait,
    note: &mut dyn FnMut(&str),
) -> Option<OwnerConn> {
    use tokio::time::{Instant, sleep_until};
    if wait.bound.is_zero() {
        return OwnerConn::connect_classified(owner).await.ok();
    }
    let start = Instant::now();
    let deadline = start + wait.bound;
    let mut next_note = start + wait.note_every;
    let mut said = false;
    let say = |note: &mut dyn FnMut(&str), said: &mut bool| {
        if *said {
            note(&format!(
                "migrate: still waiting for the database ({} s)",
                start.elapsed().as_secs()
            ));
        } else {
            note(WAITING);
            *said = true;
        }
    };
    let mut step = wait.first;
    loop {
        let attempted = attempt(owner);
        tokio::pin!(attempted);
        let result = loop {
            tokio::select! {
                r = &mut attempted => break Some(r),
                _ = sleep_until(deadline) => break None,
                _ = sleep_until(next_note), if !wait.note_every.is_zero() => {
                    say(note, &mut said);
                    next_note += wait.note_every;
                }
            }
        };
        match result {
            None | Some(Attempt::Refused) => return None,
            Some(Attempt::Connected(c)) => return Some(c),
            Some(Attempt::NotReady) => {}
        }
        if !said {
            say(note, &mut said);
        }
        // sleep, never past the deadline, still saying how long it has been
        let wake = (Instant::now() + step).min(deadline);
        loop {
            tokio::select! {
                _ = sleep_until(wake) => break,
                _ = sleep_until(next_note), if !wait.note_every.is_zero() => {
                    say(note, &mut said);
                    next_note += wait.note_every;
                }
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        step = (step + wait.first).min(wait.cap);
    }
}

/// [`migrate_with`], waiting up to `wait.bound` for a database that is not ready yet. `note` receives each short progress line. Only the CONNECT
/// is retried: once there is a connection, a failing migration is returned at once and nothing is rerun.
pub async fn migrate_with_wait(
    cfg: &Config,
    registry: &Registry,
    wait: DbWait,
    note: &mut dyn FnMut(&str),
) -> Outcome {
    let Some(db) = &cfg.database else {
        return refused("the config has no [database] table");
    };
    let owner = match owner_config(db) {
        Ok(c) => c,
        Err(e @ (OwnerConfigError::Missing(_) | OwnerConfigError::PasswordUnreadable)) => {
            return refused(&e.to_string());
        }
    };
    let order = registry.db_modules();
    let mut sets: Vec<MigrationSet> = vec![platform_migrations()];
    sets.extend(registry.pieces().map(|p| p.migrations()));
    let Some(conn) = connect_owner(&owner, wait, note).await else {
        return Outcome {
            code: EXIT_FAILURE,
            stderr: Some(CONNECT_REFUSED.to_owned()),
        };
    };
    run(cfg, &order, &mut DbMigrator::new(conn, sets)).await
}

/// The command: the config, the assembly, then [`migrate_with`] (no wait). The returned outcome is what `main` prints and exits with.
pub async fn command(config: &Path) -> Outcome {
    command_with(config, DbWait::NONE, &mut |_| {}).await
}

/// [`command`] with a wait policy and a progress sink: the production entry passes the policy the environment named.
pub async fn command_with(config: &Path, wait: DbWait, note: &mut dyn FnMut(&str)) -> Outcome {
    let cfg = match cortex_config::load(config, &Env::from_process()) {
        Ok(c) => c,
        Err(e) => {
            return refused(&format!(
                "configuration: {}",
                e.to_string().replace(['\n', '\r'], " ")
            ));
        }
    };
    if let Some(twice) = duplicate_enabled(&cfg.modules.enabled) {
        return refused(&format!("module {} is enabled twice", twice.as_str()));
    }
    let registry = match assemble(&cfg) {
        Ok(r) => r,
        Err(errs) => return refused(&startup_line(&errs)),
    };
    migrate_with_wait(&cfg, &registry, wait, note).await
}

/// The process entry: the wait variable FIRST (a bad value is refused before the config is read and before any connection), then a runtime,
/// [`command_with`], the refusal on stderr, an exit code, and a runtime that is torn down in bounded time.
pub fn run_command(config: &Path) -> i32 {
    let Some(secs) = parse_wait(std::env::var_os(WAIT_ENV)) else {
        eprintln!("{KNOB_REFUSED}");
        return EXIT_FAILURE;
    };
    let wait = DbWait::production(secs);
    let outcome = block_on_bounded(
        command_with(config, wait, &mut |line| eprintln!("{line}")),
        TEARDOWN,
    );
    let Some(outcome) = outcome else {
        eprintln!("migrate refused: could not start the runtime");
        return EXIT_FAILURE;
    };
    if let Some(line) = &outcome.stderr {
        eprintln!("{line}");
    }
    if outcome.code == EXIT_OK {
        EXIT_OK
    } else {
        outcome.code
    }
}

/// Run `fut` on a runtime of its own and tear the runtime down in BOUNDED time (DEFECT S, MWD-SAM-02): a blocking task that cannot be cancelled (a
/// host-name lookup in the resolver) must not keep the process alive after the answer is known. `None` only if the runtime cannot be built.
pub fn block_on_bounded<F: std::future::Future>(fut: F, teardown: Duration) -> Option<F::Output> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .ok()?;
    let out = rt.block_on(fut);
    rt.shutdown_timeout(teardown);
    Some(out)
}
