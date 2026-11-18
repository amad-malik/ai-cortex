//! `cortex upgrade` (piece 14; cox W1-79 and W1-80, kai row 40bae21e): the command group. The rules by which the process builds the
//! engine's `Env` from its environment live HERE (`unit_dir_from`, `entry_from`) as pure functions of what the caller read
//! (`Vars`): no file is looked at and nothing is read from the process, so they are cells (`tests/upgrade_group.rs`). A refusal is
//! ONE plain line that names a rule and never echoes the value.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use clap::{Arg, ArgMatches, Command};
use cortex_client::ClientError;
use cortex_types::SystemClock;
use cortex_upgrade::engine::{
    Args, EXIT_DONE, EXIT_NEWER, EXIT_REFUSED, Engine, Env, HttpStack, Mode, Outcome, parse_args,
};
use cortex_upgrade::fetch::{FetchConfig, FetchError, Fetcher, Request, Response, Transport};
use cortex_upgrade::http::{LocalTransport, UreqTransport};
use cortex_upgrade::layout::Home;
use cortex_upgrade::ports::SystemPace;
use cortex_upgrade::real::{RealContainers, RealDumper, RealPsql};

use crate::registry::{Ctx, Group, Output};

pub fn group() -> Group {
    Group {
        name: "upgrade",
        owner: "finn@helix",
        // the S3 twin has no `cortex upgrade` string: this command is local (the engine of the release), not an API operation
        implements: &[],
        commands,
        run,
    }
}

fn commands() -> Vec<Command> {
    vec![
        Command::new("upgrade")
            .about("Upgrade the installed release, check what an upgrade would do, or roll back")
            .long_about(concat!(
                "Upgrade the installed release to the newest one, or to --to VERSION. --check shows what would happen and changes nothing; ",
                "--rollback goes back to the previous release.\n\n",
                "usage: cortex upgrade [--check | --rollback] [--to VERSION]\n\n",
                "Environment: HOME (the user units go to $HOME/.config/systemd/user), CORTEX_HOME (default $HOME/cortex-v2), ",
                "CORTEX_PORT (default 8601; if you installed with --port, set CORTEX_PORT to it), CORTEX_BASE_URL (the https location ",
                "of the releases; no default), CORTEX_TOKEN_FILE or --token-file (the operator's token file).\n\n",
                "Global options such as --token-file go before the engine's own options (--check, --rollback, --to, the timeouts); ",
                "after them they are refused."
            ))
            .arg(
                Arg::new("args")
                    .value_name("ARGS")
                    .num_args(0..)
                    .allow_hyphen_values(true)
                    .trailing_var_arg(true)
                    .help("[--check | --rollback] [--to VERSION] and the timeouts: the engine reads them"),
            ),
    ]
}

/// The call of the command: the engine's arguments, the environment (the token FLAG beats `CORTEX_TOKEN_FILE`: the first source
/// that is present decides), then `run_with`. The engine's result is printed by `to_output`.
fn run(ctx: &Ctx<'_>, _command: &str, matches: &ArgMatches) -> Result<Output, ClientError> {
    let args: Vec<String> = matches
        .get_many::<String>("args")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let mut vars = Vars::from_lookup(ctx.env);
    if let Some(flag) = &ctx.token_file {
        vars.token_file = Some(flag.clone().into_os_string());
    }
    Ok(to_output(&run_with(&args, &vars)))
}

/// What the command prints: the engine's lines, on STDOUT for a result (exit 0, done; exit 10, a newer release exists) and on STDERR
/// for everything else (a refusal, a failed switch, an incomplete rollback: exits 2, 3, 4, ...); the exit code is the engine's own.
pub fn to_output(outcome: &Outcome) -> Output {
    let text: String = outcome.lines.iter().map(|l| format!("{l}\n")).collect();
    if matches!(outcome.code, EXIT_DONE | EXIT_NEWER) {
        Output {
            exit: outcome.code,
            stdout: text,
            stderr: String::new(),
        }
    } else {
        Output {
            exit: outcome.code,
            stdout: String::new(),
            stderr: text,
        }
    }
}

fn refused(line: String) -> Outcome {
    Outcome {
        code: EXIT_REFUSED,
        lines: vec![line],
    }
}

/// The refusal of an operation that needs the release host when no `CORTEX_BASE_URL` is set (cox W1-80, corrected by W1-88): the online
/// path is OFF, and the fix is one sentence.
pub const NO_LOCATION: &str =
    "no download location is configured (set CORTEX_BASE_URL to an https URL)";

