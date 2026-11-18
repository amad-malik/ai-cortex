//! The upgrade engine (design §4), built in steps. THIS increment: the arguments, the ports, the adapter for the local
//! server, and steps 2 and 3 (read the running release, the database check, the token pre-flight, the manifest, the
//! choice) with `--check` on top. Staging, unpack, comparison, dump, load, commit, roles, migrate, health and rollback
//! are NOT built yet. Everything outside the files the crate owns is behind a port (`pieces/14-...-ENGINE-PORTS-r0.md`).

use crate::choose::{Decision, Installed, Wanted, choose};
use crate::compose::compare_db_service;
use crate::fetch::{FetchError, Fetcher, Request, Transport};
use crate::layout::{
    CommitStep, Flip, Home, Incomplete, LayoutError, PrevLink, Record, RecordResult, StepRecord,
};
use crate::manifest::{self, Digest, MigrationClass, Release};
use crate::members::{self, Kind, Layout as MemberLayout, UNIT_FILES};
use crate::stack_env::{STACK_ENV_MAX_BYTES, parse_stack_env};
use crate::text::{cut, shown};
use crate::unpack::{UnpackLimits, create_private_dir_exact, unpack_release_gz};
use crate::version::{Version, parse_version};
use cortex_types::{Clock, Secret};
use sha2::Digest as _;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The exit codes of design §4.
pub const EXIT_DONE: u8 = 0;
pub const EXIT_NEWER: u8 = 10;
pub const EXIT_REFUSED: u8 = 2;
pub const EXIT_ROLLED_BACK: u8 = 3;
pub const EXIT_ROLLBACK_FAILED: u8 = 4;
/// A resumed upgrade that failed again (cox W1-56 B): nothing was rolled back, the operator decides. The same code 4 as "needs the operator".
pub const EXIT_RESUME_FAILED: u8 = 4;

/// What `upgrade` asks at its two seams: right after the marker of step 8 was written and BEFORE the flip, and after each of the three effects of the
/// flip (returning false stops it, as `Home::commit_with`). Always empty in a production build: only `Hooks` (below) can fill it.
#[derive(Default)]
struct HookSet<'a> {
    after_marker: Option<&'a dyn Fn()>,
    commit_step: Option<&'a dyn Fn(CommitStep) -> bool>,
    member_point: Option<&'a dyn Fn(&str)>,
    before_late_marker: Option<&'a dyn Fn()>,
}

/// Test hooks (cox W1-56: the lost-process cells), ONLY in a build with the `test-hooks` feature (off by default, never in a release recipe: mike's finding on
/// packet 15 r1): `after_marker` runs right after the marker was written and BEFORE the flip, `commit_step` after each of the three effects of the flip. A
/// test makes either panic to stand for a lost process. The real caller uses `run`, which passes none.
#[cfg(feature = "test-hooks")]
#[derive(Default)]
pub struct Hooks<'a> {
    pub after_marker: Option<&'a dyn Fn()>,
    pub commit_step: Option<&'a dyn Fn(CommitStep) -> bool>,
    /// Called with the name of every boundary of the member protocol (`plan:written`, `<member>:replaced`, `promote:S2`, ...): a test panics at one to stand for a
    /// lost process.
    pub member_point: Option<&'a dyn Fn(&str)>,
    /// Called just BEFORE the later read of the marker of an unfinished upgrade (R-6c, cox W1-105): a test uses it to change the home between the early read
    /// and this one, as a normal concurrent upgrade does (a check holds no lock).
    pub before_late_marker: Option<&'a dyn Fn()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Upgrade,
    Check,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub to: Option<Version>,
    pub health_interval: Duration,
    pub health_timeout: Duration,
    pub roles_timeout: Duration,
    /// How long the one-shot `migrate` container may run (its own bound, apart from `health_timeout`).
    pub migrate_timeout: Duration,
}

/// What the caller (`cortex-cli`) read from the environment, once. The engine reads no environment variable itself.
#[derive(Debug, Clone)]
pub struct Env {
    pub home: PathBuf,
    /// The manifest's URL on the release host (https, the release host only).
    pub manifest_url: String,
    /// The running server, `http://127.0.0.1:<port>`.
    pub local_url: String,
    /// The operator's token file (`CORTEX_TOKEN_FILE`).
    pub operator_token_file: PathBuf,
    /// Where the backup user units live (`$HOME/.config/systemd/user`, the entry point's rule is install.sh's; R-5). Absolute; checked before step 8 writes anything.
    pub unit_dir: PathBuf,
    /// The verified ABSOLUTE compose provider the entry read from `<home>/compose-provider` ONCE (`provider::read_compose_provider`; R-6, cox W1-68, W1-91):
    /// the real containers port was built with the same path and carries it to every compose call. An upgrade or a rollback whose provider is not an absolute
    /// path (a caller that did not read it) is refused before anything is written; a check starts no container and needs none.
    pub compose_provider: PathBuf,
}

/// What a run prints and returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub code: u8,
    pub lines: Vec<String>,
}

pub use crate::ports::{
    Containers, DumpLabel, DumpResult, Dumper, NoAnswer, Pace, PortError, Psql, Ready, Service,
    ServiceState, Stack, Whoami,
};

/// The role script's own transaction budget inside the database (`budget=30` in deploy/db-roles.sh, held by checker rule S5-22) and the margin
/// the engine's `run_roles` deadline must keep over it (cox 19:41Z on mike's R6). The wrapper can only end the script's own sh; the psql in the
/// container is another process that could still COMMIT, so the script's SESSION carries a budget the server enforces, and the deadline is at
/// least the budget plus the margin: `--roles-timeout` below that floor (45 s) is refused.
pub const ROLES_SCRIPT_BUDGET: Duration = Duration::from_secs(30);
pub const ROLES_MARGIN: Duration = Duration::from_secs(15);
/// The eleven steps of an upgrade, in the order the record lists them.
const STEP_NAMES: [&str; 11] = [
    "lock", "read", "choose", "stage", "unpack", "dump", "load", "commit", "restart", "check",
    "record",
];

/// THE RELATION (cox W1-55; mike's M-4), mirrored from `deploy/db-roles.sh` and checker rule S5-22. The CALLER passes its absolute deadline `E` (the real adapter: the moment it
/// launches the script + the `run_roles` timeout); the script reads no clock of its own. The last statement of the roles transaction refuses when the database clock is past
/// `E - ROLES_LEAD`, and the lead is the sum of three terms: `ROLES_IDLE` (a client that stalls between the guard and COMMIT is ended by `idle_in_transaction_session_timeout`),
/// `ROLES_COMMIT_LATENCY` (COMMIT itself) and `ROLES_SLACK` (what is left before `E`). So the LATEST COMPLETED COMMIT IS `E - ROLES_SLACK`, whatever delayed the launch, the lock or the
/// client. Cell `en_09` asserts each term, the sum and the distance; the script's own lines `idle=5`, `latency=2`, `slack=2`, `lead=$((idle + latency + slack))` are pinned by S5-22.
pub const ROLES_IDLE: Duration = Duration::from_secs(5);
pub const ROLES_COMMIT_LATENCY: Duration = Duration::from_secs(2);
pub const ROLES_SLACK: Duration = Duration::from_secs(2);
pub const ROLES_LEAD: Duration = Duration::from_secs(
    ROLES_IDLE.as_secs() + ROLES_COMMIT_LATENCY.as_secs() + ROLES_SLACK.as_secs(),
);
/// The deadline of the ONE confirmation re-run after a `TimedOut` (W1-49 B): fresh, and exactly the floor (budget + margin).
pub const ROLES_CONFIRM_TIMEOUT: Duration = Duration::from_secs(45);

/// Why step 9 (restart) stopped. `RolesUnknown` is not an ordinary failure: the role step and its one confirmation both timed out, so the
/// engine does not know whether the module roles were applied, and (cox W1-49 B) it does NOT roll back.
enum RestartError {
    Failed(String),
    RolesUnknown(String),
}

impl From<String> for RestartError {
    fn from(why: String) -> Self {
        Self::Failed(why)
    }
}

fn seconds(flag: &str, value: Option<&String>) -> Result<Duration, String> {
    let v = value.ok_or_else(|| format!("{flag} needs a number of seconds"))?;
    let ok = (1..=4).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit());
    match v.parse::<u64>() {
        Ok(n) if ok && (1..=3600).contains(&n) => Ok(Duration::from_secs(n)),
        _ => Err(format!(
            "{flag} needs a whole number of seconds from 1 to 3600"
        )),
    }
}

/// `[--check | --rollback] [--to VERSION] [--health-interval S] [--health-timeout S] [--roles-timeout S] [--migrate-timeout S]`; every flag at
/// most once; nothing else is accepted. The error is one line.
pub fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut out = Args {
        mode: Mode::Upgrade,
        to: None,
        health_interval: Duration::from_secs(2),
        health_timeout: Duration::from_secs(120),
        roles_timeout: Duration::from_secs(60),
        migrate_timeout: Duration::from_secs(1800),
    };
    let (mut mode_set, mut hi, mut ht, mut rt, mut mt) = (false, false, false, false, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let once = |seen: &mut bool, flag: &str| -> Result<(), String> {
            if std::mem::replace(seen, true) {
                Err(format!("{flag} is given twice"))
            } else {
                Ok(())
            }
        };
        match a.as_str() {
            "--check" | "--rollback" => {
                once(&mut mode_set, "--check or --rollback")?;
                out.mode = if a == "--check" {
                    Mode::Check
                } else {
                    Mode::Rollback
                };
            }
            "--to" => {
                if out.to.is_some() {
                    return Err("--to is given twice".into());
                }
                let v = it.next().ok_or("--to needs a version")?;
                out.to = Some(parse_version(v).map_err(|e| format!("--to: {e}"))?);
            }
            "--health-interval" => {
                once(&mut hi, "--health-interval")?;
                out.health_interval = seconds("--health-interval", it.next())?;
            }
            "--health-timeout" => {
                once(&mut ht, "--health-timeout")?;
                out.health_timeout = seconds("--health-timeout", it.next())?;
            }
            "--roles-timeout" => {
                once(&mut rt, "--roles-timeout")?;
                out.roles_timeout = seconds("--roles-timeout", it.next())?;
                let floor = ROLES_SCRIPT_BUDGET + ROLES_MARGIN;
                if out.roles_timeout < floor {
                    return Err(format!(
                        "--roles-timeout must be at least {} seconds: the role script's own transaction budget is {} s and the deadline must exceed it by {} s",
                        floor.as_secs(),
                        ROLES_SCRIPT_BUDGET.as_secs(),
                        ROLES_MARGIN.as_secs()
                    ));
                }
            }
            "--migrate-timeout" => {
                once(&mut mt, "--migrate-timeout")?;
                out.migrate_timeout = seconds("--migrate-timeout", it.next())?;
            }
            other => {
                return Err(format!(
                    "unknown argument {}; usage: cortex upgrade [--check | --rollback] [--to VERSION]",
                    cut(other)
                ));
            }
        }
    }
    if out.mode == Mode::Rollback && out.to.is_some() {
        return Err("--rollback and --to cannot be given together".into());
    }
    Ok(out)
}

/// The adapter that talks to the local server through a [`Transport`] (the real one is `LocalTransport`).
pub struct HttpStack<T: Transport> {
    pub transport: T,
    pub base: String,
}

/// The 401 codes the pre-flight may print (design §4c). Anything else the server says is never printed.
const CLOSED_CODES: [&str; 7] = [
    "token_missing",
    "token_invalid",
    "token_expired",
    "token_revoked",
    "token_superseded",
    "generation_stale",
    "worker_retired",
];

