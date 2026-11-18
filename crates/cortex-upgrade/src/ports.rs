//! The ports of the upgrade engine (`pieces/14-cortex-upgrade-ENGINE-PORTS-r0.md`, `...-REAL-PORTS-BRIEF-r0.md`).
//!
//! Everything the engine needs from outside the files the crate owns is a trait in THIS file. The engine is tested with
//! fakes of these traits; the real implementations live in `src/real/` and are thin wrappers that start one program
//! with an argument vector.

use crate::manifest::Digest;
use crate::version::Version;
use cortex_types::{ModuleId, Secret};
use std::path::Path;
use std::time::Duration;

// ---------------------------------------------------------------- what a port may say back

/// The three compose services the engine ever names. A closed set on purpose: a wrapper cannot be handed another name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Db,
    Migrate,
    Server,
}

impl Service {
    pub const ALL: [Service; 3] = [Service::Db, Service::Migrate, Service::Server];
    /// The service's name in `compose.yml`.
    pub fn name(self) -> &'static str {
        match self {
            Service::Db => "db",
            Service::Migrate => "migrate",
            Service::Server => "server",
        }
    }
}

/// The most characters of a child's line that are kept (before the escaping and the `...`).
const ONE_LINE_MAX: usize = 200;

/// ONE line that came from a child program, bounded and escaped, so that no free text from outside ever reaches a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneLine(String);

impl OneLine {
    /// Escaped, at most 200 characters of `raw`, `...` after them when something was cut.
    pub fn new(raw: &str) -> OneLine {
        let mut out = String::new();
        for c in raw.chars().take(ONE_LINE_MAX) {
            match c {
                // readable text; a backslash and every other character that is not plain text is escaped
                '"' | '\'' => out.push(c),
                _ => out.extend(c.escape_debug()),
            }
        }
        if raw.chars().nth(ONE_LINE_MAX).is_some() {
            out.push_str("...");
        }
        OneLine(out)
    }
    /// The last non-empty line of `output` (bytes that are not UTF-8 are replaced), as a `OneLine`; empty output gives
    /// an empty line.
    pub fn last_line_of(output: &[u8]) -> OneLine {
        let text = String::from_utf8_lossy(output);
        let last = text
            .split('\n')
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        OneLine::new(last)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for OneLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A port call that did not give an answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    /// The program could not be started, or it ended in a way the port treats as a failure; the last line it wrote to
    /// its error stream.
    #[error("failed: {0}")]
    Failed(OneLine),
    /// It ran longer than the timeout and was ended.
    #[error("timed out")]
    TimedOut,
}

// ---------------------------------------------------------------- Stack: the running server

/// The authenticated read of the running server (`GET /v2/whoami`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Whoami {
    Ok,
    /// Refused with 401; the code is one of the closed list, or `None` when the body held anything else.
    Refused(Option<&'static str>),
    /// Anything else: a status such as `503`, or `timeout`, or `no answer`.
    Unanswered(String),
}

/// `GET /v2/health/ready`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ready {
    Yes,
    /// A status such as `503`, or `timeout`, or `no answer`.
    No(String),
}

pub trait Stack {
    fn ready(&self) -> Ready;
    fn whoami(&self, token: &Secret) -> Whoami;
}

// ---------------------------------------------------------------- Psql: the database check

/// The database could not be asked (not running, not reachable, the command failed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoAnswer;

pub trait Psql {
    /// The raw answer to "how many trust rules" (design §4a); the engine judges it.
    fn trust_rule_count(&self) -> Result<String, NoAnswer>;
}

// ---------------------------------------------------------------- Containers: podman and compose

/// What `podman` says about one service's container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    /// No such container.
    Missing,
    /// Running and it has no health check.
    Running,
    /// Running, health check not passing yet.
    Starting,
    Healthy,
    Unhealthy,
    /// Ended, with this exit code (the one-shot `migrate` container).
    Exited(i32),
    /// Anything else (created, paused, stopping, removing).
    Other,
}

pub trait Containers {
    /// Load the image archive; the digest of the one image it holds. An archive that loads no image or more than one
    /// is `Failed`.
    fn load_image(&self, archive: &Path) -> Result<Digest, PortError>;
    /// `compose up -d <services>` for the release in `release_dir`, with that release's `compose.yml` and `stack.env`.
    fn compose_up(&self, release_dir: &Path, services: &[Service]) -> Result<(), PortError>;
    /// One reading of one service's container (the engine polls; a port never waits).
    fn state(&self, release_dir: &Path, service: Service) -> Result<ServiceState, PortError>;
    /// The module-roles script of the release in `release_dir` with the module ids, under `timeout`. A refusal by the
    /// script is `Failed` with its one line.
    fn run_roles(
        &self,
        release_dir: &Path,
        modules: &[ModuleId],
        timeout: Duration,
    ) -> Result<(), PortError>;
}

// ---------------------------------------------------------------- Dumper: cortex-dump

/// The label of a pre-upgrade dump, `pre-upgrade-<from>-to-<to>`; it can be made only from two checked versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpLabel(String);

impl DumpLabel {
    pub fn new(from: &Version, to: &Version) -> DumpLabel {
        DumpLabel(format!("pre-upgrade-{from}-to-{to}"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What `cortex-dump` did. Only `exit` decides; the other two are reported, never trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpResult {
    pub exit: i32,
    /// The last line on its standard output (the set path on success), at most 4096 bytes; `None` when it wrote none.
    pub last_stdout_line: Option<String>,
    pub last_stderr_line: OneLine,
}

pub trait Dumper {
    fn dump(&self, label: &DumpLabel, timeout: Duration) -> Result<DumpResult, PortError>;
}

// ---------------------------------------------------------------- Pace: time

/// The engine's clock and its way of waiting, so that tests of timeouts do not wait.
pub trait Pace {
    /// Time since some fixed start; never goes back.
    fn now(&self) -> Duration;
    fn sleep(&self, d: Duration);
}

/// The real one.
pub struct SystemPace {
    start: std::time::Instant,
}

impl SystemPace {
    pub fn new() -> SystemPace {
        SystemPace {
            start: std::time::Instant::now(),
        }
    }
}

impl Default for SystemPace {
    fn default() -> Self {
        Self::new()
    }
}

impl Pace for SystemPace {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}