/// The whole call with the real release-host transport: see [`run_with_transport`].
pub fn run_with(args: &[String], vars: &Vars) -> Outcome {
    run_with_transport(args, vars, UreqTransport::new(RELEASE_REQUEST_TIMEOUT))
}

/// The release-host transport behind a switch: with the online path OFF it never reaches the inner transport, whatever the engine asks
/// (the answer is the fetcher's bad-URL refusal, which [`translate_offline`] turns into the one line of [`NO_LOCATION`]).
pub struct Guarded<T> {
    pub inner: T,
    pub online: bool,
}

impl<T: Transport> Transport for Guarded<T> {
    fn get(&self, request: &Request<'_>) -> Result<Response, FetchError> {
        if !self.online {
            return Err(FetchError::BadUrl);
        }
        self.inner.get(request)
    }
}

/// Whether the home holds LOCAL work the engine can do without the manifest: a rollback that was interrupted or not finished (its intent
/// or its marker is there, valid or not) or an upgrade marker (unfinished, stale, or unreadable: the engine says which). Read only; a home
/// that does not exist has none.
pub fn local_work_pending(home: &Path) -> bool {
    let home = Home::new(home);
    home.has_unfinished_rollback() || !matches!(home.incomplete_state(), Ok(None))
}

/// With the online path off, the engine's refusal of the empty manifest URL (the fetcher's bad-URL line, as the LAST line of a refusal)
/// becomes the one line of [`NO_LOCATION`]; any earlier line (what the engine did locally first) stays. Everything else is untouched.
pub fn translate_offline(mut outcome: Outcome, online: bool) -> Outcome {
    if !online
        && outcome.code == EXIT_REFUSED
        && outcome
            .lines
            .last()
            .is_some_and(|l| *l == FetchError::BadUrl.to_string())
        && let Some(last) = outcome.lines.last_mut()
    {
        *last = NO_LOCATION.to_owned();
    }
    outcome
}

/// The whole call: the arguments (the engine's own parser), then the environment, then the real ports and the engine. A refusal of
/// either is one plain line with the engine's refusal code, made before anything is read or written. The engine reads no environment
/// variable itself.
///
/// THE ONLINE PATH (cox W1-88): without a `CORTEX_BASE_URL` the entry has no manifest URL. A `--rollback` ignores the variable's value (it is
/// looked up with the others, never validated or used) and always enters the engine. An upgrade or a check that finds NO local work pending refuses at once with [`NO_LOCATION`], before the engine
/// (no lock, no change, no request). When local work IS pending (an interrupted rollback or upgrade), the engine is entered so that it can
/// recover or explain, with a release-host transport that cannot reach the network; if it then reaches the manifest, its refusal becomes
/// [`NO_LOCATION`] too.
pub fn run_with_transport<T: Transport>(args: &[String], vars: &Vars, transport: T) -> Outcome {
    let args = match parse_args(args) {
        Ok(a) => a,
        Err(line) => return refused(line),
    };
    let entry = match entry_from(vars, args.mode) {
        Ok(e) => e,
        Err(line) => return refused(line),
    };
    let online = entry.manifest_url.is_some();
    if args.mode != Mode::Rollback && !online && !local_work_pending(&entry.home) {
        return refused(NO_LOCATION.to_owned());
    }
    // R-6b (cox W1-68, W1-91 Q1): the verified compose provider, read ONCE, here, before anything is built; the SAME path goes to the adapter and
    // to the engine's `Env`. A refusal is the library's own line, with the engine's refusal code, before the engine is entered.
    let provider = match compose_provider_for(&entry, args.mode) {
        Ok(p) => p,
        Err(line) => return refused(line),
    };
    let outcome = run_engine(
        &args,
        &entry,
        provider.as_deref(),
        Guarded {
            inner: transport,
            online,
        },
    );
    if args.mode == Mode::Rollback {
        outcome
    } else {
        translate_offline(outcome, online)
    }
}