impl<T: Transport> Stack for HttpStack<T> {
    fn ready(&self) -> Ready {
        // No token: the operator's token goes to /v2/whoami only. Only a 200 is ready; nothing from the body is read.
        let url = format!("{}/v2/health/ready", self.base);
        match self.transport.get(&Request {
            url: &url,
            bearer: None,
        }) {
            Ok(r) if r.status == 200 => Ready::Yes,
            Ok(r) => Ready::No(r.status.to_string()),
            Err(FetchError::Timeout) => Ready::No("timeout".into()),
            Err(_) => Ready::No("no answer".into()),
        }
    }

    fn whoami(&self, token: &Secret) -> Whoami {
        let url = format!("{}/v2/whoami", self.base);
        let mut response = match self.transport.get(&Request {
            url: &url,
            bearer: Some(token.expose()),
        }) {
            Ok(r) => r,
            Err(FetchError::Timeout) => return Whoami::Unanswered("timeout".into()),
            Err(_) => return Whoami::Unanswered("no answer".into()),
        };
        match response.status {
            200 => Whoami::Ok,
            401 => {
                let mut body = Vec::new();
                if response.body.read_to_end(&mut body).is_err() {
                    return Whoami::Refused(None);
                }
                let code = serde_json::from_slice::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v.get("code")
                            .and_then(|c| c.as_str())
                            .and_then(|c| CLOSED_CODES.iter().find(|k| **k == c).copied())
                    });
                Whoami::Refused(code)
            }
            other => Whoami::Unanswered(other.to_string()),
        }
    }
}

pub struct Engine<'a, T: Transport> {
    pub env: &'a Env,
    pub fetcher: &'a Fetcher<T>,
    pub stack: &'a dyn Stack,
    pub psql: &'a dyn Psql,
    pub containers: &'a dyn Containers,
    pub dumper: &'a dyn Dumper,
    pub pace: &'a dyn Pace,
    /// Wall time, for the name of the record only.
    pub clock: &'a dyn Clock,
}

/// Why a rollback did not happen: refused before anything was changed, or started and then failed.
enum RollbackError {
    Refused(String),
    Failed(String, Flip),
    /// The rollback was STARTED (its intent is written) and stopped on an error before the links were all flipped; the pair is
    /// what `current` and `previous` named before, when they could be read.
    Interrupted(String, Option<Flip>),
    /// The backup scripts and units (R-5) cannot be handled: the inventory of the upgrade is lost or unusable BEFORE the flip (nothing was changed), or an
    /// entry could not be put back (the text names it). Exit 4 either way (cox W1-59 C): never a warning.
    Members(String),
}

impl RollbackError {
    fn text(&self) -> String {
        match self {
            RollbackError::Refused(r) => r.clone(),
            RollbackError::Failed(why, _)
            | RollbackError::Interrupted(why, _)
            | RollbackError::Members(why) => why.clone(),
        }
    }
}

/// A no-op crash point: what the real caller passes to the member protocol.
fn no_point(_: &str) {}

/// Said when a rollback stopped after it started: where the way back is and what to do.
fn interrupted_lines(why: &str) -> Vec<String> {
    vec![
        format!("the rollback was started and stopped: {why}"),
        "the way back is written down in rollback.intent in the home directory and nothing was deleted: fix the cause and run cortex upgrade --rollback again, which finishes the links first and then starts the old release".to_string(),
    ]
}

fn refusal_text(e: LayoutError) -> String {
    match e {
        LayoutError::RollbackRefused(r) => r.to_string(),
        other => other.to_string(),
    }
}

/// Said when a rollback has flipped the links and the old release does not run: what the next run will do.
const UNFINISHED_LINE: &str = "the rollback is still unfinished: the next run of cortex upgrade (or cortex upgrade --rollback) starts the old release again before it does anything else, and nothing can switch until then";

/// kai's sentence (DV-4 condition 1), word for word.
const RESTORE_SENTENCE: &str =
    "restore ONLY with cortex-restore; a plain pg_restore can make revoked tokens valid again";

/// The text of exit 4 (design §5): the pre-upgrade set, kai's sentence, the order for 0.2.001, and what a restore does to tokens.
fn exit4_lines(set: Option<&str>) -> Vec<String> {
    vec![
        match set {
            Some(s) => format!("the pre-upgrade dump set is {}", shown(s)),
            None => "no pre-upgrade dump set is recorded".to_string(),
        },
        RESTORE_SENTENCE.to_string(),
        "order for this release: stop the server, restore the set with cortex-restore into a NEW empty database, point the stack at it, start it".to_string(),
        "every token issued before the restore is refused: re-issue the lead token with install.sh --reissue-token and point CORTEX_TOKEN_FILE at the new file".to_string(),
    ]
}

/// `dump_set` of the newest upgrade record that went from `now` to `left` and has one: the set belongs to the upgrade that is
/// being left, and a newer record of another attempt is never borrowed (mike, packet 7, choice 6). Best effort: the records
/// are ours.
fn dump_set_for(home: &Path, left: &Version, now: &Version) -> Option<String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(home.join("upgrades"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    // oldest first: by stamp, then by the -N suffix of a second record written in the same second
    files.sort_by_key(|p| {
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        match stem.rsplit_once('-') {
            Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
                (base.to_string(), n.parse::<u32>().unwrap_or(0))
            }
            _ => (stem, 1),
        }
    });
    for file in files.iter().rev() {
        let Some(text) = read_small(file, 1 << 20) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let names = |key: &str, v: &Version| {
            value.get(key).and_then(|x| x.as_str()) == Some(v.to_string().as_str())
        };
        if !names("from", now) || !names("to", left) {
            continue;
        }
        if let Some(set) = value.get("dump_set").and_then(|d| d.as_str()) {
            return Some(crate::ports::OneLine::new(set).as_str().to_string());
        }
    }
    None
}

/// Where the links stand after a failed `Home::commit`, judged from `current` alone: still the old release (the switch did not
/// happen), the new one (it did, and something after it failed), or anything else, including a link that cannot be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchState {
    NotSwitched,
    Switched,
    Unknown,
}

/// The state of the links after a failed commit from `from` to `to` (mike, packet 7, finding 3).
pub fn switch_state(
    current: Result<Option<Version>, LayoutError>,
    from: &Version,
    to: &Version,
) -> SwitchState {
    match current {
        Ok(Some(c)) if c == *from => SwitchState::NotSwitched,
        Ok(Some(c)) if c == *to => SwitchState::Switched,
        _ => SwitchState::Unknown,
    }
}

/// What one poll of a wait says.
enum Wait {
    Done,
    Pending,
    Fail(String),
}

/// A file of the release directory's own (`manifest.json`, `SHA256SUMS`): new, 0644, written whole.
fn write_plain(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
}

/// The regular files under `dir` as paths relative to it (with `/`), except the top-level `SHA256SUMS`.
fn list_files(dir: &Path, prefix: &str, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            list_files(&entry.path(), &rel, out)?;
        } else if kind.is_file() && rel != "SHA256SUMS" {
            out.push(rel);
        }
    }
    Ok(())
}

/// Steps 4 and 5 done: the release is downloaded, unpacked and checked, and nothing is installed yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub from: Version,
    pub release: Box<Release>,
    /// `releases/<v>.part`, unpacked and verified.
    pub staging: PathBuf,
    /// `<home>/downloads/<v>`: the release archive and the server image archive, digest-checked.
    pub downloads: PathBuf,
    /// The plan, in the order it was found.
    pub lines: Vec<String>,
}

impl Prepared {
    /// Remove what this run created, by exact path: the staging directory, the download directory and the `downloads`
    /// directory when nothing else is in it. A link is never followed. Returns a warning for every path that is still
    /// there afterwards (a cleanup that fails is reported, never swallowed).
    pub fn discard(&self) -> Vec<String> {
        remove_and_report(&self.staging, &self.downloads)
    }
}

/// `remove_dir_all` of `path` when it is a real directory; a link, a file or nothing is left alone.
fn remove_real_dir(path: &Path) {
    if let Ok(m) = std::fs::symlink_metadata(path)
        && m.is_dir()
    {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// Remove the staging and download directories (and `downloads` when empty); say which of the two are still there.
fn remove_and_report(staging: &Path, downloads: &Path) -> Vec<String> {
    remove_real_dir(staging);
    remove_real_dir(downloads);
    if let Some(root) = downloads.parent() {
        let _ = std::fs::remove_dir(root);
    }
    let mut warnings = Vec::new();
    for (path, parent) in [(staging, "releases"), (downloads, "downloads")] {
        if std::fs::symlink_metadata(path).is_ok() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            warnings.push(format!(
                "warning: {parent}/{name} could not be removed; remove it by hand"
            ));
        }
    }
    warnings
}

/// Removes what a failed `stage` created, whichever way it returns, and reports a path it could not remove.
struct Cleanup {
    staging: PathBuf,
    downloads: PathBuf,
    armed: bool,
    report: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.armed {
            let warnings = remove_and_report(&self.staging, &self.downloads);
            self.report.borrow_mut().extend(warnings);
        }
    }
}

fn digest_text(d: &Digest) -> String {
    format!("sha256:{}", cortex_types::Sha256(d.0).to_hex())
}

/// The whole text of a small file, or `None` when it cannot be read, is not text, or is over `max` bytes.
fn read_small(path: &Path, max: usize) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(max as u64 + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() <= max).then_some(text)
}

fn file_sha256(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut h)?;
    Ok(h.finalize().into())
}

fn not_usable(why: &str) -> Outcome {
    refuse(format!("the dump set is not usable: {why}"))
}

/// `sha256sum`'s output for the files of a set, checked again (design §4 step 6; R14-04, R14-05): each line is 64 lowercase
/// hex digits, then two spaces or a space and `*`, then a plain file name; every named file is a regular file in the set
/// with that hash; at least one and at most 1000 lines; nothing else.
fn set_checksums_hold(set: &Path) -> bool {
    let Some(text) = read_small(&set.join("SHA256SUMS"), 1 << 20) else {
        return false;
    };
    let mut seen = 0;
    for line in text.lines() {
        let Some((hex, rest)) = line.split_at_checked(64) else {
            return false;
        };
        let Some(name) = rest.strip_prefix("  ").or_else(|| rest.strip_prefix(" *")) else {
            return false;
        };
        let name_ok = (1..=128).contains(&name.len())
            && !name.starts_with('.')
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !name_ok {
            return false;
        }
        let file = set.join(name);
        let regular = std::fs::symlink_metadata(&file)
            .map(|m| m.file_type().is_file())
            .unwrap_or(false);
        let Ok(got) = file_sha256(&file) else {
            return false;
        };
        let got_hex = cortex_types::Sha256(got).to_hex();
        if !regular || got_hex != hex {
            return false;
        }
        seen += 1;
        if seen > 1000 {
            return false;
        }
    }
    seen > 0
}

/// The `sha256:` part of `name@sha256:<64 hex>`.
fn image_digest(image_ref: &str) -> Option<Digest> {
    let hex = image_ref.rsplit_once("@sha256:")?.1;
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        let nib = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        out[i] = nib(pair[0])? << 4 | nib(pair[1])?;
    }
    Some(Digest(out))
}

/// Steps 2 and 3 done: what the next steps will work on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    NothingToDo(Version),
    Upgrade {
        from: Version,
        to: Box<Release>,
        /// The manifest as fetched (verbatim): the release directory keeps it as its verified copy.
        manifest: Vec<u8>,
    },
}

fn refuse(line: impl Into<String>) -> Outcome {
    Outcome {
        code: EXIT_REFUSED,
        lines: vec![line.into()],
    }
}