/// How long ONE request to the release host may take (the manifest, or the image archive, which is the large asset).
pub const RELEASE_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);
/// How long ONE request to the local server may take (`/v2/health/ready`, `/v2/whoami`).
pub const LOCAL_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The three real adapters, built from the entry (one instance of each per process: the engine's own contract, hue's answer to M-5;
/// the project of the release is the constant `cv2-s5`). The psql and the dumper adapters look at the CURRENT release, which is
/// `<home>/current`.
pub struct RealPorts {
    pub containers: RealContainers,
    pub psql: RealPsql,
    pub dumper: RealDumper,
}

/// The adapters, from the home and the port the entry derived, and the compose provider the entry read (`Some` for an upgrade and a rollback:
/// [`RealContainers::new`] REQUIRES it; `None` only for a check, which starts no container: [`RealContainers::for_check`], named so that it
/// is never a silent fallback).
pub fn real_ports(entry: &Entry, provider: Option<&Path>) -> RealPorts {
    let current = entry.home.join("current");
    RealPorts {
        containers: match provider {
            Some(p) => RealContainers::new(entry.home.clone(), entry.port, p.to_path_buf()),
            None => RealContainers::for_check(entry.home.clone(), entry.port),
        },
        psql: RealPsql::new(current.clone()),
        dumper: RealDumper::new(entry.home.clone(), current),
    }
}

/// The adapter for the local server (health and `whoami`; the operator's token goes to this transport and to no other): the literal
/// `http://127.0.0.1:<port>` the entry derived.
pub fn local_stack(entry: &Entry) -> HttpStack<LocalTransport> {
    HttpStack {
        transport: LocalTransport::new(LOCAL_REQUEST_TIMEOUT),
        base: entry.local_url.clone(),
    }
}

/// The engine's `Env` from the entry and the compose provider the entry read (an empty path for a check, which needs none): each value in its
/// own field (the engine reads no environment variable itself).
pub fn engine_env(entry: &Entry, provider: Option<&Path>) -> Env {
    Env {
        home: entry.home.clone(),
        manifest_url: entry.manifest_url.clone().unwrap_or_default(),
        local_url: entry.local_url.clone(),
        operator_token_file: entry.token_file.clone(),
        unit_dir: entry.unit_dir.clone(),
        compose_provider: provider.map(Path::to_path_buf).unwrap_or_default(),
    }
}

/// R-6b: the engine's `Env` AND the real adapters from the ONE provider the entry read, so that the SAME path reaches both and a cell can SEE it
/// (found by the mutant run: `run_engine` could build the adapters without the provider and no cell noticed).
pub fn engine_parts(entry: &Entry, provider: Option<&Path>) -> (Env, RealPorts) {
    (engine_env(entry, provider), real_ports(entry, provider))
}

/// R-6b (cox W1-68, W1-91 Q1): the verified compose provider, read ONCE for the operations that start containers (an upgrade and a rollback):
/// `<home>/compose-provider` through `cortex_upgrade::provider::read_compose_provider` (the record, the provider and every directory above them
/// checked; one plain line otherwise). A check starts no container and needs none: `None`, and no record is looked at.
pub fn compose_provider_for(entry: &Entry, mode: Mode) -> Result<Option<PathBuf>, String> {
    if mode == Mode::Check {
        return Ok(None);
    }
    cortex_upgrade::provider::read_compose_provider(&entry.home).map(Some)
}

/// THE ONE FUNCTION behind which the engine sits (cox W1-80: if the engine must ever leave the CLI binary, this is a file move).
fn run_engine<T: Transport>(
    args: &Args,
    entry: &Entry,
    provider: Option<&Path>,
    release_host: T,
) -> Outcome {
    let (env, ports) = engine_parts(entry, provider);
    let fetcher = Fetcher::new(release_host, FetchConfig::default(), None);
    let stack = local_stack(entry);
    let pace = SystemPace::new();
    let clock = SystemClock;
    Engine {
        env: &env,
        fetcher: &fetcher,
        stack: &stack,
        psql: &ports.psql,
        containers: &ports.containers,
        dumper: &ports.dumper,
        pace: &pace,
        clock: &clock,
    }
    .run(args)
}

/// install.sh's words (`deploy/install.sh`, the `case ${HOME:-} in /*)` line).
const HOME_NOT_ABSOLUTE: &str =
    "HOME must be an absolute path (the units go to $HOME/.config/systemd/user)";

/// The directory of the user units, by install.sh's OWN rule (`deploy/install.sh`: `HOME` must be an absolute path, the
/// units go to `$HOME/.config/systemd/user`; `XDG_CONFIG_HOME` is NOT consulted), then validated like the home directory
/// of R-5 (`members::check_home`: only letters, digits and `. _ / -`, no empty component, no `..`). The test that `HOME`
/// is absolute comes first, as in install.sh; the path is kept as written (nothing is normalised). Pure: the directory
/// itself (a real directory, the user's own, not writable by others) is checked by the engine before step 8.
pub fn unit_dir_from(home: Option<&OsStr>) -> Result<PathBuf, String> {
    let home = match home {
        Some(h) if h.as_bytes().first() == Some(&b'/') => h,
        _ => return Err(HOME_NOT_ABSOLUTE.to_owned()),
    };
    let Some(home) = home.to_str() else {
        return Err("HOME is not valid UTF-8".to_owned());
    };
    if !home
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
    {
        return Err(
            "HOME holds a character other than letters, digits and . _ / - (it is written into systemd units)"
                .to_owned(),
        );
    }
    let parts: Vec<&str> = home[1..].split('/').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(
            "HOME has an empty component (a double slash, a trailing slash or the root alone)"
                .to_owned(),
        );
    }
    if parts.contains(&"..") {
        return Err("HOME has a .. component".to_owned());
    }
    Ok(PathBuf::from(format!("{home}/.config/systemd/user")))
}

/// What the process environment holds, read ONCE through the CLI's own injectable lookup (`Ctx::env`: cortex-client's `env.rs` is the
/// only file that reads the process environment, cell EN-01) and handed to the pure rules. `XDG_CONFIG_HOME` is
/// deliberately not here: install.sh does not read it.
#[derive(Debug, Clone, Default)]
pub struct Vars {
    pub home: Option<OsString>,
    pub cortex_home: Option<OsString>,
    pub port: Option<OsString>,
    pub token_file: Option<OsString>,
    pub base_url: Option<OsString>,
}

impl Vars {
    /// Asks `lookup` for exactly five names, once each: `HOME`, `CORTEX_HOME`, `CORTEX_PORT`, `CORTEX_TOKEN_FILE` and
    /// `CORTEX_BASE_URL` (never `XDG_CONFIG_HOME`).
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<OsString>) -> Vars {
        Vars {
            home: lookup("HOME"),
            cortex_home: lookup("CORTEX_HOME"),
            port: lookup("CORTEX_PORT"),
            token_file: lookup("CORTEX_TOKEN_FILE"),
            base_url: lookup("CORTEX_BASE_URL"),
        }
    }
}

/// What the entry derives from the environment: the values of the engine's `Env`, and the port the real adapters need (the compose
/// child gets it as `CORTEX_PORT`). Plain values: this part knows no type of the engine, so it is checked before the engine is linked.
#[derive(Debug, Clone)]
pub struct Entry {
    pub home: PathBuf,
    pub port: u16,
    pub local_url: String,
    pub token_file: PathBuf,
    /// `None`: the online path is OFF (no `CORTEX_BASE_URL`); the operations that need no manifest still work (cox W1-88).
    pub manifest_url: Option<String>,
    pub unit_dir: PathBuf,
}

/// What the port refusals and the help tell: install.sh persists nothing of its `--port` in 0.2.001 (cox W1-80).
const PORT_HINT: &str = "if you installed with --port, set CORTEX_PORT to it";

/// The ports of the v1 stack that install.sh refuses.
const V1_PORTS: [u16; 5] = [8501, 5499, 5500, 8765, 8766];

/// `${CORTEX_HOME:-$HOME/cortex-v2}` by install.sh's `--home` rule (absolute, only letters, digits and `. _ / -`, no empty or
/// `..` component), with `CORTEX_HOME` in the words.
fn home_from(vars: &Vars, home: &str) -> Result<PathBuf, String> {
    let Some(given) = vars.cortex_home.as_deref().filter(|h| !h.is_empty()) else {
        // `home` is HOME, already checked by `unit_dir_from` (absolute, the alphabet, no empty or .. component)
        return Ok(PathBuf::from(format!("{home}/cortex-v2")));
    };
    if given.as_bytes().first() != Some(&b'/') {
        return Err("CORTEX_HOME must be an absolute path".to_owned());
    }
    let Some(text) = given.to_str() else {
        return Err("CORTEX_HOME is not valid UTF-8".to_owned());
    };
    if !text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
    {
        return Err(
            "CORTEX_HOME may contain only letters, digits and . _ / - (it is written into systemd units)"
                .to_owned(),
        );
    }
    let parts: Vec<&str> = text[1..].split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "..") {
        return Err(
            "CORTEX_HOME may not hold an empty or a .. component (a double slash, a trailing slash or a ..)"
                .to_owned(),
        );
    }
    Ok(PathBuf::from(text))
}