/// The operator's token from its file: text, 1 to 4096 bytes once trimmed. The refusal never holds the file's content.
fn read_token(path: &std::path::Path) -> Result<Secret, String> {
    let mut text = String::new();
    let file = std::fs::File::open(path)
        .map_err(|_| "the operator token file cannot be read (CORTEX_TOKEN_FILE)".to_string())?;
    file.take(4097)
        .read_to_string(&mut text)
        .map_err(|_| "the operator token file is not readable text".to_string())?;
    if text.len() > 4096 {
        return Err("the operator token file is larger than 4 KiB".into());
    }
    let token = text.trim();
    if token.is_empty() {
        return Err("the operator token file is empty".into());
    }
    Ok(Secret::new(token))
}

impl<T: Transport> Engine<'_, T> {
    /// Steps 2 and 3, in the order of design §4: the running release; the database check (§4a); the token pre-flight
    /// (§4c); the manifest; the choice. Every refusal is exit 2 and nothing has been changed (nothing is written).
    pub fn plan(&self, args: &Args) -> Result<Plan, Outcome> {
        let home = Home::new(&self.env.home);
        let current = match home.current() {
            Ok(Some(v)) => v,
            Ok(None) => {
                return Err(refuse(
                    "there is no running release (no current link in the home directory)",
                ));
            }
            Err(e) => return Err(refuse(format!("the running release cannot be read: {e}"))),
        };
        let stack_env = home.read_stack_env(&current).map_err(|e| {
            refuse(format!(
                "the running release's stack.env cannot be used: {e}"
            ))
        })?;

        // cox 14:30Z (Q-8): the pre-upgrade dump uses the ALREADY INSTALLED tool; without it nothing is asked and nothing is changed
        if args.mode == Mode::Upgrade {
            let tool = self.env.home.join("bin").join("cortex-dump");
            let usable = std::fs::symlink_metadata(&tool)
                .map(|m| m.file_type().is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false);
            if !usable {
                return Err(refuse(
                    "the dump tool bin/cortex-dump is not installed (or not executable); the upgrade takes a dump first and does not go on without it",
                ));
            }
        }

        // §4a: the number of trust rules, asked of the RUNNING database; it must be a number and it must be 0
        let raw = self.psql.trust_rule_count().unwrap_or_default();
        let count = raw.trim();
        if count.is_empty() || count.len() > 9 || !count.bytes().all(|b| b.is_ascii_digit()) {
            return Err(refuse(format!(
                "could not read a count of trust rules (got: {})",
                cut(&raw)
            )));
        }
        if count.bytes().any(|b| b != b'0') {
            return Err(refuse(format!(
                "the database has {} trust rule(s), connections allowed without a password; an upgrade does not go on (initdb flags act on an empty volume only)",
                count.trim_start_matches('0')
            )));
        }

        // §4c: one authenticated read with the operator's token, before anything is fetched
        let token = read_token(&self.env.operator_token_file).map_err(refuse)?;
        match self.stack.whoami(&token) {
            Whoami::Ok => {}
            Whoami::Refused(code) => {
                return Err(refuse(format!(
                    "the operator token is refused ({}); re-issue it with install.sh --reissue-token and point CORTEX_TOKEN_FILE at the new file",
                    code.unwrap_or("401")
                )));
            }
            Whoami::Unanswered(what) => {
                return Err(refuse(format!(
                    "the running stack did not answer an authenticated read ({what})"
                )));
            }
        }

        let body = self
            .fetcher
            .fetch_manifest(&self.env.manifest_url)
            .map_err(|e| refuse(e.to_string()))?;
        let manifest = manifest::parse(&body).map_err(|e| refuse(e.to_string()))?;
        let wanted = args.to.clone().map_or(Wanted::Newest, Wanted::Exactly);
        let installed = Installed {
            version: current.clone(),
            db_image: stack_env.db_image.0.clone(),
        };
        match choose(&installed, &manifest, &wanted).map_err(|e| refuse(e.to_string()))? {
            Decision::UpToDate => Ok(Plan::NothingToDo(current)),
            Decision::Upgrade(release) => Ok(Plan::Upgrade {
                from: current,
                to: Box::new(release.clone()),
                manifest: body,
            }),
        }
    }

    /// Steps 4 and 5 (design §4): download the release archive and the server image archive into
    /// `<home>/downloads/<v>/` (each checked against its digest as it streams), unpack the release into
    /// `releases/<v>.part/`, and check it: `db-roles.sh` against the manifest's digest (cox 14:30Z), `stack.env` against the
    /// manifest, and the `db` service against the running release's (§4b). Every refusal is exit 2 and the home is as it
    /// was except for what it says: what this run created is removed again (and a path that cannot be is reported), a
    /// leftover of an earlier run at the same two paths, which is OURS (we hold the lock), is removed first and said.
    pub fn stage(
        &self,
        from: Version,
        release: Box<Release>,
        manifest: &[u8],
    ) -> Result<Prepared, Outcome> {
        let report = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut result = self.stage_inner(from, release, manifest, &report);
        if let Err(outcome) = &mut result {
            // the guard inside has been dropped by now: a path it could not remove is said, not swallowed
            outcome.lines.extend(report.borrow().iter().cloned());
        }
        result
    }

    fn stage_inner(
        &self,
        from: Version,
        release: Box<Release>,
        manifest: &[u8],
        report: &std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    ) -> Result<Prepared, Outcome> {
        let home = Home::new(&self.env.home);
        let to = release.version.clone();
        let staging = home.staging_dir(&to);
        let root = self.env.home.join("downloads");
        let downloads = root.join(to.to_string());
        let mut lines = vec![format!("upgrade {from} -> {to}")];

        let tar_name = format!("cortex-{to}-x86_64-linux.tar.gz");
        let Some(tar_asset) = release.assets.iter().find(|a| a.name == tar_name) else {
            return Err(refuse(format!(
                "the manifest has no release archive {tar_name}"
            )));
        };
        let Some(img_asset) = release
            .assets
            .iter()
            .find(|a| a.name == release.cortex.server_image.asset)
        else {
            return Err(refuse(
                "the manifest has no server image archive for this release",
            ));
        };

        // the parents first, and as what they are (a link is never followed): `releases` must be a real directory, and
        // `downloads` absent or a real one, BEFORE anything below either is looked at or removed
        let is_real_dir = |p: &Path| {
            std::fs::symlink_metadata(p)
                .map(|m| m.is_dir())
                .unwrap_or(false)
        };
        if !is_real_dir(&self.env.home.join("releases")) {
            return Err(refuse(
                "the releases path exists and is not a directory; the install is damaged and nothing was touched",
            ));
        }
        let root_exists = match std::fs::symlink_metadata(&root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Ok(m) if m.is_dir() => true,
            _ => {
                return Err(refuse(
                    "the downloads path exists and is not a directory; remove it by hand",
                ));
            }
        };
        for (path, what) in [
            (&staging, "staging directory"),
            (&downloads, "download directory"),
        ] {
            match std::fs::symlink_metadata(path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(refuse(format!("the {what} cannot be examined"))),
                Ok(m) if m.is_dir() => {
                    std::fs::remove_dir_all(path)
                        .map_err(|_| refuse(format!("the leftover {what} cannot be removed")))?;
                    lines.push(format!("removed a leftover {what} of an earlier run"));
                }
                Ok(_) => {
                    return Err(refuse(format!(
                        "the {what} path exists and is not a directory; remove it by hand"
                    )));
                }
            }
        }
        let mut cleanup = Cleanup {
            staging: staging.clone(),
            downloads: downloads.clone(),
            armed: true,
            report: report.clone(),
        };
        if !root_exists {
            create_private_dir_exact(&root)
                .map_err(|_| refuse("the downloads directory cannot be created"))?;
        }
        create_private_dir_exact(&downloads).map_err(|e| refuse(e.to_string()))?;

        let tar_path = downloads.join("release.tar.gz");
        let img_path = downloads.join("server-image.oci.tar");
        for (asset, path) in [(tar_asset, &tar_path), (img_asset, &img_path)] {
            self.fetcher
                .download(&self.env.manifest_url, &asset.url, &asset.digest, path)
                .map_err(|e| refuse(e.to_string()))?;
        }
        let archive = std::fs::File::open(&tar_path)
            .map_err(|_| refuse("the downloaded release archive cannot be read"))?;
        unpack_release_gz(archive, &staging, &UnpackLimits::default())
            .map_err(|e| refuse(e.to_string()))?;

        // the roles script is privileged code that came in the release: it must be the one the manifest names
        let roles = file_sha256(&staging.join("db-roles.sh"))
            .map_err(|_| refuse("db-roles.sh in the release cannot be read"))?;
        if roles != release.cortex.roles_script_digest.0 {
            return Err(refuse(
                "db-roles.sh does not match the digest in the manifest",
            ));
        }
        // the staged stack.env must be usable and say what the manifest says
        let text = read_small(&staging.join("stack.env"), STACK_ENV_MAX_BYTES)
            .ok_or_else(|| refuse("the release's stack.env cannot be read"))?;
        let candidate_env = parse_stack_env(&text)
            .map_err(|e| refuse(format!("the release's stack.env cannot be used: {e}")))?;
        if candidate_env.version != to {
            return Err(refuse(
                "the release's stack.env names another version than the manifest",
            ));
        }
        if candidate_env.server_image.0 != release.cortex.server_image.image_ref {
            return Err(refuse(
                "the release's stack.env names another server image than the manifest",
            ));
        }
        if candidate_env.db_image.0 != release.cortex.db_image {
            return Err(refuse(
                "the release's stack.env names another database image than the manifest",
            ));
        }
        // §4b: the db service must not change
        let current_dir = home.release_dir(&from);
        let current_env = home.read_stack_env(&from).map_err(|e| {
            refuse(format!(
                "the running release's stack.env cannot be used: {e}"
            ))
        })?;
        let current_compose = read_small(&current_dir.join("compose.yml"), 1 << 20)
            .ok_or_else(|| refuse("the running release's compose.yml cannot be read"))?;
        let candidate_compose = read_small(&staging.join("compose.yml"), 1 << 20)
            .ok_or_else(|| refuse("the release's compose.yml cannot be read"))?;
        compare_db_service(
            (&current_compose, &current_env),
            (&candidate_compose, &candidate_env),
        )
        .map_err(|e| refuse(e.to_string()))?;

        // the release directory's own files (design §2): the verified copy of the manifest that installed it, and the
        // checksum of every file in it, so that a later run can tell a complete directory from a half-written one
        write_plain(&staging.join("manifest.json"), manifest)
            .map_err(|_| refuse("the release's manifest copy could not be written"))?;
        let mut names = Vec::new();
        list_files(&staging, "", &mut names)
            .map_err(|_| refuse("the release directory could not be listed"))?;
        names.sort();
        let mut sums = String::new();
        for name in &names {
            let hash = file_sha256(&staging.join(name))
                .map_err(|_| refuse("a file of the release could not be read"))?;
            sums.push_str(&format!(
                "{}  {name}\n",
                cortex_types::Sha256(hash).to_hex()
            ));
        }
        write_plain(&staging.join("SHA256SUMS"), sums.as_bytes())
            .map_err(|_| refuse("the release's checksum list could not be written"))?;

        let class = match release.cortex.migration_class {
            MigrationClass::Expand => "expand",
            MigrationClass::Contract => "contract",
        };
        let modules: Vec<&str> = release.cortex.modules.iter().map(|m| m.as_str()).collect();
        lines.push(format!(
            "release archive {}",
            digest_text(&tar_asset.digest)
        ));
        lines.push(format!(
            "server image archive {}",
            digest_text(&img_asset.digest)
        ));
        lines.push(format!(
            "roles script {}",
            digest_text(&release.cortex.roles_script_digest)
        ));
        lines.push("database service unchanged".to_string());
        lines.push(format!(
            "{} migration(s); modules: {}; class {class}",
            release.cortex.migrations.len(),
            modules.join(",")
        ));
        cleanup.armed = false;
        Ok(Prepared {
            from,
            release,
            staging,
            downloads,
            lines,
        })
    }

    /// Step 6 (design §4): `cortex-dump --label pre-upgrade-<from>-to-<to> --timeout 1800`. Only the exit status decides; the
    /// set path (the last stdout line) is used only when it is a directory directly under `dumps/pre-upgrade/`, made by
    /// this run, whose own `SHA256SUMS` verifies again. Returns the canonical path of the set.
    fn dump_step(&self, from: &Version, to: &Version) -> Result<PathBuf, Outcome> {
        let label = DumpLabel::new(from, to);
        // what the labelled area held BEFORE the tool ran: the set it names must not be one of these (the tool publishes
        // a fresh directory each time; a time window cannot prove a new backup, mike packet 6)
        // A listing that fails for any reason but "there is no such directory yet" is a refusal BEFORE the tool runs: an empty
        // inventory by mistake would let an old, checksum-valid set pass as new (mike, packet 7).
        let not_listed = |e: std::io::Error| {
            refuse(format!(
                "the pre-upgrade dump area could not be listed ({}); the dump tool was not run",
                e.kind()
            ))
        };
        let before: std::collections::HashSet<std::ffi::OsString> =
            match std::fs::read_dir(self.env.home.join("dumps").join("pre-upgrade")) {
                Ok(entries) => entries
                    .map(|e| e.map(|e| e.file_name()))
                    .collect::<Result<_, _>>()
                    .map_err(not_listed)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
                Err(e) => return Err(not_listed(e)),
            };
        let result = self
            .dumper
            .dump(&label, Duration::from_secs(1800))
            .map_err(|e| match e {
                PortError::TimedOut => refuse("the pre-upgrade dump timed out"),
                PortError::Failed(line) => {
                    refuse(format!("the pre-upgrade dump could not be run: {line}"))
                }
            })?;
        if result.exit != 0 {
            let why = match result.exit {
                1 => "failed".to_string(),
                2 => "was refused".to_string(),
                3 => "did not start, another dump is running".to_string(),
                4 => "timed out".to_string(),
                n => format!("ended with status {n}"),
            };
            return Err(refuse(format!(
                "the pre-upgrade dump {why}: {}",
                result.last_stderr_line
            )));
        }
        let Some(line) = result.last_stdout_line.as_deref() else {
            return Err(not_usable("the tool named no path"));
        };
        let path = Path::new(line);
        if line.is_empty() || !path.is_absolute() {
            return Err(not_usable("the path is not absolute"));
        }
        let area = std::fs::canonicalize(self.env.home.join("dumps").join("pre-upgrade"))
            .map_err(|_| not_usable("the dump area dumps/pre-upgrade does not exist"))?;
        let set = std::fs::canonicalize(path).map_err(|_| not_usable("the set cannot be found"))?;
        if set.parent() != Some(area.as_path()) {
            return Err(not_usable(
                "the set is not directly under dumps/pre-upgrade",
            ));
        }
        let meta = std::fs::metadata(&set).map_err(|_| not_usable("the set cannot be read"))?;
        if !meta.is_dir() {
            return Err(not_usable("the set is not a directory"));
        }
        if set.file_name().is_some_and(|n| before.contains(n)) {
            return Err(not_usable("the set already existed before this run"));
        }
        if !set_checksums_hold(&set) {
            return Err(refuse("the dump set failed its checksum check"));
        }
        Ok(set)
    }

    /// Step 7 (design §4): load the server image archive; the digest of the image it holds must be the manifest's.
    fn load_step(&self, p: &Prepared) -> Result<(), Outcome> {
        let archive = p.downloads.join("server-image.oci.tar");
        let got = self.containers.load_image(&archive).map_err(|e| match e {
            PortError::TimedOut => refuse("the server image could not be loaded: timed out"),
            PortError::Failed(line) => {
                refuse(format!("the server image could not be loaded: {line}"))
            }
        })?;
        if image_digest(&p.release.cortex.server_image.image_ref) != Some(got) {
            return Err(refuse(
                "the loaded server image is not the one the manifest names",
            ));
        }
        Ok(())
    }

    /// Poll `probe` every `interval` until it says done or fails, for at most `timeout` of `Pace` time (the last sleep is
    /// shortened so the total never passes the timeout). The error is a short phrase.
    fn wait(
        &self,
        interval: Duration,
        timeout: Duration,
        mut probe: impl FnMut() -> Wait,
    ) -> Result<(), String> {
        let start = self.pace.now();
        loop {
            match probe() {
                Wait::Done => return Ok(()),
                Wait::Fail(why) => return Err(why),
                Wait::Pending => {}
            }
            let spent = self.pace.now().saturating_sub(start);
            if spent >= timeout {
                return Err("timed out".to_string());
            }
            self.pace.sleep(interval.min(timeout - spent));
        }
    }

    /// Step 9 (design §4): the database first, its roles, then the one-shot `migrate` (which must exit 0) and the server.
    fn restart(
        &self,
        args: &Args,
        dir: &Path,
        release: &Release,
    ) -> Result<Vec<String>, RestartError> {
        let up = |e: PortError| match e {
            PortError::TimedOut => "compose timed out".to_string(),
            PortError::Failed(line) => format!("compose failed: {line}"),
        };
        let state = |s: Service| self.containers.state(dir, s);
        // R-4a (cox W1-65, W1-66, W1-67, W1-91): FIRST, before anything starts from this directory, the release directory is verified (the whole
        // inventory against its SHA256SUMS, the roles script against the manifest's digest) and the verified bytes of `db-roles.sh` and
        // `compose.yml` are written to a PRIVATE stage; the roles step runs from the stage, in BOTH of its attempts (the first and the confirming
        // re-run), and the stage is removed when this function returns. `compose_up` and `state` keep using the verified directory. A refusal is
        // one plain line naming the file; the marker stays.
        let verified = crate::verify::verify_release_dir_bytes(dir)
            .map_err(|e| RestartError::Failed(e.to_string()))?;
        let stage =
            crate::stage::build_stage(&self.env.home, &release.version.to_string(), &verified)
                .map_err(RestartError::Failed)?;
        // R-4b (cox, on sam's CP4): the project is the ONE the roles script targets, checked on the VERIFIED bytes (the stage's copy of `compose.yml`,
        // read by the adapter's own `project_name`) BEFORE the first compose call: a release of another project, or with no valid project name, must
        // not start its database only for the roles step to refuse it. One plain line; the stage is removed when this function returns; the marker stays.
        match crate::real::name::project_name(stage.path()) {
            Ok(project) if project == crate::real::ROLES_PROJECT => {}
            Ok(_) => {
                return Err(RestartError::Failed(
                    "the release is not of the project the roles script targets; nothing was started"
                        .into(),
                ));
            }
            Err(_) => {
                return Err(RestartError::Failed(
                    "the release's compose.yml does not name a valid project; nothing was started"
                        .into(),
                ));
            }
        }
        self.containers
            .compose_up(dir, &[Service::Db])
            .map_err(up)?;
        self.wait(args.health_interval, args.health_timeout, || {
            match state(Service::Db) {
                Ok(ServiceState::Healthy) => Wait::Done,
                Ok(ServiceState::Starting | ServiceState::Running | ServiceState::Other) => {
                    Wait::Pending
                }
                Ok(_) => Wait::Fail("the database container is not healthy".into()),
                Err(e) => Wait::Fail(up(e)),
            }
        })
        .map_err(|w| format!("the database did not become healthy: {w}"))?;
        // the role step (W1-49): a refusal is final; a timeout is CONFIRMED by ONE re-run of the idempotent script under a fresh deadline of exactly the
        // floor (success proves the roles are applied); a second timeout is UNKNOWN and is not rolled back
        let mut notes = Vec::new();
        let roles = |timeout: Duration| {
            self.containers
                .run_roles(stage.path(), &release.cortex.modules, timeout)
        };
        match roles(args.roles_timeout) {
            Ok(()) => {}
            Err(PortError::Failed(line)) => {
                return Err(RestartError::Failed(format!(
                    "the role step refused: {line}"
                )));
            }
            Err(PortError::TimedOut) => match roles(ROLES_CONFIRM_TIMEOUT) {
                Ok(()) => notes.push(
                    "the role step timed out once; a second run confirmed the module roles are applied"
                        .to_string(),
                ),
                Err(PortError::Failed(line)) => {
                    return Err(RestartError::Failed(format!(
                        "the role step refused: {line}"
                    )));
                }
                Err(PortError::TimedOut) => {
                    return Err(RestartError::RolesUnknown(
                        "the role step timed out twice".to_string(),
                    ));
                }
            },
        }
        self.containers
            .compose_up(dir, &[Service::Migrate, Service::Server])
            .map_err(up)?;
        // the migration has its own bound: a long one is not a failed health check (mike, packet 7, choice 4)
        self.wait(args.health_interval, args.migrate_timeout, || {
            match state(Service::Migrate) {
                Ok(ServiceState::Exited(0)) => Wait::Done,
                Ok(ServiceState::Exited(n)) => Wait::Fail(format!("exited with status {n}")),
                Ok(ServiceState::Missing) => Wait::Fail("the container does not exist".into()),
                Ok(_) => Wait::Pending,
                Err(e) => Wait::Fail(up(e)),
            }
        })
        .map_err(|w| format!("the migrations did not finish: {w}"))?;
        self.wait_healthy(args, dir, Service::Server, "server")?;
        Ok(notes)
    }

    /// Poll one container until it reports healthy.
    fn wait_healthy(
        &self,
        args: &Args,
        dir: &Path,
        service: Service,
        label: &str,
    ) -> Result<(), String> {
        let probe = || match self.containers.state(dir, service) {
            Ok(ServiceState::Healthy) => Wait::Done,
            Ok(ServiceState::Starting | ServiceState::Running | ServiceState::Other) => {
                Wait::Pending
            }
            Ok(_) => Wait::Fail(format!("the {label} container is not healthy")),
            Err(PortError::TimedOut) => Wait::Fail("compose timed out".to_string()),
            Err(PortError::Failed(line)) => Wait::Fail(format!("compose failed: {line}")),
        };
        self.wait(args.health_interval, args.health_timeout, probe)
            .map_err(|w| format!("the {label} did not become healthy: {w}"))
    }

    /// Start the server of release `now` and run the same checks as step 10 (design §5).
    fn start_old(&self, args: &Args, home: &Home, now: &Version) -> Result<(), String> {
        let dir = home.release_dir(now);
        self.containers
            .compose_up(&dir, &[Service::Server])
            .map_err(|e| match e {
                PortError::TimedOut => "compose timed out".to_string(),
                PortError::Failed(line) => format!("compose failed: {line}"),
            })?;
        self.wait_healthy(args, &dir, Service::Server, "server")?;
        self.check(args)
    }

    /// The rollback (design §5): flip the links back (a marker stays until the end), start the old server, check it, and then
    /// forget `previous` and the marker so that a second rollback cannot flip forward again. The schema stays; nothing is
    /// restored. Returns the warnings.
    fn roll_back(
        &self,
        args: &Args,
        home: &Home,
        hooks: &HookSet,
    ) -> Result<(Flip, Vec<String>), RollbackError> {
        // what the links name BEFORE the flip: the pair a failed flip is reported against
        let hint = match (home.current(), home.previous()) {
            (Ok(Some(left)), Ok(Some(now))) => Some(Flip { left, now }),
            _ => None,
        };
        // R-5 (cox W1-59 C): a rollback whose backup-file inventory is lost or unusable is stopped BEFORE anything is changed (exit 4, nothing touched)
        if let Some(h) = &hint {
            self.members_inventory(home, &h.left)
                .map_err(RollbackError::Members)?;
        }
        let flip = home.begin_rollback().map_err(|e| match e {
            // after the intent was published: the state may have changed and the intent is kept
            LayoutError::FlipFailed { .. } | LayoutError::FlipInterrupted(_) => {
                RollbackError::Interrupted(e.to_string(), hint.clone())
            }
            // a precondition refusal, or an error before anything was written: nothing changed
            other => RollbackError::Refused(refusal_text(other)),
        })?;
        self.resume_rollback(args, home, flip, hooks)
    }

    /// The inventory of the backup files of the upgrade being left (R-5): by the marker's from, to and id while an upgrade is unfinished, by `members.last`
    /// bound to the release being left otherwise. `Ok(None)` when no record exists at all; the text of the refusal when it is lost or unusable.
    fn members_inventory(
        &self,
        home: &Home,
        left: &Version,
    ) -> Result<Option<members::Inventory>, String> {
        let marker = match home.incomplete_state() {
            // (a marker that reads as "stale" here is the unfinished upgrade's own: the rollback already moved `current` back to `from`; a marker of a switch
            // that never happened was removed at the start of the run)
            Ok(Some((inc, _))) => Some((inc.from.to_string(), inc.to.to_string(), inc.id)),
            Ok(None) => None,
            Err(e) => {
                return Err(format!(
                    "the marker of the unfinished upgrade cannot be used ({e}); nothing was changed"
                ));
            }
        };
        members::inventory(
            &self.env.home,
            marker
                .as_ref()
                .map(|(f, t, i)| (f.as_str(), t.as_str(), i.as_str())),
            Some(&left.to_string()),
        )
        .map_err(|e| format!("{e}; nothing was changed"))
    }

    /// The rest of a rollback whose links are flipped: the old release's server, the checks, then the marker. A failure
    /// leaves the marker where it is, so every later run starts HERE (never with another flip) until this succeeds.
    fn resume_rollback(
        &self,
        args: &Args,
        home: &Home,
        flip: Flip,
        hooks: &HookSet,
    ) -> Result<(Flip, Vec<String>), RollbackError> {
        self.start_old(args, home, &flip.now)
            .map_err(|why| RollbackError::Failed(why, flip.clone()))?;
        let mut warnings = Vec::new();
        // R-5 (cox W1-59 C): the backup scripts and units go back AS PART OF the rollback, before its marker is cleared; the marker (and with it the record
        // and every backup) stays when any entry cannot be put back, and the next run starts here again
        let point = hooks.member_point.unwrap_or(&no_point);
        let inventory = self
            .members_inventory(home, &flip.left)
            .map_err(|why| RollbackError::Failed(why, flip.clone()))?;
        if let Some(inv) = &inventory {
            match members::restore(&self.env.home, &self.env.unit_dir, inv, point) {
                Ok(names) if names.is_empty() => {}
                Ok(names) => {
                    warnings.push(format!(
                        "put back {} backup file(s) (scripts and units) to the versions of {}",
                        names.len(),
                        flip.now
                    ));
                    if names.iter().any(|n| UNIT_FILES.contains(&n.as_str())) {
                        warnings.push(
                            "the backup units changed; run: systemctl --user daemon-reload"
                                .to_string(),
                        );
                    }
                }
                Err(e) => {
                    return Err(RollbackError::Failed(
                        format!(
                            "the backup scripts and units could not be put back ({e}); the marker, the record and every backup are kept: fix the cause and run cortex upgrade --rollback again"
                        ),
                        flip.clone(),
                    ));
                }
            }
        }
        match home.finish_rollback() {
            Ok(()) => {
                if let Some(inv) = &inventory
                    && let Err(e) = members::forget(&self.env.home, inv)
                {
                    warnings.push(format!(
                        "warning: the record of the backup files could not be removed ({e}); everything in it is restored, and it is removed by the next upgrade"
                    ));
                }
            }
            Err(e) => warnings.push(format!(
                "warning: the rollback is complete but its marker could not be cleared ({e}); it is tried again at the next run, and until it is cleared nothing else can switch"
            )),
        }
        Ok((flip, warnings))
    }

    /// The record of an attempt that stopped before the restart: a warning when it cannot be written.
    fn write_attempt(
        &self,
        home: &Home,
        result: RecordResult,
        steps: Vec<StepRecord>,
        from: &Version,
        to: &Version,
        set: &Path,
    ) -> Option<String> {
        let record = Record {
            from: from.to_string(),
            to: to.to_string(),
            result,
            steps,
            dump_set: Some(set.display().to_string()),
            members: Vec::new(),
        };
        home.write_record(self.clock, &record)
            .err()
            .map(|e| format!("warning: the upgrade record could not be written ({e})"))
    }

    /// The role step and its confirmation both timed out (cox W1-49 B): UNKNOWN, and NOT rolled back. The install stays on the new release with
    /// the old one kept as `previous`; the server and the migrations were not started; the set is kept; the record says `roles_unknown`; exit 4.
    fn stop_roles_unknown(
        &self,
        home: &Home,
        mut all: Vec<String>,
        steps: Vec<StepRecord>,
        from: &Version,
        to: &Version,
        set: &Path,
    ) -> Outcome {
        let set_text = set.display().to_string();
        // the reason is rewritten, and the marker written ahead of the switch knows where previous pointed: that is kept, so it is rewritten only when it is readable
        match home.incomplete() {
            Ok(Some(first)) => {
                let marker = Incomplete {
                    from: from.clone(),
                    to: to.clone(),
                    set: set_text.clone(),
                    prev: first.prev,
                    why: "the module roles are unknown: the role step and its confirmation both timed out"
                        .to_string(),
                    id: first.id,
                };
                if let Err(err) = home.mark_incomplete(&marker) {
                    all.push(format!(
                        "warning: the marker of the unfinished upgrade could not be updated ({err})"
                    ));
                }
            }
            _ => all.push(
                "warning: the marker of the unfinished upgrade could not be read, so it was not updated"
                    .to_string(),
            ),
        }
        all.push(
            "the module roles are UNKNOWN: the first run and its confirmation both timed out, so they may or may not be applied; nothing was rolled back"
                .to_string(),
        );
        all.push(format!(
            "release {to} is the current release and {from} is kept as the previous one; the server and the migrations were not started"
        ));
        all.push(format!("pre-upgrade dump set kept at {}", shown(&set_text)));
        all.push(format!(
            "to finish it run cortex upgrade (the role step is safe to repeat; it picks up where this stopped); to go back to {from} run cortex upgrade --rollback"
        ));
        let record = Record {
            from: from.to_string(),
            to: to.to_string(),
            result: RecordResult::RolesUnknown,
            steps,
            dump_set: Some(set_text),
            members: Vec::new(),
        };
        if let Err(e) = home.write_record(self.clock, &record) {
            all.push(format!(
                "warning: the upgrade record could not be written ({e})"
            ));
        }
        Outcome {
            code: EXIT_ROLLBACK_FAILED,
            lines: all,
        }
    }

    /// A failure after the switch (step 9 or 10): roll back, say so, record it. Exit 3 when the old release runs again,
    /// exit 4 when it cannot (and then the text of design §5, with the set).
    #[allow(clippy::too_many_arguments)]
    fn fail_after_switch(
        &self,
        args: &Args,
        home: &Home,
        mut all: Vec<String>,
        steps: Vec<StepRecord>,
        from: &Version,
        to: &Version,
        set: &Path,
        hooks: &HookSet,
    ) -> Outcome {
        let set_text = set.display().to_string();
        let (result, code) = match self.roll_back(args, home, hooks) {
            Ok((_, warnings)) => {
                all.extend(warnings);
                all.push(format!(
                    "rolled back: {from} is running again; the failed release {to} stays in releases/ until a later upgrade removes it"
                ));
                all.push(format!("pre-upgrade dump set kept at {}", shown(&set_text)));
                (RecordResult::RolledBack, EXIT_ROLLED_BACK)
            }
            Err(e) => {
                match &e {
                    RollbackError::Refused(r) => all.push(format!("the rollback was refused: {r}")),
                    RollbackError::Failed(why, _) => {
                        all.push(format!("the rollback failed too: {why}"));
                        all.push(UNFINISHED_LINE.to_string());
                    }
                    RollbackError::Interrupted(why, _) => all.extend(interrupted_lines(why)),
                    RollbackError::Members(why) => {
                        all.push(format!("the rollback was not started: {why}"));
                    }
                }
                all.extend(exit4_lines(Some(&set_text)));
                (RecordResult::RollbackFailed, EXIT_ROLLBACK_FAILED)
            }
        };
        let record = Record {
            from: from.to_string(),
            to: to.to_string(),
            result,
            steps,
            dump_set: Some(set_text),
            members: Vec::new(),
        };
        if let Err(e) = home.write_record(self.clock, &record) {
            all.push(format!(
                "warning: the upgrade record could not be written ({e})"
            ));
        }
        Outcome { code, lines: all }
    }

    /// `--rollback` (design §5): refused with exit 2 and nothing changed when there is nothing to go back to or the
    /// release being left is a contract release; otherwise the same rollback as above, exit 0 or exit 4. `resumed` is the
    /// rollback an earlier run started and did not finish: then only its rest is done, with no flip.
    fn rollback_mode(
        &self,
        args: &Args,
        home: &Home,
        resumed: Option<Flip>,
        mut all: Vec<String>,
        hooks: &HookSet,
    ) -> Outcome {
        let done = match resumed {
            Some(flip) => self.resume_rollback(args, home, flip, hooks),
            None => self.roll_back(args, home, hooks),
        };
        let (code, flip, ok) = match done {
            Err(RollbackError::Refused(r)) => {
                all.push(format!("rollback refused: {r}"));
                return Outcome {
                    code: EXIT_REFUSED,
                    lines: all,
                };
            }
            Ok((flip, warnings)) => {
                all.extend(warnings);
                all.push(format!("rolled back {} -> {}", flip.left, flip.now));
                (EXIT_DONE, flip, true)
            }
            Err(RollbackError::Failed(why, flip)) => {
                all.push(format!("the rollback failed: {why}"));
                all.push(UNFINISHED_LINE.to_string());
                all.extend(exit4_lines(
                    dump_set_for(&self.env.home, &flip.left, &flip.now).as_deref(),
                ));
                (EXIT_ROLLBACK_FAILED, flip, false)
            }
            Err(RollbackError::Members(why)) => {
                // the inventory of the backup files is lost or unusable: nothing was changed, exit 4 (cox W1-59 C)
                all.push(format!("the rollback was not started: {why}"));
                return Outcome {
                    code: EXIT_ROLLBACK_FAILED,
                    lines: all,
                };
            }
            Err(RollbackError::Interrupted(why, hint)) => {
                // started and stopped: exit 4, the intent kept, and the same fixed text as any rollback that failed
                all.extend(interrupted_lines(&why));
                all.extend(exit4_lines(
                    hint.as_ref()
                        .and_then(|f| dump_set_for(&self.env.home, &f.left, &f.now))
                        .as_deref(),
                ));
                if let Some(flip) = hint {
                    all.extend(self.write_rollback_record(home, &flip, false));
                }
                return Outcome {
                    code: EXIT_ROLLBACK_FAILED,
                    lines: all,
                };
            }
        };
        all.extend(self.write_rollback_record(home, &flip, ok));
        Outcome { code, lines: all }
    }

    /// The record of a `--rollback` (or of the rest of one): from the release left to the one went back to, with the dump set of
    /// the upgrade that is being left. Returns a warning when it cannot be written.
    fn write_rollback_record(&self, home: &Home, flip: &Flip, ok: bool) -> Option<String> {
        let record = Record {
            from: flip.left.to_string(),
            to: flip.now.to_string(),
            result: if ok {
                RecordResult::RolledBack
            } else {
                RecordResult::RollbackFailed
            },
            steps: vec![
                StepRecord {
                    step: 1,
                    name: "lock".into(),
                    ok: true,
                },
                StepRecord {
                    step: 2,
                    name: "rollback".into(),
                    ok,
                },
            ],
            dump_set: dump_set_for(&self.env.home, &flip.left, &flip.now),
            members: Vec::new(),
        };
        home.write_record(self.clock, &record)
            .err()
            .map(|e| format!("warning: the record could not be written ({e})"))
    }

    /// The sweep of W1-74 and W1-75 for the release `release` (the one a trusted `members.last` must be bound to): remove the backups that record retired, never the
    /// ones of `attempt` or of the record itself; when there is no trusted record, delete NOTHING and say so once if any recognised backup is there. Returns the
    /// warning lines.
    fn sweep_retired_by(&self, release: &str, attempt: &str, point: &dyn Fn(&str)) -> Vec<String> {
        let mut lines = Vec::new();
        match members::trusted_retired(&self.env.home, release) {
            members::Trust::Trusted(retired) => {
                let last_id = members::read_last(&self.env.home)
                    .ok()
                    .flatten()
                    .map(|l| l.id)
                    .unwrap_or_default();
                let named: Vec<&str> = retired.iter().map(String::as_str).collect();
                if let Err(e) = members::sweep_retired(
                    &self.env.home,
                    &self.env.unit_dir,
                    &named,
                    &[attempt, last_id.as_str()],
                    point,
                ) {
                    lines.push(format!(
                        "warning: backup files of earlier upgrades could not be removed ({e})"
                    ));
                }
            }
            skipped => {
                let found = members::recognised_backups(&self.env.home, &self.env.unit_dir);
                if !found.is_empty() {
                    let why = match skipped {
                        members::Trust::Untrusted(why) => {
                            format!("members.last cannot be relied on ({why})")
                        }
                        _ => "there is no members.last".to_string(),
                    };
                    lines.push(format!(
                        "warning: {why}; the {} backup file(s) of earlier upgrades were KEPT: nothing was deleted (remove them by hand when you know they are stale)",
                        found.len()
                    ));
                }
            }
        }
        lines
    }

    /// Step 10 (design §4): `GET /v2/health/ready` every interval until 200, then the authenticated read.
    fn check(&self, args: &Args) -> Result<(), String> {
        self.wait(args.health_interval, args.health_timeout, || {
            match self.stack.ready() {
                Ready::Yes => Wait::Done,
                Ready::No(_) => Wait::Pending,
            }
        })
        .map_err(|w| format!("the server did not answer ready: {w}"))?;
        let token = read_token(&self.env.operator_token_file)?;
        match self.stack.whoami(&token) {
            Whoami::Ok => Ok(()),
            Whoami::Refused(code) => Err(format!(
                "the operator token is refused ({})",
                code.unwrap_or("401")
            )),
            Whoami::Unanswered(what) => Err(format!(
                "the server did not answer an authenticated read ({what})"
            )),
        }
    }

    /// Steps 6 to 11 for a staged release. Until the rollback is built, a failure after step 8 is exit 4.
    fn upgrade(&self, args: &Args, mut all: Vec<String>, p: Prepared, hooks: &HookSet) -> Outcome {
        let home = Home::new(&self.env.home);
        let (from, to) = (p.from.clone(), p.release.version.clone());
        let names = STEP_NAMES;
        let mut steps: Vec<StepRecord> = Vec::new();
        let mark = |steps: &mut Vec<StepRecord>, ok: bool| {
            let n = steps.len();
            steps.push(StepRecord {
                step: (n + 1) as u8,
                name: names[n].to_string(),
                ok,
            });
        };
        for _ in 0..5 {
            mark(&mut steps, true);
        }
        all.extend(p.lines.iter().cloned());
        let stop = |p: &Prepared, mut all: Vec<String>, o: Outcome| {
            all.extend(o.lines);
            all.extend(p.discard());
            Outcome {
                code: o.code,
                lines: all,
            }
        };
        // 6 and 7: a failure of either is exit 2 with the install as it was (the dump set stays)
        let set = match self.dump_step(&from, &to) {
            Ok(set) => set,
            Err(o) => return stop(&p, all, o),
        };
        all.push("pre-upgrade dump set verified".to_string());
        mark(&mut steps, true);
        if let Err(o) = self.load_step(&p) {
            // exit 2, but not "nothing happened": the dump set was made and is kept, and the attempt is recorded
            all.push(format!("pre-upgrade dump set kept at {}", shown(&set)));
            mark(&mut steps, false);
            all.extend(self.write_attempt(&home, RecordResult::Failed, steps, &from, &to, &set));
            return stop(&p, all, o);
        }
        all.push("server image loaded".to_string());
        mark(&mut steps, true);
        // 8: the switch. The marker goes FIRST, ahead of the flip (like `rollback.intent`): an upgrade lost anywhere from here to step 11, even between the
        // flip and the next line, must not read as "up to date" or as finished (mike's HIGH on packet 14). While `current` is still `from` the marker is stale.
        let previous_before = home.previous();
        // where `previous` points NOW is what a stale marker must put back (run() refused an upgrade over a previous that is not a plain link, so Err is not
        // expected here; it is a failed marker if it ever happens)
        let prev = match &previous_before {
            Ok(None) => Ok(PrevLink::Absent),
            Ok(Some(v)) => Ok(PrevLink::Release(v.clone())),
            Err(_) => Err(LayoutError::BadLink { link: "previous" }),
        };
        let marker = prev.and_then(|prev| {
            Ok(Incomplete {
                from: from.clone(),
                to: to.clone(),
                set: set.display().to_string(),
                prev,
                why: "the switch, the restart and the checks (steps 8 to 11) did not finish"
                    .to_string(),
                id: crate::layout::new_attempt_id()?,
            })
        });
        // the id of THIS attempt (cox W1-62): in the marker, in members.update and members.last, in the names of the backups
        let attempt = marker.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let point = hooks.member_point.unwrap_or(&no_point);
        // W1-74, W1-75: a deletion needs a TRUSTED record that POSITIVELY names the backup as retired. members.last present, readable, valid, complete and bound to the
        // release being left names the ids its promotion retired; those go at the start of an upgrade (a sweep that was lost is finished here). Anything else (absent,
        // unreadable, corrupt, not bound) names nobody: the sweep is SKIPPED, one warning says the backups were kept, and the result of the upgrade is not changed.
        // (The home and the unit directory were checked at the very start of the run, before any write.)
        all.extend(self.sweep_retired_by(&from.to_string(), &attempt, point));
        // R-5: the home (it is written into the units) and the unit directory are checked BEFORE the marker and the flip: a refusal here is a switch that did not
        // happen, nothing to roll back
        let switched: Result<(), String> = members::check_home(&self.env.home)
            .and_then(|()| members::check_unit_dir(&self.env.unit_dir))
            .map_err(|e| e.to_string())
            .and_then(|()| {
                let marker = marker.map_err(|e| e.to_string())?;
                home.mark_incomplete(&marker).map_err(|e| e.to_string())?;
                // the plan of the ten files, durable BEFORE the flip: the marker never stands without the inventory it will be rolled back by
                // (the release is still in its staging directory until the flip renames it)
                members::plan(
                    &MemberLayout {
                        release_dir: &p.staging,
                        home: &self.env.home,
                        unit_dir: &self.env.unit_dir,
                        db_container: members::DB_CONTAINER,
                    },
                    &from.to_string(),
                    &to.to_string(),
                    &marker.id,
                    point,
                )
                .map_err(|e| e.to_string())?;
                if let Some(lose) = hooks.after_marker {
                    lose();
                }
                home.commit_with(&to, &mut |step| hooks.commit_step.is_none_or(|f| f(step)))
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = switched {
            all.extend(p.discard());
            mark(&mut steps, false);
            let set_text = set.display().to_string();
            // act on where the links ACTUALLY stand, not on the error alone (mike, packet 7, finding 3)
            match switch_state(home.current(), &from, &to) {
                SwitchState::NotSwitched => {
                    // the switch did not happen: the marker written ahead of it goes (a failure to remove it is harmless: it is stale while current is `from`)
                    if let Err(err) = members::discard_plan(&self.env.home, &attempt) {
                        all.push(format!(
                            "warning: the plan of the backup files of the upgrade that did not start could not be removed ({err})"
                        ));
                    }
                    if let Err(err) = home.clear_incomplete() {
                        all.push(format!(
                            "warning: the marker of the upgrade that did not start could not be removed ({err})"
                        ));
                    }
                    // `previous` is set before `current`: a commit that stopped between them has overwritten the release that was the
                    // rollback target before this run; put it back (mike, packet 9)
                    if let Ok(was) = &previous_before
                        && let Err(err) = home.restore_previous(was.as_ref())
                    {
                        all.push(format!(
                            "warning: the previous link could not be put back ({err})"
                        ));
                    }
                    // nothing was started or restarted; say "still running" only after the old release has answered
                    return match self.check(args) {
                        Ok(()) => {
                            all.push(format!(
                                "the switch to {to} failed ({e}); {from} is still current and answered its checks; nothing was started"
                            ));
                            all.push("the server image stays loaded".to_string());
                            all.push(format!("pre-upgrade dump set kept at {}", shown(&set_text)));
                            all.extend(self.write_attempt(
                                &home,
                                RecordResult::Failed,
                                steps,
                                &from,
                                &to,
                                &set,
                            ));
                            Outcome {
                                code: EXIT_ROLLED_BACK,
                                lines: all,
                            }
                        }
                        Err(why) => {
                            all.push(format!(
                                "the switch to {to} failed ({e}) and {from} does not answer its checks: {why}"
                            ));
                            all.extend(exit4_lines(Some(&set_text)));
                            all.extend(self.write_attempt(
                                &home,
                                RecordResult::RollbackFailed,
                                steps,
                                &from,
                                &to,
                                &set,
                            ));
                            Outcome {
                                code: EXIT_ROLLBACK_FAILED,
                                lines: all,
                            }
                        }
                    };
                }
                SwitchState::Switched => {
                    all.push(format!(
                        "the switch to {to} reported an error ({e}) after the links were moved; rolling back"
                    ));
                    return self
                        .fail_after_switch(args, &home, all, steps, &from, &to, &set, hooks);
                }
                SwitchState::Unknown => {
                    all.push(format!(
                        "the switch to {to} failed ({e}); the links in the home directory may be half way: check them before the next run"
                    ));
                    all.push("the server image stays loaded".to_string());
                    all.extend(exit4_lines(Some(&set_text)));
                    all.extend(self.write_attempt(
                        &home,
                        RecordResult::RollbackFailed,
                        steps,
                        &from,
                        &to,
                        &set,
                    ));
                    return Outcome {
                        code: EXIT_ROLLBACK_FAILED,
                        lines: all,
                    };
                }
            }
        }
        all.extend(p.discard());
        mark(&mut steps, true);
        // R-5: the backup scripts and units of the NEW release replace the old ones (the previous copies are kept as .prev and put back by a rollback)
        match self.install_members(&home, &from, &to, &attempt, point) {
            Ok(lines) => all.extend(lines),
            Err(why) => {
                all.push(format!(
                    "step 8 ({}): the backup scripts and units could not be installed ({why})",
                    names[7]
                ));
                return self.fail_after_switch(args, &home, all, steps, &from, &to, &set, hooks);
            }
        }
        self.restart_check_record(
            args,
            &home,
            all,
            steps,
            (&from, &to),
            &set,
            &p.release,
            false,
            (&attempt, hooks),
        )
    }

    /// A marker that cannot be used (cox W1-56 F1): refused in EVERY mode before any mode-specific action, naming the file, changing nothing; removing it is
    /// the operator's decision after checking the links.
    fn unreadable_marker(err: &LayoutError) -> Outcome {
        refuse(format!(
            "the marker of an unfinished upgrade cannot be used ({err}): upgrade.incomplete in the home directory; nothing was changed; removing it is your decision after you have checked the links (current and previous in the home directory)"
        ))
    }

    /// The refusal that `--check`, `--to` and an unreadable marker answer with while an upgrade did not finish (mike's HIGH on packet 14).
    fn unfinished_refusal(inc: &Incomplete) -> Outcome {
        refuse(format!(
            "an upgrade from {} to {} did not finish ({}): run cortex upgrade to finish it, or cortex upgrade --rollback to go back; nothing was asked",
            inc.from, inc.to, inc.why
        ))
    }

    /// The retry of an upgrade that was switched and did not finish (`upgrade.incomplete`): steps 9 to 11 again for the release `to`, whose manifest copy
    /// step 5 wrote into it. Never "up to date". Success: the marker goes and the upgrade is finished; a refusal or any other failure: the normal
    /// rollback to `from`; unknown roles again: exit 4, the marker stays, no rollback.
    fn finish_upgrade(
        &self,
        args: &Args,
        home: &Home,
        inc: Incomplete,
        mut all: Vec<String>,
        hooks: &HookSet,
    ) -> Outcome {
        // R-6c (cox W1-105, sam's R6UT-SAM01): a CHECK holds no lock and writes nothing; it must never reach the member install, the restart or a compose
        // call. The marker it can meet here was published by a concurrent upgrade AFTER the check's early read of the marker; it is refused read-only, in the
        // words of the early refusal. This is the one entry of the mutating resume (the run that holds the lock is the only other way in, and it is not a check).
        if args.mode == Mode::Check {
            return Self::unfinished_refusal(&inc);
        }
        let Some(release) = home.read_release(&inc.to) else {
            return refuse(format!(
                "the interrupted upgrade {} -> {} cannot be finished: the manifest copy inside release {} cannot be read; nothing was changed; run cortex upgrade --rollback to go back",
                inc.from, inc.to, inc.to
            ));
        };
        all.push(format!(
            "finishing the interrupted upgrade {} -> {} ({})",
            inc.from, inc.to, inc.why
        ));
        // R-5: the run that was interrupted may have been lost before it installed the backup files, or half way through: install them again (what is
        // already ours is untouched, the marker of the lost run keeps its entries); a failure stops here, nothing is rolled back
        let point = hooks.member_point.unwrap_or(&no_point);
        let installed = self.install_members(home, &inc.from, &inc.to, &inc.id, point);
        // steps 1 to 8 were done by the run that was interrupted (the marker is written after step 8)
        let steps: Vec<StepRecord> = STEP_NAMES[..8]
            .iter()
            .enumerate()
            .map(|(n, name)| StepRecord {
                step: (n + 1) as u8,
                name: name.to_string(),
                ok: true,
            })
            .collect();
        match installed {
            Ok(lines) => all.extend(lines),
            Err(why) => {
                all.push(format!(
                    "step 8 ({}): the backup scripts and units could not be installed ({why})",
                    STEP_NAMES[7]
                ));
                return self.resume_failed(
                    home,
                    all,
                    steps,
                    &inc.from,
                    &inc.to,
                    Path::new(&inc.set),
                );
            }
        }
        self.restart_check_record(
            args,
            home,
            all,
            steps,
            (&inc.from, &inc.to),
            Path::new(&inc.set),
            &release,
            true,
            (&inc.id, hooks),
        )
    }

    /// R-5: carry out the plan of release `to` (the ten members outside the release directory, see `members`): the lines to print (what changed, what to run),
    /// or the cause. A resume whose lost run got as far as promoting the record (members.update gone, members.last of this attempt) has nothing left to install.
    fn install_members(
        &self,
        home: &Home,
        from: &Version,
        to: &Version,
        id: &str,
        point: members::Point,
    ) -> Result<Vec<String>, String> {
        let (f, t) = (from.to_string(), to.to_string());
        match members::inventory(&self.env.home, Some((&f, &t, id)), None) {
            Err(e) => return Err(e.to_string()),
            Ok(Some(inv)) if inv.source == members::Source::Last => return Ok(Vec::new()),
            Ok(_) => {}
        }
        let release_dir = home.release_dir(to);
        let update = members::install(
            &MemberLayout {
                release_dir: &release_dir,
                home: &self.env.home,
                unit_dir: &self.env.unit_dir,
                db_container: members::DB_CONTAINER,
            },
            &f,
            &t,
            id,
            point,
        )
        .map_err(|e| e.to_string())?;
        let mut lines = Vec::new();
        if update.entries.is_empty() {
            return Ok(lines);
        }
        lines.push(format!(
            "installed {} backup file(s) of release {to} (the versions they replaced are kept as <file>.prev-{id})",
            update.entries.len()
        ));
        let units: Vec<&(Kind, String, bool)> = update
            .entries
            .iter()
            .filter(|e| e.0 == Kind::Unit)
            .collect();
        if !units.is_empty() {
            lines.push("the backup units changed; run: systemctl --user daemon-reload".to_string());
        }
        let fresh: Vec<&str> = units.iter().filter(|e| e.2).map(|e| e.1.as_str()).collect();
        if !fresh.is_empty() {
            lines.push(format!(
                "new in this release and not enabled by an upgrade: {}; to enable a timer run: systemctl --user enable --now <timer>",
                fresh.join(", ")
            ));
        }
        Ok(lines)
    }

    /// A stale marker (`current` is still its `from`: the switch never happened) goes, durably, with ONE line (cox W1-56 A), by a run that holds the lock.
    /// Cox's rule (F2 on packet 15 r1), with P the link now and R the previous link the marker recorded: P == R: just remove it; P != R and P == from (the
    /// interrupted flip sets `previous` to the old current BEFORE it moves `current`): put R back (a release, or no link at all), THEN remove the marker, so
    /// a run lost between the two only removes; ANY OTHER pair is refused, nothing changed, the marker kept (after a finished rollback `previous` is
    /// forgotten: that is not the flip's footprint, and guessing a repair is worse than refusing).
    fn remove_stale_marker(
        &self,
        home: &Home,
        stale: &Incomplete,
        lines: &mut Vec<String>,
    ) -> Result<(), Outcome> {
        let recorded = match &stale.prev {
            PrevLink::Release(p) => Some(p.clone()),
            PrevLink::Absent => None,
        };
        let now = match home.previous() {
            Ok(p) => p,
            Err(e) => {
                return Err(refuse(format!(
                    "the stale marker of an upgrade from {} to {} cannot be removed: the previous link cannot be read ({e}); nothing was changed; check the links and remove upgrade.incomplete yourself",
                    stale.from, stale.to
                )));
            }
        };
        let mut note = String::new();
        if now != recorded {
            if now.as_ref() != Some(&stale.from) {
                return Err(refuse(format!(
                    "the stale marker of an upgrade from {} to {} stays: the previous link ({}) is neither what the marker recorded ({}) nor the release that upgrade set; nothing was changed; check the links and remove upgrade.incomplete yourself",
                    stale.from,
                    stale.to,
                    now.as_ref().map_or("none".to_string(), ToString::to_string),
                    stale.prev
                )));
            }
            if let Err(e) = home.restore_previous(recorded.as_ref()) {
                return Err(refuse(format!(
                    "the stale marker of an upgrade from {} to {} records where the previous link pointed ({}), but it cannot be put back ({e}); the marker is kept and nothing else was changed",
                    stale.from, stale.to, stale.prev
                )));
            }
            note = match &recorded {
                Some(p) => format!(
                    "; the previous link, which that upgrade had already moved, is back on {p}"
                ),
                None => "; the previous link is removed (that upgrade had already set it and there was none before)".to_string(),
            };
        }
        // the plan of that attempt (nothing happened to any entry: the switch never did) goes first, so a run lost between the two only removes the marker
        if let Err(e) = members::discard_plan(&self.env.home, &stale.id) {
            return Err(refuse(format!(
                "the stale marker of an upgrade from {} to {} stays: {e}",
                stale.from, stale.to
            )));
        }
        match home.clear_incomplete() {
            Ok(()) => lines.push(format!(
                "removed the stale marker of an upgrade from {} to {} that never switched (current is still {}){note}",
                stale.from, stale.to, stale.from
            )),
            Err(err) => lines.push(format!(
                "warning: the stale marker of an upgrade that never switched could not be removed ({err})"
            )),
        }
        Ok(())
    }

    /// A RESUMED upgrade that failed again (cox W1-56 B): it rolls FORWARD only. The marker stands for a process lost anywhere between steps 8 and 11, the
    /// new release may have served for hours, and a rollback flips links only (the schema stays, nothing is restored): so nothing is rolled back, the marker
    /// stays, exit 4, and the operator chooses between the retry and `cortex upgrade --rollback`.
    fn resume_failed(
        &self,
        home: &Home,
        mut all: Vec<String>,
        steps: Vec<StepRecord>,
        from: &Version,
        to: &Version,
        set: &Path,
    ) -> Outcome {
        all.push(
            "nothing was rolled back: a resumed upgrade only goes forward (the new release may have served already, and a rollback flips the links only, it restores no data)"
                .to_string(),
        );
        all.push(format!(
            "release {to} is the current release and {from} is kept as the previous one"
        ));
        all.push(format!("pre-upgrade dump set kept at {}", shown(set)));
        all.push(format!(
            "fix the cause and run cortex upgrade again to finish it, or run cortex upgrade --rollback to go back to {from}"
        ));
        let record = Record {
            from: from.to_string(),
            to: to.to_string(),
            result: RecordResult::ResumeFailed,
            steps,
            dump_set: Some(set.display().to_string()),
            members: Vec::new(),
        };
        if let Err(e) = home.write_record(self.clock, &record) {
            all.push(format!(
                "warning: the upgrade record could not be written ({e})"
            ));
        }
        Outcome {
            code: EXIT_RESUME_FAILED,
            lines: all,
        }
    }

    /// Steps 9 to 11 of release `to` (the restart: roles with the confirmation re-run, migrate, server, health; the check; the record and the prune), and the
    /// removal of the marker of an unfinished upgrade at the end. Shared by `upgrade` and by `finish_upgrade` (the retry of an interrupted one, `resumed`):
    /// `steps` has the first eight steps marked done.
    #[allow(clippy::too_many_arguments)]
    fn restart_check_record(
        &self,
        args: &Args,
        home: &Home,
        mut all: Vec<String>,
        mut steps: Vec<StepRecord>,
        (from, to): (&Version, &Version),
        set: &Path,
        release: &Release,
        resumed: bool,
        (id, hooks): (&str, &HookSet),
    ) -> Outcome {
        let names = STEP_NAMES;
        let mark = |steps: &mut Vec<StepRecord>, ok: bool| {
            let n = steps.len();
            steps.push(StepRecord {
                step: (n + 1) as u8,
                name: names[n].to_string(),
                ok,
            });
        };
        // 9 and 10
        let dir = home.release_dir(to);
        let failure = match self.restart(args, &dir, release) {
            Err(RestartError::RolesUnknown(why)) => {
                mark(&mut steps, false);
                all.push(format!("step 9 ({}) failed: {why}", names[8]));
                return self.stop_roles_unknown(home, all, steps, from, to, set);
            }
            Err(RestartError::Failed(why)) => Err((9u8, why)),
            Ok(notes) => {
                all.extend(notes);
                mark(&mut steps, true);
                self.check(args).map_err(|why| (10u8, why))
            }
        };
        if let Err((n, why)) = failure {
            mark(&mut steps, false);
            all.push(format!(
                "step {n} ({}) failed: {why}",
                names[(n - 1) as usize]
            ));
            if resumed {
                return self.resume_failed(home, all, steps, from, to, set);
            }
            return self.fail_after_switch(args, home, all, steps, from, to, set, hooks);
        }
        mark(&mut steps, true);
        // 11: the record of the backup files is promoted (cox W1-61: S1 members.last.new, S2 its rename over members.last, S3 the removal of members.update), the
        // upgrade record is written, the older releases go (a failure of either is only a warning), the marker is cleared (S4) and the backups of other attempts
        // are swept (S5). A promotion that fails stops here with the marker standing: the next run finishes it.
        mark(&mut steps, true);
        let point = hooks.member_point.unwrap_or(&no_point);
        let info = members::read_update(&self.env.home)
            .ok()
            .flatten()
            .map(|r| members::info_lines(&r))
            .unwrap_or_default();
        if let Err(e) = members::promote(
            &self.env.home,
            &from.to_string(),
            &to.to_string(),
            id,
            point,
        ) {
            all.push(format!(
                "the record of the backup files could not be promoted ({e}); the upgrade marker stays"
            ));
            return self.resume_failed(home, all, steps, from, to, set);
        }
        let record = Record {
            from: from.to_string(),
            to: to.to_string(),
            result: RecordResult::Done,
            steps,
            dump_set: Some(set.display().to_string()),
            members: info,
        };
        if let Err(e) = home.write_record(self.clock, &record) {
            all.push(format!(
                "warning: the upgrade record could not be written ({e})"
            ));
        }
        match home.prune() {
            Ok(removed) if removed.is_empty() => {}
            Ok(removed) => {
                let names: Vec<String> = removed.iter().map(|v| v.to_string()).collect();
                all.push(format!("removed older releases: {}", names.join(", ")));
            }
            Err(e) => all.push(format!(
                "warning: older releases could not be removed ({e})"
            )),
        }
        // the upgrade is finished: the marker goes (a failure to remove it is only a warning: the next run would finish what is finished, which is safe)
        if let Err(e) = home.clear_incomplete() {
            all.push(format!(
                "warning: the marker of the unfinished upgrade could not be removed ({e})"
            ));
        }
        // S5: the backups the NEW members.last retired go (the previous trusted attempt and the ids it carried), by their names alone; this attempt's stay for a later
        // --rollback; idempotent, a failure is only a warning
        all.extend(self.sweep_retired_by(&to.to_string(), id, point));
        all.push(format!("pre-upgrade dump set kept at {}", shown(set)));
        all.push(if resumed {
            format!("finished the interrupted upgrade {from} -> {to}")
        } else {
            format!("upgraded {from} -> {to}")
        });
        Outcome {
            code: EXIT_DONE,
            lines: all,
        }
    }

    /// `--check` is complete. An upgrade takes the lock, finishes an interrupted rollback, chooses, stages and verifies
    /// (steps 1 to 5), dumps and loads (6, 7), switches (8), restarts (9), checks (10) and records (11). A failure after
    /// step 8 is NOT rolled back yet (the next increment): it is reported as exit 4 with the install on the new release.
    pub fn run(&self, args: &Args) -> Outcome {
        self.run_with(args, &HookSet::default())
    }

    /// `run` with test hooks (see `Hooks`).
    #[cfg(feature = "test-hooks")]
    pub fn run_hooked(&self, args: &Args, hooks: &Hooks) -> Outcome {
        self.run_with(
            args,
            &HookSet {
                after_marker: hooks.after_marker,
                commit_step: hooks.commit_step,
                member_point: hooks.member_point,
                before_late_marker: hooks.before_late_marker,
            },
        )
    }

    fn run_with(&self, args: &Args, hooks: &HookSet) -> Outcome {
        let mut lines = Vec::new();
        let home = Home::new(&self.env.home);
        // R-6 (cox W1-68, W1-91): the verified compose provider the entry read from `<home>/compose-provider` ONCE. A caller that did not read it is refused
        // FIRST, before the lock, a file or a port; a check starts no container and needs none.
        if args.mode != Mode::Check && !self.env.compose_provider.is_absolute() {
            return refuse(
                "no compose provider is set (the entry point reads <home>/compose-provider before it starts the engine); nothing was changed",
            );
        }
        // W1-74 (2), W1-75 (4): the destination of the ten member files is validated FIRST in an upgrade: before the lock, the staging directory, the download, the
        // dump set and any sweep or member write, so a refusal leaves the home and the unit directory byte for byte as they were
        if args.mode == Mode::Upgrade
            && let Err(e) = members::check_home(&self.env.home)
                .and_then(|()| members::check_unit_dir(&self.env.unit_dir))
        {
            return refuse(format!("{e}; nothing was changed"));
        }
        // a check over an unfinished rollback would compare a link with a release other than the one the server runs (mike, packet 9)
        if args.mode == Mode::Check && home.has_unfinished_rollback() {
            return refuse(
                "an unfinished rollback is pending (rollback.intent or rollback.unfinished in the home directory): run cortex upgrade --rollback, or cortex upgrade, to finish it first; nothing was asked",
            );
        }
        // a check over an unfinished upgrade would compare a manifest with a release the server may not be running (mike's HIGH on packet 14)
        // (an unreadable marker is refused below, for every mode)
        if let (Mode::Check, Ok(Some(inc))) = (&args.mode, home.incomplete()) {
            return Self::unfinished_refusal(&inc);
        }
        let mut resumed: Option<Flip> = None;
        let _lock = if args.mode != Mode::Check {
            let lock = match home.lock() {
                Ok(l) => l,
                Err(e) => return refuse(e.to_string()),
            };
            let mut completed = false;
            match home.recover() {
                Ok(None) => {}
                Ok(Some(flip)) => {
                    completed = true;
                    lines.push(format!(
                        "completed an interrupted rollback ({} to {})",
                        flip.left, flip.now
                    ));
                }
                Err(e @ LayoutError::FlipFailed { .. }) => {
                    // recover started redoing the links and stopped: the intent is kept, the links may be half way
                    let mut all = vec![format!(
                        "the earlier rollback could not be completed ({e}); the way back is still written down in rollback.intent in the home directory and nothing was deleted"
                    )];
                    all.push("fix the cause and run cortex upgrade again (or cortex upgrade --rollback): it finishes the links first".to_string());
                    return Outcome {
                        code: EXIT_ROLLBACK_FAILED,
                        lines: all,
                    };
                }
                Err(e) => {
                    return refuse(format!(
                        "an earlier rollback was interrupted and cannot be completed ({e}); nothing was changed"
                    ));
                }
            }
            // a rollback whose links are flipped and whose old release was not confirmed running (an interrupted flip that
            // recover just finished, or a run that stopped after it): its rest comes FIRST, and it never flips again
            match home.unfinished_rollback() {
                Ok(None) => {}
                Ok(Some(flip)) => {
                    if !completed {
                        lines.push(format!(
                            "finishing the earlier rollback ({} to {})",
                            flip.left, flip.now
                        ));
                    }
                    resumed = Some(flip);
                }
                Err(e) => {
                    return refuse(format!(
                        "the marker of an earlier rollback cannot be used ({e}); nothing was changed"
                    ));
                }
            }
            Some(lock)
        } else {
            None
        };
        // the marker of an unfinished upgrade (written ahead of the switch). An unreadable one is refused in EVERY mode (mike's F1 on packet 15 r1: `--rollback`
        // used to go through it, change the links and remove it); a stale one (the switch never happened) is removed by a run that holds the lock, with the
        // previous link put back when an interrupted flip had already moved it (F2). Neither while a rollback is pending: its finish clears the marker.
        if resumed.is_none() {
            match home.incomplete_state() {
                Err(err) => return Self::unreadable_marker(&err),
                Ok(Some((stale, true))) if args.mode != Mode::Check => {
                    if let Err(outcome) = self.remove_stale_marker(&home, &stale, &mut lines) {
                        return outcome;
                    }
                }
                Ok(_) => {}
            }
        }
        if args.mode == Mode::Rollback {
            return self.rollback_mode(args, &home, resumed, lines, hooks);
        }
        if let Some(flip) = resumed {
            match self.resume_rollback(args, &home, flip, hooks) {
                Ok((flip, warnings)) => {
                    lines.extend(self.write_rollback_record(&home, &flip, true));
                    if !warnings.is_empty() {
                        lines.extend(warnings);
                        lines.push(
                            "nothing else was changed; clear the marker and run again".to_string(),
                        );
                        return Outcome {
                            code: EXIT_REFUSED,
                            lines,
                        };
                    }
                    lines.push(format!("rolled back {} -> {}", flip.left, flip.now));
                }
                Err(RollbackError::Failed(why, flip)) => {
                    lines.push(format!("the rollback failed: {why}"));
                    lines.push(UNFINISHED_LINE.to_string());
                    lines.extend(exit4_lines(
                        dump_set_for(&self.env.home, &flip.left, &flip.now).as_deref(),
                    ));
                    lines.extend(self.write_rollback_record(&home, &flip, false));
                    return Outcome {
                        code: EXIT_ROLLBACK_FAILED,
                        lines,
                    };
                }
                Err(other) => {
                    // resume_rollback only fails by Failed; anything else is said and stops the run
                    lines.push(format!("rollback refused: {}", other.text()));
                    return Outcome {
                        code: EXIT_REFUSED,
                        lines,
                    };
                }
            }
        }
        // a `previous` that is not a plain release link is not ours to overwrite: `commit` would replace it and a later failure could not put it
        // back, so an upgrade stops here, before the dump or any request (mike, packet 10)
        if args.mode == Mode::Upgrade
            && let Err(e) = home.previous()
        {
            return refuse(format!(
                "the previous link in the home directory is not a plain release link ({e}); nothing was changed; fix or remove it first"
            ));
        }
        // an upgrade that was switched and did not finish: never "up to date", never a newer release on top of it; `cortex upgrade` finishes it
        if let Some(seam) = hooks.before_late_marker {
            seam();
        }
        match home.incomplete() {
            Ok(None) => {}
            Ok(Some(inc)) => {
                if args.to.is_some() {
                    return Self::unfinished_refusal(&inc);
                }
                return self.finish_upgrade(args, &home, inc, lines, hooks);
            }
            Err(err) => return Self::unreadable_marker(&err),
        }
        let with = |o: Outcome| {
            let mut all = lines.clone();
            all.extend(o.lines);
            Outcome {
                code: o.code,
                lines: all,
            }
        };
        match self.plan(args) {
            Err(outcome) => with(outcome),
            Ok(Plan::NothingToDo(v)) => with(Outcome {
                code: EXIT_DONE,
                lines: vec![format!("up to date ({v})")],
            }),
            Ok(Plan::Upgrade { to, .. }) if args.mode == Mode::Check => with(Outcome {
                code: EXIT_NEWER,
                lines: vec![format!("a newer release is available: {}", to.version)],
            }),
            Ok(Plan::Upgrade { from, to, manifest }) => match self.stage(from, to, &manifest) {
                Err(outcome) => with(outcome),
                Ok(p) => self.upgrade(args, lines.clone(), p, hooks),
            },
        }
    }
}