/// `${CORTEX_PORT:-8601}`: a plain decimal number from 1 to 65535 (digits only, no leading zero), and not a port of the v1 stack.
fn port_from(vars: &Vars) -> Result<u16, String> {
    let Some(given) = vars.port.as_deref().filter(|p| !p.is_empty()) else {
        return Ok(8601);
    };
    let plain = format!(
        "CORTEX_PORT must be a whole number from 1 to 65535 (digits only, no leading zero); {PORT_HINT}"
    );
    let bytes = given.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 5
        || bytes[0] == b'0'
        || !bytes.iter().all(u8::is_ascii_digit)
    {
        return Err(plain);
    }
    let port: u16 = given.to_str().and_then(|t| t.parse().ok()).ok_or(plain)?;
    if V1_PORTS.contains(&port) {
        return Err(format!(
            "CORTEX_PORT belongs to the live v1 stack; {PORT_HINT}"
        ));
    }
    Ok(port)
}

/// The operator's token file: set, absolute, no NUL. The file itself is the engine's to read.
fn token_file_from(vars: &Vars) -> Result<PathBuf, String> {
    let Some(given) = vars
        .token_file
        .as_deref()
        .filter(|t| t.as_bytes().first() == Some(&b'/'))
    else {
        return Err(
            "CORTEX_TOKEN_FILE must be set to the operator's token file (an absolute path)"
                .to_owned(),
        );
    };
    if given.as_bytes().contains(&0) {
        return Err("CORTEX_TOKEN_FILE holds a NUL byte".to_owned());
    }
    Ok(PathBuf::from(given))
}

/// `${CORTEX_BASE_URL%/}/releases/manifest.json` (cox W1-80, the shape hue's own cells use): the base is set (no default in the
/// tree: unset or empty is the online path OFF), `https://` in lower case with a host after it, only graphic ASCII (no space, no
/// control, nothing outside ASCII), at most 2000 bytes. ONE trailing slash is removed, as the shell's `%/` does; nothing else is
/// normalised. The engine parses the result; this is the shape check before anything is built.
fn manifest_url_from(vars: &Vars, mode: Mode) -> Result<Option<String>, String> {
    // a rollback never fetches: the variable is looked up with the others, but its value is ignored here (not validated, not used) (cox W1-88)
    if mode == Mode::Rollback {
        return Ok(None);
    }
    // no default in the tree: unset or empty is the online path OFF, which is not a refusal here (the operations that need a manifest refuse
    // in `run_with_transport`, the local ones do not)
    let Some(given) = vars.base_url.as_deref().filter(|u| !u.is_empty()) else {
        return Ok(None);
    };
    let bytes = given.as_bytes();
    if !bytes.starts_with(b"https://") || matches!(bytes.get(8), None | Some(b'/')) {
        return Err("CORTEX_BASE_URL must be an https URL".to_owned());
    }
    let plain = "CORTEX_BASE_URL holds a space, a control or a non-ASCII character";
    if !bytes.iter().all(|b| (0x21..=0x7e).contains(b)) {
        return Err(plain.to_owned());
    }
    if given.len() > 2000 {
        return Err("CORTEX_BASE_URL is longer than 2000 bytes".to_owned());
    }
    // graphic ASCII is valid UTF-8
    let text = given.to_str().ok_or_else(|| plain.to_owned())?;
    let base = text.strip_suffix('/').unwrap_or(text);
    Ok(Some(format!("{base}/releases/manifest.json")))
}

/// The entry's values from what the process environment holds. The order of the checks is the units directory (HOME), the
/// home, the port, the token file, the manifest URL: the first fault is the one line returned. Pure: no file is read.
pub fn entry_from(vars: &Vars, mode: Mode) -> Result<Entry, String> {
    let unit_dir = unit_dir_from(vars.home.as_deref())?;
    // `unit_dir_from` accepted HOME: it is present, absolute and UTF-8
    let home_text = vars
        .home
        .as_deref()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    let home = home_from(vars, home_text)?;
    let port = port_from(vars)?;
    let operator_token_file = token_file_from(vars)?;
    let manifest_url = manifest_url_from(vars, mode)?;
    Ok(Entry {
        home,
        port,
        local_url: format!("http://127.0.0.1:{port}"),
        token_file: operator_token_file,
        manifest_url,
        unit_dir,
    })
}
