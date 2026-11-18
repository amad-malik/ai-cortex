//! Piece 14, design §4 steps 2 and 3 and `--check`: UP-01, UP-02, UP-13, UP-14, UP-20, UP-25 and UP-30, through the
//! engine with FAKE ports (a recorded event order, a fake release host, a fake database, a fake local server) and the
//! `HttpStack` adapter against loopback servers. The files are a synthetic home (`common::install`).

mod common;

use common::*;
use cortex_types::Secret;
use cortex_upgrade::engine::{
    Args, Containers, DumpLabel, DumpResult, Dumper, Engine, Env, HttpStack, Mode, NoAnswer,
    Outcome, Pace, PortError, Psql, Stack, Whoami, parse_args,
};
use cortex_upgrade::fetch::{FetchConfig, FetchError, Fetcher, Request, Response, Transport};
use cortex_upgrade::http::LocalTransport;
use cortex_upgrade::manifest::Digest;
use cortex_upgrade::ports::{OneLine, Service, ServiceState};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MANIFEST_URL: &str = "https://rel.example.invalid/releases/manifest.json";
const TOKEN: &str = "SYNTHETIC-TOKEN-1"; // what common::install writes to operator.token
const RC1: &str = "0.2.1-rc.1";
const RC2: &str = "0.2.1-rc.2";

type Events = Arc<Mutex<Vec<String>>>;

// ---------------------------------------------------------------- fakes

type OnGet = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
struct FakeHost {
    events: Events,
    routes: Mutex<HashMap<String, (u16, Vec<u8>)>>,
    fail: Mutex<Option<FetchError>>,
    /// Called with the URL of every request, before it is answered.
    on_get: Mutex<Option<OnGet>>,
}
struct HostT(Arc<FakeHost>);
impl Transport for HostT {
    fn get(&self, r: &Request<'_>) -> Result<Response, FetchError> {
        self.0.events.lock().unwrap().push(format!("GET {}", r.url));
        let hook = self.0.on_get.lock().unwrap().clone();
        if let Some(f) = hook {
            f(r.url);
        }
        if let Some(e) = self.0.fail.lock().unwrap().clone() {
            return Err(e);
        }
        let (status, body) = self
            .0
            .routes
            .lock()
            .unwrap()
            .get(r.url)
            .cloned()
            .unwrap_or((404, b"no".to_vec()));
        Ok(Response {
            status,
            location: None,
            body: Box::new(Cursor::new(body)),
        })
    }
}

struct FakeStack {
    events: Events,
    answer: Mutex<Whoami>,
    /// Answers of `whoami`, one per call; once empty, `answer`.
    answers: Mutex<std::collections::VecDeque<Whoami>>,
    tokens: Mutex<Vec<String>>,
    /// Answers of `ready`, one per call; once empty, `Yes`.
    ready_seq: Mutex<std::collections::VecDeque<cortex_upgrade::engine::Ready>>,
}
impl Stack for FakeStack {
    fn ready(&self) -> cortex_upgrade::engine::Ready {
        self.events.lock().unwrap().push("ready".into());
        self.ready_seq
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(cortex_upgrade::engine::Ready::Yes)
    }
    fn whoami(&self, token: &Secret) -> Whoami {
        self.events.lock().unwrap().push("whoami".into());
        self.tokens.lock().unwrap().push(token.expose().to_string());
        let queued = self.answers.lock().unwrap().pop_front();
        queued.unwrap_or_else(|| self.answer.lock().unwrap().clone())
    }
}

struct FakePsql {
    events: Events,
    answer: Mutex<Result<String, NoAnswer>>,
}
impl Psql for FakePsql {
    fn trust_rule_count(&self) -> Result<String, NoAnswer> {
        self.events.lock().unwrap().push("psql".into());
        self.answer.lock().unwrap().clone()
    }
}

/// What the install looks like at this moment: where `current` points and which release directories exist.
fn observe(home: &std::path::Path) -> String {
    let cur = std::fs::read_link(home.join("current"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .unwrap_or_else(|_| "-".into());
    let mut names: Vec<String> = std::fs::read_dir(home.join("releases"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    format!("current={cur} releases={}", names.join(","))
}

/// A hook run with the directory `run_roles` was given.
type RolesHook = Arc<dyn Fn(&std::path::Path) + Send + Sync>;

/// What the fake `podman` says to `load_image`.
struct FakeContainers {
    home: std::path::PathBuf,
    /// "<call> <observe>" at every call that matters for "nothing changes before the dump".
    seen: Events,
    events: Events,
    load: Mutex<Result<Digest, PortError>>,
    /// Answers of `state`, per service name, one per call; once empty: db and server Healthy, migrate Exited(0).
    states: Mutex<HashMap<&'static str, std::collections::VecDeque<ServiceState>>>,
    /// Answers of `compose_up`, one per call; once empty: Ok.
    ups: Mutex<std::collections::VecDeque<Result<(), PortError>>>,
    /// The answer of `run_roles`.
    roles: Mutex<Result<(), PortError>>,
    /// Answers of `run_roles`, one per call (W1-49 B: a timeout is confirmed by one re-run); once empty: `roles`.
    roles_seq: Mutex<std::collections::VecDeque<Result<(), PortError>>>,
    /// Run at the start of `load_image` (a test makes the file system misbehave at exactly this point).
    on_load: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Run at the start of every `compose_up`.
    on_up: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Run at the start of every `run_roles` with the directory it was given (R-4a: the stage; a cell reads it or replaces the original script).
    on_roles: Mutex<Option<RolesHook>>,
}
impl Containers for FakeContainers {
    fn load_image(&self, archive: &std::path::Path) -> Result<Digest, PortError> {
        let hook = self.on_load.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
        self.seen
            .lock()
            .unwrap()
            .push(format!("load {}", observe(&self.home)));
        self.events
            .lock()
            .unwrap()
            .push(format!("load {}", archive.display()));
        self.load.lock().unwrap().clone()
    }
    fn compose_up(&self, d: &std::path::Path, s: &[Service]) -> Result<(), PortError> {
        let hook = self.on_up.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
        let names: Vec<&str> = s.iter().map(|x| x.name()).collect();
        self.seen
            .lock()
            .unwrap()
            .push(format!("compose_up {}", observe(&self.home)));
        self.events.lock().unwrap().push(format!(
            "compose_up {} {}",
            names.join(","),
            d.file_name().unwrap().to_string_lossy()
        ));
        self.ups.lock().unwrap().pop_front().unwrap_or(Ok(()))
    }
    fn state(&self, _d: &std::path::Path, s: Service) -> Result<ServiceState, PortError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("state {}", s.name()));
        let queued = self
            .states
            .lock()
            .unwrap()
            .get_mut(s.name())
            .and_then(|q| q.pop_front());
        Ok(queued.unwrap_or(match s {
            Service::Migrate => ServiceState::Exited(0),
            _ => ServiceState::Healthy,
        }))
    }
    fn run_roles(
        &self,
        d: &std::path::Path,
        m: &[cortex_types::ModuleId],
        t: Duration,
    ) -> Result<(), PortError> {
        let hook = self.on_roles.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(d);
        }
        let ids: Vec<&str> = m.iter().map(|x| x.as_str()).collect();
        self.events
            .lock()
            .unwrap()
            .push(format!("run_roles {} {}", ids.join(","), t.as_secs()));
        if let Some(answer) = self.roles_seq.lock().unwrap().pop_front() {
            return answer;
        }
        self.roles.lock().unwrap().clone()
    }
}

/// How the fake `cortex-dump` behaves.
#[derive(Clone)]
enum DumpMode {
    /// Make a good set under dumps/pre-upgrade/<label> and say so.
    Good,
    /// Exit with this status and this last stderr line; make nothing.
    Exit(i32, &'static str),
    /// The port itself fails.
    Port(PortError),
    /// Say exit 0 with this last stdout line; make nothing.
    Path(String),
    /// Make the set with this SHA256SUMS text (`None`: no such file).
    Sums(Option<String>),
    /// Exit 0 and write nothing to stdout.
    NoStdout,
}

struct FakeDumper {
    seen: Events,
    events: Events,
    home: std::path::PathBuf,
    mode: Mutex<DumpMode>,
}

fn sha_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    cortex_types::Sha256(sha2::Sha256::digest(bytes).into()).to_hex()
}

/// `dumps/pre-upgrade/<label>/` with `dump.sql` and a SHA256SUMS of the given text.
fn make_set(home: &std::path::Path, label: &str, sums: Option<String>) -> std::path::PathBuf {
    let set = home.join("dumps").join("pre-upgrade").join(label);
    std::fs::create_dir_all(&set).unwrap();
    std::fs::write(set.join("dump.sql"), "-- a synthetic dump\n").unwrap();
    // things a SHA256SUMS could wrongly name and that would hash right: a link, a hidden file, and a file beside the set
    std::os::unix::fs::symlink("dump.sql", set.join("link.sql")).unwrap();
    std::fs::write(set.join(".hidden"), "-- a synthetic dump\n").unwrap();
    // names of exactly 128 and of 129 characters (the limit is 128)
    std::fs::write(set.join("b".repeat(128)), "-- a synthetic dump\n").unwrap();
    std::fs::write(set.join("a".repeat(129)), "-- a synthetic dump\n").unwrap();
    std::fs::write(
        set.parent().unwrap().join("dump.sql"),
        "-- a synthetic dump\n",
    )
    .unwrap();
    if let Some(text) = sums {
        std::fs::write(set.join("SHA256SUMS"), text).unwrap();
    }
    set
}

impl Dumper for FakeDumper {
    fn dump(&self, label: &DumpLabel, timeout: Duration) -> Result<DumpResult, PortError> {
        self.seen
            .lock()
            .unwrap()
            .push(format!("dump {}", observe(&self.home)));
        self.events
            .lock()
            .unwrap()
            .push(format!("dump {} {}", label.as_str(), timeout.as_secs()));
        let good_sums = format!("{}  dump.sql\n", sha_hex(b"-- a synthetic dump\n"));
        let reply = |set: &std::path::Path| DumpResult {
            exit: 0,
            last_stdout_line: Some(set.display().to_string()),
            last_stderr_line: OneLine::new("restore ONLY with cortex-restore"),
        };
        match self.mode.lock().unwrap().clone() {
            DumpMode::Good => {
                // the real tool publishes a FRESH directory per run (a stamp in its name): a second dump for the same pair, after a lost run, gets another name
                let mut name = label.as_str().to_string();
                while self.home.join("dumps/pre-upgrade").join(&name).exists() {
                    name.push_str("-again");
                }
                Ok(reply(&make_set(&self.home, &name, Some(good_sums))))
            }
            DumpMode::Exit(code, line) => Ok(DumpResult {
                exit: code,
                last_stdout_line: None,
                last_stderr_line: OneLine::new(line),
            }),
            DumpMode::Port(e) => Err(e),
            DumpMode::Path(p) => Ok(DumpResult {
                exit: 0,
                last_stdout_line: Some(p),
                last_stderr_line: OneLine::new(""),
            }),
            DumpMode::Sums(s) => Ok(reply(&make_set(&self.home, label.as_str(), s))),
            DumpMode::NoStdout => Ok(DumpResult {
                exit: 0,
                last_stdout_line: None,
                last_stderr_line: OneLine::new(""),
            }),
        }
    }
}

struct FakePace {
    events: Events,
    now: Mutex<Duration>,
}
impl Pace for FakePace {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap()
    }
    fn sleep(&self, d: Duration) {
        self.events
            .lock()
            .unwrap()
            .push(format!("sleep {}", d.as_secs()));
        *self.now.lock().unwrap() += d;
    }
}

struct Harness {
    tmp: Tmp,
    env: Env,
    events: Events,
    host: Arc<FakeHost>,
    stack: FakeStack,
    psql: FakePsql,
    containers: FakeContainers,
    dumper: FakeDumper,
    pace: FakePace,
    clock: FixedClock,
    seen: Events,
}

fn release_json(version: &str, min: Option<&str>, db_image: &str) -> Value {
    let tar = format!("cortex-{version}-x86_64-linux.tar.gz");
    let oci = format!("cortex-server-{version}-x86_64.oci.tar");
    let mut cortex = json!({
        "label": "x", "members": 2, "server_image": { "ref": format!("localhost/cortex-server@{DIGEST_A}"), "asset": oci },
        "db_image": db_image, "roles_script_digest": DIGEST_A, "sidecar_images": [], "migrations": ["platform/0001_platform"],
        "modules": ["audit"], "migration_class": "expand"
    });
    if let Some(m) = min {
        cortex["min_upgrade_from"] = json!(m);
    }
    json!({ "version": version, "date": "2026-11-12T00:00:00Z", "notes_url": "https://example.invalid/n",
        "assets": [ { "name": tar, "url": tar, "digest": DIGEST_A }, { "name": oci, "url": oci, "digest": DIGEST_A } ], "cortex": cortex })
}

fn db_image() -> String {
    format!("example.invalid/db:1@{DIGEST_A}")
}

fn manifest(releases: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({ "schema": 1, "releases": releases })).unwrap()
}

fn harness(installed: &str, served: Vec<Value>) -> Harness {
    harness_in("engine", installed, served)
}

/// `harness` with the tag of the home directory's name (R-5: a home whose name the units cannot carry).
fn harness_in(tag: &str, installed: &str, served: Vec<Value>) -> Harness {
    // pr_13 runs the engine in a child process under a strict umask; the parent built the home, the child only uses it
    let borrowed = std::env::var("CORTEX_TEST_UMASK_ENGINE").ok();
    let tmp = match &borrowed {
        Some(path) => Tmp(std::path::PathBuf::from(path)),
        None => Tmp::new(tag),
    };
    let tmp_path = tmp.path().to_path_buf();
    let seen: Events = Arc::default();
    if borrowed.is_none() {
        install(tmp.path(), installed);
    }
    let events: Events = Arc::default();
    let host = Arc::new(FakeHost {
        events: events.clone(),
        ..FakeHost::default()
    });
    host.routes
        .lock()
        .unwrap()
        .insert(MANIFEST_URL.into(), (200, manifest(served)));
    // the user-unit directory of the backup units (R-5): a private directory beside the home, as the entry point's rule makes it
    let unit_dir = tmp.path().join("units-test");
    std::fs::create_dir_all(&unit_dir).unwrap();
    std::fs::set_permissions(
        &unit_dir,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let env = Env {
        home: tmp.path().to_path_buf(),
        manifest_url: MANIFEST_URL.into(),
        local_url: "http://127.0.0.1:8601".into(),
        operator_token_file: tmp.path().join("operator.token"),
        unit_dir,
        compose_provider: std::path::PathBuf::from("/usr/bin/podman-compose"),
    };
    Harness {
        tmp,
        env,
        events: events.clone(),
        host,
        stack: FakeStack {
            events: events.clone(),
            answer: Mutex::new(Whoami::Ok),
            answers: Mutex::default(),
            tokens: Mutex::default(),
            ready_seq: Mutex::default(),
        },
        psql: FakePsql {
            events: events.clone(),
            answer: Mutex::new(Ok("0".into())),
        },
        containers: FakeContainers {
            home: tmp_path.clone(),
            seen: seen.clone(),
            events: events.clone(),
            load: Mutex::new(Ok(Digest([0xaa; 32]))),
            states: Mutex::default(),
            ups: Mutex::default(),
            roles: Mutex::new(Ok(())),
            roles_seq: Mutex::new(std::collections::VecDeque::new()),
            on_load: Mutex::default(),
            on_up: Mutex::default(),
            on_roles: Mutex::default(),
        },
        dumper: FakeDumper {
            seen: seen.clone(),
            events: events.clone(),
            home: tmp_path,
            mode: Mutex::new(DumpMode::Good),
        },
        pace: FakePace {
            events,
            now: Mutex::new(Duration::ZERO),
        },
        clock: FixedClock(1_794_000_000),
        seen,
    }
}

fn args(list: &[&str]) -> Args {
    parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>()).expect("arguments parse")
}

fn run(h: &Harness, list: &[&str]) -> Outcome {
    let fetcher = Fetcher::new(HostT(h.host.clone()), FetchConfig::default(), None);
    Engine {
        env: &h.env,
        fetcher: &fetcher,
        stack: &h.stack,
        psql: &h.psql,
        containers: &h.containers,
        dumper: &h.dumper,
        pace: &h.pace,
        clock: &h.clock,
    }
    .run(&args(list))
}

/// `run` with test hooks (cox W1-56): the lost-process cells.
fn run_hooked(h: &Harness, list: &[&str], hooks: &cortex_upgrade::engine::Hooks) -> Outcome {
    let fetcher = Fetcher::new(HostT(h.host.clone()), FetchConfig::default(), None);
    Engine {
        env: &h.env,
        fetcher: &fetcher,
        stack: &h.stack,
        psql: &h.psql,
        containers: &h.containers,
        dumper: &h.dumper,
        pace: &h.pace,
        clock: &h.clock,
    }
    .run_hooked(&args(list), hooks)
}

fn events(h: &Harness) -> Vec<String> {
    h.events.lock().unwrap().clone()
}

fn assert_safe(o: &Outcome) {
    for l in &o.lines {
        assert!(
            !l.is_empty() && l.len() < 400 && !l.chars().any(char::is_control),
            "{l:?}"
        );
        assert!(
            !l.contains(TOKEN) && !l.contains("SYNTHETIC"),
            "a token or body is in: {l}"
        );
    }
}

fn said(o: &Outcome, text: &str) -> bool {
    o.lines.iter().any(|l| l.contains(text))
}

// ---------------------------------------------------------------- arguments

#[test]
fn en_01_arguments() {
    let a = args(&[]);
    assert_eq!((a.mode, a.to.clone()), (Mode::Upgrade, None));
    assert_eq!(
        (a.health_interval, a.health_timeout, a.roles_timeout),
        (
            Duration::from_secs(2),
            Duration::from_secs(120),
            Duration::from_secs(60)
        )
    );
    assert_eq!(
        a.migrate_timeout,
        Duration::from_secs(1800),
        "the migration has its own bound"
    );
    assert_eq!(
        args(&["--migrate-timeout", "9"]).migrate_timeout,
        Duration::from_secs(9)
    );
    assert_eq!(args(&["--check"]).mode, Mode::Check);
    assert_eq!(args(&["--rollback"]).mode, Mode::Rollback);
    let a = args(&[
        "--to",
        RC2,
        "--health-interval",
        "3",
        "--health-timeout",
        "30",
        "--roles-timeout",
        "90",
    ]);
    assert_eq!(
        (a.to, a.health_interval, a.health_timeout, a.roles_timeout),
        (
            Some(RC2.parse().unwrap()),
            Duration::from_secs(3),
            Duration::from_secs(30),
            Duration::from_secs(90)
        )
    );
    assert_eq!(args(&["--check", "--to", RC2]).mode, Mode::Check);
    for bad in [
        vec!["--nope"],
        vec!["upgrade"],
        vec!["--to"],
        vec!["--to", RC1, "--to", RC2],
        vec!["--check", "--rollback"],
        vec!["--rollback", "--to", RC1],
        vec!["--check", "--check"],
        vec!["--health-interval", "0"],
        vec!["--health-interval", "abc"],
        vec!["--health-interval", "99999"],
        vec!["--health-timeout", "-5"],
        vec!["--roles-timeout"],
        vec!["--roles-timeout", "44"],
        vec!["--roles-timeout", "1", "--roles-timeout", "2"],
        vec!["--migrate-timeout"],
        vec!["--migrate-timeout", "0"],
        vec!["--migrate-timeout", "3601"],
        vec!["--migrate-timeout", "1", "--migrate-timeout", "2"],
        vec!["--to", "0.2.001"],
        vec!["--to", "1.0.0;touch x"],
        vec!["--to", "$(id)"],
        vec!["--to", "1.0.0\nx"],
        vec!["--to", &"9".repeat(200)],
        vec!["--to="],
        vec!["--to=0.2.1"],
    ] {
        let e = parse_args(&bad.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .expect_err(&format!("{bad:?}"));
        assert!(
            !e.is_empty() && e.len() < 300 && !e.chars().any(char::is_control),
            "{e:?}"
        );
        assert!(!e.contains("touch x") || e.len() < 300);
    }
}

// ---------------------------------------------------------------- cox 19:41Z (mike R6): the role script's own budget and the engine's deadline

#[test]
fn en_09_the_roles_deadline_exceeds_the_role_scripts_server_side_budget_by_the_margin() {
    use cortex_upgrade::engine::{
        ROLES_COMMIT_LATENCY, ROLES_CONFIRM_TIMEOUT, ROLES_IDLE, ROLES_LEAD, ROLES_MARGIN,
        ROLES_SCRIPT_BUDGET, ROLES_SLACK,
    };
    // THE RELATION, written once (cox W1-55; mike's M-4; deploy/db-roles.sh has idle=5, latency=2, slack=2 and lead=$((idle + latency + slack)), pinned by checker rule S5-22 and the
    // self-test). The CALLER passes its absolute deadline E (the real adapter: the moment it launches the script + the run_roles timeout); the script reads no clock of its own. The last
    // statement of the roles transaction refuses when the database clock is past E - LEAD, and LEAD is the sum of three terms: IDLE (a client that stalls between the guard and COMMIT is
    // ended by idle_in_transaction_session_timeout), COMMIT_LATENCY (COMMIT itself) and SLACK (what is left). So the LATEST COMPLETED COMMIT IS E - SLACK, whatever delayed the launch, the lock
    // or the client. Each term has its own assertion below, and a mutation of each in mutants/engine.json.
    assert_eq!(ROLES_SCRIPT_BUDGET, Duration::from_secs(30));
    assert_eq!(ROLES_IDLE, Duration::from_secs(5));
    assert_eq!(ROLES_COMMIT_LATENCY, Duration::from_secs(2));
    assert_eq!(ROLES_SLACK, Duration::from_secs(2));
    assert_eq!(ROLES_LEAD, ROLES_IDLE + ROLES_COMMIT_LATENCY + ROLES_SLACK);
    assert_eq!(ROLES_LEAD, Duration::from_secs(9));
    // the latest completed commit, as a distance before E: the guard passes at E - LEAD at the latest, then the client may stall IDLE and COMMIT may take COMMIT_LATENCY
    let latest_commit_before_e = ROLES_LEAD - ROLES_IDLE - ROLES_COMMIT_LATENCY;
    assert_eq!(latest_commit_before_e, ROLES_SLACK);
    assert_eq!(latest_commit_before_e, Duration::from_secs(2));
    assert_eq!(ROLES_MARGIN, Duration::from_secs(15));
    let floor = ROLES_SCRIPT_BUDGET + ROLES_MARGIN;
    // the guard leaves the roles step a window after the launch: the floor minus the lead (36 s), far more than the work needs, and the budget still fits inside the floor
    assert_eq!(floor - ROLES_LEAD, Duration::from_secs(36));
    // the confirmation re-run after a timeout gets a fresh deadline of exactly the floor (B)
    assert_eq!(ROLES_CONFIRM_TIMEOUT, floor);
    assert!(args(&[]).roles_timeout >= floor, "the default");
    // exactly the floor is accepted (positive control, passes at once), one second less is refused with the number in the sentence
    assert_eq!(args(&["--roles-timeout", "45"]).roles_timeout, floor);
    let e = parse_args(&["--roles-timeout".to_string(), "44".to_string()])
        .expect_err("below the floor");
    assert!(e.contains("45"), "{e}");
    assert!(e.contains("--roles-timeout"), "{e}");
    for n in ["1", "2", "30", "44"] {
        assert!(
            parse_args(&["--roles-timeout".to_string(), n.to_string()]).is_err(),
            "{n}"
        );
    }
}

// ---------------------------------------------------------------- UP-01: --check

#[test]
fn en_02_check_says_newer_same_or_lower_and_writes_nothing() {
    for (served, code, text) in [
        (RC2, 10u8, "newer"),
        (RC1, 0, "up to date"),
        ("0.2.0", 0, "up to date"),
    ] {
        let h = harness(RC1, vec![release_json(served, None, &db_image())]);
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, code, "{served}: {o:?}");
        assert!(said(&o, text), "{served}: {o:?}");
        assert_safe(&o);
        assert_eq!(
            tree_hash(h.tmp.path()),
            before,
            "--check changed the tree ({served})"
        );
        assert!(
            !h.tmp.path().join("upgrade.lock").exists(),
            "--check took the lock"
        );
        assert_eq!(
            events(&h),
            ["psql", "whoami", &format!("GET {MANIFEST_URL}")],
            "the order of steps 2: database check, pre-flight, manifest"
        );
    }
    // the newest of several
    let h = harness(
        RC1,
        vec![
            release_json(RC2, None, &db_image()),
            release_json("0.2.1-rc.3", None, &db_image()),
            release_json("0.2.0", None, &db_image()),
        ],
    );
    let o = run(&h, &["--check"]);
    assert_eq!(o.code, 10);
    assert!(said(&o, "0.2.1-rc.3"), "{o:?}");
}

// ---------------------------------------------------------------- UP-02: a bad manifest, a bad host

#[test]
fn en_03_a_bad_manifest_or_host_is_refused_with_its_rule_and_nothing_else_is_requested() {
    let good = || release_json(RC2, None, &db_image());
    let bad_digest = {
        let mut r = good();
        r["assets"][0]["digest"] = json!("md5:abc");
        r
    };
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("not json", b"not json".to_vec(), "json"),
        (
            "schema 2",
            serde_json::to_vec(&json!({"schema": 2, "releases": []})).unwrap(),
            "schema",
        ),
        ("a bad digest", manifest(vec![bad_digest]), "digest"),
        (
            "over 1 MiB",
            vec![b' '; (1 << 20) + 1],
            "larger than the limit",
        ),
    ];
    for (what, body, word) in cases {
        let h = harness(RC1, vec![]);
        h.host
            .routes
            .lock()
            .unwrap()
            .insert(MANIFEST_URL.into(), (200, body));
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 2, "{what}: {o:?}");
        assert!(said(&o, word), "{what}: {o:?}");
        assert_safe(&o);
        assert_eq!(
            events(&h).iter().filter(|e| e.starts_with("GET ")).count(),
            1,
            "{what}: only the manifest was requested"
        );
        assert_eq!(tree_hash(h.tmp.path()), before);
    }
    for (what, setup) in [
        ("a 404", (404u16, b"SYNTHETIC body".to_vec())),
        ("a 500", (500, b"SYNTHETIC body".to_vec())),
    ] {
        let h = harness(RC1, vec![]);
        h.host
            .routes
            .lock()
            .unwrap()
            .insert(MANIFEST_URL.into(), setup);
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 2, "{what}");
        assert!(said(&o, "status"), "{o:?}");
        assert_safe(&o);
    }
    for e in [
        FetchError::Timeout,
        FetchError::Transport,
        FetchError::TooLarge,
    ] {
        let h = harness(RC1, vec![]);
        *h.host.fail.lock().unwrap() = Some(e.clone());
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 2, "{e:?}");
        assert_safe(&o);
    }
}

// ---------------------------------------------------------------- UP-13: --to and compatibility

#[test]
fn en_04_to_and_the_compatibility_rules() {
    let h = harness(
        RC2,
        vec![
            release_json(RC1, None, &db_image()),
            release_json("0.2.0", None, &db_image()),
        ],
    );
    let o = run(&h, &["--check", "--to", RC1]);
    assert_eq!(o.code, 2);
    assert!(said(&o, "downgrade"), "{o:?}");

    let h = harness(RC1, vec![release_json("0.3.0", Some(RC2), &db_image())]);
    let o = run(&h, &["--check"]);
    assert_eq!(o.code, 2);
    assert!(said(&o, "min_upgrade_from"), "{o:?}");

    let other = format!("example.invalid/db:2@{DIGEST_A}");
    let h = harness(RC1, vec![release_json(RC2, None, &other)]);
    let o = run(&h, &["--check"]);
    assert_eq!(o.code, 2);
    assert!(said(&o, "db_image"), "{o:?}");

    let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
    let o = run(&h, &["--check", "--to", "0.9.9"]);
    assert_eq!(o.code, 2);
    assert!(said(&o, "no usable release"), "{o:?}");
    assert_eq!(
        run(&h, &["--check", "--to", RC1]).code,
        0,
        "--to the installed version is nothing to do"
    );
    assert_eq!(run(&h, &["--check", "--to", RC2]).code, 10);
}

// ---------------------------------------------------------------- UP-25: the database check (§4a)

#[test]
fn en_05_the_database_check_comes_first_and_accepts_only_a_zero() {
    for (answer, word) in [
        (Ok("3".to_string()), "3 trust rule"),
        (Ok(String::new()), "could not read a count of trust rules"),
        (
            Ok("abc".to_string()),
            "could not read a count of trust rules",
        ),
        (
            Ok("-1".to_string()),
            "could not read a count of trust rules",
        ),
        (
            Ok("0\n0".to_string()),
            "could not read a count of trust rules",
        ),
        (
            Ok("1 2".to_string()),
            "could not read a count of trust rules",
        ),
        (
            Ok("99999999999999999999".to_string()),
            "could not read a count of trust rules",
        ),
        (
            Ok("\u{1b}[31mxyz".to_string()),
            "could not read a count of trust rules",
        ),
        (Err(NoAnswer), "could not read a count of trust rules"),
    ] {
        let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
        *h.psql.answer.lock().unwrap() = answer.clone();
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 2, "{answer:?}: {o:?}");
        assert!(said(&o, word), "{answer:?}: {o:?}");
        assert_safe(&o);
        assert_eq!(
            events(&h),
            ["psql"],
            "{answer:?}: nothing after the database check"
        );
        assert_eq!(tree_hash(h.tmp.path()), before);
    }
    for ok in ["0", "00", " 0 ", "0\n"] {
        let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
        *h.psql.answer.lock().unwrap() = Ok(ok.to_string());
        assert_eq!(run(&h, &["--check"]).code, 10, "{ok:?} counts as zero");
    }
}

// ---------------------------------------------------------------- UP-30: the token pre-flight (§4c)

#[test]
fn en_06_the_pre_flight_comes_before_the_manifest_and_prints_only_a_closed_list() {
    for code in [
        "token_missing",
        "token_invalid",
        "token_expired",
        "token_revoked",
        "token_superseded",
        "generation_stale",
        "worker_retired",
    ] {
        let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
        *h.stack.answer.lock().unwrap() =
            Whoami::Refused(Some(Box::leak(code.to_string().into_boxed_str())));
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 2, "{code}");
        assert!(
            said(
                &o,
                &format!(
                    "the operator token is refused ({code}); re-issue it with install.sh --reissue-token and point CORTEX_TOKEN_FILE at the new file"
                )
            ),
            "{o:?}"
        );
        assert_safe(&o);
        assert_eq!(
            events(&h),
            ["psql", "whoami"],
            "no manifest request after a refused token"
        );
        assert_eq!(tree_hash(h.tmp.path()), before);
    }
    let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
    *h.stack.answer.lock().unwrap() = Whoami::Refused(None);
    let o = run(&h, &["--check"]);
    assert!(
        said(&o, "the operator token is refused (401); re-issue it"),
        "{o:?}"
    );
    for (answer, text) in [
        (Whoami::Unanswered("503".into()), "(503)"),
        (Whoami::Unanswered("timeout".into()), "(timeout)"),
        (Whoami::Unanswered("no answer".into()), "(no answer)"),
    ] {
        let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
        *h.stack.answer.lock().unwrap() = answer.clone();
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 2, "{answer:?}");
        assert!(
            said(&o, "the running stack did not answer an authenticated read") && said(&o, text),
            "{o:?}"
        );
        assert_eq!(events(&h), ["psql", "whoami"]);
    }
    // a good answer goes on, and the engine gave the stack the token from the file, exactly
    let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
    assert_eq!(run(&h, &["--check"]).code, 10);
    assert_eq!(*h.stack.tokens.lock().unwrap(), vec![TOKEN.to_string()]);
}

#[test]
fn en_07_the_operator_token_file_must_be_there_and_sane() {
    for (what, content) in [
        ("empty", Some("")),
        ("blank", Some("  \n")),
        ("too large", Some("x")),
        ("missing", None),
    ] {
        let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
        let p = h.tmp.path().join("operator.token");
        match (what, content) {
            ("too large", _) => std::fs::write(&p, "x".repeat(5000)).unwrap(),
            (_, Some(c)) => std::fs::write(&p, c).unwrap(),
            (_, None) => std::fs::remove_file(&p).unwrap(),
        }
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 2, "{what}");
        assert!(said(&o, "operator token file"), "{what}: {o:?}");
        assert_safe(&o);
        assert!(
            h.stack.tokens.lock().unwrap().is_empty(),
            "{what}: whoami was not called"
        );
        assert!(
            !events(&h).iter().any(|e| e.starts_with("GET ")),
            "{what}: no manifest request"
        );
    }
    // surrounding white space and a trailing newline are not part of the token
    let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
    std::fs::write(
        h.tmp.path().join("operator.token"),
        "  SYNTHETIC-TOKEN-1  \n",
    )
    .unwrap();
    assert_eq!(run(&h, &["--check"]).code, 10);
    assert_eq!(*h.stack.tokens.lock().unwrap(), vec![TOKEN.to_string()]);
}

#[test]
fn en_08_the_current_release_must_be_readable() {
    let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
    std::fs::remove_file(h.tmp.path().join("current")).unwrap();
    let o = run(&h, &["--check"]);
    assert_eq!(o.code, 2);
    assert!(said(&o, "no running release"), "{o:?}");
    assert!(events(&h).is_empty(), "nothing was asked of anything");
    let h = harness(RC1, vec![release_json(RC2, None, &db_image())]);
    std::fs::write(
        h.tmp.path().join("releases").join(RC1).join("stack.env"),
        "CORTEX_VERSION=0.2.1-rc.1\nEXTRA=1\n",
    )
    .unwrap();
    let o = run(&h, &["--check"]);
    assert_eq!(o.code, 2);
    assert!(said(&o, "stack.env"), "{o:?}");
}

// ---------------------------------------------------------------- the local server adapter

mod local {
    use super::*;
    use common::http::{Reply, response, serve};

    fn stack_for(base: String) -> HttpStack<LocalTransport> {
        HttpStack {
            transport: LocalTransport::new(Duration::from_secs(5)),
            base,
        }
    }
    fn token() -> Secret {
        Secret::new(TOKEN)
    }

    #[test]
    fn st_01_a_200_is_ok_and_the_request_is_the_whoami_with_the_bearer() {
        let s = serve(|_| response("200 OK", &[], b"{}"));
        assert_eq!(
            stack_for(format!("http://127.0.0.1:{}", s.addr.port())).whoami(&token()),
            Whoami::Ok
        );
        let seen = s.requests();
        assert!(
            seen[0].request_line.starts_with("GET /v2/whoami HTTP/1.1"),
            "{:?}",
            seen[0].request_line
        );
        assert_eq!(
            seen[0].header("authorization"),
            Some(&format!("Bearer {TOKEN}")[..])
        );
    }

    #[test]
    fn st_02_a_401_is_a_code_of_the_closed_list_or_none_and_the_body_is_never_kept() {
        for code in [
            "token_missing",
            "token_invalid",
            "token_expired",
            "token_revoked",
            "token_superseded",
            "generation_stale",
            "worker_retired",
        ] {
            let body = format!("{{\"code\":\"{code}\",\"detail\":\"SYNTHETIC detail\"}}");
            let s = serve(move |_| response("401 Unauthorized", &[], body.as_bytes()));
            match stack_for(format!("http://127.0.0.1:{}", s.addr.port())).whoami(&token()) {
                Whoami::Refused(Some(c)) => assert_eq!(c, code),
                other => panic!("{code}: {other:?}"),
            }
        }
        let big = format!(
            "{{\"code\":\"token_revoked\",\"pad\":\"{}\"}}",
            "x".repeat(70_000)
        );
        let hostile: Vec<Vec<u8>> = vec![
            b"{\"code\":\"made_up\"}".to_vec(),
            b"{\"code\":5}".to_vec(),
            b"{}".to_vec(),
            b"not json".to_vec(),
            b"\x1b[31mSYNTHETIC-TOKEN-9 $(id)".to_vec(),
            b"{\"code\":\"token_revoked \"}".to_vec(),
            b"{\"code\":\"TOKEN_REVOKED\"}".to_vec(),
            big.into_bytes(),
            vec![],
        ];
        for body in hostile {
            let b2 = body.clone();
            let s = serve(move |_| response("401 Unauthorized", &[], &b2));
            let r = stack_for(format!("http://127.0.0.1:{}", s.addr.port())).whoami(&token());
            assert_eq!(
                r,
                Whoami::Refused(None),
                "{:?}",
                String::from_utf8_lossy(&body[..body.len().min(40)])
            );
            assert!(!format!("{r:?}").contains("SYNTHETIC"));
        }
    }

    #[test]
    fn st_03_anything_else_is_unanswered_with_a_status_or_a_word() {
        for (line, word) in [
            ("503 Service Unavailable", "503"),
            ("500 Internal Server Error", "500"),
            ("302 Found", "302"),
            ("404 Not Found", "404"),
            ("403 Forbidden", "403"),
            ("204 No Content", "204"),
        ] {
            let s = serve(move |_| response(line, &[("Location", "/x")], b"SYNTHETIC"));
            assert_eq!(
                stack_for(format!("http://127.0.0.1:{}", s.addr.port())).whoami(&token()),
                Whoami::Unanswered(word.into()),
                "{line}"
            );
        }
        let s = serve(|_| Reply::Hang);
        let base = format!("http://127.0.0.1:{}", s.addr.port());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let st = HttpStack {
                transport: LocalTransport::new(Duration::from_millis(500)),
                base,
            };
            let _ = tx.send(st.whoami(&token()));
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(8)).expect("no timeout"),
            Whoami::Unanswered("timeout".into())
        );
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        assert_eq!(
            stack_for(format!("http://127.0.0.1:{port}")).whoami(&token()),
            Whoami::Unanswered("no answer".into())
        );
        // a base that is not the literal loopback address never connects
        assert_eq!(
            stack_for("http://localhost:8601".into()).whoami(&token()),
            Whoami::Unanswered("no answer".into())
        );
    }
    #[test]
    fn st_04_ready_is_a_200_on_the_health_path_without_any_token_and_everything_else_is_a_word() {
        use cortex_upgrade::engine::Ready;
        let s = serve(|_| response("200 OK", &[], b"{}"));
        assert_eq!(
            stack_for(format!("http://127.0.0.1:{}", s.addr.port())).ready(),
            Ready::Yes
        );
        let seen = s.requests();
        assert!(
            seen[0]
                .request_line
                .starts_with("GET /v2/health/ready HTTP/1.1"),
            "{:?}",
            seen[0].request_line
        );
        assert_eq!(
            seen[0].header("authorization"),
            None,
            "the operator's token goes to /v2/whoami only"
        );
        for (line, word) in [
            ("503 Service Unavailable", "503"),
            ("500 Internal Server Error", "500"),
            ("302 Found", "302"),
            ("404 Not Found", "404"),
            ("401 Unauthorized", "401"),
            ("204 No Content", "204"),
        ] {
            let s = serve(move |_| response(line, &[("Location", "/x")], b"SYNTHETIC"));
            let r = stack_for(format!("http://127.0.0.1:{}", s.addr.port())).ready();
            assert_eq!(r, Ready::No(word.into()), "{line}");
            assert!(!format!("{r:?}").contains("SYNTHETIC"), "no body text");
        }
        let s = serve(|_| Reply::Hang);
        let base = format!("http://127.0.0.1:{}", s.addr.port());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let st = HttpStack {
                transport: LocalTransport::new(Duration::from_millis(500)),
                base,
            };
            let _ = tx.send(st.ready());
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(8)).expect("no timeout"),
            Ready::No("timeout".into())
        );
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        assert_eq!(
            stack_for(format!("http://127.0.0.1:{port}")).ready(),
            Ready::No("no answer".into())
        );
        assert_eq!(
            stack_for("http://localhost:8601".into()).ready(),
            Ready::No("no answer".into())
        );
    }
}

// ---------------------------------------------------------------- steps 1, 4 and 5: lock, recover, stage, unpack, verify

mod prepare {
    use super::*;
    use common::tar;
    use cortex_upgrade::engine::{Plan, Prepared};
    use cortex_upgrade::layout::{FlipStep, Home, Incomplete, LayoutError, PrevLink};
    use sha2::Digest as _;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const COMPOSE: &str = include_str!("fixtures/s5-compose.yml");
    const ROLES: &[u8] = b"#!/bin/sh\n# the module roles\n";
    const BASE: &str = "https://rel.example.invalid/releases/";
    const RC3: &str = "0.2.1-rc.3";

    type Files = Vec<(String, Vec<u8>, u32)>;

    fn digest_text(bytes: &[u8]) -> String {
        format!(
            "sha256:{}",
            cortex_types::Sha256(sha2::Sha256::digest(bytes).into()).to_hex()
        )
    }

    fn stack_env(version: &str) -> String {
        format!(
            "CORTEX_VERSION={version}\nCORTEX_SERVER_IMAGE=localhost/cortex-server@{DIGEST_A}\nCORTEX_DB_IMAGE={}\n",
            db_image()
        )
    }

    /// The members list 2 added to list 1 (cox R-1/R-4): the example config, the backup scripts in `bin/`, the user units in `systemd/`.
    const LIST2_MEMBERS: [&str; 11] = [
        "cortex.toml.example",
        "bin/cortex-dump",
        "bin/cortex-dump-status",
        "bin/cortex-dump-alert",
        "bin/cortex-restore",
        "bin/restore-compare",
        "systemd/cortex-dump.service",
        "systemd/cortex-dump.timer",
        "systemd/cortex-dump-status.service",
        "systemd/cortex-dump-status.timer",
        "systemd/cortex-dump-alert.service",
    ];

    /// The files of a good release archive.
    fn files(version: &str) -> Files {
        vec![
            ("cortex".into(), b"#!/bin/sh\necho cortex\n".to_vec(), 0o755),
            ("compose.yml".into(), COMPOSE.as_bytes().to_vec(), 0o644),
            ("stack.env".into(), stack_env(version).into_bytes(), 0o644),
            ("db-roles.sh".into(), ROLES.to_vec(), 0o755),
            (
                "initdb/00-cortex-roles.sh".into(),
                b"#!/bin/sh\n:\n".to_vec(),
                0o644,
            ),
        ]
        .into_iter()
        .chain(LIST2_MEMBERS.iter().map(|n| {
            let mode = if n.starts_with("bin/") { 0o755 } else { 0o644 };
            (n.to_string(), format!("synthetic {n}\n").into_bytes(), mode)
        }))
        .collect()
    }

    fn archive(f: &Files) -> Vec<u8> {
        let mut entries = Vec::new();
        let mut dirs_done: Vec<&str> = Vec::new();
        for (name, data, mode) in f {
            for d in ["initdb", "bin", "systemd"] {
                if name.starts_with(&format!("{d}/")) && !dirs_done.contains(&d) {
                    entries.push(tar::dir(&format!("{d}/")));
                    dirs_done.push(d);
                }
            }
            entries.push(tar::file(name, data, *mode));
        }
        tar::gz(&tar::finish(&entries))
    }

    /// The manifest entry for `version` with the REAL digests of the archive, the image and the roles script.
    fn real_release(version: &str, tgz: &[u8], oci: &[u8]) -> Value {
        let tar_name = format!("cortex-{version}-x86_64-linux.tar.gz");
        let oci_name = format!("cortex-server-{version}-x86_64.oci.tar");
        json!({ "version": version, "date": "2026-11-12T00:00:00Z", "notes_url": "https://example.invalid/n",
            "assets": [
                { "name": tar_name, "url": tar_name, "digest": digest_text(tgz) },
                { "name": oci_name, "url": oci_name, "digest": digest_text(oci) } ],
            "cortex": { "label": "x", "members": 2, "server_image": { "ref": format!("localhost/cortex-server@{DIGEST_A}"), "asset": oci_name },
                "db_image": db_image(), "roles_script_digest": digest_text(ROLES), "sidecar_images": [],
                "migrations": ["platform/0001_platform"], "modules": ["audit", "tokens"], "migration_class": "expand" } })
    }

    const OCI: &[u8] = b"SYNTHETIC-OCI-ARCHIVE";

    /// RC1 installed (with the S5 compose file), the manifest serving `version` built from `f`; `tweak` edits the manifest entry.
    fn staged_harness(version: &str, f: Files, tweak: impl FnOnce(&mut Value)) -> Harness {
        staged_harness_in("engine", version, f, tweak)
    }

    fn staged_harness_in(
        tag: &str,
        version: &str,
        f: Files,
        tweak: impl FnOnce(&mut Value),
    ) -> Harness {
        let tgz = archive(&f);
        let mut rel = real_release(version, &tgz, OCI);
        tweak(&mut rel);
        let h = harness_in(tag, RC1, vec![rel]);
        fs::write(
            h.tmp.path().join("releases").join(RC1).join("compose.yml"),
            COMPOSE,
        )
        .unwrap();
        let mut routes = h.host.routes.lock().unwrap();
        routes.insert(
            format!("{BASE}cortex-{version}-x86_64-linux.tar.gz"),
            (200, tgz),
        );
        routes.insert(
            format!("{BASE}cortex-server-{version}-x86_64.oci.tar"),
            (200, OCI.to_vec()),
        );
        drop(routes);
        h
    }

    fn engine_prepare(h: &Harness) -> Result<Prepared, Outcome> {
        let fetcher = Fetcher::new(HostT(h.host.clone()), FetchConfig::default(), None);
        let e = Engine {
            env: &h.env,
            fetcher: &fetcher,
            stack: &h.stack,
            psql: &h.psql,
            containers: &h.containers,
            dumper: &h.dumper,
            pace: &h.pace,
            clock: &h.clock,
        };
        match e.plan(&args(&[]))? {
            Plan::Upgrade { from, to, manifest } => e.stage(from, to, &manifest),
            Plan::NothingToDo(v) => panic!("nothing to do at {v}"),
        }
    }

    fn mode_of(p: &std::path::Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o7777
    }

    /// A refusal: exit 2, safe lines, and the home is byte for byte what it was.
    fn refused_clean(h: &Harness, before: &str, r: Result<Prepared, Outcome>, text: &str) {
        let o = match r {
            Err(o) => o,
            Ok(p) => panic!("expected a refusal containing {text:?}, got {p:?}"),
        };
        assert_eq!(o.code, 2, "{o:?}");
        assert_safe(&o);
        assert!(said(&o, text), "{text:?} not in {o:?}");
        assert_eq!(
            tree_hash(h.tmp.path()),
            before,
            "the home changed after a refusal ({text})"
        );
    }

    #[test]
    fn pr_01_a_good_release_is_downloaded_unpacked_and_verified_and_nothing_is_installed() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let before = tree_hash(h.tmp.path());
        let p = engine_prepare(&h).unwrap();
        let home = h.tmp.path();
        assert_eq!(
            (p.from.to_string(), p.release.version.to_string()),
            (RC1.into(), RC2.into())
        );
        assert_eq!(p.staging, home.join("releases").join(format!("{RC2}.part")));
        assert_eq!(p.downloads, home.join("downloads").join(RC2));
        // the staged tree: our modes
        assert_eq!(mode_of(&p.staging), 0o700);
        assert_eq!(mode_of(&p.staging.join("cortex")), 0o755);
        assert_eq!(mode_of(&p.staging.join("db-roles.sh")), 0o755);
        assert_eq!(mode_of(&p.staging.join("compose.yml")), 0o644);
        assert_eq!(fs::read(p.staging.join("db-roles.sh")).unwrap(), ROLES);
        // the downloads: both archives, 0600, the bytes that were served
        assert_eq!(mode_of(&p.downloads), 0o700);
        assert_eq!(mode_of(&home.join("downloads")), 0o700);
        for name in ["release.tar.gz", "server-image.oci.tar"] {
            assert_eq!(mode_of(&p.downloads.join(name)), 0o600, "{name}");
        }
        assert_eq!(
            fs::read(p.downloads.join("server-image.oci.tar")).unwrap(),
            OCI
        );
        // nothing is installed: no release directory, the links as they were
        assert!(!home.join("releases").join(RC2).exists());
        assert_eq!(
            fs::read_link(home.join("current")).unwrap(),
            std::path::Path::new(&format!("releases/{RC1}"))
        );
        assert!(!home.join("previous").exists());
        // the order: the database and the token first, then the manifest, then the two archives
        assert_eq!(
            events(&h),
            [
                "psql".to_string(),
                "whoami".to_string(),
                format!("GET {MANIFEST_URL}"),
                format!("GET {BASE}cortex-{RC2}-x86_64-linux.tar.gz"),
                format!("GET {BASE}cortex-server-{RC2}-x86_64.oci.tar"),
            ]
        );
        // the plan names what will be used, including the roles script digest (cox 14:30Z)
        let text = p.lines.join("\n");
        for want in [
            format!("upgrade {RC1} -> {RC2}"),
            format!("roles script {}", digest_text(ROLES)),
            "database service unchanged".to_string(),
            "modules: audit,tokens".to_string(),
        ] {
            assert!(text.contains(&want), "{want:?} not in:\n{text}");
        }
        assert!(
            p.lines
                .iter()
                .all(|l| !l.is_empty() && !l.chars().any(char::is_control))
        );
        // discarding puts the home back as it was
        p.discard();
        assert_eq!(tree_hash(h.tmp.path()), before);
    }

    #[test]
    fn pr_02_a_lock_that_is_held_stops_an_upgrade_before_anything_is_asked_but_not_a_check() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let home = Home::new(h.tmp.path());
        let guard = home.lock().unwrap();
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &[]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "another upgrade is running"), "{o:?}");
        assert!(
            events(&h).is_empty(),
            "nothing was asked or fetched: {:?}",
            events(&h)
        );
        assert_eq!(tree_hash(h.tmp.path()), before);
        // --check only reads: it is not blocked by the lock
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 10, "{o:?}");
        drop(guard);
        // control: with the lock free, an upgrade gets as far as asking the database
        let o = run(&h, &[]);
        assert!(events(&h).iter().any(|e| e == "psql"), "{o:?}");
    }

    #[test]
    fn pr_03_an_interrupted_rollback_is_finished_first_and_an_unusable_intent_stops_everything() {
        // current = RC2, previous = RC1; a rollback stopped after its intent was written
        let build = || {
            let h = staged_harness(RC3, files(RC3), |_| {});
            stage(h.tmp.path(), RC2, "expand");
            let home = Home::new(h.tmp.path());
            home.commit(&v(RC2)).unwrap();
            let r = home.flip_back_with(&mut |s| s != FlipStep::IntentWritten);
            assert!(r.is_err(), "the rollback was interrupted");
            assert!(h.tmp.path().join("rollback.intent").is_file());
            h
        };
        let h = build();
        let o = run(&h, &[]);
        // the unfinished rollback is finished FIRST (RC2 to RC1); what the run does after that is the upgrade's business
        assert!(
            said(
                &o,
                &format!("completed an interrupted rollback ({RC2} to {RC1})")
            ),
            "{o:?}"
        );
        assert!(!h.tmp.path().join("rollback.intent").exists());
        // an intent that cannot be used: exit 2 before anything is asked, and the file is left alone
        let h = build();
        fs::write(h.tmp.path().join("rollback.intent"), "garbage").unwrap();
        // upgrade.lock is part of the layout (design §2) and the first locker creates it: make it exist before the snapshot
        drop(Home::new(h.tmp.path()).lock().unwrap());
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &[]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "rollback"), "{o:?}");
        assert!(events(&h).is_empty(), "{:?}", events(&h));
        assert_eq!(tree_hash(h.tmp.path()), before);
        assert_safe(&o);
    }

    #[test]
    fn pr_04_leftovers_of_an_earlier_run_are_removed_by_exact_path_and_a_link_in_their_place_is_refused()
     {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let home = h.tmp.path();
        fs::create_dir_all(home.join("releases").join(format!("{RC2}.part"))).unwrap();
        fs::write(
            home.join("releases")
                .join(format!("{RC2}.part"))
                .join("junk"),
            "x",
        )
        .unwrap();
        fs::create_dir_all(home.join("downloads").join(RC2)).unwrap();
        fs::write(home.join("downloads").join(RC2).join("old"), "x").unwrap();
        let p = engine_prepare(&h).unwrap();
        assert!(!p.staging.join("junk").exists() && !p.downloads.join("old").exists());
        let text = p.lines.join("\n");
        assert!(
            text.contains("removed a leftover staging directory"),
            "{text}"
        );
        assert!(
            text.contains("removed a leftover download directory"),
            "{text}"
        );
        p.discard();
        // a symlink where the staging directory goes: refused, and the link and its target are untouched
        let h = staged_harness(RC2, files(RC2), |_| {});
        let home = h.tmp.path();
        let outside = home.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), "keep").unwrap();
        std::os::unix::fs::symlink(
            "../outside",
            home.join("releases").join(format!("{RC2}.part")),
        )
        .unwrap();
        let before = tree_hash(home);
        refused_clean(&h, &before, engine_prepare(&h), "not a directory");
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
    }

    #[test]
    fn pr_05_a_bad_digest_on_either_archive_leaves_nothing_and_the_second_is_not_fetched_after_the_first_fails()
     {
        let h = staged_harness(RC2, files(RC2), |r| {
            r["assets"][0]["digest"] = json!(DIGEST_A)
        });
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(&h, &before, r, "digest");
        let gets: Vec<String> = events(&h)
            .into_iter()
            .filter(|e| e.starts_with("GET"))
            .collect();
        assert_eq!(
            gets.len(),
            2,
            "the manifest and the release archive only: {gets:?}"
        );
        let h = staged_harness(RC2, files(RC2), |r| {
            r["assets"][1]["digest"] = json!(DIGEST_A)
        });
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(&h, &before, r, "digest");
    }

    #[test]
    fn pr_06_a_manifest_without_the_release_archive_or_the_image_archive_is_refused_before_any_download()
     {
        let h = staged_harness(RC2, files(RC2), |r| {
            r["assets"][0]["name"] = json!("other.tar.gz")
        });
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(&h, &before, r, &format!("cortex-{RC2}-x86_64-linux.tar.gz"));
        assert_eq!(
            events(&h).iter().filter(|e| e.starts_with("GET")).count(),
            1,
            "only the manifest"
        );
        let h = staged_harness(RC2, files(RC2), |r| {
            r["cortex"]["server_image"]["asset"] = json!("nothing.oci.tar")
        });
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(&h, &before, r, "server image archive");
        assert_eq!(
            events(&h).iter().filter(|e| e.starts_with("GET")).count(),
            1,
            "only the manifest"
        );
    }

    #[test]
    fn pr_07_a_hostile_or_incomplete_archive_is_refused_and_everything_is_removed() {
        let mut f = files(RC2);
        f.push(("../evil".into(), b"x".to_vec(), 0o644));
        let h = staged_harness(RC2, f, |_| {});
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(&h, &before, r, "not allowed in a release");
        let mut f = files(RC2);
        f.retain(|(n, _, _)| n != "db-roles.sh");
        let h = staged_harness(RC2, f, |_| {});
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(&h, &before, r, "lacks `db-roles.sh`");
    }

    #[test]
    fn pr_08_the_roles_script_must_be_the_one_the_manifest_names() {
        let h = staged_harness(RC2, files(RC2), |r| {
            r["cortex"]["roles_script_digest"] = json!(DIGEST_A)
        });
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(
            &h,
            &before,
            r,
            "db-roles.sh does not match the digest in the manifest",
        );
        // one byte different in the archive, the manifest unchanged
        let mut f = files(RC2);
        f[3].1 = b"#!/bin/sh\n# the module roles, changed\n".to_vec();
        let tgz = archive(&f);
        let h = staged_harness(RC2, files(RC2), |_| {});
        h.host.routes.lock().unwrap().insert(
            format!("{BASE}cortex-{RC2}-x86_64-linux.tar.gz"),
            (200, tgz.clone()),
        );
        // the served archive no longer matches its own digest either: the earlier check fires first
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(&h, &before, r, "digest");
    }

    #[test]
    fn pr_09_the_staged_stack_env_must_be_valid_and_agree_with_the_manifest() {
        let bad_version = stack_env("0.2.1-rc.9");
        let bad_server = stack_env(RC2).replace(
            &format!("localhost/cortex-server@{DIGEST_A}"),
            &format!("localhost/cortex-server@sha256:{}", "b".repeat(64)),
        );
        let bad_db = stack_env(RC2).replace(
            &db_image(),
            &format!("example.invalid/db:2@sha256:{}", "c".repeat(64)),
        );
        let extra = format!("{}CORTEX_PASSWORD=hunter2\n", stack_env(RC2));
        for (what, text, want) in [
            ("version", bad_version, "another version"),
            ("server image", bad_server, "another server image"),
            ("database image", bad_db, "another database image"),
            ("unusable", extra, "stack.env refused"),
        ] {
            let mut f = files(RC2);
            f[2].1 = text.into_bytes();
            let h = staged_harness(RC2, f, |_| {});
            let before = tree_hash(h.tmp.path());
            let r = engine_prepare(&h);
            refused_clean(&h, &before, r, want);
            let _ = what;
        }
    }

    #[test]
    fn pr_10_the_db_service_must_not_change_and_a_shape_it_cannot_read_is_a_refusal_on_either_side()
    {
        // a changed db service: named by its key path, no value
        let changed = COMPOSE.replacen("POSTGRES_DB: cortex", "POSTGRES_DB: other", 1);
        assert_ne!(changed, COMPOSE);
        let mut f = files(RC2);
        f[1].1 = changed.into_bytes();
        let h = staged_harness(RC2, f, |_| {});
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(
            &h,
            &before,
            r,
            "the db service differs at services.db.environment.POSTGRES_DB",
        );
        // a shape the reader refuses, in the candidate and in the running release
        let mut f = files(RC2);
        f[1].1 = COMPOSE
            .replacen(
                "    init: true\n",
                "    env_file: ./db.env\n    init: true\n",
                1,
            )
            .into_bytes();
        let h = staged_harness(RC2, f, |_| {});
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(
            &h,
            &before,
            r,
            "candidate release's compose.yml has env_file",
        );
        let h = staged_harness(RC2, files(RC2), |_| {});
        fs::write(
            h.tmp.path().join("releases").join(RC1).join("compose.yml"),
            "services: {}\n",
        )
        .unwrap();
        let before = tree_hash(h.tmp.path());
        let r = engine_prepare(&h);
        refused_clean(&h, &before, r, "running release's compose.yml");
        // control: a change to another service is no difference
        let server_only = COMPOSE.replacen("mem_limit: 1g", "mem_limit: 3g", 1);
        assert_ne!(server_only, COMPOSE);
        let mut f = files(RC2);
        f[1].1 = server_only.into_bytes();
        let h = staged_harness(RC2, f, |_| {});
        let p = engine_prepare(&h).expect("a change outside the db service is allowed");
        p.discard();
    }
    // ------------------------------------------------------------ steps 6 and 7: the pre-upgrade dump and the image load

    const LABEL: &str = "pre-upgrade-0.2.1-rc.1-to-0.2.1-rc.2";

    /// A bad set path: what it is, and how to make the fake dumper say it.
    type PathCase = (&'static str, Box<dyn Fn(&Harness) -> DumpMode>);

    fn set_dump(h: &Harness, m: DumpMode) {
        *h.dumper.mode.lock().unwrap() = m;
    }

    /// The install itself is as it was: RC1 is current, nothing is staged, no downloads, no new release directory.
    fn install_unchanged(h: &Harness) {
        let home = h.tmp.path();
        assert_eq!(
            fs::read_link(home.join("current")).unwrap(),
            std::path::Path::new(&format!("releases/{RC1}"))
        );
        let mut names: Vec<String> = fs::read_dir(home.join("releases"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, [RC1.to_string()], "no staging or release directory");
        assert!(!home.join("downloads").exists(), "no downloads directory");
        assert!(!home.join("previous").exists());
    }

    fn gets(h: &Harness) -> usize {
        events(h).iter().filter(|e| e.starts_with("GET")).count()
    }

    #[test]
    fn dl_01_after_staging_the_dump_is_taken_verified_and_then_the_image_is_loaded() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let o = run(&h, &[]);
        let home = h.tmp.path();
        assert_safe(&o);
        let ev = events(&h);
        assert_eq!(
            ev[..7],
            [
                "psql".to_string(),
                "whoami".to_string(),
                format!("GET {MANIFEST_URL}"),
                format!("GET {BASE}cortex-{RC2}-x86_64-linux.tar.gz"),
                format!("GET {BASE}cortex-server-{RC2}-x86_64.oci.tar"),
                format!("dump {LABEL} 1800"),
                format!(
                    "load {}",
                    home.join("downloads")
                        .join(RC2)
                        .join("server-image.oci.tar")
                        .display()
                ),
            ]
        );
        let set = home.join("dumps").join("pre-upgrade").join(LABEL);
        assert!(
            set.join("SHA256SUMS").is_file(),
            "the dump set is never deleted"
        );
        let text = o.lines.join("\n");
        // the path is printed once, at the end, not at the step
        assert!(text.contains("pre-upgrade dump set verified\n"), "{text}");
        assert_eq!(
            text.matches(&set.display().to_string()).count(),
            1,
            "{text}"
        );
        assert!(
            text.contains(&format!("pre-upgrade dump set kept at {}", set.display())),
            "{text}"
        );
        assert!(text.contains("server image loaded"), "{text}");
        // the dump step comes after the plan and the load after the dump
        let at = |needle: &str| {
            o.lines
                .iter()
                .position(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("{needle}: {text}"))
        };
        assert!(at("roles script") < at("pre-upgrade dump set verified"));
        assert!(at("pre-upgrade dump set verified") < at("server image loaded"));
    }

    #[test]
    fn dl_02_the_installed_dump_tool_is_required_before_anything_is_asked_and_a_check_does_not_need_it()
     {
        for what in ["missing", "not executable", "a directory", "a link"] {
            let h = staged_harness(RC2, files(RC2), |_| {});
            let tool = h.tmp.path().join("bin").join("cortex-dump");
            match what {
                "missing" => fs::remove_file(&tool).unwrap(),
                "not executable" => {
                    fs::set_permissions(&tool, PermissionsExt::from_mode(0o644)).unwrap()
                }
                "a directory" => {
                    fs::remove_file(&tool).unwrap();
                    fs::create_dir(&tool).unwrap();
                }
                _ => {
                    fs::remove_file(&tool).unwrap();
                    std::os::unix::fs::symlink("/bin/true", &tool).unwrap();
                }
            }
            drop(Home::new(h.tmp.path()).lock().unwrap());
            let before = tree_hash(h.tmp.path());
            let o = run(&h, &[]);
            assert_eq!(o.code, 2, "{what}: {o:?}");
            assert!(
                said(&o, "the dump tool bin/cortex-dump is not installed"),
                "{what}: {o:?}"
            );
            assert!(
                events(&h).is_empty(),
                "{what}: nothing was asked or fetched: {:?}",
                events(&h)
            );
            assert_eq!(tree_hash(h.tmp.path()), before, "{what}");
            assert_safe(&o);
            // --check only reads: it does not need the tool
            let o = run(&h, &["--check"]);
            assert_eq!(o.code, 10, "{what}: {o:?}");
        }
    }

    #[test]
    fn dl_03_every_exit_of_the_dump_tool_but_0_is_a_refusal_with_its_one_line_and_nothing_else_happens()
     {
        for (code, want) in [
            (1, "the pre-upgrade dump failed: disk is full"),
            (2, "the pre-upgrade dump was refused: disk is full"),
            (
                3,
                "the pre-upgrade dump did not start, another dump is running: disk is full",
            ),
            (4, "the pre-upgrade dump timed out: disk is full"),
            (7, "the pre-upgrade dump ended with status 7: disk is full"),
            (
                -1,
                "the pre-upgrade dump ended with status -1: disk is full",
            ),
        ] {
            let h = staged_harness(RC2, files(RC2), |_| {});
            set_dump(&h, DumpMode::Exit(code, "disk is full"));
            let o = run(&h, &[]);
            assert_eq!(o.code, 2, "{code}: {o:?}");
            assert!(said(&o, want), "{code}: {want:?} not in {o:?}");
            assert_safe(&o);
            assert!(
                !events(&h).iter().any(|e| e.starts_with("load")),
                "{code}: no load after a failed dump"
            );
            install_unchanged(&h);
        }
    }

    #[test]
    fn dl_04_a_port_error_from_the_dump_is_a_refusal_too() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        set_dump(&h, DumpMode::Port(PortError::TimedOut));
        let o = run(&h, &[]);
        assert_eq!(o.code, 2);
        assert!(said(&o, "the pre-upgrade dump timed out"), "{o:?}");
        install_unchanged(&h);
        let h = staged_harness(RC2, files(RC2), |_| {});
        set_dump(
            &h,
            DumpMode::Port(PortError::Failed(OneLine::new("could not start podman"))),
        );
        let o = run(&h, &[]);
        assert_eq!(o.code, 2);
        assert!(
            said(
                &o,
                "the pre-upgrade dump could not be run: could not start podman"
            ),
            "{o:?}"
        );
        assert!(!events(&h).iter().any(|e| e.starts_with("load")));
        install_unchanged(&h);
    }

    #[test]
    fn dl_05_the_set_path_must_be_a_fresh_directory_directly_under_the_labelled_area() {
        let area = |h: &Harness| h.tmp.path().join("dumps").join("pre-upgrade");
        let cases: Vec<PathCase> = vec![
            ("no path", Box::new(|_| DumpMode::NoStdout)),
            (
                "no dump area",
                Box::new(|h| DumpMode::Path(h.tmp.path().join("dumps/x").display().to_string())),
            ),
            (
                "a relative path",
                Box::new(|_| DumpMode::Path("dumps/pre-upgrade/x".into())),
            ),
            ("an empty path", Box::new(|_| DumpMode::Path(String::new()))),
            (
                "outside the area",
                Box::new(|h| {
                    fs::create_dir_all(h.tmp.path().join("dumps/other")).unwrap();
                    fs::create_dir_all(h.tmp.path().join("dumps/pre-upgrade")).unwrap();
                    DumpMode::Path(h.tmp.path().join("dumps/other").display().to_string())
                }),
            ),
            (
                "the area itself",
                Box::new(move |h| {
                    fs::create_dir_all(area(h)).unwrap();
                    DumpMode::Path(area(h).display().to_string())
                }),
            ),
            (
                "a set that does not exist",
                Box::new(move |h| {
                    fs::create_dir_all(area(h)).unwrap();
                    DumpMode::Path(area(h).join("nope").display().to_string())
                }),
            ),
            (
                "a file",
                Box::new(move |h| {
                    fs::create_dir_all(area(h)).unwrap();
                    fs::write(area(h).join("afile"), "x").unwrap();
                    DumpMode::Path(area(h).join("afile").display().to_string())
                }),
            ),
            (
                "a link out of the area",
                Box::new(move |h| {
                    fs::create_dir_all(area(h)).unwrap();
                    fs::create_dir_all(h.tmp.path().join("elsewhere")).unwrap();
                    std::os::unix::fs::symlink("../../elsewhere", area(h).join("lnk")).unwrap();
                    DumpMode::Path(area(h).join("lnk").display().to_string())
                }),
            ),
            (
                "two levels down",
                Box::new(move |h| {
                    fs::create_dir_all(area(h).join("a/b")).unwrap();
                    DumpMode::Path(area(h).join("a/b").display().to_string())
                }),
            ),
            (
                "a set that was there before this run",
                Box::new(|h| {
                    // a complete, verified set made a moment ago (so its mtime is fresh): not this run's backup
                    let good = format!("{}  dump.sql\n", sha_hex(b"-- a synthetic dump\n"));
                    let set = make_set(
                        h.tmp.path(),
                        "20261106T000000Z-pre-upgrade-earlier",
                        Some(good),
                    );
                    DumpMode::Path(set.display().to_string())
                }),
            ),
        ];
        for (what, mode) in cases {
            // each shape is refused for ITS reason, not for whatever fails next
            let why = match what {
                "no path" => "named no path",
                "a relative path" | "an empty path" => "not absolute",
                "outside the area"
                | "the area itself"
                | "a link out of the area"
                | "two levels down" => "not directly under",
                "a set that does not exist" => "cannot be found",
                "no dump area" => "does not exist",
                "a file" => "not a directory",
                "a set that was there before this run" => "already existed before this run",
                other => panic!("no reason written for {other}"),
            };
            let h = staged_harness(RC2, files(RC2), |_| {});
            let m = mode(&h);
            set_dump(&h, m);
            let o = run(&h, &[]);
            assert_eq!(o.code, 2, "{what}: {o:?}");
            assert!(said(&o, "the dump set is not usable"), "{what}: {o:?}");
            assert!(said(&o, why), "{what}: the reason {why:?} is not in {o:?}");
            assert_safe(&o);
            assert!(
                !events(&h).iter().any(|e| e.starts_with("load")),
                "{what}: no load"
            );
            assert!(!h.tmp.path().join("downloads").exists(), "{what}");
            assert!(
                !h.tmp
                    .path()
                    .join("releases")
                    .join(format!("{RC2}.part"))
                    .exists(),
                "{what}"
            );
        }
    }

    #[test]
    fn dl_06_the_set_is_checked_against_its_own_sha256sums_again() {
        let good = format!("{}  dump.sql\n", sha_hex(b"-- a synthetic dump\n"));
        let bad: Vec<(&str, Option<String>)> = vec![
            ("no SHA256SUMS", None),
            ("an empty one", Some(String::new())),
            ("garbage", Some("garbage\n".into())),
            (
                "a wrong hash",
                Some(format!("{}  dump.sql\n", "0".repeat(64))),
            ),
            (
                "a file that is not there",
                Some(format!("{}  nothere.sql\n", sha_hex(b"x"))),
            ),
            (
                "a path in the name",
                Some(format!("{}  ../dump.sql\n", sha_hex(b"x"))),
            ),
            (
                "a name starting with a dot",
                Some(format!("{}  .hidden\n", sha_hex(b"x"))),
            ),
            ("uppercase hex", Some(good.to_uppercase())),
            (
                "one good line and one bad",
                Some(format!("{good}not a line\n")),
            ),
            (
                "one good line and a wrong hash",
                Some(format!("{good}{}  dump.sql\n", "1".repeat(64))),
            ),
            (
                "a line without a name",
                Some(format!("{}\n", sha_hex(b"x"))),
            ),
            (
                "a link that hashes right",
                Some(format!("{}  link.sql\n", sha_hex(b"-- a synthetic dump\n"))),
            ),
            (
                "a hidden file that hashes right",
                Some(format!("{}  .hidden\n", sha_hex(b"-- a synthetic dump\n"))),
            ),
            (
                "a name of 129 characters that hashes right",
                Some(format!(
                    "{}  {}\n",
                    sha_hex(b"-- a synthetic dump\n"),
                    "a".repeat(129)
                )),
            ),
            (
                "a path that hashes right",
                Some(format!(
                    "{}  ../dump.sql\n",
                    sha_hex(b"-- a synthetic dump\n")
                )),
            ),
        ];
        for (what, sums) in bad {
            let h = staged_harness(RC2, files(RC2), |_| {});
            set_dump(&h, DumpMode::Sums(sums));
            let o = run(&h, &[]);
            assert_eq!(o.code, 2, "{what}: {o:?}");
            assert!(
                said(&o, "the dump set failed its checksum check"),
                "{what}: {o:?}"
            );
            assert_safe(&o);
            assert!(
                !events(&h).iter().any(|e| e.starts_with("load")),
                "{what}: no load"
            );
            assert!(
                h.tmp.path().join("dumps/pre-upgrade").join(LABEL).is_dir(),
                "{what}: the set is kept"
            );
        }
        // controls: the binary-mode spelling of sha256sum is read too, and so is a set with a second good file
        for (what, sums) in [
            (
                "binary mode",
                format!("{}  dump.sql\n", sha_hex(b"-- a synthetic dump\n")).replace("  ", " *"),
            ),
            ("plain mode", good.clone()),
            (
                "a name of exactly 128 characters",
                format!(
                    "{}  {}\n",
                    sha_hex(b"-- a synthetic dump\n"),
                    "b".repeat(128)
                ),
            ),
        ] {
            let h = staged_harness(RC2, files(RC2), |_| {});
            set_dump(&h, DumpMode::Sums(Some(sums)));
            run(&h, &[]);
            assert!(
                events(&h).iter().any(|e| e.starts_with("load")),
                "{what}: accepted, so the image is loaded"
            );
        }
    }

    #[test]
    fn dl_07_the_loaded_image_must_be_the_one_the_manifest_names() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.load.lock().unwrap() = Err(PortError::Failed(OneLine::new("no space left")));
        let o = run(&h, &[]);
        assert_eq!(o.code, 2);
        assert!(
            said(&o, "the server image could not be loaded: no space left"),
            "{o:?}"
        );
        // a refusal after the dump says what it left behind (mike, packet 6): the set, never deleted
        assert!(
            said(
                &o,
                &format!("pre-upgrade dump set kept at {}", set_path(&h))
            ),
            "{o:?}"
        );
        assert!(
            h.tmp.path().join("dumps/pre-upgrade").join(LABEL).is_dir(),
            "the dump set stays"
        );
        install_unchanged(&h);
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.load.lock().unwrap() = Err(PortError::TimedOut);
        let o = run(&h, &[]);
        assert!(
            said(&o, "the server image could not be loaded: timed out"),
            "{o:?}"
        );
        install_unchanged(&h);
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.load.lock().unwrap() = Ok(Digest([0xbb; 32]));
        let o = run(&h, &[]);
        assert_eq!(o.code, 2);
        assert!(
            said(
                &o,
                "the loaded server image is not the one the manifest names"
            ),
            "{o:?}"
        );
        assert!(!said(&o, "server image loaded"), "{o:?}");
        install_unchanged(&h);
    }

    #[test]
    fn dl_08_a_check_and_a_refused_stage_never_reach_the_dump_or_the_load() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 10);
        assert!(
            !events(&h)
                .iter()
                .any(|e| e.starts_with("dump") || e.starts_with("load")),
            "{:?}",
            events(&h)
        );
        let h = staged_harness(RC2, files(RC2), |r| {
            r["assets"][0]["digest"] = json!(DIGEST_A)
        });
        let o = run(&h, &[]);
        assert_eq!(o.code, 2);
        assert!(
            !events(&h)
                .iter()
                .any(|e| e.starts_with("dump") || e.starts_with("load")),
            "{:?}",
            events(&h)
        );
        assert_eq!(gets(&h), 2);
    }
    // ------------------------------------------------------------ the release directory's own files (design §2)

    #[test]
    fn pr_11_the_staged_release_gets_the_verified_manifest_copy_and_a_checksum_list() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let p = engine_prepare(&h).unwrap();
        // the manifest as it was fetched, byte for byte, and it parses and lists this release
        let served = h
            .host
            .routes
            .lock()
            .unwrap()
            .get(MANIFEST_URL)
            .unwrap()
            .1
            .clone();
        let copy = fs::read(p.staging.join("manifest.json")).unwrap();
        assert_eq!(copy, served);
        assert_eq!(mode_of(&p.staging.join("manifest.json")), 0o644);
        // SHA256SUMS: every file of the release but itself, sorted by path, `sha256sum` format
        let sums = fs::read_to_string(p.staging.join("SHA256SUMS")).unwrap();
        assert_eq!(mode_of(&p.staging.join("SHA256SUMS")), 0o644);
        let names: Vec<&str> = sums.lines().map(|l| &l[66..]).collect();
        assert_eq!(
            names,
            [
                "bin/cortex-dump",
                "bin/cortex-dump-alert",
                "bin/cortex-dump-status",
                "bin/cortex-restore",
                "bin/restore-compare",
                "compose.yml",
                "cortex",
                "cortex.toml.example",
                "db-roles.sh",
                "initdb/00-cortex-roles.sh",
                "manifest.json",
                "stack.env",
                "systemd/cortex-dump-alert.service",
                "systemd/cortex-dump-status.service",
                "systemd/cortex-dump-status.timer",
                "systemd/cortex-dump.service",
                "systemd/cortex-dump.timer"
            ]
        );
        assert!(sums.ends_with('\n'));
        for line in sums.lines() {
            let (hex, name) = (&line[..64], &line[66..]);
            assert_eq!(&line[64..66], "  ");
            assert_eq!(
                hex,
                cortex_types::Sha256(
                    sha2::Sha256::digest(fs::read(p.staging.join(name)).unwrap()).into()
                )
                .to_hex(),
                "{name}"
            );
        }
        // and the layout can now tell this release's class from the copy (a rollback needs it)
        p.discard();
    }
    // ------------------------------------------------------------ steps 8 to 11: commit, restart, check, record (the good path)

    fn records(h: &Harness) -> Vec<std::path::PathBuf> {
        let dir = h.tmp.path().join("upgrades");
        let mut v: Vec<_> = fs::read_dir(&dir)
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
            .unwrap_or_default();
        // oldest first: by stamp, then by the -N suffix of a second record in the same second (name order would put -2 first)
        v.sort_by_key(|p| {
            let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
            match stem.rsplit_once('-') {
                Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
                    (base.to_string(), n.parse::<u32>().unwrap())
                }
                _ => (stem, 1),
            }
        });
        v
    }

    #[test]
    fn go_01_a_good_upgrade_switches_the_links_keeps_the_old_release_and_writes_one_record() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let home = h.tmp.path();
        let old_release = tree_hash(&home.join("releases").join(RC1));
        let toml = fs::read(home.join("cortex.toml")).unwrap();
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_safe(&o);
        assert!(said(&o, &format!("upgraded {RC1} -> {RC2}")), "{o:?}");
        let layout = Home::new(home);
        assert_eq!(layout.current().unwrap(), Some(v(RC2)));
        assert_eq!(layout.previous().unwrap(), Some(v(RC1)));
        assert_eq!(
            fs::read_link(home.join("bin/cortex")).unwrap(),
            std::path::Path::new("../current/cortex")
        );
        // the new release directory: the archive's files, the manifest copy and the checksums; no staging, no downloads
        for f in [
            "cortex",
            "compose.yml",
            "stack.env",
            "db-roles.sh",
            "manifest.json",
            "SHA256SUMS",
            "initdb/00-cortex-roles.sh",
        ]
        .iter()
        .copied()
        .chain(LIST2_MEMBERS.iter().copied())
        {
            assert!(home.join("releases").join(RC2).join(f).is_file(), "{f}");
        }
        assert!(!home.join("releases").join(format!("{RC2}.part")).exists());
        assert!(!home.join("downloads").exists());
        // untouched: the old release, the operator's config, the old dump set
        assert_eq!(tree_hash(&home.join("releases").join(RC1)), old_release);
        assert_eq!(fs::read(home.join("cortex.toml")).unwrap(), toml);
        assert!(home.join("dumps/set-1/dump.sql").is_file());
        // exactly one record, typed fields, the dump set path, no secret
        let recs = records(&h);
        assert_eq!(recs.len(), 1, "{recs:?}");
        assert_eq!(mode_of(&recs[0]), 0o600);
        let text = fs::read_to_string(&recs[0]).unwrap();
        let rec: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            (
                rec["from"].as_str(),
                rec["to"].as_str(),
                rec["result"].as_str()
            ),
            (Some(RC1), Some(RC2), Some("done"))
        );
        let set = home.join("dumps/pre-upgrade").join(LABEL);
        assert_eq!(rec["dump_set"].as_str(), Some(&*set.display().to_string()));
        let steps: Vec<(u64, &str, bool)> = rec["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["step"].as_u64().unwrap(),
                    s["name"].as_str().unwrap(),
                    s["ok"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            steps,
            [
                (1, "lock", true),
                (2, "read", true),
                (3, "choose", true),
                (4, "stage", true),
                (5, "unpack", true),
                (6, "dump", true),
                (7, "load", true),
                (8, "commit", true),
                (9, "restart", true),
                (10, "check", true),
                (11, "record", true)
            ]
        );
        assert!(!text.contains(TOKEN), "no token in the record");
        // the operator's token went to the two authenticated reads only
        assert_eq!(
            *h.stack.tokens.lock().unwrap(),
            [TOKEN.to_string(), TOKEN.to_string()]
        );
    }

    #[test]
    fn go_02_nothing_of_the_install_changes_before_the_dump_and_the_load_and_the_order_after_is_fixed()
     {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        let seen = h.seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            [
                format!("dump current={RC1} releases={RC1},{RC2}.part"),
                format!("load current={RC1} releases={RC1},{RC2}.part"),
                format!("compose_up current={RC2} releases={RC1},{RC2}"),
                format!("compose_up current={RC2} releases={RC1},{RC2}"),
            ],
            "the links and the release directories at each call"
        );
        let ev = events(&h);
        assert_eq!(
            ev[7..],
            [
                format!("compose_up db {RC2}"),
                "state db".to_string(),
                "run_roles audit,tokens 60".to_string(),
                format!("compose_up migrate,server {RC2}"),
                "state migrate".to_string(),
                "state server".to_string(),
                "ready".to_string(),
                "whoami".to_string(),
            ],
            "database up and healthy, then the role step, then migrate and the server, then the checks"
        );
    }

    #[test]
    fn go_03_every_wait_is_a_poll_with_the_given_interval_and_stops_when_the_answer_is_good() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        {
            use cortex_upgrade::engine::Ready;
            let mut st = h.containers.states.lock().unwrap();
            st.insert(
                "db",
                vec![
                    ServiceState::Starting,
                    ServiceState::Starting,
                    ServiceState::Healthy,
                ]
                .into(),
            );
            st.insert(
                "migrate",
                vec![ServiceState::Running, ServiceState::Exited(0)].into(),
            );
            st.insert(
                "server",
                vec![ServiceState::Starting, ServiceState::Healthy].into(),
            );
            *h.stack.ready_seq.lock().unwrap() = vec![
                Ready::No("503".into()),
                Ready::No("no answer".into()),
                Ready::Yes,
            ]
            .into();
        }
        let o = run(&h, &["--health-interval", "5", "--roles-timeout", "90"]);
        assert_eq!(o.code, 0, "{o:?}");
        let ev = events(&h);
        let after: Vec<&str> = ev[7..].iter().map(|s| s.as_str()).collect();
        assert_eq!(
            after,
            [
                &format!("compose_up db {RC2}")[..],
                "state db",
                "sleep 5",
                "state db",
                "sleep 5",
                "state db",
                "run_roles audit,tokens 90",
                &format!("compose_up migrate,server {RC2}")[..],
                "state migrate",
                "sleep 5",
                "state migrate",
                "state server",
                "sleep 5",
                "state server",
                "ready",
                "sleep 5",
                "ready",
                "sleep 5",
                "ready",
                "whoami",
            ]
        );
    }

    #[test]
    fn go_04_after_a_good_upgrade_only_current_and_previous_are_kept_and_no_dump_set_is_touched() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let home = h.tmp.path();
        fs::create_dir_all(home.join("releases/0.2.0")).unwrap();
        fs::write(home.join("releases/0.2.0/cortex"), "old").unwrap();
        fs::write(
            home.join("releases/0.2.0.part"),
            "a file named like a staging directory",
        )
        .unwrap();
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(
            said(&o, "removed older releases: 0.2.0"),
            "the removal is reported: {o:?}"
        );
        assert!(
            !home.join("releases/0.2.0").exists(),
            "an older release is pruned"
        );
        assert!(
            home.join("releases/0.2.0.part").is_file(),
            "anything that is not a release directory is left alone"
        );
        for keep in [RC1, RC2] {
            assert!(home.join("releases").join(keep).is_dir(), "{keep}");
        }
        assert!(home.join("dumps/set-1/SHA256SUMS").is_file());
        assert!(
            home.join("dumps/pre-upgrade")
                .join(LABEL)
                .join("SHA256SUMS")
                .is_file()
        );
    }

    #[test]
    fn go_05_a_second_run_after_a_good_upgrade_has_nothing_to_do_and_writes_nothing() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        assert_eq!(run(&h, &[]).code, 0);
        let before = tree_hash(h.tmp.path());
        h.events.lock().unwrap().clear();
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, &format!("up to date ({RC2})")), "{o:?}");
        assert!(
            !events(&h).iter().any(|e| e.starts_with("dump")
                || e.starts_with("load")
                || e.starts_with("compose_up")),
            "{:?}",
            events(&h)
        );
        assert_eq!(records(&h).len(), 1, "no second record");
        assert_eq!(tree_hash(h.tmp.path()), before);
    }
    // ------------------------------------------------------------ the failure path: rollback, exit 3 and 4, and --rollback

    const SENTENCE: &str =
        "restore ONLY with cortex-restore; a plain pg_restore can make revoked tokens valid again";

    fn set_path(h: &Harness) -> String {
        h.tmp
            .path()
            .join("dumps/pre-upgrade")
            .join(LABEL)
            .display()
            .to_string()
    }

    fn record_values(h: &Harness) -> Vec<Value> {
        records(h)
            .iter()
            .map(|p| serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap())
            .collect()
    }

    fn step_flags(rec: &Value) -> Vec<(u64, bool)> {
        rec["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["step"].as_u64().unwrap(), s["ok"].as_bool().unwrap()))
            .collect()
    }

    /// RC1 is running again: current, no previous, the failed release kept for inspection, nothing staged.
    fn assert_rolled_back(h: &Harness) {
        let home = h.tmp.path();
        let layout = Home::new(home);
        assert_eq!(layout.current().unwrap(), Some(v(RC1)));
        assert_eq!(
            layout.previous().unwrap(),
            None,
            "previous is forgotten so that a second rollback cannot flip forward"
        );
        assert!(
            home.join("releases").join(RC2).is_dir(),
            "the failed release stays for inspection"
        );
        assert!(!home.join("releases").join(format!("{RC2}.part")).exists());
        assert!(!home.join("downloads").exists());
        assert!(
            home.join("dumps/pre-upgrade")
                .join(LABEL)
                .join("SHA256SUMS")
                .is_file(),
            "the dump set is never deleted"
        );
    }

    #[test]
    fn rb_01_a_failed_compose_after_the_switch_is_rolled_back_to_the_old_release_exit_3() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.ups.lock().unwrap() = vec![
            Ok(()),
            Err(PortError::Failed(OneLine::new("exit status 1"))),
        ]
        .into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 3, "{o:?}");
        assert_safe(&o);
        assert!(
            said(&o, "step 9 (restart) failed: compose failed: exit status 1"),
            "{o:?}"
        );
        assert!(
            said(&o, &format!("rolled back: {RC1} is running again")),
            "{o:?}"
        );
        assert!(
            said(
                &o,
                &format!("pre-upgrade dump set kept at {}", set_path(&h))
            ),
            "{o:?}"
        );
        assert_rolled_back(&h);
        let ev = events(&h);
        let at = ev
            .iter()
            .position(|e| e == &format!("compose_up migrate,server {RC2}"))
            .unwrap();
        assert_eq!(
            ev[at + 1..],
            [
                format!("compose_up server {RC1}"),
                "state server".to_string(),
                "ready".to_string(),
                "whoami".to_string()
            ],
            "the last compose line is for the OLD release, then its checks"
        );
        let recs = record_values(&h);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["result"].as_str(), Some("rolled_back"));
        assert_eq!(
            step_flags(&recs[0]),
            [
                (1, true),
                (2, true),
                (3, true),
                (4, true),
                (5, true),
                (6, true),
                (7, true),
                (8, true),
                (9, false)
            ]
        );
        assert_eq!(recs[0]["dump_set"].as_str(), Some(&*set_path(&h)));
        assert!(!fs::read_to_string(&records(&h)[0]).unwrap().contains(TOKEN));
    }

    #[test]
    fn rb_02_a_database_that_never_gets_healthy_stops_at_the_timeout_and_never_sleeps_past_it() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        h.containers
            .states
            .lock()
            .unwrap()
            .insert("db", vec![ServiceState::Starting; 50].into());
        let o = run(&h, &["--health-timeout", "3"]);
        assert_eq!(o.code, 3, "{o:?}");
        assert!(
            said(&o, "the database did not become healthy: timed out"),
            "{o:?}"
        );
        let ev = events(&h);
        let at = ev
            .iter()
            .position(|e| e == &format!("compose_up db {RC2}"))
            .unwrap();
        assert_eq!(
            ev[at..at + 6],
            [
                "compose_up db 0.2.1-rc.2",
                "state db",
                "sleep 2",
                "state db",
                "sleep 1",
                "state db"
            ]
            .map(String::from),
            "polls at 0, 2 and 3 seconds: the last sleep is shortened"
        );
        assert_eq!(
            *h.pace.now.lock().unwrap(),
            Duration::from_secs(3),
            "exactly the timeout, not more"
        );
        assert!(
            !ev.iter().any(|e| e.starts_with("run_roles")),
            "no role step on an unhealthy database"
        );
        assert_rolled_back(&h);
    }

    #[test]
    fn rb_03_a_migration_that_fails_is_rolled_back_and_the_server_is_never_waited_for() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        h.containers
            .states
            .lock()
            .unwrap()
            .insert("migrate", vec![ServiceState::Exited(1)].into());
        let o = run(&h, &[]);
        assert_eq!(o.code, 3, "{o:?}");
        assert!(
            said(&o, "the migrations did not finish: exited with status 1"),
            "{o:?}"
        );
        let ev = events(&h);
        let at = ev.iter().position(|e| e == "state migrate").unwrap();
        assert_eq!(
            ev[at + 1],
            format!("compose_up server {RC1}"),
            "straight to the rollback"
        );
        assert_rolled_back(&h);
    }

    #[test]
    fn rb_04_the_role_script_refusing_stops_everything_after_it() {
        // (a timeout is no longer here: it is confirmed by one re-run, cf_01 to cf_05)
        let line = "cortex_x_login must not be a member of the module role cortex_m_records";
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.roles.lock().unwrap() = Err(PortError::Failed(OneLine::new(line)));
        let o = run(&h, &[]);
        assert_eq!(o.code, 3, "{o:?}");
        assert!(said(&o, &format!("the role step refused: {line}")), "{o:?}");
        let ev = events(&h);
        assert!(
            !ev.iter()
                .any(|e| e == &format!("compose_up migrate,server {RC2}")),
            "no migrate or server after the role step"
        );
        assert_rolled_back(&h);
    }

    // ---- cox W1-49 (B): a role step that times out is CONFIRMED by one re-run; a second timeout is "unknown" and is NOT rolled back

    fn role_runs(h: &Harness) -> Vec<String> {
        events(h)
            .into_iter()
            .filter(|e| e.starts_with("run_roles"))
            .collect()
    }

    #[test]
    fn cf_01_a_role_step_that_times_out_once_is_confirmed_by_one_rerun_and_the_upgrade_goes_on() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.roles_seq.lock().unwrap() = vec![Err(PortError::TimedOut), Ok(())].into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_safe(&o);
        assert_eq!(
            role_runs(&h),
            vec!["run_roles audit,tokens 60", "run_roles audit,tokens 45"],
            "the first run under the operator's deadline, the confirmation under a fresh one of exactly the floor"
        );
        assert!(
            said(
                &o,
                "the role step timed out once; a second run confirmed the module roles are applied"
            ),
            "{o:?}"
        );
        assert!(
            events(&h).contains(&format!("compose_up migrate,server {RC2}")),
            "the upgrade went on"
        );
    }

    #[test]
    fn cf_02_two_timeouts_stop_at_unknown_with_a_plain_message_and_no_rollback() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.roles_seq.lock().unwrap() =
            vec![Err(PortError::TimedOut), Err(PortError::TimedOut)].into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        assert_safe(&o);
        assert_eq!(role_runs(&h).len(), 2, "exactly one re-run, never more");
        assert!(said(&o, "the role step timed out twice"), "{o:?}");
        assert!(said(&o, "UNKNOWN"), "{o:?}");
        assert!(said(&o, "nothing was rolled back"), "{o:?}");
        assert!(
            said(&o, "cortex upgrade --rollback"),
            "the way back is named: {o:?}"
        );
        assert!(
            !events(&h)
                .iter()
                .any(|e| e == &format!("compose_up migrate,server {RC2}")),
            "no migrate or server after an unknown role step"
        );
        // NOT rolled back: the install stays on the new release, the old one stays as `previous`, the set is kept
        let layout = Home::new(h.tmp.path());
        assert_eq!(layout.current().unwrap(), Some(v(RC2)));
        assert_eq!(layout.previous().unwrap(), Some(v(RC1)));
        assert!(
            h.tmp
                .path()
                .join("dumps/pre-upgrade")
                .join(LABEL)
                .join("SHA256SUMS")
                .is_file(),
            "the dump set is never deleted"
        );
        assert!(
            said(
                &o,
                &format!("pre-upgrade dump set kept at {}", set_path(&h))
            ),
            "{o:?}"
        );
        let recs = records(&h);
        assert_eq!(recs.len(), 1, "one record");
        let text = fs::read_to_string(&recs[0]).unwrap();
        assert!(
            text.contains("\"result\": \"roles_unknown\"")
                || text.contains("\"result\":\"roles_unknown\""),
            "{text}"
        );
    }

    #[test]
    fn cf_03_a_refusal_on_the_rerun_is_a_refusal_and_is_rolled_back() {
        let line = "cortex_x_login must not be a member of the module role cortex_m_records";
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.roles_seq.lock().unwrap() = vec![
            Err(PortError::TimedOut),
            Err(PortError::Failed(OneLine::new(line))),
        ]
        .into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 3, "{o:?}");
        assert!(said(&o, &format!("the role step refused: {line}")), "{o:?}");
        assert_eq!(role_runs(&h).len(), 2);
        assert_rolled_back(&h);
    }

    #[test]
    fn cf_04_a_refusal_on_the_first_run_is_not_retried() {
        let line = "cortex_x_login must not be a member of the module role cortex_m_records";
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.roles_seq.lock().unwrap() =
            vec![Err(PortError::Failed(OneLine::new(line)))].into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 3, "{o:?}");
        assert_eq!(role_runs(&h).len(), 1, "a refusal is final: no re-run");
        assert_rolled_back(&h);
    }

    #[test]
    fn cf_05_the_rerun_deadline_is_the_floor_whatever_the_operator_asked_for() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.roles_seq.lock().unwrap() = vec![Err(PortError::TimedOut), Ok(())].into();
        let o = run(&h, &["--roles-timeout", "90"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_eq!(
            role_runs(&h),
            vec!["run_roles audit,tokens 90", "run_roles audit,tokens 45"]
        );
    }

    // ---- mike's HIGH on packet 14: an upgrade that did not finish leaves a DURABLE marker, `cortex upgrade` finishes it, `--check` and `--to` refuse, `--rollback` clears it

    fn marker(h: &Harness) -> std::path::PathBuf {
        h.tmp.path().join("upgrade.incomplete")
    }

    /// Run an upgrade whose role step times out twice (the unknown outcome): exit 4, nothing rolled back.
    fn unknown_upgrade() -> Harness {
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.roles_seq.lock().unwrap() =
            vec![Err(PortError::TimedOut), Err(PortError::TimedOut)].into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        h
    }

    #[test]
    fn cf_06_the_unknown_outcome_leaves_a_durable_marker_that_a_fresh_engine_sees() {
        let h = unknown_upgrade();
        assert!(marker(&h).is_file(), "upgrade.incomplete is there");
        let text = fs::read_to_string(marker(&h)).unwrap();
        assert!(
            text.contains(&format!("from={RC1}\n")) && text.contains(&format!("to={RC2}\n")),
            "{text}"
        );
        assert!(text.contains(&format!("set={}\n", set_path(&h))), "{text}");
        assert!(
            text.contains("\nprev=none\n"),
            "the marker the unknown outcome rewrites keeps the previous link the first one recorded: {text}"
        );
        // a FRESH Home over the same directory (a new process) reads it
        let inc = Home::new(h.tmp.path()).incomplete().unwrap().unwrap();
        assert_eq!((inc.from.clone(), inc.to.clone()), (v(RC1), v(RC2)));
        assert!(!inc.why.is_empty() && !inc.why.contains('\n'), "{inc:?}");
        assert!(
            inc.why.contains("roles are unknown"),
            "the reason is the unknown roles, not the generic one: {inc:?}"
        );
        assert_eq!(
            fs::metadata(marker(&h)).unwrap().permissions().mode() & 0o777,
            0o600,
            "the marker is private to its owner"
        );
        assert!(
            !fs::read_dir(h.tmp.path()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("upgrade.incomplete.")),
            "no temporary file is left"
        );
    }

    #[test]
    fn cf_07_a_retry_after_the_unknown_outcome_finishes_the_upgrade_it_is_not_up_to_date() {
        // mike's probe (mike_unknown_retry_probe): two role timeouts, then a retry used to exit 0 "up to date" with no roles run
        let h = unknown_upgrade();
        let before = role_runs(&h).len();
        h.events.lock().unwrap().clear();
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_safe(&o);
        assert!(!said(&o, "up to date"), "{o:?}");
        assert!(
            said(
                &o,
                &format!("finished the interrupted upgrade {RC1} -> {RC2}")
            ),
            "{o:?}"
        );
        assert_eq!(
            role_runs(&h).len(),
            1,
            "the roles ran again (before: {before}): {:?}",
            events(&h)
        );
        let ev = events(&h);
        assert!(
            ev.contains(&format!("compose_up migrate,server {RC2}")),
            "migrate and the server were started: {ev:?}"
        );
        assert!(
            !marker(&h).exists(),
            "the marker is gone once the upgrade is finished"
        );
        let layout = Home::new(h.tmp.path());
        assert_eq!(layout.current().unwrap(), Some(v(RC2)));
        assert_eq!(layout.previous().unwrap(), Some(v(RC1)));
        let recs = record_values(&h);
        assert_eq!(recs.len(), 2, "{recs:?}");
        assert_eq!(recs[0]["result"].as_str(), Some("roles_unknown"));
        assert_eq!(recs[1]["result"].as_str(), Some("done"));
        // the record of the retry is the record of a whole upgrade: eleven steps, all ok (steps 1 to 8 were done by the run that was interrupted)
        assert_eq!(
            step_flags(&recs[1]),
            (1..=11).map(|n| (n, true)).collect::<Vec<_>>()
        );
        // and a THIRD run, now that it is finished, is up to date
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, &format!("up to date ({RC2})")), "{o:?}");
    }

    /// cox W1-56 (B): a RESUMED upgrade rolls forward only. Any non-success keeps the marker, rolls nothing back (roll_back flips links only: the schema
    /// stays, and the new release may have served for hours), exits 4 ("needs the operator"), names the step and the two ways out.
    fn assert_resume_stopped(h: &Harness, o: &Outcome, step: &str) {
        assert_eq!(o.code, 4, "{o:?}");
        assert_safe(o);
        assert!(said(o, step), "{o:?}");
        assert!(said(o, "nothing was rolled back"), "{o:?}");
        assert!(!said(o, "is running again"), "a rollback says so: {o:?}");
        assert!(said(o, "cortex upgrade --rollback"), "{o:?}");
        assert!(said(o, "run cortex upgrade again"), "{o:?}");
        assert!(marker(h).is_file(), "the marker stays");
        let layout = Home::new(h.tmp.path());
        assert_eq!(layout.current().unwrap(), Some(v(RC2)));
        assert_eq!(layout.previous().unwrap(), Some(v(RC1)));
        let recs = record_values(h);
        assert_eq!(
            recs.last().unwrap()["result"].as_str(),
            Some("resume_failed"),
            "{recs:?}"
        );
    }

    #[test]
    fn cf_08_a_retry_whose_role_step_is_refused_keeps_the_marker_and_rolls_nothing_back() {
        let line = "cortex_x_login must not be a member of the module role cortex_m_records";
        let h = unknown_upgrade();
        *h.containers.roles_seq.lock().unwrap() =
            vec![Err(PortError::Failed(OneLine::new(line)))].into();
        let o = run(&h, &[]);
        assert_resume_stopped(&h, &o, "step 9 (restart) failed");
        assert!(said(&o, &format!("the role step refused: {line}")), "{o:?}");
        assert_eq!(
            role_runs(&h).len(),
            3,
            "one more run, a refusal is not retried"
        );
    }

    #[test]
    fn cf_23_a_retry_whose_migrations_fail_keeps_the_marker_and_rolls_nothing_back() {
        let h = unknown_upgrade();
        *h.containers.ups.lock().unwrap() =
            vec![Err(PortError::Failed(OneLine::new("exit status 1")))].into();
        let o = run(&h, &[]);
        assert_resume_stopped(&h, &o, "step 9 (restart) failed");
    }

    #[test]
    fn cf_24_a_retry_whose_server_never_answers_keeps_the_marker_and_rolls_nothing_back() {
        use cortex_upgrade::engine::Ready;
        let h = unknown_upgrade();
        *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        let o = run(&h, &["--health-timeout", "3"]);
        assert_resume_stopped(&h, &o, "step 10 (check) failed");
    }

    #[test]
    fn cf_09_a_retry_that_is_unknown_again_keeps_the_marker_and_does_not_roll_back() {
        let h = unknown_upgrade();
        *h.containers.roles_seq.lock().unwrap() =
            vec![Err(PortError::TimedOut), Err(PortError::TimedOut)].into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "the role step timed out twice"), "{o:?}");
        assert!(marker(&h).is_file(), "the marker stays");
        let layout = Home::new(h.tmp.path());
        assert_eq!(layout.current().unwrap(), Some(v(RC2)));
        assert_eq!(layout.previous().unwrap(), Some(v(RC1)));
        assert_eq!(role_runs(&h).len(), 4, "two runs per attempt, never more");
    }

    #[test]
    fn cf_10_check_and_to_refuse_while_an_upgrade_is_unfinished_and_ask_nothing() {
        let h = unknown_upgrade();
        h.events.lock().unwrap().clear();
        let before = tree_hash(h.tmp.path());
        for list in [vec!["--check"], vec!["--to", RC2], vec!["--to", RC1]] {
            let o = run(&h, &list);
            assert_eq!(o.code, 2, "{list:?}: {o:?}");
            assert_safe(&o);
            assert!(
                said(
                    &o,
                    &format!("an upgrade from {RC1} to {RC2} did not finish")
                ),
                "{list:?}: {o:?}"
            );
            assert!(
                said(&o, "cortex upgrade --rollback"),
                "{list:?}: the way out is named: {o:?}"
            );
        }
        assert!(events(&h).is_empty(), "no port call: {:?}", events(&h));
        assert_eq!(tree_hash(h.tmp.path()), before, "nothing changed");
    }

    #[test]
    fn cf_11_rollback_with_an_unfinished_upgrade_goes_back_checks_the_old_release_and_clears_the_marker()
     {
        let h = unknown_upgrade();
        assert!(
            marker(&h).is_file(),
            "the marker is there before the rollback"
        );
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, &format!("rolled back {RC2} -> {RC1}")), "{o:?}");
        assert_rolled_back(&h);
        assert!(!marker(&h).exists(), "the rollback cleared the marker");
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 10, "a newer release is available again: {o:?}");
    }

    #[test]
    fn cf_12_a_process_lost_after_the_flip_leaves_the_marker_and_the_next_run_finishes() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let h = staged_harness(RC2, files(RC2), |_| {});
        // the engine dies at its first compose call after the switch (the process is lost: the panic stands for it)
        *h.containers.on_up.lock().unwrap() = Some(std::sync::Arc::new(|| panic!("process lost")));
        let died = catch_unwind(AssertUnwindSafe(|| run(&h, &[])));
        assert!(died.is_err(), "the run did not return");
        *h.containers.on_up.lock().unwrap() = None;
        let layout = Home::new(h.tmp.path());
        assert_eq!(layout.current().unwrap(), Some(v(RC2)), "the flip happened");
        assert!(
            marker(&h).is_file(),
            "the marker was written BEFORE step 9 began"
        );
        // the next run does not say "up to date": it finishes the upgrade
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(!said(&o, "up to date"), "{o:?}");
        assert!(
            said(
                &o,
                &format!("finished the interrupted upgrade {RC1} -> {RC2}")
            ),
            "{o:?}"
        );
        assert!(!marker(&h).exists());
    }

    #[test]
    fn cf_13_an_unreadable_marker_is_refused_in_plain_words_and_nothing_is_changed() {
        let bad: [(&str, String); 22] = [
            ("garbage", "not a marker\n".to_string()),
            (
                "an unknown key",
                format!(
                    "from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=0123456789abcdef\nextra=1\n"
                ),
            ),
            (
                "a version that is not SemVer",
                format!("from=0.2.001\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=0123456789abcdef\n"),
            ),
            (
                "a from and a to that are neither current",
                "from=0.2.1-rc.0\nto=0.2.1-rc.3\nset=x\nprev=none\nwhy=y\nid=0123456789abcdef\n"
                    .to_string(),
            ),
            (
                "a missing key",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\n"),
            ),
            (
                "an old four-line marker (no prev)",
                format!("from={RC1}\nto={RC2}\nset=x\nwhy=y\nid=0123456789abcdef\n"),
            ),
            (
                "a prev that is not none or a release",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=latest\nwhy=y\nid=0123456789abcdef\n"),
            ),
            (
                "prev=unknown (the engine never writes it)",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=unknown\nwhy=y\nid=0123456789abcdef\n"),
            ),
            (
                "no final newline",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=yz\nid=0123456789abcdef"),
            ),
            (
                "no final newline, and an id that is valid once its last character is cut (LM-70: the missing newline must be refused as such)",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=0123456789abcdef0"),
            ),
            (
                "a to that is current but a from that is not previous",
                format!(
                    "from=0.2.1-rc.0\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=0123456789abcdef\n"
                ),
            ),
            (
                "a from that is previous but a to that is not current",
                format!(
                    "from={RC1}\nto=0.2.1-rc.3\nset=x\nprev=none\nwhy=y\nid=0123456789abcdef\n"
                ),
            ),
            (
                "an empty set",
                format!("from={RC1}\nto={RC2}\nset=\nprev=none\nwhy=y\nid=0123456789abcdef\n"),
            ),
            (
                "a tab in why",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=a\tb\nid=0123456789abcdef\n"),
            ),
            // W1-62: the marker carries the id of the attempt as its SIXTH line; a marker without a valid id is refused in every mode
            (
                "no id (the five lines of before W1-62)",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=y\n"),
            ),
            (
                "an empty id",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=\n"),
            ),
            (
                "an id of 15 characters",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=0123456789abcde\n"),
            ),
            (
                "an id of 17 characters",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=0123456789abcdef0\n"),
            ),
            (
                "an id with an upper-case letter",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=0123456789abcdeF\n"),
            ),
            (
                "an id that is not hex",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=0123456789abcdeg\n"),
            ),
            (
                "an id before why",
                format!("from={RC1}\nto={RC2}\nset=x\nprev=none\nid=0123456789abcdef\nwhy=y\n"),
            ),
            (
                "the id twice",
                format!(
                    "from={RC1}\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=0123456789abcdef\nid=0123456789abcdef\n"
                ),
            ),
        ];
        for (what, text) in bad {
            let h = staged_harness(RC2, files(RC2), |_| {});
            assert_eq!(run(&h, &[]).code, 0);
            fs::write(marker(&h), text).unwrap();
            h.events.lock().unwrap().clear();
            let before = tree_hash(h.tmp.path());
            for list in [
                vec![],
                vec!["--check"],
                vec!["--rollback"],
                vec!["--to", "0.9.9"],
            ] {
                let o = run(&h, &list);
                assert_eq!(o.code, 2, "{what} {list:?}: {o:?}");
                assert!(
                    said(&o, "the marker of an unfinished upgrade cannot be used"),
                    "{what} {list:?}: {o:?}"
                );
                // cox W1-56 (F1): it names the file and says that removing it is the operator's decision after checking the links
                assert!(
                    said(&o, "upgrade.incomplete")
                        && said(
                            &o,
                            "removing it is your decision after you have checked the links"
                        ),
                    "{what} {list:?}: {o:?}"
                );
            }
            assert!(events(&h).is_empty(), "{what}: no port call");
            assert_eq!(tree_hash(h.tmp.path()), before, "{what}: nothing changed");
        }
    }

    /// The id the marker of the attempt carries: its sixth line, `id=` and 16 lowercase hex characters (W1-62).
    fn marker_id(h: &Harness) -> String {
        let text = fs::read_to_string(marker(h)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 6, "six lines: {text:?}");
        let id = lines[5].strip_prefix("id=").expect("the sixth line is id=");
        assert!(
            id.len() == 16
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "16 lowercase hex characters: {id:?}"
        );
        id.to_string()
    }

    #[test]
    fn cf_37_the_marker_of_an_attempt_carries_its_id_as_the_sixth_line() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let lose: &dyn Fn() = &|| panic!("process lost");
        lost_at(
            &h,
            &cortex_upgrade::engine::Hooks {
                after_marker: Some(lose),
                ..Default::default()
            },
        );
        let text = fs::read_to_string(marker(&h)).unwrap();
        assert!(
            text.starts_with(&format!("from={RC1}\nto={RC2}\nset=")),
            "{text:?}"
        );
        marker_id(&h);
    }

    #[test]
    fn cf_38_the_same_pair_attempted_twice_gets_two_ids() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let lose: &dyn Fn() = &|| panic!("process lost");
        let hooks = cortex_upgrade::engine::Hooks {
            after_marker: Some(lose),
            ..Default::default()
        };
        lost_at(&h, &hooks);
        let first = marker_id(&h);
        // the first attempt never switched: its marker is stale, the second run removes it and starts a NEW attempt, lost at the same place
        lost_at(&h, &hooks);
        let second = marker_id(&h);
        assert_ne!(first, second, "the id is new for every attempt");
    }

    #[test]
    fn cf_39_a_marker_rewritten_for_the_unknown_roles_keeps_the_id_of_its_attempt() {
        // the marker's reason is rewritten when the roles are UNKNOWN: the attempt is the same attempt, so the id stays
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.roles_seq.lock().unwrap() =
            vec![Err(PortError::TimedOut), Err(PortError::TimedOut)].into();
        let seen: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
        let (s2, m2) = (seen.clone(), marker(&h));
        let record: &dyn Fn() = &move || {
            *s2.lock().unwrap() = fs::read_to_string(&m2)
                .ok()
                .and_then(|t| t.lines().nth(5).map(str::to_string));
        };
        let o = run_hooked(
            &h,
            &[],
            &cortex_upgrade::engine::Hooks {
                after_marker: Some(record),
                ..Default::default()
            },
        );
        assert_eq!(o.code, 4, "{o:?}");
        let after = fs::read_to_string(marker(&h)).unwrap();
        let id_after = after.lines().nth(5).unwrap().to_string();
        assert!(id_after.starts_with("id="), "{after:?}");
        assert_eq!(
            seen.lock().unwrap().clone(),
            Some(id_after),
            "the id of the attempt did not change when the reason was rewritten"
        );
    }

    #[test]
    fn cf_14_the_marker_is_written_before_the_first_step_9_call_and_removed_after_the_record_of_done()
     {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<bool>::new()));
        let (s2, m2) = (seen.clone(), marker(&h));
        *h.containers.on_up.lock().unwrap() = Some(std::sync::Arc::new(move || {
            s2.lock().unwrap().push(m2.is_file())
        }));
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        let seen = seen.lock().unwrap().clone();
        assert!(
            !seen.is_empty() && seen.iter().all(|present| *present),
            "at every compose call of step 9 the marker was there: {seen:?}"
        );
        assert!(!marker(&h).exists(), "and it is gone after a good upgrade");
        assert_eq!(record_values(&h).len(), 1);
        assert_eq!(record_values(&h)[0]["result"].as_str(), Some("done"));
    }

    #[test]
    fn cf_15_a_marker_that_cannot_be_written_stops_before_the_switch() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        // a directory where the marker's temporary file goes: the write fails, whatever the permissions
        fs::create_dir(
            h.tmp
                .path()
                .join(format!("upgrade.incomplete.tmp{}", std::process::id())),
        )
        .unwrap();
        let o = run(&h, &[]);
        assert_eq!(o.code, 3, "{o:?}");
        assert_safe(&o);
        // the marker is written AHEAD of the switch: nothing was switched, so there is nothing to roll back
        assert!(said(&o, &format!("the switch to {RC2} failed")), "{o:?}");
        assert!(
            said(
                &o,
                &format!("{RC1} is still current and answered its checks")
            ),
            "{o:?}"
        );
        assert!(!said(&o, "rolled back"), "{o:?}");
        let layout = Home::new(h.tmp.path());
        assert_eq!(layout.current().unwrap(), Some(v(RC1)));
        assert_eq!(layout.previous().unwrap(), None);
        assert!(
            !events(&h).iter().any(|e| e.starts_with("compose_up")),
            "nothing was started: {:?}",
            events(&h)
        );
        assert!(role_runs(&h).is_empty());
        assert!(!marker(&h).exists());
        let recs = record_values(&h);
        assert_eq!(recs.len(), 1, "{recs:?}");
        assert_eq!(recs[0]["result"].as_str(), Some("failed"));
        assert_eq!(step_flags(&recs[0]).last(), Some(&(8, false)));
    }

    #[test]
    fn cf_16_an_interrupted_upgrade_whose_manifest_copy_cannot_be_read_is_refused_and_nothing_is_changed()
     {
        let h = unknown_upgrade();
        fs::remove_file(
            h.tmp
                .path()
                .join("releases")
                .join(RC2)
                .join("manifest.json"),
        )
        .unwrap();
        h.events.lock().unwrap().clear();
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &[]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(
            said(
                &o,
                &format!("the interrupted upgrade {RC1} -> {RC2} cannot be finished")
            ),
            "{o:?}"
        );
        assert!(said(&o, "cortex upgrade --rollback"), "{o:?}");
        assert!(events(&h).is_empty(), "no port call: {:?}", events(&h));
        assert_eq!(tree_hash(h.tmp.path()), before, "nothing changed");
        assert!(marker(&h).is_file(), "the marker stays");
    }

    #[test]
    fn cf_17_the_marker_size_limit_and_a_from_that_equals_to_are_exact() {
        // a valid marker of exactly 4096 bytes is a marker; one byte more cannot be used
        let h = unknown_upgrade();
        let head = format!("from={RC1}\nto={RC2}\nset=");
        let tail = "\nprev=none\nwhy=y\nid=0123456789abcdef\n";
        let fill = 4096 - head.len() - tail.len();
        for (extra, usable) in [(0, true), (1, false)] {
            let text = format!("{head}{}{tail}", "a".repeat(fill + extra));
            assert_eq!(text.len(), 4096 + extra);
            fs::write(marker(&h), &text).unwrap();
            let o = run(&h, &["--check"]);
            assert_eq!(o.code, 2, "{extra}: {o:?}");
            assert_eq!(
                said(&o, "did not finish"),
                usable,
                "{extra}: a marker of {} bytes: {o:?}",
                text.len()
            );
            assert_eq!(
                said(&o, "cannot be used"),
                !usable,
                "{extra}: a marker of {} bytes: {o:?}",
                text.len()
            );
        }
        // from == to is refused by that rule alone (current IS that release, so the links alone would call the marker stale)
        let h = unknown_upgrade();
        fs::write(
            marker(&h),
            format!("from={RC2}\nto={RC2}\nset=x\nprev=none\nwhy=y\nid=0123456789abcdef\n"),
        )
        .unwrap();
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "cannot be used"), "{o:?}");
    }

    #[test]
    fn cf_18_the_marker_writer_refuses_a_set_or_why_that_is_not_one_plain_line_and_replaces_an_older_marker()
     {
        let h = staged_harness(RC2, files(RC2), |_| {});
        assert_eq!(run(&h, &[]).code, 0);
        let home = Home::new(h.tmp.path());
        let mk = |set: &str, why: &str| Incomplete {
            from: v(RC1),
            to: v(RC2),
            set: set.to_string(),
            prev: PrevLink::Absent,
            why: why.to_string(),
            id: "0123456789abcdef".to_string(),
        };
        for (set, why) in [
            ("", "y"),
            ("a\nb", "y"),
            ("a\tb", "y"),
            ("x", ""),
            ("x", "a\nb"),
            ("x", "a\u{7f}b"),
        ] {
            assert!(
                matches!(
                    home.mark_incomplete(&mk(set, why)),
                    Err(LayoutError::BadIncomplete)
                ),
                "{set:?} {why:?}"
            );
            assert!(!marker(&h).exists(), "{set:?} {why:?}: nothing was written");
        }
        // an id that is not 16 lowercase hex characters is refused by the writer too (W1-62): nothing is written
        for bad_id in [
            "",
            "0123456789abcde",
            "0123456789abcdef0",
            "0123456789ABCDEF",
            "0123456789abcdeg",
            "0123456789abcde\n",
        ] {
            let mut m = mk("one", "first");
            m.id = bad_id.to_string();
            assert!(
                matches!(home.mark_incomplete(&m), Err(LayoutError::BadIncomplete)),
                "{bad_id:?}"
            );
            assert!(!marker(&h).exists(), "{bad_id:?}: nothing was written");
        }
        home.mark_incomplete(&mk("one", "first")).unwrap();
        home.mark_incomplete(&mk("two", "second")).unwrap();
        let inc = home.incomplete().unwrap().unwrap();
        assert_eq!((inc.set.as_str(), inc.why.as_str()), ("two", "second"));
    }

    #[test]
    fn cf_19_a_stale_temporary_file_of_a_lost_run_does_not_stop_the_marker() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        fs::write(
            h.tmp
                .path()
                .join(format!("upgrade.incomplete.tmp{}", std::process::id())),
            "left by a run that was lost\n",
        )
        .unwrap();
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(!marker(&h).exists());
        assert!(
            !fs::read_dir(h.tmp.path()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("upgrade.incomplete.")),
            "the stale file is gone"
        );
    }

    #[test]
    fn cf_20_the_retry_reads_the_entry_of_the_release_it_finishes_whatever_else_the_manifest_copy_lists()
     {
        let h = unknown_upgrade();
        let copy = h
            .tmp
            .path()
            .join("releases")
            .join(RC2)
            .join("manifest.json");
        let served: Value = serde_json::from_slice(&fs::read(&copy).unwrap()).unwrap();
        let rc2 = served["releases"][0].clone();
        assert_eq!(rc2["version"].as_str(), Some(RC2));
        // a decoy FIRST: another usable release (the installed one)
        let decoy = real_release(RC1, &archive(&files(RC1)), OCI);
        let changed = manifest(vec![decoy, rc2]);
        fs::write(&copy, &changed).unwrap();
        // R-4a (cox W1-67): the retry verifies the release directory before it restarts anything, and the manifest copy is one of the files
        // `SHA256SUMS` lists: it was changed on purpose here, so its line is changed with it (the rest of the listing is untouched)
        let sums = copy.with_file_name("SHA256SUMS");
        let new_line = format!(
            "{}  manifest.json",
            digest_text(&changed).trim_start_matches("sha256:")
        );
        let text: String = fs::read_to_string(&sums)
            .unwrap()
            .lines()
            .map(|l| {
                if l.ends_with("  manifest.json") {
                    format!("{new_line}\n")
                } else {
                    format!("{l}\n")
                }
            })
            .collect();
        fs::write(&sums, text).unwrap();
        let entry = Home::new(h.tmp.path()).read_release(&v(RC2)).unwrap();
        assert_eq!(entry.version, v(RC2), "the entry of the release itself");
        assert!(
            Home::new(h.tmp.path())
                .read_release(&v("0.2.1-rc.9"))
                .is_none(),
            "a release the copy does not list"
        );
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(
            said(
                &o,
                &format!("finished the interrupted upgrade {RC1} -> {RC2}")
            ),
            "{o:?}"
        );
    }

    #[test]
    fn cf_22_a_marker_left_by_a_run_lost_before_the_switch_is_stale_and_ignored() {
        // the marker is written AHEAD of the switch; a run lost between the two leaves a marker over links that never moved
        let h = staged_harness(RC2, files(RC2), |_| {});
        fs::write(
            marker(&h),
            format!("from={RC1}\nto={RC2}\nset=/gone/set\nprev=none\nwhy=lost before the switch\nid=0123456789abcdef\n"),
        )
        .unwrap();
        assert!(
            Home::new(h.tmp.path()).incomplete().unwrap().is_none(),
            "current is still the release the marker leaves"
        );
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 10, "{o:?}");
        assert!(
            said(&o, &format!("a newer release is available: {RC2}")),
            "{o:?}"
        );
        assert!(!said(&o, "did not finish"), "{o:?}");
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, &format!("upgraded {RC1} -> {RC2}")), "{o:?}");
        assert!(!marker(&h).exists(), "the finished upgrade clears it");
        let layout = Home::new(h.tmp.path());
        assert_eq!(layout.current().unwrap(), Some(v(RC2)));
        assert_eq!(layout.previous().unwrap(), Some(v(RC1)));
    }

    #[test]
    fn cf_25_a_stale_marker_is_removed_durably_with_one_line_by_a_run_that_changes_things() {
        // cox W1-56 (A): the next run removes it, says so in one line, plans normally (here it plans a release that does not exist and refuses)
        let h = staged_harness(RC2, files(RC2), |_| {});
        fs::write(
            marker(&h),
            format!("from={RC1}\nto={RC2}\nset=/gone/set\nprev=none\nwhy=lost before the switch\nid=0123456789abcdef\n"),
        )
        .unwrap();
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 10, "{o:?}");
        assert!(
            marker(&h).is_file(),
            "a check changes nothing: the stale marker stays"
        );
        assert!(!said(&o, "stale marker"), "{o:?}");
        let o = run(&h, &["--to", "0.9.9"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(
            said(
                &o,
                &format!(
                    "removed the stale marker of an upgrade from {RC1} to {RC2} that never switched"
                )
            ),
            "{o:?}"
        );
        assert_eq!(
            o.lines
                .iter()
                .filter(|l| l.contains("stale marker"))
                .count(),
            1,
            "one line: {o:?}"
        );
        assert!(!marker(&h).exists(), "removed");
    }

    /// The process is lost at `point`: the run does not return.
    fn lost_at(h: &Harness, hooks: &cortex_upgrade::engine::Hooks) {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let died = catch_unwind(AssertUnwindSafe(|| run_hooked(h, &[], hooks)));
        assert!(died.is_err(), "the run did not return");
    }

    #[test]
    fn cf_26_a_run_lost_before_the_marker_leaves_nothing_and_the_next_run_upgrades() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let h = staged_harness(RC2, files(RC2), |_| {});
        // the last port call before the marker is the image load (step 7)
        *h.containers.on_load.lock().unwrap() =
            Some(std::sync::Arc::new(|| panic!("process lost")));
        let died = catch_unwind(AssertUnwindSafe(|| run(&h, &[])));
        assert!(died.is_err());
        *h.containers.on_load.lock().unwrap() = None;
        assert!(!marker(&h).exists());
        assert_eq!(Home::new(h.tmp.path()).current().unwrap(), Some(v(RC1)));
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, &format!("upgraded {RC1} -> {RC2}")), "{o:?}");
        assert!(!marker(&h).exists());
    }

    #[test]
    fn cf_27_a_run_lost_after_the_marker_and_before_the_flip_is_stale_and_the_next_run_upgrades() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let lose: &dyn Fn() = &|| panic!("process lost");
        lost_at(
            &h,
            &cortex_upgrade::engine::Hooks {
                after_marker: Some(lose),
                ..Default::default()
            },
        );
        assert!(marker(&h).is_file(), "the marker is there");
        assert_eq!(Home::new(h.tmp.path()).current().unwrap(), Some(v(RC1)));
        let o = run(&h, &[]);
        assert!(said(&o, "removed the stale marker"), "{o:?}");
        assert!(!said(&o, "cannot be used"), "{o:?}");
        assert_eq!(o.code, 0, "{o:?}");
        assert!(!marker(&h).exists());
    }

    #[test]
    fn cf_28_a_run_lost_with_previous_set_and_current_not_moved_is_stale_and_not_refused() {
        use cortex_upgrade::layout::CommitStep;
        let h = staged_harness(RC2, files(RC2), |_| {});
        let lose: &dyn Fn(CommitStep) -> bool = &|s| {
            if s == CommitStep::PreviousSet {
                panic!("process lost")
            }
            true
        };
        lost_at(
            &h,
            &cortex_upgrade::engine::Hooks {
                commit_step: Some(lose),
                ..Default::default()
            },
        );
        let layout = Home::new(h.tmp.path());
        assert_eq!(
            layout.current().unwrap(),
            Some(v(RC1)),
            "current did not move"
        );
        assert!(
            marker(&h).is_file(),
            "the marker was there before the flip began"
        );
        let o = run(&h, &[]);
        assert!(said(&o, "removed the stale marker"), "{o:?}");
        assert!(!said(&o, "cannot be used"), "{o:?}");
        assert!(!marker(&h).exists());
    }

    #[test]
    fn cf_29_a_run_lost_right_after_the_flip_leaves_the_marker_and_the_next_run_finishes() {
        use cortex_upgrade::layout::CommitStep;
        let h = staged_harness(RC2, files(RC2), |_| {});
        let lose: &dyn Fn(CommitStep) -> bool = &|s| {
            if s == CommitStep::CurrentSet {
                panic!("process lost")
            }
            true
        };
        lost_at(
            &h,
            &cortex_upgrade::engine::Hooks {
                commit_step: Some(lose),
                ..Default::default()
            },
        );
        let layout = Home::new(h.tmp.path());
        assert_eq!(layout.current().unwrap(), Some(v(RC2)), "the flip happened");
        assert!(
            marker(&h).is_file(),
            "no instant with current at the new release and no marker"
        );
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(
            said(
                &o,
                &format!("finished the interrupted upgrade {RC1} -> {RC2}")
            ),
            "{o:?}"
        );
        assert!(!marker(&h).exists());
    }

    /// `previous` points at `releases/<v>` (a real directory), as an install that was upgraded before.
    fn with_previous(h: &Harness, v: &str) {
        fs::create_dir_all(h.tmp.path().join("releases").join(v)).unwrap();
        std::os::unix::fs::symlink(format!("releases/{v}"), h.tmp.path().join("previous")).unwrap();
    }

    fn lose_at_previous_set(h: &Harness) {
        use cortex_upgrade::layout::CommitStep;
        let lose: &dyn Fn(CommitStep) -> bool = &|s| {
            if s == CommitStep::PreviousSet {
                panic!("process lost")
            }
            true
        };
        lost_at(
            h,
            &cortex_upgrade::engine::Hooks {
                commit_step: Some(lose),
                ..Default::default()
            },
        );
    }

    #[test]
    fn cf_30_a_run_lost_after_previous_was_set_gets_the_old_previous_back_with_the_stale_marker() {
        // mike's F2 on packet 15 r1: the half-done flip overwrote the rollback target; the marker is the only record of it
        let h = staged_harness(RC2, files(RC2), |_| {});
        with_previous(&h, "0.2.0");
        lose_at_previous_set(&h);
        let layout = Home::new(h.tmp.path());
        assert_eq!(layout.current().unwrap(), Some(v(RC1)));
        assert_eq!(
            layout.previous().unwrap(),
            Some(v(RC1)),
            "the interrupted flip overwrote previous"
        );
        let text = fs::read_to_string(marker(&h)).unwrap();
        assert!(text.contains("prev=0.2.0\n"), "{text}");
        let o = run(&h, &["--to", "0.9.9"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "removed the stale marker"), "{o:?}");
        assert!(said(&o, "is back on 0.2.0"), "{o:?}");
        assert!(!marker(&h).exists());
        assert_eq!(
            layout.previous().unwrap(),
            Some(v("0.2.0")),
            "the rollback target is restored"
        );
    }

    #[test]
    fn cf_31_a_run_lost_after_previous_was_set_removes_a_previous_that_did_not_exist() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        lose_at_previous_set(&h);
        let layout = Home::new(h.tmp.path());
        assert_eq!(layout.previous().unwrap(), Some(v(RC1)));
        let text = fs::read_to_string(marker(&h)).unwrap();
        assert!(text.contains("prev=none\n"), "{text}");
        let o = run(&h, &["--to", "0.9.9"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "removed the stale marker"), "{o:?}");
        assert!(said(&o, "the previous link is removed"), "{o:?}");
        assert!(!marker(&h).exists());
        assert_eq!(layout.previous().unwrap(), None);
    }

    #[test]
    fn cf_32_a_stale_marker_over_any_other_previous_is_refused_and_nothing_is_changed() {
        // cox W1-56 (F2): previous == recorded prev: just remove; previous != recorded prev and previous == from (the interrupted flip): put it back;
        // ANY OTHER pair is refused, marker kept, links unchanged (after a finished rollback previous is forgotten: it is not the interrupted flip's footprint)
        let h = staged_harness(RC2, files(RC2), |_| {});
        assert_eq!(run(&h, &[]).code, 0);
        assert_eq!(run(&h, &["--rollback"]).code, 0);
        assert_rolled_back(&h);
        let stale = format!(
            "from={RC1}\nto={RC2}\nset=/gone/set\nprev=0.2.0\nwhy=left by a clear that failed\nid=0123456789abcdef\n"
        );
        fs::write(marker(&h), &stale).unwrap();
        fs::create_dir_all(h.tmp.path().join("releases").join("0.2.0")).unwrap();
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &["--to", "0.9.9"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "neither what the marker recorded"), "{o:?}");
        assert!(said(&o, "nothing was changed"), "{o:?}");
        assert_eq!(fs::read_to_string(marker(&h)).unwrap(), stale);
        assert_eq!(
            tree_hash(h.tmp.path()),
            before,
            "the links and everything else are as they were"
        );
        // the same stale marker with prev=none over the forgotten previous: previous == recorded prev, so it is only removed
        fs::write(
            marker(&h),
            format!(
                "from={RC1}\nto={RC2}\nset=/gone/set\nprev=none\nwhy=left by a clear that failed\nid=0123456789abcdef\n"
            ),
        )
        .unwrap();
        let o = run(&h, &["--to", "0.9.9"]);
        assert!(said(&o, "removed the stale marker"), "{o:?}");
        assert!(!said(&o, "is back on"), "{o:?}");
        assert!(!marker(&h).exists());
    }

    #[test]
    fn cf_34_a_run_lost_between_the_restore_and_the_removal_only_removes_the_marker() {
        // restore first, remove second: the state after the restore (previous == recorded prev) with the marker still there
        let h = staged_harness(RC2, files(RC2), |_| {});
        with_previous(&h, "0.2.0");
        lose_at_previous_set(&h);
        Home::new(h.tmp.path())
            .restore_previous(Some(&v("0.2.0")))
            .unwrap();
        assert!(marker(&h).is_file());
        let o = run(&h, &["--to", "0.9.9"]);
        assert!(said(&o, "removed the stale marker"), "{o:?}");
        assert!(!said(&o, "is back on"), "{o:?}");
        assert!(!marker(&h).exists());
        assert_eq!(
            Home::new(h.tmp.path()).previous().unwrap(),
            Some(v("0.2.0"))
        );
    }

    #[test]
    fn cf_35_a_genuine_interrupted_rollback_with_a_valid_marker_still_completes() {
        // cox W1-56 (F1): the marker check runs on the state recover leaves; it never obstructs an interrupted rollback, which clears the marker itself
        let h = staged_harness(RC3, files(RC3), |_| {});
        stage(h.tmp.path(), RC2, "expand");
        let home = Home::new(h.tmp.path());
        home.commit(&v(RC2)).unwrap();
        fs::write(
            marker(&h),
            format!(
                "from={RC1}\nto={RC2}\nset=/gone/set\nprev=none\nwhy=the switch did not finish\nid=0123456789abcdef\n"
            ),
        )
        .unwrap();
        // an unfinished upgrade always has its inventory (the plan is written before the flip): an empty one here
        fs::write(
            h.tmp.path().join("members.update"),
            cortex_upgrade::members::serialize(&cortex_upgrade::members::Record {
                from: RC1.to_string(),
                to: RC2.to_string(),
                id: "0123456789abcdef".to_string(),
                retired: Vec::new(),
                entries: Vec::new(),
            }),
        )
        .unwrap();
        assert!(
            home.begin_rollback_with(&mut |s| s != FlipStep::PreviousSet)
                .is_err()
        );
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(
            said(
                &o,
                &format!("completed an interrupted rollback ({RC2} to {RC1})")
            ),
            "{o:?}"
        );
        assert!(!said(&o, "cannot be used"), "{o:?}");
        assert!(
            !said(&o, "stale marker"),
            "the rollback's finish clears it: {o:?}"
        );
        assert!(!marker(&h).exists());
    }

    #[test]
    fn cf_33_a_previous_that_cannot_be_put_back_refuses_the_run_and_keeps_the_marker() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        with_previous(&h, "0.2.0");
        lose_at_previous_set(&h);
        // the release the marker says previous pointed at is gone
        fs::remove_dir_all(h.tmp.path().join("releases").join("0.2.0")).unwrap();
        let before = fs::read(marker(&h)).unwrap();
        let o = run(&h, &["--to", "0.9.9"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "cannot be put back"), "{o:?}");
        assert!(said(&o, "the marker is kept"), "{o:?}");
        assert_eq!(
            fs::read(marker(&h)).unwrap(),
            before,
            "the marker is the only record: unchanged"
        );
    }

    #[test]
    fn cf_36_the_unknown_roles_marker_keeps_the_previous_release_the_first_marker_recorded() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        with_previous(&h, "0.2.0");
        *h.containers.roles_seq.lock().unwrap() =
            vec![Err(PortError::TimedOut), Err(PortError::TimedOut)].into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        let text = fs::read_to_string(marker(&h)).unwrap();
        assert!(text.contains("\nprev=0.2.0\n"), "{text}");
        assert!(text.contains("roles are unknown"), "{text}");
    }

    // ---- R-5 (cox W1-59/60/61/62): the engine plans the ten files outside the release directory before the flip, installs them at step 8 by the per-entry record,
    // restores them INSIDE every rollback (never a warning, exit 4 with everything kept), promotes the record at step 11 and recognises an unfinished upgrade's
    // inventory by from, to AND id

    fn synthetic(dir: &str, name: &str) -> Vec<u8> {
        // the fixture release carries `synthetic <member name>` for each of the ten (the units hold no placeholder)
        format!("synthetic {dir}/{name}\n").into_bytes()
    }

    fn old_text(name: &str) -> Vec<u8> {
        format!("OLD {name}\n").into_bytes()
    }

    /// The ten destinations: (path, the member's directory in the release, name, our mode).
    fn member_paths(h: &Harness) -> Vec<(std::path::PathBuf, &'static str, &'static str, u32)> {
        use cortex_upgrade::members::{BIN_FILES, UNIT_FILES};
        let mut v = Vec::new();
        for n in BIN_FILES {
            v.push((h.tmp.path().join("bin").join(n), "bin", n, 0o755));
        }
        for n in UNIT_FILES {
            v.push((h.env.unit_dir.join(n), "systemd", n, 0o644));
        }
        v
    }

    fn put_files(h: &Harness, text: impl Fn(&str, &str) -> Vec<u8>) {
        fs::create_dir_all(h.tmp.path().join("bin")).unwrap();
        for (p, dir, n, mode) in member_paths(h) {
            fs::write(&p, text(dir, n)).unwrap();
            fs::set_permissions(&p, PermissionsExt::from_mode(mode)).unwrap();
        }
    }

    fn assert_files(h: &Harness, text: impl Fn(&str, &str) -> Vec<u8>) {
        for (p, dir, n, mode) in member_paths(h) {
            assert_eq!(fs::read(&p).unwrap(), text(dir, n), "{n}");
            assert_eq!(
                fs::metadata(&p).unwrap().permissions().mode() & 0o7777,
                mode,
                "{n}"
            );
        }
    }

    fn members_marker(h: &Harness) -> std::path::PathBuf {
        h.tmp.path().join("members.update")
    }

    fn members_last(h: &Harness) -> std::path::PathBuf {
        h.tmp.path().join("members.last")
    }

    /// Every backup file of one attempt, `<member>.prev-<id>`, in the two member directories.
    fn backups_of(h: &Harness, id: &str) -> Vec<String> {
        let mut v = Vec::new();
        for (p, _, _, _) in member_paths(h) {
            let mut b = p.into_os_string();
            b.push(format!(".prev-{id}"));
            if std::path::Path::new(&b).exists() {
                v.push(std::path::PathBuf::from(b).display().to_string());
            }
        }
        v
    }

    fn any_backup(h: &Harness) -> bool {
        [h.tmp.path().join("bin"), h.env.unit_dir.clone()]
            .iter()
            .any(|d| {
                fs::read_dir(d)
                    .unwrap()
                    .flatten()
                    .any(|e| e.file_name().to_string_lossy().contains(".prev-"))
            })
    }

    /// An upgrade run that records the id of its marker (the marker is gone at the end of a good one).
    fn run_with_id(h: &Harness, list: &[&str]) -> (Outcome, String) {
        let seen: std::sync::Arc<std::sync::Mutex<String>> = Default::default();
        let (s2, m2) = (seen.clone(), marker(h));
        let record: &dyn Fn() = &move || {
            if let Ok(t) = fs::read_to_string(&m2) {
                *s2.lock().unwrap() = t.lines().nth(5).unwrap_or("").replace("id=", "");
            }
        };
        let o = run_hooked(
            h,
            list,
            &cortex_upgrade::engine::Hooks {
                after_marker: Some(record),
                ..Default::default()
            },
        );
        let id = seen.lock().unwrap().clone();
        (o, id)
    }

    /// A run lost (a panic) at the member-protocol boundary `point`.
    fn lost_at_member(h: &Harness, point: &str) {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let target = point.to_string();
        let lose: &dyn Fn(&str) = &move |p: &str| {
            if p == target {
                panic!("process lost at {p}");
            }
        };
        let died = catch_unwind(AssertUnwindSafe(|| {
            run_hooked(
                h,
                &[],
                &cortex_upgrade::engine::Hooks {
                    member_point: Some(lose),
                    ..Default::default()
                },
            )
        }));
        assert!(died.is_err(), "the run was lost at {point}");
    }

    fn corrupt(path: &std::path::Path, mode: u32) {
        fs::remove_file(path).unwrap();
        fs::write(path, "not the file that was backed up\n").unwrap();
        fs::set_permissions(path, PermissionsExt::from_mode(mode)).unwrap();
    }

    const RELOAD: &str = "the backup units changed; run: systemctl --user daemon-reload";

    #[test]
    fn rm_01_a_good_upgrade_installs_the_ten_files_promotes_the_record_and_says_what_to_run() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let (o, id) = run_with_id(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_safe(&o);
        assert_files(&h, synthetic);
        assert!(
            !members_marker(&h).exists(),
            "members.update is promoted away at step 11"
        );
        let last = cortex_upgrade::members::read_last(h.tmp.path())
            .unwrap()
            .expect("members.last");
        assert_eq!(
            (last.from.as_str(), last.to.as_str(), last.id.as_str()),
            (RC1, RC2, id.as_str()),
            "members.last carries the id of the attempt"
        );
        assert_eq!(last.entries.len(), 10);
        assert!(
            last.entries
                .iter()
                .all(|e| e.state == cortex_upgrade::members::State::Replaced)
        );
        assert!(said(&o, RELOAD), "{o:?}");
        assert!(said(&o, "new in this release and not enabled"), "{o:?}");
        let recs = record_values(&h);
        assert_eq!(recs[0]["members"].as_array().unwrap().len(), 10, "{recs:?}");
        assert!(!marker(&h).exists());
    }

    #[test]
    fn rm_02_files_that_are_already_ours_are_not_touched_and_nothing_is_said() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, synthetic);
        let (o, id) = run_with_id(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(!said(&o, RELOAD), "{o:?}");
        assert!(!said(&o, "new in this release"), "{o:?}");
        assert!(
            !said(&o, "backup file(s)"),
            "an install that changed nothing says nothing: {o:?}"
        );
        assert!(!any_backup(&h));
        assert!(record_values(&h)[0].get("members").is_none());
        // the record is still promoted: an upgrade that changed nothing is an upgrade with an empty inventory
        let last = cortex_upgrade::members::read_last(h.tmp.path())
            .unwrap()
            .expect("members.last");
        assert_eq!(last.id, id);
        assert!(last.entries.is_empty());
    }

    #[test]
    fn rm_03_a_failed_health_check_restores_the_old_files_inside_the_rollback() {
        use cortex_upgrade::engine::Ready;
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        let o = run(&h, &["--health-timeout", "3"]);
        assert_eq!(o.code, 3, "{o:?}");
        assert_files(&h, |_, n| old_text(n));
        assert!(!members_marker(&h).exists() && !members_last(&h).exists());
        assert!(!any_backup(&h), "the backups go once every entry is back");
        assert!(said(&o, "put back 10 backup file(s)"), "{o:?}");
        assert!(
            !said(&o, "new in this release"),
            "every unit existed already, so none is new: {o:?}"
        );
        // one reload line after the install and ONE MORE after the restore
        assert_eq!(
            o.lines.iter().filter(|l| l.as_str() == RELOAD).count(),
            2,
            "{o:?}"
        );
        assert_rolled_back(&h);
        assert!(!marker(&h).exists());
    }

    #[test]
    fn rm_04_a_unit_directory_that_is_not_ours_to_write_is_refused_before_anything_is_written() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        fs::set_permissions(&h.env.unit_dir, PermissionsExt::from_mode(0o770)).unwrap();
        let before = snapshot(h.tmp.path());
        let o = run(&h, &[]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "writable by group or other"), "{o:?}");
        assert!(said(&o, "nothing was changed"), "{o:?}");
        assert_eq!(Home::new(h.tmp.path()).current().unwrap(), Some(v(RC1)));
        assert_eq!(
            snapshot(h.tmp.path()),
            before,
            "the whole home and unit directory trees are byte-identical: no staging, no dump set, no lock, no marker"
        );
        assert!(events(&h).is_empty(), "nothing was asked of anyone");
    }

    #[test]
    fn rm_05_a_resume_uses_the_plan_a_lost_run_wrote_and_keeps_its_attempt_id() {
        use cortex_upgrade::layout::CommitStep;
        let h = staged_harness(RC2, files(RC2), |_| {});
        let lose: &dyn Fn(CommitStep) -> bool = &|s| {
            if s == CommitStep::CurrentSet {
                panic!("process lost")
            }
            true
        };
        lost_at(
            &h,
            &cortex_upgrade::engine::Hooks {
                commit_step: Some(lose),
                ..Default::default()
            },
        );
        assert!(
            !h.tmp.path().join("bin/cortex-dump-status").exists(),
            "the lost run never installed anything"
        );
        let id = marker_id(&h);
        let planned = cortex_upgrade::members::read_update(h.tmp.path())
            .unwrap()
            .expect("the plan was written before the flip");
        assert_eq!(planned.id, id, "the plan carries the marker's id");
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(
            said(
                &o,
                &format!("finished the interrupted upgrade {RC1} -> {RC2}")
            ),
            "{o:?}"
        );
        assert_files(&h, synthetic);
        assert!(!members_marker(&h).exists());
        let last = cortex_upgrade::members::read_last(h.tmp.path())
            .unwrap()
            .unwrap();
        assert_eq!(last.id, id, "the resume is the same attempt");
    }

    struct Unlock(std::path::PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, PermissionsExt::from_mode(0o700));
        }
    }

    #[test]
    fn rm_06_a_failed_install_after_the_switch_is_restored_from_the_record_and_rolled_back() {
        use cortex_upgrade::layout::CommitStep;
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        let _unlock = Unlock(h.env.unit_dir.clone());
        let units = h.env.unit_dir.clone();
        // between the flip and the install the unit directory stops being writable: the scripts replace, the units fail
        let lock: &dyn Fn(CommitStep) -> bool = &move |s| {
            if s == CommitStep::CurrentSet {
                fs::set_permissions(&units, PermissionsExt::from_mode(0o500)).unwrap();
            }
            true
        };
        let o = run_hooked(
            &h,
            &[],
            &cortex_upgrade::engine::Hooks {
                commit_step: Some(lock),
                ..Default::default()
            },
        );
        fs::set_permissions(&h.env.unit_dir, PermissionsExt::from_mode(0o700)).unwrap();
        assert_eq!(o.code, 3, "{o:?}");
        assert!(said(&o, "could not be installed"), "{o:?}");
        assert_files(&h, |_, n| old_text(n));
        assert!(!members_marker(&h).exists() && !any_backup(&h));
        assert_rolled_back(&h);
    }

    #[test]
    fn rm_07_a_resumed_upgrade_whose_install_fails_goes_nowhere_and_finishes_when_fixed() {
        use cortex_upgrade::layout::CommitStep;
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        let lose: &dyn Fn(CommitStep) -> bool = &|s| {
            if s == CommitStep::CurrentSet {
                panic!("process lost")
            }
            true
        };
        lost_at(
            &h,
            &cortex_upgrade::engine::Hooks {
                commit_step: Some(lose),
                ..Default::default()
            },
        );
        let _unlock = Unlock(h.env.unit_dir.clone());
        fs::set_permissions(&h.env.unit_dir, PermissionsExt::from_mode(0o500)).unwrap();
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "could not be installed"), "{o:?}");
        assert!(said(&o, "nothing was rolled back"), "{o:?}");
        assert!(marker(&h).is_file(), "the upgrade marker stays");
        assert!(members_marker(&h).is_file(), "the record stays");
        fs::set_permissions(&h.env.unit_dir, PermissionsExt::from_mode(0o700)).unwrap();
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_files(&h, synthetic);
        assert!(!marker(&h).exists() && !members_marker(&h).exists());
    }

    #[test]
    fn rm_08_a_home_the_units_cannot_carry_is_refused_before_anything_is_written() {
        // the home is written into ExecStart lines: a name with a space would split them
        let h = staged_harness_in("a space", RC2, files(RC2), |_| {});
        let before = snapshot(h.tmp.path());
        let o = run(&h, &[]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "holds a character other than"), "{o:?}");
        assert!(said(&o, "nothing was changed"), "{o:?}");
        assert_eq!(Home::new(h.tmp.path()).current().unwrap(), Some(v(RC1)));
        assert_eq!(snapshot(h.tmp.path()), before, "byte-identical");
        assert!(events(&h).is_empty(), "nothing was asked of anyone");
    }

    #[test]
    fn rm_09_a_rollback_of_a_finished_upgrade_restores_from_members_last_bound_to_the_release_left()
    {
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_files(&h, synthetic);
        assert!(members_last(&h).is_file());
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, &format!("rolled back {RC2} -> {RC1}")), "{o:?}");
        assert_files(&h, |_, n| old_text(n));
        assert!(said(&o, "put back 10 backup file(s)"), "{o:?}");
        assert!(said(&o, RELOAD), "{o:?}");
        assert!(!any_backup(&h), "every backup is gone once all is back");
        assert!(
            !members_last(&h).exists(),
            "the record of a rolled-back upgrade is forgotten"
        );
    }

    #[test]
    fn rm_10_a_rollback_of_an_upgrade_that_changed_no_backup_file_says_nothing_about_them() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, synthetic);
        assert_eq!(run(&h, &[]).code, 0);
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(!said(&o, "backup file(s)"), "{o:?}");
        assert!(!said(&o, RELOAD), "{o:?}");
        assert_files(&h, synthetic);
        assert!(!members_last(&h).exists());
    }

    /// An upgrade whose roles end UNKNOWN (exit 4, the marker and the live record stay), over files that are all OLD.
    fn unknown_over_old_files() -> Harness {
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        *h.containers.roles_seq.lock().unwrap() =
            vec![Err(PortError::TimedOut), Err(PortError::TimedOut)].into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        assert_files(&h, synthetic);
        assert!(members_marker(&h).is_file() && marker(&h).is_file());
        h
    }

    #[test]
    fn rm_11_a_rollback_after_unknown_roles_puts_the_files_back_from_the_live_record() {
        // mike's F2: the explicit --rollback used to exit 0 with the old current and the NEW scripts and units still installed, members.update left behind
        let h = unknown_over_old_files();
        let id = marker_id(&h);
        assert_eq!(backups_of(&h, &id).len(), 10);
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_files(&h, |_, n| old_text(n));
        assert!(said(&o, "put back 10 backup file(s)"), "{o:?}");
        assert!(
            !members_marker(&h).exists() && !members_last(&h).exists() && !any_backup(&h),
            "the record and the backups go once every entry is verified"
        );
        assert!(!marker(&h).exists());
        assert_eq!(
            Home::new(h.tmp.path()).current().unwrap(),
            Some(v(RC1)),
            "the old release is current"
        );
    }

    #[test]
    fn rm_12_a_rollback_that_cannot_put_one_entry_back_is_exit_4_with_everything_kept_and_resumes()
    {
        let h = unknown_over_old_files();
        let id = marker_id(&h);
        let victim = h
            .env
            .unit_dir
            .join("cortex-dump.timer.prev-".to_string() + &id);
        let good = fs::read(&victim).unwrap();
        corrupt(&victim, 0o644);
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "cortex-dump.timer"), "names the entry: {o:?}");
        assert!(
            said(&o, &format!("cortex-dump.timer.prev-{id}")),
            "names the file: {o:?}"
        );
        assert!(
            !o.lines
                .iter()
                .any(|l| l.to_lowercase().contains("rolled back")),
            "no line says rolled back: {o:?}"
        );
        assert!(marker(&h).is_file(), "the upgrade marker is kept");
        assert!(members_marker(&h).is_file(), "the record is kept");
        assert_eq!(backups_of(&h, &id).len(), 10, "every backup is kept");
        // the cause goes: the next run RESUMES the rest of the rollback and finishes it
        fs::remove_file(&victim).unwrap();
        fs::write(&victim, &good).unwrap();
        fs::set_permissions(&victim, PermissionsExt::from_mode(0o644)).unwrap();
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_files(&h, |_, n| old_text(n));
        assert!(!members_marker(&h).exists() && !any_backup(&h) && !marker(&h).exists());
    }

    #[test]
    fn rm_13_a_lost_or_unusable_inventory_stops_a_rollback_before_anything_changes_exit_4() {
        type Spoil = fn(&Harness);
        let cases: [(&str, Spoil); 5] = [
            ("members.update is gone", |h| {
                fs::remove_file(members_marker(h)).unwrap();
            }),
            ("members.update is garbage", |h| {
                fs::write(members_marker(h), "garbage\n").unwrap();
            }),
            ("members.update is of another attempt", |h| {
                let t = fs::read_to_string(members_marker(h)).unwrap();
                let id = t.lines().nth(3).unwrap().to_string();
                fs::write(members_marker(h), t.replace(&id, "id=ffffffffffffffff")).unwrap();
            }),
            (
                "members.last of the same pair and another id with the live record gone",
                |h| {
                    let t = fs::read_to_string(members_marker(h)).unwrap();
                    let id = t.lines().nth(3).unwrap().to_string();
                    fs::write(members_last(h), t.replace(&id, "id=ffffffffffffffff")).unwrap();
                    fs::remove_file(members_marker(h)).unwrap();
                },
            ),
            (
                "members.last of another pair with the live record gone",
                |h| {
                    let t = fs::read_to_string(members_marker(h)).unwrap();
                    fs::write(
                        members_last(h),
                        t.replace(&format!("from={RC1}"), "from=0.2.0"),
                    )
                    .unwrap();
                    fs::remove_file(members_marker(h)).unwrap();
                },
            ),
        ];
        for (what, spoil) in cases {
            let h = unknown_over_old_files();
            spoil(&h);
            let before = tree_hash(h.tmp.path());
            h.events.lock().unwrap().clear();
            let o = run(&h, &["--rollback"]);
            assert_eq!(o.code, 4, "{what}: {o:?}");
            assert!(said(&o, "nothing was changed"), "{what}: {o:?}");
            assert!(
                !o.lines
                    .iter()
                    .any(|l| l.to_lowercase().contains("rolled back")),
                "{what}: {o:?}"
            );
            assert!(events(&h).is_empty(), "{what}: no port call");
            assert_eq!(tree_hash(h.tmp.path()), before, "{what}: nothing changed");
            assert_eq!(
                Home::new(h.tmp.path()).current().unwrap(),
                Some(v(RC2)),
                "{what}: the links did not move"
            );
        }
    }

    #[test]
    fn rm_14_a_resume_whose_inventory_is_lost_goes_nowhere_exit_4() {
        let h = unknown_over_old_files();
        fs::remove_file(members_marker(&h)).unwrap();
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "lost"), "{o:?}");
        assert!(said(&o, "nothing was rolled back"), "{o:?}");
        assert!(marker(&h).is_file(), "the marker stays");
    }

    #[test]
    fn rm_15_a_rollback_of_a_finished_upgrade_refuses_a_members_last_of_another_release() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        assert_eq!(run(&h, &[]).code, 0);
        let t = fs::read_to_string(members_last(&h)).unwrap();
        fs::write(
            members_last(&h),
            t.replace(&format!("to={RC2}"), "to=0.2.1-rc.9"),
        )
        .unwrap();
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "members.last"), "{o:?}");
        assert_eq!(tree_hash(h.tmp.path()), before, "nothing changed");
        assert_eq!(Home::new(h.tmp.path()).current().unwrap(), Some(v(RC2)));
    }

    #[test]
    fn rm_16_the_promotion_lost_at_any_of_its_steps_is_finished_by_a_resume() {
        for p in ["promote:S1", "promote:S2", "promote:S3"] {
            let h = staged_harness(RC2, files(RC2), |_| {});
            put_files(&h, |_, n| old_text(n));
            lost_at_member(&h, p);
            assert!(marker(&h).is_file(), "{p}: the marker stands");
            let id = marker_id(&h);
            let o = run(&h, &[]);
            assert_eq!(o.code, 0, "{p}: {o:?}");
            assert!(
                said(
                    &o,
                    &format!("finished the interrupted upgrade {RC1} -> {RC2}")
                ),
                "{p}: {o:?}"
            );
            assert_files(&h, synthetic);
            let last = cortex_upgrade::members::read_last(h.tmp.path())
                .unwrap()
                .unwrap();
            assert_eq!(last.id, id, "{p}: the same attempt");
            assert!(!members_marker(&h).exists() && !marker(&h).exists(), "{p}");
            assert_eq!(
                backups_of(&h, &id).len(),
                10,
                "{p}: the backups of the current record stay"
            );
        }
    }

    #[test]
    fn rm_17_a_rollback_after_a_promotion_lost_at_any_step_restores_by_the_record_that_holds_the_inventory()
     {
        for p in ["promote:S1", "promote:S2", "promote:S3"] {
            let h = staged_harness(RC2, files(RC2), |_| {});
            put_files(&h, |_, n| old_text(n));
            lost_at_member(&h, p);
            let o = run(&h, &["--rollback"]);
            assert_eq!(o.code, 0, "{p}: {o:?}");
            assert_files(&h, |_, n| old_text(n));
            assert!(said(&o, "put back 10 backup file(s)"), "{p}: {o:?}");
            assert!(
                !members_marker(&h).exists() && !members_last(&h).exists() && !any_backup(&h),
                "{p}"
            );
            assert!(!marker(&h).exists(), "{p}");
        }
    }

    #[test]
    fn rm_18_a_run_lost_after_the_plan_and_before_the_flip_leaves_a_stale_marker_and_a_plan_that_the_next_run_discards()
     {
        let h = staged_harness(RC2, files(RC2), |_| {});
        lost_at_member(&h, "plan:written");
        assert!(marker(&h).is_file() && members_marker(&h).is_file());
        assert_eq!(Home::new(h.tmp.path()).current().unwrap(), Some(v(RC1)));
        let first = marker_id(&h);
        let (o, second) = run_with_id(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, "removed the stale marker"), "{o:?}");
        assert_ne!(first, second, "a new attempt, a new id");
        let last = cortex_upgrade::members::read_last(h.tmp.path())
            .unwrap()
            .unwrap();
        assert_eq!(last.id, second);
        assert_files(&h, synthetic);
    }

    #[test]
    fn rm_28_a_stale_marker_takes_its_plan_with_it_whatever_the_next_command_is() {
        // lost after the plan and before the flip; the next command is a --rollback (refused: nothing to go back to), which still removes the stale marker
        // and the plan of the attempt that never switched
        let h = staged_harness(RC2, files(RC2), |_| {});
        lost_at_member(&h, "plan:written");
        assert!(marker(&h).is_file() && members_marker(&h).is_file());
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "removed the stale marker"), "{o:?}");
        assert!(!marker(&h).exists(), "the stale marker went");
        assert!(!members_marker(&h).exists(), "and its plan with it");
    }

    #[test]
    fn rm_19_members_last_of_an_earlier_upgrade_is_never_deleted_when_the_next_one_starts() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let earlier = cortex_upgrade::members::Record {
            from: "0.2.0".to_string(),
            to: RC1.to_string(),
            id: "1111111111111111".to_string(),
            retired: Vec::new(),
            entries: Vec::new(),
        };
        fs::write(
            members_last(&h),
            cortex_upgrade::members::serialize(&earlier),
        )
        .unwrap();
        let bytes = fs::read(members_last(&h)).unwrap();
        lost_at_member(&h, "plan:written");
        assert_eq!(
            fs::read(members_last(&h)).unwrap(),
            bytes,
            "untouched by the start"
        );
        let (o, id) = run_with_id(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        let last = cortex_upgrade::members::read_last(h.tmp.path())
            .unwrap()
            .unwrap();
        assert_eq!(last.id, id, "replaced only when the new upgrade completed");
    }

    #[test]
    fn rm_20_the_sweep_at_the_end_removes_only_what_the_new_members_last_retired_and_keeps_the_rest()
     {
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        let bin = h.tmp.path().join("bin");
        // a TRUSTED members.last of an earlier upgrade (id P1, bound to the release being left) that has itself retired R1
        let earlier = cortex_upgrade::members::Record {
            from: "0.2.0".to_string(),
            to: RC1.to_string(),
            id: "1111111111111111".to_string(),
            retired: vec!["3333333333333333".to_string()],
            entries: Vec::new(),
        };
        fs::write(
            members_last(&h),
            cortex_upgrade::members::serialize(&earlier),
        )
        .unwrap();
        let retired_by_promotion = bin.join("cortex-dump.prev-1111111111111111");
        let retired_before = h
            .env
            .unit_dir
            .join("cortex-dump.timer.prev-3333333333333333");
        let unnamed = bin.join("cortex-restore.prev-2222222222222222");
        let foreign = bin.join("cortex-dump.prev-1111111111111111.bak");
        for p in [&retired_by_promotion, &retired_before, &unnamed, &foreign] {
            fs::write(p, "x").unwrap();
        }
        let (o, id) = run_with_id(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(
            !retired_by_promotion.exists() && !retired_before.exists(),
            "what a trusted record names as retired goes (the previous attempt, and the one it had retired)"
        );
        assert!(
            unnamed.exists(),
            "a backup that no trusted record names is NEVER deleted: unknown means keep"
        );
        assert!(
            foreign.exists(),
            "a name that is only almost a backup name is left alone"
        );
        assert_eq!(
            backups_of(&h, &id).len(),
            10,
            "this attempt's backups stay for a later --rollback"
        );
        let last = cortex_upgrade::members::read_last(h.tmp.path())
            .unwrap()
            .unwrap();
        assert_eq!(
            last.retired,
            vec![
                "3333333333333333".to_string(),
                "1111111111111111".to_string()
            ]
        );
    }

    #[test]
    fn rm_22_a_run_lost_in_the_middle_of_the_sweep_has_finished_the_upgrade_and_the_next_upgrade_finishes_the_sweep()
     {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        // mike's boundary: an interruption while the retired generation's backups are being removed (after the marker was cleared)
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        let bin = h.tmp.path().join("bin");
        let earlier = cortex_upgrade::members::Record {
            from: "0.2.0".to_string(),
            to: RC1.to_string(),
            id: "1111111111111111".to_string(),
            retired: Vec::new(),
            entries: Vec::new(),
        };
        fs::write(
            members_last(&h),
            cortex_upgrade::members::serialize(&earlier),
        )
        .unwrap();
        let stale = ["cortex-dump-status", "cortex-restore", "cortex-dump-alert"];
        // the earlier attempt's backups appear just before the end of the upgrade (the sweep at its START found none to remove) and the process is lost after the first removal
        let b2 = bin.clone();
        let hook: &dyn Fn(&str) = &move |p: &str| {
            if p == "promote:S3" {
                for n in stale {
                    fs::write(b2.join(format!("{n}.prev-1111111111111111")), "x").unwrap();
                }
            }
            if p.starts_with("sweep:removed:") && p.ends_with("1111111111111111") {
                panic!("process lost at {p}");
            }
        };
        let died = catch_unwind(AssertUnwindSafe(|| {
            run_hooked(
                &h,
                &[],
                &cortex_upgrade::engine::Hooks {
                    member_point: Some(hook),
                    ..Default::default()
                },
            )
        }));
        assert!(died.is_err(), "the run was lost in the sweep");
        // the upgrade was complete before the sweep started: no marker, the record promoted (naming the earlier attempt as retired), this attempt's backups kept
        assert!(!marker(&h).exists() && !members_marker(&h).exists());
        let last = cortex_upgrade::members::read_last(h.tmp.path())
            .unwrap()
            .expect("members.last");
        assert_eq!(last.retired, vec!["1111111111111111".to_string()]);
        assert_eq!(backups_of(&h, &last.id).len(), 10);
        let left = |bin: &std::path::Path| {
            stale
                .iter()
                .filter(|n| bin.join(format!("{n}.prev-1111111111111111")).exists())
                .count()
        };
        assert_eq!(left(&bin), 2, "one was removed before the loss");
        // nothing is left to upgrade: the next run only says so and members.last is untouched
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, "up to date"), "{o:?}");
        assert_eq!(
            cortex_upgrade::members::read_last(h.tmp.path())
                .unwrap()
                .unwrap(),
            last,
            "members.last was not touched"
        );
        // the NEXT upgrade finishes the sweep under the SAME trusted record, and the attempt that just became current keeps its backups until it is retired in turn
        serve_next(&h, RC3);
        let (o, id3) = run_with_id(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_eq!(
            left(&bin),
            0,
            "the rest of the retired attempt's backups went at the start of the next upgrade"
        );
        assert_eq!(
            backups_of(&h, &id3).len(),
            10,
            "the new attempt's backups are kept"
        );
        assert!(
            backups_of(&h, &last.id).is_empty(),
            "the attempt the new record retired is swept at its end"
        );
    }

    #[test]
    fn rm_23_a_rollback_resumed_after_a_failed_restore_stops_when_the_inventory_has_been_lost_meanwhile()
     {
        let h = unknown_over_old_files();
        let id = marker_id(&h);
        corrupt(
            &h.env.unit_dir.join(format!("cortex-dump.timer.prev-{id}")),
            0o644,
        );
        assert_eq!(run(&h, &["--rollback"]).code, 4, "the first attempt fails");
        // the rollback is started (its links are flipped); the live record is lost before the retry
        fs::remove_file(members_marker(&h)).unwrap();
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "lost"), "{o:?}");
        assert!(marker(&h).is_file(), "the marker is kept");
        assert!(
            !o.lines
                .iter()
                .any(|l| l.to_lowercase().contains("rolled back:")),
            "{o:?}"
        );
    }

    #[test]
    fn rm_24_a_rollback_counts_the_entries_it_put_back_and_says_to_reload_only_when_a_unit_was_among_them()
     {
        // three scripts differ from the release (the others already are the release's): three entries, no unit among them
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, synthetic);
        for n in ["cortex-dump-status", "cortex-restore", "restore-compare"] {
            let p = h.tmp.path().join("bin").join(n);
            fs::write(&p, old_text(n)).unwrap();
            fs::set_permissions(&p, PermissionsExt::from_mode(0o755)).unwrap();
        }
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(!said(&o, RELOAD), "no unit changed: {o:?}");
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, "put back 3 backup file(s)"), "{o:?}");
        assert!(!said(&o, RELOAD), "no unit among them: {o:?}");
        // one unit among them: the reload line
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, synthetic);
        let p = h.env.unit_dir.join("cortex-dump.timer");
        fs::write(&p, old_text("cortex-dump.timer")).unwrap();
        fs::set_permissions(&p, PermissionsExt::from_mode(0o644)).unwrap();
        assert_eq!(run(&h, &[]).code, 0);
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, "put back 1 backup file(s)"), "{o:?}");
        assert!(said(&o, RELOAD), "{o:?}");
    }

    #[test]
    fn rm_25_the_start_of_an_upgrade_sweeps_only_the_backups_a_trusted_record_retired() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let bin = h.tmp.path().join("bin");
        let earlier = cortex_upgrade::members::Record {
            from: "0.2.0".to_string(),
            to: RC1.to_string(),
            id: "1111111111111111".to_string(),
            retired: vec!["2222222222222222".to_string()],
            entries: Vec::new(),
        };
        fs::write(
            members_last(&h),
            cortex_upgrade::members::serialize(&earlier),
        )
        .unwrap();
        let keep = bin.join("cortex-dump.prev-1111111111111111");
        let retired = bin.join("cortex-dump.prev-2222222222222222");
        let unnamed = bin.join("cortex-dump.prev-4444444444444444");
        fs::write(&keep, "kept: the current members.last's").unwrap();
        fs::write(&retired, "retired by the trusted record").unwrap();
        fs::write(&unnamed, "named by nobody").unwrap();
        lost_at_member(&h, "plan:written");
        assert!(keep.exists(), "the backups of members.last's attempt stay");
        assert!(
            !retired.exists(),
            "what the trusted record retired goes at the START of an upgrade"
        );
        assert!(
            unnamed.exists(),
            "a backup that no trusted record names stays: unknown means keep"
        );
    }

    const RC4: &str = "0.2.1-rc.4";

    /// Serve one more release on the same harness: the next upgrade of a chain.
    fn serve_next(h: &Harness, version: &str) {
        // the ten members carry the version in their text, so this upgrade REPLACES them (and so makes backups): the fixture's default text is the same in every release
        let mut f = files(version);
        for (name, data, _) in f.iter_mut() {
            if LIST2_MEMBERS.contains(&name.as_str()) {
                *data = format!("synthetic {name} for {version}\n").into_bytes();
            }
        }
        let tgz = archive(&f);
        let rel = real_release(version, &tgz, OCI);
        let mut routes = h.host.routes.lock().unwrap();
        routes.insert(MANIFEST_URL.into(), (200, manifest(vec![rel])));
        routes.insert(
            format!("{BASE}cortex-{version}-x86_64-linux.tar.gz"),
            (200, tgz),
        );
        routes.insert(
            format!("{BASE}cortex-server-{version}-x86_64.oci.tar"),
            (200, OCI.to_vec()),
        );
    }

    /// Every path under `root` with its kind, mode and content hash (or link target): two snapshots are equal when the tree is byte for byte what it was.
    fn snapshot(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
        fn walk(
            dir: &std::path::Path,
            root: &std::path::Path,
            out: &mut std::collections::BTreeMap<String, String>,
        ) {
            for e in fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(root).unwrap().display().to_string();
                let md = fs::symlink_metadata(&p).unwrap();
                let mode = md.permissions().mode() & 0o7777;
                if md.file_type().is_symlink() {
                    out.insert(rel, format!("l {}", fs::read_link(&p).unwrap().display()));
                } else if md.file_type().is_dir() {
                    out.insert(rel, format!("d {mode:o}"));
                    walk(&p, root, out);
                } else {
                    out.insert(
                        rel,
                        format!("f {mode:o} {}", sha_hex(&fs::read(&p).unwrap())),
                    );
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    fn kept_warnings(o: &Outcome) -> usize {
        o.lines.iter().filter(|l| l.contains("were KEPT")).count()
    }

    #[test]
    fn rm_29_an_inventory_that_cannot_be_trusted_at_the_start_of_an_upgrade_never_makes_the_sweep_delete_anything()
     {
        // mike's F1 on packet 17: an RC1 -> RC2 upgrade with 10 backups, members.last spoilt, RC2 -> RC3: all ten backups vanished and the run returned 0
        type Spoil = fn(&std::path::Path);
        let forms: [(&str, Spoil); 4] = [
            ("unreadable by mode", |p| {
                fs::set_permissions(p, PermissionsExt::from_mode(0o000)).unwrap()
            }),
            ("corrupt", |p| fs::write(p, "garbage\n").unwrap()),
            ("well-formed lines with an invalid id", |p| {
                fs::write(
                    p,
                    format!("members-record 1\nfrom={RC1}\nto={RC2}\nid=NOT-AN-ID\n"),
                )
                .unwrap()
            }),
            ("absent while ten recognised backups exist", |p| {
                fs::remove_file(p).unwrap()
            }),
        ];
        for (n, (label, spoil)) in forms.iter().enumerate() {
            let h = staged_harness_in(&format!("rm29-{n}"), RC2, files(RC2), |_| {});
            put_files(&h, |_, n| old_text(n));
            let (o, id1) = run_with_id(&h, &[]);
            assert_eq!(o.code, 0, "{label}: {o:?}");
            assert_eq!(backups_of(&h, &id1).len(), 10, "{label}");
            spoil(&members_last(&h));
            serve_next(&h, RC3);
            let (o, id2) = run_with_id(&h, &[]);
            assert_eq!(
                o.code, 0,
                "{label}: the result of the upgrade is unchanged: {o:?}"
            );
            assert!(
                said(&o, &format!("upgraded {RC2} -> {RC3}")),
                "{label}: {o:?}"
            );
            assert_eq!(backups_of(&h, &id1).len(), 10, "{label}: ZERO deletions");
            assert_eq!(
                kept_warnings(&o),
                1,
                "{label}: ONE plain warning says the old backups were kept: {o:?}"
            );
            let last = cortex_upgrade::members::read_last(h.tmp.path())
                .unwrap()
                .unwrap();
            assert_eq!(last.id, id2, "{label}");
            assert!(
                last.retired.is_empty(),
                "{label}: nothing was named, so nothing will ever be swept"
            );
            assert_eq!(backups_of(&h, &id2).len(), 10, "{label}");
        }
    }

    #[test]
    fn rm_30_a_unit_directory_that_is_not_ours_is_refused_before_the_sweep_and_the_recognised_backup_is_intact()
     {
        // mike's F2 on packet 17: a group-writable unit directory and a recognised backup: the file was deleted, then the run said the switch failed and nothing was started
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        let victim = h
            .env
            .unit_dir
            .join("cortex-dump.service.prev-0123456789abcdef");
        fs::write(&victim, "a recognised backup").unwrap();
        fs::set_permissions(&h.env.unit_dir, PermissionsExt::from_mode(0o775)).unwrap();
        let before = snapshot(h.tmp.path());
        let o = run(&h, &[]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "writable by group or other"), "{o:?}");
        assert!(said(&o, "nothing was changed"), "{o:?}");
        assert_eq!(
            fs::read(&victim).unwrap(),
            b"a recognised backup",
            "the file is intact"
        );
        assert_eq!(
            snapshot(h.tmp.path()),
            before,
            "the whole tree is byte-identical"
        );
    }

    #[test]
    fn rm_31_the_old_backups_survive_a_skipped_sweep_a_promotion_a_crash_a_restart_and_the_next_sweep()
     {
        // cox W1-75 (3), mike's end-to-end cell: RC1 -> RC2 backups, a corrupt AND an absent members.last, RC2 -> RC3 promotion, a crash and a restart, the next sweep
        for absent in [false, true] {
            let h = staged_harness_in(&format!("rm31-{absent}"), RC2, files(RC2), |_| {});
            put_files(&h, |_, n| old_text(n));
            let (o, id1) = run_with_id(&h, &[]);
            assert_eq!(o.code, 0, "{o:?}");
            if absent {
                fs::remove_file(members_last(&h)).unwrap();
            } else {
                fs::write(members_last(&h), "garbage\n").unwrap();
            }
            serve_next(&h, RC3);
            // RC2 -> RC3: the promotion is done and the process is lost right after it
            lost_at_member(&h, "promote:S3");
            let (o, id2) = run_with_id(&h, &[]);
            assert_eq!(o.code, 0, "the restart finishes the upgrade: {o:?}");
            assert_eq!(
                backups_of(&h, &id1).len(),
                10,
                "absent={absent}: the ORIGINAL backups survive the promotion and the restart"
            );
            let last = cortex_upgrade::members::read_last(h.tmp.path())
                .unwrap()
                .unwrap();
            assert!(
                last.retired.is_empty(),
                "absent={absent}: the new record names nobody"
            );
            // the next sweep (RC3 -> RC4): the previous attempt is retired by a trusted promotion, the ORIGINAL backups were never named and stay
            serve_next(&h, RC4);
            let (o, id3) = run_with_id(&h, &[]);
            assert_eq!(o.code, 0, "{o:?}");
            let _ = id2;
            assert_eq!(
                backups_of(&h, &id1).len(),
                10,
                "absent={absent}: the original backups survive until explicitly justified"
            );
            assert_eq!(backups_of(&h, &id3).len(), 10, "absent={absent}");
        }
    }

    #[test]
    fn rm_32_with_the_marker_present_the_attempts_own_backups_are_kept_and_only_a_trusted_promotion_names_the_stale_ones()
     {
        let plant = |h: &Harness| {
            let bin = h.tmp.path().join("bin");
            let p1 = bin.join("cortex-dump.prev-1111111111111111");
            let r1 = bin.join("cortex-restore.prev-3333333333333333");
            let un = h
                .env
                .unit_dir
                .join("cortex-dump.timer.prev-5555555555555555");
            for p in [&p1, &r1, &un] {
                fs::write(p, "x").unwrap();
            }
            [p1, r1, un]
        };
        // a trusted previous record: the resumed upgrade's promotion retires it and the one it had retired; the unnamed backup stays; the attempt's own are kept throughout
        let h = unknown_over_old_files();
        let a = marker_id(&h);
        let earlier = cortex_upgrade::members::Record {
            from: "0.2.0".to_string(),
            to: RC1.to_string(),
            id: "1111111111111111".to_string(),
            retired: vec!["3333333333333333".to_string()],
            entries: Vec::new(),
        };
        fs::write(
            members_last(&h),
            cortex_upgrade::members::serialize(&earlier),
        )
        .unwrap();
        let [p1, r1, un] = plant(&h);
        assert_eq!(backups_of(&h, &a).len(), 10);
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(
            !p1.exists() && !r1.exists(),
            "named by the trusted promotion: gone"
        );
        assert!(un.exists(), "named by nobody: kept");
        assert_eq!(
            backups_of(&h, &a).len(),
            10,
            "the attempt's own backups are kept"
        );
        // the same with a corrupt previous record: nothing is named, nothing is deleted
        let h = unknown_over_old_files();
        let a = marker_id(&h);
        fs::write(members_last(&h), "garbage\n").unwrap();
        let [p1, r1, un] = plant(&h);
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(
            p1.exists() && r1.exists() && un.exists(),
            "an untrusted record names nobody: all kept"
        );
        assert_eq!(backups_of(&h, &a).len(), 10);
        assert!(
            cortex_upgrade::members::read_last(h.tmp.path())
                .unwrap()
                .unwrap()
                .retired
                .is_empty()
        );
    }

    #[test]
    fn rm_33_a_rollback_into_a_unit_directory_that_is_not_ours_changes_nothing_exit_4() {
        let h = unknown_over_old_files();
        let id = marker_id(&h);
        fs::set_permissions(&h.env.unit_dir, PermissionsExt::from_mode(0o775)).unwrap();
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "writable by group or other"), "{o:?}");
        assert!(
            marker(&h).is_file() && members_marker(&h).is_file(),
            "marker and record are kept"
        );
        assert_eq!(backups_of(&h, &id).len(), 10, "every backup is kept");
        assert_files(&h, synthetic);
    }

    #[test]
    fn rm_34_a_same_attempt_members_last_with_a_planned_entry_stops_the_promotion_and_the_rollback_still_puts_the_files_back()
     {
        // mike's REWORK of packet 18, end to end: the unfinished upgrade (marker, a complete members.update) meets a same-attempt members.last that has an entry planned;
        // the promotion used to treat them as one record and delete the good live record, which left a rollback with nothing it could use
        use cortex_upgrade::members::{State, read_update, serialize};
        let h = unknown_over_old_files();
        let id = marker_id(&h);
        let mut last = read_update(h.tmp.path()).unwrap().unwrap();
        last.entries[0].state = State::Planned;
        fs::write(members_last(&h), serialize(&last)).unwrap();
        let (u, l) = (
            fs::read(members_marker(&h)).unwrap(),
            fs::read(members_last(&h)).unwrap(),
        );
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "could not be promoted"), "{o:?}");
        assert!(marker(&h).is_file(), "the marker stands");
        assert_eq!(
            fs::read(members_marker(&h)).unwrap(),
            u,
            "the live record is untouched"
        );
        assert_eq!(fs::read(members_last(&h)).unwrap(), l, "so is members.last");
        // the rollback uses the live record (every entry replaced) and puts the ten files back; nothing is stranded
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_files(&h, |_, n| old_text(n));
        assert!(
            !members_marker(&h).exists()
                && !members_last(&h).exists()
                && !any_backup(&h)
                && !marker(&h).exists()
        );
        let _ = id;
    }

    #[test]
    fn rm_26_a_commit_that_stopped_before_the_links_moved_leaves_neither_marker_nor_plan() {
        use cortex_upgrade::layout::CommitStep;
        let h = staged_harness(RC2, files(RC2), |_| {});
        let stop: &dyn Fn(CommitStep) -> bool = &|s| s != CommitStep::DirRenamed;
        let o = run_hooked(
            &h,
            &[],
            &cortex_upgrade::engine::Hooks {
                commit_step: Some(stop),
                ..Default::default()
            },
        );
        assert_eq!(o.code, 3, "{o:?}");
        assert!(said(&o, &format!("the switch to {RC2} failed")), "{o:?}");
        assert_eq!(Home::new(h.tmp.path()).current().unwrap(), Some(v(RC1)));
        assert!(!marker(&h).exists(), "the marker went");
        assert!(
            !members_marker(&h).exists(),
            "the plan of an attempt that never switched went with it"
        );
    }

    #[test]
    fn rm_27_a_record_that_cannot_be_promoted_stops_the_upgrade_at_exit_4_with_the_marker_standing()
    {
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        // after the last entry is installed something takes the place of members.last.new: S1 of the promotion cannot start
        let dir = h.tmp.path().join("members.last.new");
        let hook: &dyn Fn(&str) = &move |p: &str| {
            if p == "cortex-dump-alert.service:replaced" {
                fs::create_dir(&dir).unwrap();
            }
        };
        let o = run_hooked(
            &h,
            &[],
            &cortex_upgrade::engine::Hooks {
                member_point: Some(hook),
                ..Default::default()
            },
        );
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "could not be promoted"), "{o:?}");
        assert!(marker(&h).is_file(), "the upgrade marker stands");
        assert!(members_marker(&h).is_file(), "the record stands");
        assert!(!members_last(&h).exists());
    }

    #[test]
    fn rm_21_a_restore_that_fails_inside_an_automatic_rollback_is_exit_4_not_3() {
        use cortex_upgrade::engine::Ready;
        let h = staged_harness(RC2, files(RC2), |_| {});
        put_files(&h, |_, n| old_text(n));
        *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        // while the new release is being started a backup is damaged: the rollback that follows cannot put that entry back
        let id_cell: std::sync::Arc<std::sync::Mutex<String>> = Default::default();
        let (i2, m2, bin) = (id_cell.clone(), marker(&h), h.tmp.path().join("bin"));
        *h.containers.on_up.lock().unwrap() = Some(std::sync::Arc::new(move || {
            let id = fs::read_to_string(&m2)
                .map(|t| t.lines().nth(5).unwrap_or("").replace("id=", ""))
                .unwrap_or_default();
            *i2.lock().unwrap() = id.clone();
            let p = bin.join(format!("cortex-restore.prev-{id}"));
            if p.exists() {
                corrupt(&p, 0o755);
            }
        }));
        let o = run(&h, &["--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "cortex-restore"), "{o:?}");
        assert!(
            !o.lines
                .iter()
                .any(|l| l.to_lowercase().contains("rolled back:")),
            "no line says rolled back: {o:?}"
        );
        assert!(marker(&h).is_file() && members_marker(&h).is_file());
        let id = id_cell.lock().unwrap().clone();
        assert!(
            h.tmp
                .path()
                .join("bin")
                .join(format!("cortex-dump.prev-{id}"))
                .exists(),
            "every backup is kept"
        );
    }

    #[test]
    fn rb_05_a_server_that_never_answers_ready_or_a_refused_token_is_rolled_back() {
        use cortex_upgrade::engine::Ready;
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        let o = run(&h, &["--health-timeout", "3"]);
        assert_eq!(o.code, 3, "{o:?}");
        assert!(
            said(
                &o,
                "step 10 (check) failed: the server did not answer ready: timed out"
            ),
            "{o:?}"
        );
        assert_rolled_back(&h);
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.stack.answers.lock().unwrap() =
            vec![Whoami::Ok, Whoami::Refused(Some("token_revoked"))].into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 3, "{o:?}");
        assert!(
            said(
                &o,
                "step 10 (check) failed: the operator token is refused (token_revoked)"
            ),
            "{o:?}"
        );
        assert_rolled_back(&h);
    }

    #[test]
    fn rb_06_when_the_old_release_fails_its_checks_too_it_is_exit_4_with_the_set_and_the_fixed_text()
     {
        use cortex_upgrade::engine::Ready;
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 6].into();
        let o = run(&h, &["--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert_safe(&o);
        let text = o.lines.join("\n");
        assert!(text.contains("the rollback failed too"), "{text}");
        assert_eq!(
            text.matches(&set_path(&h)).count(),
            1,
            "the set path appears exactly once: {text}"
        );
        assert!(
            text.contains(SENTENCE),
            "kai's sentence, word for word: {text}"
        );
        assert_eq!(
            text.matches("pg_restore").count(),
            1,
            "pg_restore is named only inside that sentence: {text}"
        );
        assert!(
            text.contains(
                "stop the server, restore the set with cortex-restore into a NEW empty database"
            ),
            "{text}"
        );
        assert!(
            text.contains("every token issued before the restore is refused: re-issue the lead token with install.sh --reissue-token and point CORTEX_TOKEN_FILE at the new file"),
            "{text}"
        );
        // the lines come in the fixed order
        let pos = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(
            pos(&set_path(&h)) < pos(SENTENCE)
                && pos(SENTENCE) < pos("stop the server")
                && pos("stop the server") < pos("re-issue the lead token")
        );
        let recs = record_values(&h);
        assert_eq!(recs[0]["result"].as_str(), Some("rollback_failed"));
        assert_eq!(recs[0]["dump_set"].as_str(), Some(&*set_path(&h)));
        // the links stay as the rollback left them; nothing is deleted
        assert_eq!(Home::new(h.tmp.path()).current().unwrap(), Some(v(RC1)));
        assert!(h.tmp.path().join("dumps/pre-upgrade").join(LABEL).is_dir());
    }

    #[test]
    fn rb_07_a_contract_release_is_never_rolled_back_by_the_engine_and_says_so() {
        use cortex_upgrade::engine::Ready;
        let h = staged_harness(RC2, files(RC2), |r| {
            r["cortex"]["migration_class"] = json!("contract")
        });
        *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        let o = run(&h, &["--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "the rollback was refused"), "{o:?}");
        assert!(said(&o, "contract"), "{o:?}");
        assert_eq!(
            Home::new(h.tmp.path()).current().unwrap(),
            Some(v(RC2)),
            "nothing was flipped"
        );
        assert!(o.lines.join("\n").contains(&set_path(&h)));
        assert!(o.lines.join("\n").contains(SENTENCE));
        assert!(
            !events(&h)
                .iter()
                .any(|e| e == &format!("compose_up server {RC1}")),
            "no start of the old release"
        );
    }

    #[test]
    fn rb_08_rollback_after_a_good_upgrade_flips_back_starts_the_old_server_and_forgets_previous() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        assert_eq!(run(&h, &[]).code, 0);
        h.events.lock().unwrap().clear();
        let set = fs::read(
            h.tmp
                .path()
                .join("dumps/pre-upgrade")
                .join(LABEL)
                .join("SHA256SUMS"),
        )
        .unwrap();
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_safe(&o);
        assert!(said(&o, &format!("rolled back {RC2} -> {RC1}")), "{o:?}");
        assert_rolled_back(&h);
        assert_eq!(
            events(&h),
            [
                format!("compose_up server {RC1}"),
                "state server".to_string(),
                "ready".to_string(),
                "whoami".to_string()
            ],
            "no dump, no load, no role step, no migration: the schema stays"
        );
        assert_eq!(
            fs::read(
                h.tmp
                    .path()
                    .join("dumps/pre-upgrade")
                    .join(LABEL)
                    .join("SHA256SUMS")
            )
            .unwrap(),
            set
        );
        let recs = record_values(&h);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[1]["result"].as_str(), Some("rolled_back"));
    }

    #[test]
    fn rb_09_a_second_rollback_a_contract_release_a_held_lock_and_a_fresh_install_are_refused_before_any_port_call()
     {
        // a second rollback would flip forward again: there is no previous any more
        let h = staged_harness(RC2, files(RC2), |_| {});
        assert_eq!(run(&h, &[]).code, 0);
        assert_eq!(run(&h, &["--rollback"]).code, 0);
        h.events.lock().unwrap().clear();
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "no previous release"), "{o:?}");
        assert!(events(&h).is_empty() && tree_hash(h.tmp.path()) == before);
        // a contract release
        let h = staged_harness(RC2, files(RC2), |r| {
            r["cortex"]["migration_class"] = json!("contract")
        });
        assert_eq!(run(&h, &[]).code, 0);
        h.events.lock().unwrap().clear();
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "contract"), "{o:?}");
        assert!(events(&h).is_empty());
        assert_eq!(Home::new(h.tmp.path()).current().unwrap(), Some(v(RC2)));
        // a fresh install has nothing to go back to
        let h = staged_harness(RC2, files(RC2), |_| {});
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(events(&h).is_empty());
        // a held lock
        let h = staged_harness(RC2, files(RC2), |_| {});
        assert_eq!(run(&h, &[]).code, 0);
        h.events.lock().unwrap().clear();
        let guard = Home::new(h.tmp.path()).lock().unwrap();
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "another upgrade is running"), "{o:?}");
        assert!(events(&h).is_empty());
        drop(guard);
    }

    #[test]
    fn rb_10_a_rollback_whose_checks_fail_is_exit_4_with_the_set_of_the_last_upgrade_and_it_needs_no_dump_tool()
     {
        use cortex_upgrade::engine::Ready;
        let h = staged_harness(RC2, files(RC2), |_| {});
        fs::create_dir_all(h.tmp.path().join("upgrades")).unwrap();
        fs::write(
            h.tmp.path().join("upgrades/20200101T000000Z.json"),
            r#"{"from":"0.1.0","to":"0.1.1","result":"done","steps":[],"dump_set":"/an/older/set"}"#,
        )
        .unwrap();
        assert_eq!(run(&h, &[]).code, 0);
        fs::remove_file(h.tmp.path().join("bin/cortex-dump")).unwrap();
        *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        let o = run(&h, &["--rollback", "--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        let text = o.lines.join("\n");
        assert_eq!(text.matches(&set_path(&h)).count(), 1, "{text}");
        assert!(text.contains(SENTENCE));
        assert_eq!(
            record_values(&h).last().unwrap()["result"].as_str(),
            Some("rollback_failed")
        );
    }
    #[test]
    fn rb_11_an_old_server_that_does_not_get_healthy_makes_the_rollback_fail_at_the_timeout() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        h.containers
            .states
            .lock()
            .unwrap()
            .insert("migrate", vec![ServiceState::Exited(1)].into());
        h.containers
            .states
            .lock()
            .unwrap()
            .insert("server", vec![ServiceState::Starting; 50].into());
        let o = run(&h, &["--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(
            said(
                &o,
                "the rollback failed too: the server did not become healthy: timed out"
            ),
            "{o:?}"
        );
        assert_eq!(
            *h.pace.now.lock().unwrap(),
            Duration::from_secs(3),
            "the rollback's wait is bounded by --health-timeout"
        );
        assert!(o.lines.join("\n").contains(SENTENCE));
    }
    // ------------------------------------------------------------ mike, packet 6: the rework

    #[test]
    fn pr_12_a_parent_that_is_a_link_is_refused_before_anything_below_it_is_touched() {
        // downloads -> a directory outside, which holds a real <version> directory with a sentinel
        let h = staged_harness(RC2, files(RC2), |_| {});
        let home = h.tmp.path();
        let outside = home.join("outside");
        fs::create_dir_all(outside.join(RC2)).unwrap();
        fs::write(outside.join(RC2).join("sentinel"), "keep").unwrap();
        std::os::unix::fs::symlink("outside", home.join("downloads")).unwrap();
        let before = tree_hash(home);
        refused_clean(
            &h,
            &before,
            engine_prepare(&h),
            "downloads path exists and is not a directory",
        );
        assert_eq!(
            fs::read(outside.join(RC2).join("sentinel")).unwrap(),
            b"keep"
        );
        assert!(
            fs::symlink_metadata(home.join("downloads"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // control: the same leftover as a REAL directory is removed (pr_04 does the same for the whole run)
        let h = staged_harness(RC2, files(RC2), |_| {});
        fs::create_dir_all(h.tmp.path().join("downloads").join(RC2)).unwrap();
        fs::write(h.tmp.path().join("downloads").join(RC2).join("old"), "x").unwrap();
        let p = engine_prepare(&h).expect("a real leftover directory is removed");
        assert!(!p.downloads.join("old").exists());
        p.discard();
        // releases -> a copy elsewhere: refused, and nothing is made inside the copy
        let h = staged_harness(RC2, files(RC2), |_| {});
        let home = h.tmp.path();
        fs::rename(home.join("releases"), home.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink("elsewhere", home.join("releases")).unwrap();
        let before = tree_hash(home);
        let elsewhere = tree_hash(&home.join("elsewhere"));
        refused_clean(
            &h,
            &before,
            engine_prepare(&h),
            "releases path exists and is not a directory",
        );
        assert_eq!(tree_hash(&home.join("elsewhere")), elsewhere);
    }

    #[test]
    fn pr_13_staging_works_under_a_umask_that_strips_the_owners_bits() {
        if std::env::var("CORTEX_TEST_UMASK_ENGINE").is_ok() {
            // the child: the parent built the home, this process runs under the strict umask
            let h = staged_harness(RC2, files(RC2), |_| {});
            match engine_prepare(&h) {
                Ok(p) => {
                    println!(
                        "CHILD-STAGE ok {:o} {:o}",
                        mode_of(&p.downloads),
                        mode_of(p.downloads.parent().unwrap())
                    );
                    p.discard();
                }
                Err(o) => println!("CHILD-STAGE refused {:?}", o.lines),
            }
            std::mem::forget(h); // the parent removes the home
            return;
        }
        let h = staged_harness(RC2, files(RC2), |_| {});
        let name = "prepare::pr_13_staging_works_under_a_umask_that_strips_the_owners_bits";
        for mask in ["277", "377"] {
            let out = rerun_self_umask(
                name,
                (
                    "CORTEX_TEST_UMASK_ENGINE",
                    &h.tmp.path().display().to_string(),
                ),
                mask,
            );
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            assert!(
                text.contains("CHILD-STAGE ok 700 700"),
                "umask {mask}: the download directories are exactly 0700: {text}"
            );
        }
    }

    #[test]
    fn pr_14_a_cleanup_that_fails_is_reported_not_swallowed() {
        // the image archive arrives with a wrong digest while its directory has gone read-only under the run: the partial
        // file cannot be deleted, so the stage fails and then the directory cannot be emptied either
        let h = staged_harness(RC2, files(RC2), |r| {
            r["assets"][1]["digest"] = json!(DIGEST_A)
        });
        let dl = h.tmp.path().join("downloads").join(RC2);
        let hook_dir = dl.clone();
        *h.host.on_get.lock().unwrap() = Some(Arc::new(move |url: &str| {
            if url.ends_with(".oci.tar") {
                fs::set_permissions(&hook_dir, PermissionsExt::from_mode(0o500)).unwrap();
            }
        }));
        let result = engine_prepare(&h);
        let _ = fs::set_permissions(&dl, PermissionsExt::from_mode(0o700)); // so that the temporary home can go
        let o = match result {
            Err(o) => o,
            Ok(p) => panic!("expected a refusal, got {p:?}"),
        };
        assert_eq!(o.code, 2, "{o:?}");
        assert_safe(&o);
        assert!(
            said(
                &o,
                &format!("warning: downloads/{RC2} could not be removed; remove it by hand")
            ),
            "{o:?}"
        );
    }
    // ------------------------------------------------------------ mike, packet 7: the rework (findings 1 to 3, choices 4, 6, 7)

    fn good_upgrade(h: &Harness) {
        assert_eq!(run(h, &[]).code, 0);
        h.events.lock().unwrap().clear();
    }

    /// What starting and checking the OLD release's server shows: one compose call for RC1, the poll, the two checks.
    fn tail_events() -> Vec<String> {
        vec![
            format!("compose_up server {RC1}"),
            "state server".to_string(),
            "ready".to_string(),
            "whoami".to_string(),
        ]
    }

    fn no_markers(h: &Harness) {
        assert!(!h.tmp.path().join("rollback.intent").exists());
        assert!(!h.tmp.path().join("rollback.unfinished").exists());
    }

    fn exists_in_events(h: &Harness, what: &str) -> bool {
        events(h).iter().any(|e| e.contains(what))
    }

    // ---- finding 2: the dump area is listed before the tool runs, and a listing that fails is a refusal

    #[test]
    fn rw_01_a_dump_area_that_cannot_be_listed_is_a_refusal_before_the_dump_tool_runs() {
        let good_sums = format!("{}  dump.sql\n", sha_hex(b"-- a synthetic dump\n"));
        // (a) the area is there but is not a directory
        let h = staged_harness(RC2, files(RC2), |_| {});
        fs::create_dir_all(h.tmp.path().join("dumps")).unwrap();
        fs::write(h.tmp.path().join("dumps/pre-upgrade"), "not a directory").unwrap();
        let o = run(&h, &[]);
        assert_eq!(o.code, 2, "{o:?}");
        assert_safe(&o);
        assert!(
            said(&o, "the pre-upgrade dump area could not be listed"),
            "{o:?}"
        );
        assert!(
            !exists_in_events(&h, "dump pre-upgrade"),
            "the dump tool never ran: {:?}",
            events(&h)
        );
        // (b) the area exists with an OLD valid set in it, cannot be read, and the tool names that old set: not accepted as new
        let h = staged_harness(RC2, files(RC2), |_| {});
        let old = make_set(h.tmp.path(), "an-older-set", Some(good_sums.clone()));
        set_dump(&h, DumpMode::Path(old.display().to_string()));
        let area = h.tmp.path().join("dumps/pre-upgrade");
        fs::set_permissions(&area, PermissionsExt::from_mode(0o311)).unwrap();
        if fs::read_dir(&area).is_ok() {
            eprintln!(
                "rw_01 (b) SKIPPED: this account can list a directory it has no read permission on (root?)"
            );
        } else {
            let o = run(&h, &[]);
            let _ = fs::set_permissions(&area, PermissionsExt::from_mode(0o755));
            assert_eq!(o.code, 2, "{o:?}");
            assert!(
                said(&o, "the pre-upgrade dump area could not be listed"),
                "{o:?}"
            );
            assert!(
                !exists_in_events(&h, "dump pre-upgrade"),
                "{:?}",
                events(&h)
            );
            install_unchanged(&h);
        }
        let _ = fs::set_permissions(&area, PermissionsExt::from_mode(0o755));
        // (c) control, passes at once: a readable area with an old set in it, and the tool makes a NEW set: accepted
        let h = staged_harness(RC2, files(RC2), |_| {});
        make_set(h.tmp.path(), "an-older-set", Some(good_sums));
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
    }

    // ---- choice 7: a run that changed something beyond staging leaves a record; one that changed nothing leaves none

    #[test]
    fn rw_02_a_run_that_made_a_dump_set_and_then_failed_leaves_a_record_and_one_that_changed_nothing_leaves_none()
     {
        let h = staged_harness(RC2, files(RC2), |_| {});
        set_dump(&h, DumpMode::Exit(1, "disk full"));
        assert_eq!(run(&h, &[]).code, 2);
        assert!(records(&h).is_empty(), "nothing was changed: no record");
        let h = staged_harness(RC2, files(RC2), |_| {});
        *h.containers.load.lock().unwrap() = Err(PortError::Failed(OneLine::new("no space left")));
        let o = run(&h, &[]);
        assert_eq!(o.code, 2, "{o:?}");
        let recs = record_values(&h);
        assert_eq!(recs.len(), 1, "{o:?}");
        assert_eq!(recs[0]["result"].as_str(), Some("failed"));
        assert_eq!(recs[0]["dump_set"].as_str(), Some(&*set_path(&h)));
        assert_eq!(
            (recs[0]["from"].as_str(), recs[0]["to"].as_str()),
            (Some(RC1), Some(RC2))
        );
        let flags = step_flags(&recs[0]);
        assert_eq!(flags.len(), 7, "{flags:?}");
        assert!(
            flags[..6].iter().all(|(_, ok)| *ok) && flags[6] == (7, false),
            "{flags:?}"
        );
    }

    // ---- finding 3: a commit that fails is handled by where the links actually stand

    fn sabotage(h: &Harness, how: &'static str) {
        let home = h.tmp.path().to_path_buf();
        *h.containers.on_load.lock().unwrap() = Some(Arc::new(move || match how {
            // before the first effect: the release directory is already there next to the staged one
            "release dir exists" => fs::create_dir(home.join("releases").join(RC2)).unwrap(),
            // after the directory was renamed, before `previous` is set
            "previous temp is a directory" => {
                fs::create_dir(home.join(format!(".previous.tmp{}", std::process::id()))).unwrap()
            }
            // after `previous` was set, before `current` is
            "current temp is a directory" => {
                fs::create_dir(home.join(format!(".current.tmp{}", std::process::id()))).unwrap()
            }
            // `current` cannot be read as a release link at all
            "current is an odd link" => {
                fs::remove_file(home.join("current")).unwrap();
                std::os::unix::fs::symlink("../elsewhere", home.join("current")).unwrap();
            }
            _ => unreachable!(),
        }));
    }

    #[test]
    fn rw_03_a_commit_that_fails_before_current_moved_leaves_the_old_release_running_and_proves_it()
    {
        for how in [
            "release dir exists",
            "previous temp is a directory",
            "current temp is a directory",
        ] {
            let h = staged_harness(RC2, files(RC2), |_| {});
            sabotage(&h, how);
            let o = run(&h, &[]);
            assert_eq!(o.code, 3, "{how}: {o:?}");
            assert_safe(&o);
            assert!(
                said(&o, &format!("the switch to {RC2} failed")),
                "{how}: {o:?}"
            );
            assert!(
                said(
                    &o,
                    &format!("{RC1} is still current and answered its checks")
                ),
                "{how}: {o:?}"
            );
            assert!(
                said(
                    &o,
                    &format!("pre-upgrade dump set kept at {}", set_path(&h))
                ),
                "{how}: {o:?}"
            );
            let ev = events(&h);
            assert!(
                !ev.iter().any(|e| e.starts_with("compose_up")),
                "{how}: nothing was started: {ev:?}"
            );
            let at = ev.iter().position(|e| e.starts_with("load ")).unwrap();
            assert_eq!(
                &ev[at + 1..],
                ["ready", "whoami"],
                "{how}: the old release was asked after the failure"
            );
            assert_eq!(
                Home::new(h.tmp.path()).current().unwrap(),
                Some(v(RC1)),
                "{how}"
            );
            assert!(!h.tmp.path().join("downloads").exists(), "{how}");
            assert!(
                !h.tmp
                    .path()
                    .join("releases")
                    .join(format!("{RC2}.part"))
                    .exists(),
                "{how}"
            );
            let recs = record_values(&h);
            assert_eq!(recs.len(), 1, "{how}: every failure path is recorded");
            assert_eq!(recs[0]["result"].as_str(), Some("failed"), "{how}");
            assert_eq!(recs[0]["dump_set"].as_str(), Some(&*set_path(&h)), "{how}");
            assert_eq!(step_flags(&recs[0]).last(), Some(&(8, false)), "{how}");
            no_markers(&h);
            assert!(
                !marker(&h).exists(),
                "{how}: the marker, written ahead of the switch, goes when the switch did not happen"
            );
        }
        // the old release does NOT answer: that is exit 4 with the fixed text, never exit 3
        use cortex_upgrade::engine::Ready;
        let h = staged_harness(RC2, files(RC2), |_| {});
        sabotage(&h, "release dir exists");
        *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        let o = run(&h, &["--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        let text = o.lines.join("\n");
        assert!(text.contains("does not answer its checks"), "{text}");
        assert_eq!(text.matches(&set_path(&h)).count(), 1, "{text}");
        assert!(text.contains(SENTENCE), "{text}");
        assert_eq!(
            record_values(&h)[0]["result"].as_str(),
            Some("rollback_failed")
        );
    }

    #[test]
    fn rw_04_a_commit_that_fails_with_the_links_unreadable_is_exit_4_with_the_fixed_text_and_a_record()
     {
        let h = staged_harness(RC2, files(RC2), |_| {});
        sabotage(&h, "current is an odd link");
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        assert_safe(&o);
        let text = o.lines.join("\n");
        assert!(
            text.contains("the links in the home directory may be half way"),
            "{text}"
        );
        assert_eq!(text.matches(&set_path(&h)).count(), 1, "{text}");
        assert!(text.contains(SENTENCE), "{text}");
        assert_eq!(text.matches("pg_restore").count(), 1, "{text}");
        assert!(
            text.contains(
                "stop the server, restore the set with cortex-restore into a NEW empty database"
            ),
            "{text}"
        );
        assert!(
            text.contains("re-issue the lead token with install.sh --reissue-token"),
            "{text}"
        );
        assert!(!exists_in_events(&h, "compose_up"), "{:?}", events(&h));
        let recs = record_values(&h);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["result"].as_str(), Some("rollback_failed"));
        assert_eq!(recs[0]["dump_set"].as_str(), Some(&*set_path(&h)));
        assert_eq!(step_flags(&recs[0]).last(), Some(&(8, false)));
    }

    #[test]
    fn rw_05_the_state_of_the_links_after_a_failed_commit_is_classified_from_current_alone() {
        use cortex_upgrade::engine::{SwitchState, switch_state};
        let (a, b, c) = (v(RC1), v(RC2), v(RC3));
        assert_eq!(
            switch_state(Ok(Some(a.clone())), &a, &b),
            SwitchState::NotSwitched
        );
        assert_eq!(
            switch_state(Ok(Some(b.clone())), &a, &b),
            SwitchState::Switched
        );
        assert_eq!(switch_state(Ok(Some(c)), &a, &b), SwitchState::Unknown);
        assert_eq!(switch_state(Ok(None), &a, &b), SwitchState::Unknown);
        assert_eq!(
            switch_state(Err(LayoutError::BadLink { link: "current" }), &a, &b),
            SwitchState::Unknown
        );
    }

    // ---- finding 1: an interrupted rollback is finished by the retry, and a rollback that did not finish stays closed

    #[test]
    fn rw_06_an_interrupted_rollback_is_finished_by_the_retry_and_never_flips_forward() {
        for stop in [
            FlipStep::IntentWritten,
            FlipStep::PreviousSet,
            FlipStep::CurrentSet,
            FlipStep::MarkerWritten,
        ] {
            let h = staged_harness(RC2, files(RC2), |_| {});
            good_upgrade(&h);
            let home = Home::new(h.tmp.path());
            assert!(home.begin_rollback_with(&mut |s| s != stop).is_err());
            let o = run(&h, &["--rollback"]);
            assert_eq!(o.code, 0, "{stop:?}: {o:?}");
            assert_safe(&o);
            assert!(
                said(
                    &o,
                    &format!("completed an interrupted rollback ({RC2} to {RC1})")
                ),
                "{stop:?}: {o:?}"
            );
            assert!(
                said(&o, &format!("rolled back {RC2} -> {RC1}")),
                "{stop:?}: {o:?}"
            );
            assert_rolled_back(&h);
            no_markers(&h);
            assert_eq!(
                events(&h),
                tail_events(),
                "{stop:?}: the old release is started once and the one that was left never"
            );
            let recs = record_values(&h);
            let last = recs.last().unwrap();
            assert_eq!(last["result"].as_str(), Some("rolled_back"), "{stop:?}");
            assert_eq!(
                (last["from"].as_str(), last["to"].as_str()),
                (Some(RC2), Some(RC1)),
                "{stop:?}"
            );
            assert_eq!(last["dump_set"].as_str(), Some(&*set_path(&h)), "{stop:?}");
            assert_eq!(
                run(&h, &["--rollback"]).code,
                2,
                "{stop:?}: a second one is refused"
            );
        }
    }

    #[test]
    fn rw_07_an_interrupted_rollback_is_finished_before_an_upgrade_goes_on() {
        let h = staged_harness(RC3, files(RC3), |_| {});
        stage(h.tmp.path(), RC2, "expand");
        let home = Home::new(h.tmp.path());
        home.commit(&v(RC2)).unwrap();
        assert!(
            home.begin_rollback_with(&mut |s| s != FlipStep::PreviousSet)
                .is_err()
        );
        let o = run(&h, &[]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(
            said(
                &o,
                &format!("completed an interrupted rollback ({RC2} to {RC1})")
            ),
            "{o:?}"
        );
        assert!(said(&o, &format!("rolled back {RC2} -> {RC1}")), "{o:?}");
        let ev = events(&h);
        assert_eq!(
            &ev[..4],
            tail_events().as_slice(),
            "the old server is started and checked FIRST: {ev:?}"
        );
        assert!(
            ev[4..].iter().any(|e| e.starts_with("GET")),
            "and only then the upgrade asks for anything: {ev:?}"
        );
        assert_eq!(home.current().unwrap(), Some(v(RC3)));
        no_markers(&h);
        let rolled: Vec<Value> = record_values(&h)
            .into_iter()
            .filter(|r| r["result"].as_str() == Some("rolled_back"))
            .collect();
        assert_eq!(rolled.len(), 1, "the finished rollback is recorded once");
        assert_eq!(
            (rolled[0]["from"].as_str(), rolled[0]["to"].as_str()),
            (Some(RC2), Some(RC1))
        );
    }

    #[test]
    fn rw_08_a_rollback_whose_old_server_fails_stays_unfinished_and_a_retry_never_flips_forward() {
        use cortex_upgrade::engine::Ready;
        let fail = |h: &Harness| {
            *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        };
        let h = staged_harness(RC2, files(RC2), |_| {});
        good_upgrade(&h);
        let path = h.tmp.path();
        let home = Home::new(path);
        fail(&h);
        let o = run(&h, &["--rollback", "--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        let text = o.lines.join("\n");
        assert!(text.contains("the rollback is still unfinished"), "{text}");
        assert!(text.contains(SENTENCE), "{text}");
        // the durable state: the marker, the old release current, and every switch refused
        assert!(path.join("rollback.unfinished").is_file());
        assert_eq!(home.current().unwrap(), Some(v(RC1)));
        assert_eq!(home.begin_rollback(), Err(LayoutError::RecoveryPending));
        // an upgrade does not go on over it: the tail is tried first, fails, and the run stops before asking for anything
        h.events.lock().unwrap().clear();
        fail(&h);
        let o = run(&h, &["--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "the rollback is still unfinished"), "{o:?}");
        assert!(o.lines.join("\n").contains(SENTENCE), "{o:?}");
        assert!(!exists_in_events(&h, "GET"), "{:?}", events(&h));
        // a second rollback while the old server still fails: exit 4 again, and the left release is never started
        h.events.lock().unwrap().clear();
        fail(&h);
        let o = run(&h, &["--rollback", "--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(!exists_in_events(&h, RC2), "{:?}", events(&h));
        assert_eq!(home.current().unwrap(), Some(v(RC1)));
        // the old server answers now: the retry finishes the rollback without any flip
        h.events.lock().unwrap().clear();
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_eq!(events(&h), tail_events());
        assert_rolled_back(&h);
        no_markers(&h);
    }

    #[test]
    fn rw_09_a_marker_that_cannot_be_cleared_is_reported_and_nothing_else_can_switch_until_it_is() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        good_upgrade(&h);
        let path = h.tmp.path().to_path_buf();
        let p2 = path.clone();
        *h.containers.on_up.lock().unwrap() = Some(Arc::new(move || {
            // `previous` becomes a real directory just before the rollback finishes: not ours to remove
            let prev = p2.join("previous");
            if fs::symlink_metadata(&prev)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                fs::remove_file(&prev).unwrap();
                fs::create_dir(&prev).unwrap();
            }
        }));
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "the old release runs: {o:?}");
        assert!(
            said(
                &o,
                "warning: the rollback is complete but its marker could not be cleared"
            ),
            "{o:?}"
        );
        assert!(path.join("rollback.unfinished").is_file());
        let home = Home::new(&path);
        assert_eq!(home.current().unwrap(), Some(v(RC1)));
        // another rollback is the tail again, never a flip
        *h.containers.on_up.lock().unwrap() = None;
        h.events.lock().unwrap().clear();
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, "could not be cleared"), "{o:?}");
        assert!(!exists_in_events(&h, RC2), "{:?}", events(&h));
        assert_eq!(home.current().unwrap(), Some(v(RC1)));
        // an upgrade is refused and asks for nothing
        h.events.lock().unwrap().clear();
        let o = run(&h, &[]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "could not be cleared"), "{o:?}");
        assert!(!exists_in_events(&h, "GET"), "{:?}", events(&h));
        // the operator removes the directory: the next rollback finishes cleanly
        fs::remove_dir_all(path.join("previous")).unwrap();
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(!said(&o, "could not be cleared"), "{o:?}");
        assert_rolled_back(&h);
        no_markers(&h);
    }

    // ---- choice 4: the migration has a bound of its own

    #[test]
    fn rw_10_the_migration_has_its_own_bound() {
        let slow = |h: &Harness| {
            h.containers
                .states
                .lock()
                .unwrap()
                .insert("migrate", vec![ServiceState::Running; 30].into());
        };
        let polls = |h: &Harness| events(h).iter().filter(|e| *e == "state migrate").count();
        // longer than --health-timeout, within --migrate-timeout (default): fine
        let h = staged_harness(RC2, files(RC2), |_| {});
        slow(&h);
        let o = run(&h, &["--health-timeout", "10"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_eq!(polls(&h), 31);
        // beyond its own bound: rolled back, with the reason
        let h = staged_harness(RC2, files(RC2), |_| {});
        slow(&h);
        let o = run(&h, &["--migrate-timeout", "10"]);
        assert_eq!(o.code, 3, "{o:?}");
        assert!(
            said(&o, "the migrations did not finish: timed out"),
            "{o:?}"
        );
        assert_eq!(polls(&h), 6, "polled at 0, 2, 4, 6, 8 and 10 seconds");
        assert_rolled_back(&h);
    }

    // ---- choice 6: the set in the exit 4 text of --rollback belongs to the upgrade that is being left

    #[test]
    fn rw_11_the_set_in_the_exit_4_text_belongs_to_the_upgrade_that_is_being_left() {
        use cortex_upgrade::engine::Ready;
        let later = r#"{"from":"0.2.1-rc.2","to":"0.2.1-rc.3","result":"rolled_back","steps":[],"dump_set":"/other/set"}"#;
        let fail = |h: &Harness| {
            *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        };
        let h = staged_harness(RC2, files(RC2), |_| {});
        good_upgrade(&h);
        fs::write(h.tmp.path().join("upgrades/29990101T000000Z.json"), later).unwrap();
        // an OLDER record of the same upgrade (an earlier attempt) is not the one: the newest matching record is
        fs::write(
            h.tmp.path().join("upgrades/20200101T000001Z.json"),
            r#"{"from":"0.2.1-rc.1","to":"0.2.1-rc.2","result":"rolled_back","steps":[],"dump_set":"/older/same-pair"}"#,
        )
        .unwrap();
        // records that match only ONE end of the upgrade being left are not borrowed either
        for (name, rec) in [
            (
                "29990101T000001Z",
                r#"{"from":"0.2.1-rc.0","to":"0.2.1-rc.2","result":"done","steps":[],"dump_set":"/wrong/from"}"#,
            ),
            (
                "29990101T000002Z",
                r#"{"from":"0.2.1-rc.1","to":"0.2.1-rc.3","result":"done","steps":[],"dump_set":"/wrong/to"}"#,
            ),
        ] {
            fs::write(h.tmp.path().join(format!("upgrades/{name}.json")), rec).unwrap();
        }
        fail(&h);
        let o = run(&h, &["--rollback", "--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        let text = o.lines.join("\n");
        assert_eq!(text.matches(&set_path(&h)).count(), 1, "{text}");
        assert!(
            !text.contains("/other/set") && !text.contains("/wrong/") && !text.contains("/older/"),
            "{text}"
        );
        let failed: Vec<Value> = record_values(&h)
            .into_iter()
            .filter(|r| r["result"].as_str() == Some("rollback_failed"))
            .collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0]["dump_set"].as_str(), Some(&*set_path(&h)));
        // no record of the upgrade that is being left: no set is claimed, and an unrelated one is not borrowed
        let h = staged_harness(RC2, files(RC2), |_| {});
        good_upgrade(&h);
        for r in records(&h) {
            fs::remove_file(r).unwrap();
        }
        fs::write(h.tmp.path().join("upgrades/29990101T000000Z.json"), later).unwrap();
        fail(&h);
        let o = run(&h, &["--rollback", "--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        let text = o.lines.join("\n");
        assert!(
            text.contains("no pre-upgrade dump set is recorded"),
            "{text}"
        );
        assert!(!text.contains("/other/set"), "{text}");
    }

    // ---- R6A-03 (vera, cox 6f81008d): no control character reaches the terminal from a dump-set path; the DATA keeps the real path

    /// A plain, valid home whose `dumps` is a link to a directory with a control byte in its NAME: the engine canonicalizes the set the dump tool names, so
    /// the set's canonical path holds the control character (the home alias case of R6A-01). Returns that canonical path, raw.
    fn control_dumps(h: &Harness) -> String {
        let home = h.tmp.path();
        let real = home.join("dumps\u{1}real");
        fs::rename(home.join("dumps"), &real).unwrap();
        std::os::unix::fs::symlink("dumps\u{1}real", home.join("dumps")).unwrap();
        real.join("pre-upgrade").join(LABEL).display().to_string()
    }

    /// That path as the operator must see it: the control byte ESCAPED, every other byte as it is.
    fn shown_set(real: &str) -> String {
        real.replace('\u{1}', "\\u{1}")
    }

    /// The whole output: no line holds a control character.
    fn assert_no_control(o: &Outcome) {
        for l in &o.lines {
            assert!(
                !l.chars().any(char::is_control),
                "a raw control character in: {l:?}"
            );
        }
    }

    /// The machine data keeps the REAL path: the one record's `dump_set` is the canonical path, control byte and all.
    fn assert_record_set_is_real(h: &Harness, real: &str, result: &str) {
        let recs = record_values(h);
        assert_eq!(recs.len(), 1, "{recs:?}");
        assert_eq!(recs[0]["result"].as_str(), Some(result), "{recs:?}");
        assert_eq!(recs[0]["dump_set"].as_str(), Some(real), "{recs:?}");
    }

    #[test]
    fn rs_01_a_load_that_fails_shows_a_dump_set_with_a_control_character_escaped() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let real = control_dumps(&h);
        *h.containers.load.lock().unwrap() = Err(PortError::Failed(OneLine::new("no space left")));
        let o = run(&h, &[]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(
            said(
                &o,
                &format!("pre-upgrade dump set kept at {}", shown_set(&real))
            ),
            "{o:?}"
        );
        assert_no_control(&o);
        assert_record_set_is_real(&h, &real, "failed");
    }

    #[test]
    fn rs_02_a_switch_refused_for_that_set_says_so_with_the_path_escaped_and_changes_nothing() {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let real = control_dumps(&h);
        let o = run(&h, &[]);
        // the marker (`set=` is one plain line) refuses the control character, so the switch never happens and the old release answers
        assert_eq!(o.code, 3, "{o:?}");
        assert!(
            said(&o, "is still current and answered its checks"),
            "{o:?}"
        );
        assert!(
            said(
                &o,
                &format!("pre-upgrade dump set kept at {}", shown_set(&real))
            ),
            "{o:?}"
        );
        assert_no_control(&o);
        assert_record_set_is_real(&h, &real, "failed");
        // the control byte never gets past the marker: nothing switched, no marker stands
        assert_eq!(Home::new(h.tmp.path()).current().unwrap(), Some(v(RC1)));
        assert!(!marker(&h).exists());
    }

    #[test]
    fn rs_03_an_old_release_that_does_not_answer_gets_the_exit_4_text_with_the_set_escaped() {
        use cortex_upgrade::engine::Ready;
        let h = staged_harness(RC2, files(RC2), |_| {});
        let real = control_dumps(&h);
        *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        let o = run(&h, &["--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(
            said(
                &o,
                &format!("the pre-upgrade dump set is {}", shown_set(&real))
            ),
            "{o:?}"
        );
        assert!(said(&o, SENTENCE), "{o:?}");
        assert_no_control(&o);
        assert_record_set_is_real(&h, &real, "rollback_failed");
    }

    #[test]
    fn rs_04_links_that_cannot_be_read_after_the_refused_switch_get_the_exit_4_text_with_the_set_escaped()
     {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let real = control_dumps(&h);
        sabotage(&h, "current is an odd link");
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(said(&o, "may be half way"), "{o:?}");
        assert!(
            said(
                &o,
                &format!("the pre-upgrade dump set is {}", shown_set(&real))
            ),
            "{o:?}"
        );
        assert_no_control(&o);
        assert_record_set_is_real(&h, &real, "rollback_failed");
    }

    /// The control of the series: a set read back from an upgrade RECORD (`dump_set_for`) already comes out escaped, through `OneLine`; this pins that no
    /// edit of `exit4_lines` makes it raw, and that it is not escaped twice.
    #[test]
    fn rs_05_a_set_read_back_from_a_record_with_a_control_character_is_shown_escaped_once() {
        use cortex_upgrade::engine::Ready;
        let h = staged_harness(RC2, files(RC2), |_| {});
        good_upgrade(&h);
        for r in records(&h) {
            fs::remove_file(r).unwrap();
        }
        let rec = json!({"from": RC1, "to": RC2, "result": "done", "steps": [], "dump_set": "/rec\u{1}ord/set"});
        fs::write(
            h.tmp.path().join("upgrades/29990101T000000Z.json"),
            rec.to_string(),
        )
        .unwrap();
        *h.stack.ready_seq.lock().unwrap() = vec![Ready::No("503".into()); 3].into();
        let o = run(&h, &["--rollback", "--health-timeout", "3"]);
        assert_eq!(o.code, 4, "{o:?}");
        assert!(
            said(&o, "the pre-upgrade dump set is /rec\\u{1}ord/set"),
            "{o:?}"
        );
        assert!(!said(&o, "\\\\u{1}"), "escaped twice: {o:?}");
        assert_no_control(&o);
    }

    #[test]
    fn rw_12_a_marker_that_cannot_be_used_stops_every_mode_that_takes_the_lock_before_anything_is_asked()
     {
        for mode in [vec![], vec!["--rollback"]] {
            let h = staged_harness(RC2, files(RC2), |_| {});
            good_upgrade(&h);
            fs::write(h.tmp.path().join("rollback.unfinished"), "garbage").unwrap();
            let before = tree_hash(h.tmp.path());
            let o = run(&h, &mode);
            assert_eq!(o.code, 2, "{mode:?}: {o:?}");
            assert_safe(&o);
            assert!(
                said(&o, "the marker of an earlier rollback cannot be used"),
                "{o:?}"
            );
            assert!(events(&h).is_empty(), "{:?}", events(&h));
            assert_eq!(tree_hash(h.tmp.path()), before, "{mode:?}: nothing changed");
        }
        // a check takes no lock and reads nothing of the marker, but it does not answer over one (packet 9): exit 2, nothing asked
        let h = staged_harness(RC2, files(RC2), |_| {});
        fs::write(h.tmp.path().join("rollback.unfinished"), "garbage").unwrap();
        let o = run(&h, &["--check"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(events(&h).is_empty(), "{:?}", events(&h));
    }

    // ---- mike, packet 8 (and his supplement to packet 7): a rollback that fails AFTER it started is not a refusal

    fn block(h: &Harness, stem: &str) {
        fs::create_dir(
            h.tmp
                .path()
                .join(format!("{stem}.tmp{}", std::process::id())),
        )
        .unwrap();
    }

    fn unblock(h: &Harness, stem: &str) {
        fs::remove_dir(
            h.tmp
                .path()
                .join(format!("{stem}.tmp{}", std::process::id())),
        )
        .unwrap();
    }

    #[test]
    fn rw_13_a_rollback_that_fails_after_it_started_is_exit_4_with_its_state_kept_and_the_retry_finishes_it()
     {
        // before anything was written: a refusal, exit 2, nothing changed (positive control for the cases below)
        let h = staged_harness(RC2, files(RC2), |_| {});
        good_upgrade(&h);
        block(&h, ".rollback.intent");
        let before = tree_hash(h.tmp.path());
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 2, "{o:?}");
        assert!(said(&o, "rollback refused"), "{o:?}");
        assert!(events(&h).is_empty());
        assert_eq!(tree_hash(h.tmp.path()), before, "nothing changed");
        no_markers(&h);
        // after it started, at each effect
        for (stem, links) in [
            (".previous", (RC2, RC1)),
            (".current", (RC2, RC2)),
            (".rollback.unfinished", (RC1, RC2)),
        ] {
            let h = staged_harness(RC2, files(RC2), |_| {});
            good_upgrade(&h);
            block(&h, stem);
            let o = run(&h, &["--rollback"]);
            assert_eq!(o.code, 4, "{stem}: {o:?}");
            assert_safe(&o);
            let text = o.lines.join("\n");
            assert!(
                text.contains("the rollback was started and stopped"),
                "{stem}: {text}"
            );
            assert!(
                text.contains("rollback.intent")
                    && text.contains("run cortex upgrade --rollback again"),
                "{stem}: {text}"
            );
            assert_eq!(text.matches(&set_path(&h)).count(), 1, "{stem}: {text}");
            assert!(text.contains(SENTENCE), "{stem}: {text}");
            assert!(h.tmp.path().join("rollback.intent").is_file(), "{stem}");
            let home = Home::new(h.tmp.path());
            assert_eq!(
                (home.current().unwrap(), home.previous().unwrap()),
                (Some(v(links.0)), Some(v(links.1))),
                "{stem}"
            );
            assert!(
                events(&h).is_empty(),
                "{stem}: nothing was started: {:?}",
                events(&h)
            );
            let failed: Vec<Value> = record_values(&h)
                .into_iter()
                .filter(|r| r["result"].as_str() == Some("rollback_failed"))
                .collect();
            assert_eq!(failed.len(), 1, "{stem}: recorded");
            assert_eq!(
                (failed[0]["from"].as_str(), failed[0]["to"].as_str()),
                (Some(RC2), Some(RC1)),
                "{stem}"
            );
            // the obstacle goes; the retry finishes it: the links first, then the old server, once
            unblock(&h, stem);
            let o = run(&h, &["--rollback"]);
            assert_eq!(o.code, 0, "{stem}: {o:?}");
            assert!(
                said(&o, "completed an interrupted rollback"),
                "{stem}: {o:?}"
            );
            assert_rolled_back(&h);
            no_markers(&h);
            assert_eq!(events(&h), tail_events(), "{stem}");
        }
    }

    #[test]
    fn rw_14_a_rollback_that_fails_after_it_started_inside_a_failed_upgrade_is_exit_4_and_recoverable()
     {
        let h = staged_harness(RC2, files(RC2), |_| {});
        let hh = h.tmp.path().to_path_buf();
        *h.containers.on_load.lock().unwrap() = Some(Arc::new(move || {
            fs::create_dir(hh.join(format!(".rollback.unfinished.tmp{}", std::process::id())))
                .unwrap();
        }));
        *h.containers.ups.lock().unwrap() =
            vec![Err(PortError::Failed(OneLine::new("boom")))].into();
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        assert_safe(&o);
        let text = o.lines.join("\n");
        assert!(
            text.contains("the rollback was started and stopped"),
            "{text}"
        );
        assert_eq!(text.matches(&set_path(&h)).count(), 1, "{text}");
        assert!(text.contains(SENTENCE), "{text}");
        assert!(h.tmp.path().join("rollback.intent").is_file());
        assert_eq!(
            record_values(&h).last().unwrap()["result"].as_str(),
            Some("rollback_failed")
        );
        unblock(&h, ".rollback.unfinished");
        h.events.lock().unwrap().clear();
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert_rolled_back(&h);
        no_markers(&h);
        assert_eq!(events(&h), tail_events());
    }

    #[test]
    fn rw_15_an_earlier_rollback_that_cannot_be_completed_by_recover_is_exit_4_and_keeps_its_intent()
     {
        let h = staged_harness(RC3, files(RC3), |_| {});
        stage(h.tmp.path(), RC2, "expand");
        let home = Home::new(h.tmp.path());
        home.commit(&v(RC2)).unwrap();
        assert!(
            home.begin_rollback_with(&mut |s| s != FlipStep::IntentWritten)
                .is_err()
        );
        block(&h, ".previous");
        let o = run(&h, &[]);
        assert_eq!(o.code, 4, "{o:?}");
        assert_safe(&o);
        let text = o.lines.join("\n");
        assert!(
            text.contains("the earlier rollback could not be completed")
                && text.contains("rollback.intent"),
            "{text}"
        );
        assert!(h.tmp.path().join("rollback.intent").is_file());
        assert!(events(&h).is_empty(), "nothing was asked: {:?}", events(&h));
        // the cause goes; the next run completes it
        unblock(&h, ".previous");
        let o = run(&h, &["--rollback"]);
        assert_eq!(o.code, 0, "{o:?}");
        assert!(said(&o, "completed an interrupted rollback"), "{o:?}");
        no_markers(&h);
    }

    // ---- mike, packet 9 (his edges for the integration pass): the old `previous` and `--check` over an unfinished rollback

    #[test]
    fn rw_16_a_commit_that_fails_half_way_puts_the_old_previous_back() {
        for how in [
            "current temp is a directory",
            "previous temp is a directory",
            "release dir exists",
        ] {
            // with a previous release that was there before: it is where it was
            let h = staged_harness(RC2, files(RC2), |_| {});
            let path = h.tmp.path().to_path_buf();
            fs::create_dir(path.join("releases").join("0.2.1-rc.0")).unwrap();
            std::os::unix::fs::symlink("releases/0.2.1-rc.0", path.join("previous")).unwrap();
            sabotage(&h, how);
            let o = run(&h, &[]);
            assert_eq!(o.code, 3, "{how}: {o:?}");
            assert_eq!(
                Home::new(&path).previous().unwrap(),
                Some(v("0.2.1-rc.0")),
                "{how}: the earlier previous is where it was"
            );
            assert_eq!(Home::new(&path).current().unwrap(), Some(v(RC1)), "{how}");
            // with none before: none after (the failed commit had set it to the current release)
            let h = staged_harness(RC2, files(RC2), |_| {});
            sabotage(&h, how);
            let o = run(&h, &[]);
            assert_eq!(o.code, 3, "{how}: {o:?}");
            assert_eq!(
                Home::new(h.tmp.path()).previous().unwrap(),
                None,
                "{how}: there was no previous and there is none"
            );
        }
    }

    #[test]
    fn rw_17_a_check_over_an_unfinished_rollback_is_refused_and_asks_for_nothing() {
        // a flip interrupted, and a flip finished with its marker standing; each is exit 2, no request of any kind
        for stop in [Some(FlipStep::IntentWritten), None] {
            let h = staged_harness(RC2, files(RC2), |_| {});
            good_upgrade(&h);
            let home = Home::new(h.tmp.path());
            match stop {
                Some(s) => assert!(home.begin_rollback_with(&mut |x| x != s).is_err()),
                None => {
                    home.begin_rollback().unwrap();
                }
            }
            let before = tree_hash(h.tmp.path());
            let o = run(&h, &["--check"]);
            assert_eq!(o.code, 2, "{stop:?}: {o:?}");
            assert!(said(&o, "an unfinished rollback"), "{stop:?}: {o:?}");
            assert!(events(&h).is_empty(), "{stop:?}: {:?}", events(&h));
            assert_eq!(
                tree_hash(h.tmp.path()),
                before,
                "{stop:?}: --check changes nothing"
            );
        }
        // control, passes at once: with nothing unfinished a check answers as before
        let h = staged_harness(RC2, files(RC2), |_| {});
        assert_eq!(run(&h, &["--check"]).code, 10);
        // and once the rollback is finished (by --rollback) the check answers again
        let h = staged_harness(RC2, files(RC2), |_| {});
        good_upgrade(&h);
        Home::new(h.tmp.path()).begin_rollback().unwrap();
        assert_eq!(run(&h, &["--rollback"]).code, 0);
        assert_eq!(run(&h, &["--check"]).code, 10);
    }

    // ---- mike, packet 10 (his conditional edge): a `previous` that is not a plain release link stops an upgrade BEFORE anything is made

    #[test]
    fn rw_18_a_previous_that_is_not_a_plain_release_link_is_refused_before_anything_is_made() {
        // a case: its name, and what it does to the home directory to make `previous` something other than a plain release link
        type Case = (&'static str, Box<dyn Fn(&std::path::Path)>);
        let cases: Vec<Case> = vec![
            (
                "a link out of the releases directory",
                Box::new(|p| {
                    std::os::unix::fs::symlink("../elsewhere", p.join("previous")).unwrap()
                }),
            ),
            (
                "a real directory",
                Box::new(|p| fs::create_dir(p.join("previous")).unwrap()),
            ),
            (
                "a link to a name that is not a version",
                Box::new(|p| {
                    std::os::unix::fs::symlink("releases/not-a-version", p.join("previous"))
                        .unwrap()
                }),
            ),
        ];
        for (what, make) in cases {
            let h = staged_harness(RC2, files(RC2), |_| {});
            make(h.tmp.path());
            // upgrade.lock is part of the layout and the first locker creates it: make it exist before the snapshot
            drop(Home::new(h.tmp.path()).lock().unwrap());
            let before = tree_hash(h.tmp.path());
            let o = run(&h, &[]);
            assert_eq!(o.code, 2, "{what}: {o:?}");
            assert_safe(&o);
            assert!(said(&o, "the previous link"), "{what}: {o:?}");
            assert!(said(&o, "nothing was changed"), "{what}: {o:?}");
            assert!(
                events(&h).is_empty(),
                "{what}: nothing was asked: {:?}",
                events(&h)
            );
            assert_eq!(tree_hash(h.tmp.path()), before, "{what}: nothing changed");
        }
        // positive controls, pass at once: a valid previous and none at all go on
        let h = staged_harness(RC2, files(RC2), |_| {});
        fs::create_dir(h.tmp.path().join("releases").join("0.2.1-rc.0")).unwrap();
        std::os::unix::fs::symlink("releases/0.2.1-rc.0", h.tmp.path().join("previous")).unwrap();
        assert_eq!(run(&h, &[]).code, 0);
        let h = staged_harness(RC2, files(RC2), |_| {});
        assert_eq!(run(&h, &[]).code, 0);
    }

    /// R-6c (cox W1-105, sam's R6UT-SAM01): a CHECK holds no lock, so a normal concurrent upgrade can publish its marker (and flip the links) between the
    /// check's EARLY read of the marker, which found none, and the LATER one, which used to hand the check to the mutating resume (`finish_upgrade`: it
    /// installs the member files, then restarts and composes). The later read must refuse a check, read-only, in the words of the early refusal. The seam
    /// `before_late_marker` runs just before the later read; the state at that moment is the state a lost upgrade leaves (the marker, the links, the
    /// manifest copy), the marker being held aside until then.
    mod r6c {
        use super::*;
        use cortex_upgrade::engine::{EXIT_DONE, EXIT_NEWER, EXIT_REFUSED, Hooks};
        use std::cell::{Cell, RefCell};
        use std::panic::{AssertUnwindSafe, catch_unwind};
        use std::path::Path;

        /// The state a concurrent upgrade is in for a moment: the marker, the flipped links and the manifest copy are published, the member files are NOT
        /// yet installed (the run is lost at the `current` step of the flip, as in the r4b cells).
        fn lose_at_current() -> impl Fn(cortex_upgrade::layout::CommitStep) -> bool {
            |s| {
                if s == cortex_upgrade::layout::CommitStep::CurrentSet {
                    panic!("process lost")
                }
                true
            }
        }

        fn lost_after_the_flip() -> Harness {
            let h = staged_harness(RC2, files(RC2), |_| {});
            let lose = lose_at_current();
            let hooks = Hooks {
                commit_step: Some(&lose),
                ..Default::default()
            };
            let died = catch_unwind(AssertUnwindSafe(|| run_hooked(&h, &[], &hooks)));
            assert!(died.is_err(), "the run did not return");
            assert!(marker(&h).is_file(), "the marker stands");
            assert!(
                started(&h).is_empty(),
                "nothing was started before the loss"
            );
            h
        }

        /// Every entry under `root` (paths relative to it): a directory, a link and its target, a file and its bytes. Two equal lists: nothing changed.
        fn state_of(root: &Path) -> Vec<String> {
            fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
                let mut entries: Vec<_> = fs::read_dir(dir)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .collect();
                entries.sort();
                for p in entries {
                    let rel = p.strip_prefix(root).unwrap().display().to_string();
                    let meta = fs::symlink_metadata(&p).unwrap();
                    if meta.file_type().is_symlink() {
                        out.push(format!("{rel} -> {}", fs::read_link(&p).unwrap().display()));
                    } else if meta.is_dir() {
                        out.push(format!("{rel}/"));
                        walk(&p, root, out);
                    } else {
                        out.push(format!("{rel} {:?}", fs::read(&p).unwrap()));
                    }
                }
            }
            let mut out = Vec::new();
            walk(root, root, &mut out);
            out
        }

        /// What was STARTED through the containers port: every compose call and every roles step.
        fn started(h: &Harness) -> Vec<String> {
            events(h)
                .into_iter()
                .filter(|e| e.starts_with("compose_up") || e.starts_with("run_roles"))
                .collect()
        }

        #[test]
        fn ck_01_a_check_that_meets_a_concurrent_upgrades_marker_at_its_later_read_is_refused_read_only()
         {
            let h = lost_after_the_flip();
            // the marker is not there for the check's early read: held aside under another name, put back by the seam
            let held = h.tmp.path().join("upgrade.incomplete.held");
            fs::rename(marker(&h), &held).unwrap();
            let at_the_seam: RefCell<Option<(Vec<String>, Vec<String>)>> = RefCell::new(None);
            let calls = Cell::new(0u32);
            let points: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let on_point = |p: &str| points.borrow_mut().push(p.to_string());
            let put_back = || {
                calls.set(calls.get() + 1);
                fs::rename(&held, marker(&h)).unwrap();
                *at_the_seam.borrow_mut() = Some((state_of(h.tmp.path()), events(&h)));
            };
            let o = run_hooked(
                &h,
                &["--check"],
                &Hooks {
                    before_late_marker: Some(&put_back),
                    member_point: Some(&on_point),
                    ..Default::default()
                },
            );
            assert_eq!(
                calls.get(),
                1,
                "the check reached the later marker read, once"
            );
            let (tree, ev) = at_the_seam.borrow().clone().unwrap();
            assert_eq!(o.code, EXIT_REFUSED, "{o:?}");
            assert_safe(&o);
            assert!(said(&o, "did not finish"), "{o:?}");
            assert!(said(&o, "nothing was asked"), "{o:?}");
            // read-only: no port was called after the marker appeared (no compose, no roles step, nothing), and nothing in the home or the unit
            // directory (the member files are written there) changed after it
            assert!(
                points.borrow().is_empty(),
                "no member-protocol step ran (nothing was installed): {:?}",
                points.borrow()
            );
            assert_eq!(events(&h), ev, "no port call after the marker appeared");
            assert_eq!(
                state_of(h.tmp.path()),
                tree,
                "nothing was written after the marker appeared"
            );
            // and it is the refusal a check gets when it finds the marker from the start
            let c = lost_after_the_flip();
            let early = run(&c, &["--check"]);
            assert_eq!(
                (o.code, &o.lines),
                (early.code, &early.lines),
                "the same refusal"
            );
        }

        #[test]
        fn ck_02_a_check_that_finds_the_marker_from_the_start_is_refused_read_only() {
            let h = lost_after_the_flip();
            let tree = state_of(h.tmp.path());
            let ev = events(&h);
            let o = run(&h, &["--check"]);
            assert_eq!(o.code, EXIT_REFUSED, "{o:?}");
            assert!(said(&o, "did not finish"), "{o:?}");
            assert!(said(&o, "nothing was asked"), "{o:?}");
            assert_eq!(events(&h), ev, "no port call");
            assert_eq!(state_of(h.tmp.path()), tree, "nothing was written");
        }

        #[test]
        fn ck_03_a_normal_check_is_unchanged_and_reads_only_and_the_seam_alone_changes_nothing() {
            for use_seam in [false, true] {
                let h = staged_harness(RC2, files(RC2), |_| {});
                let tree = state_of(h.tmp.path());
                let calls = Cell::new(0u32);
                let count = || calls.set(calls.get() + 1);
                let o = if use_seam {
                    run_hooked(
                        &h,
                        &["--check"],
                        &Hooks {
                            before_late_marker: Some(&count),
                            ..Default::default()
                        },
                    )
                } else {
                    run(&h, &["--check"])
                };
                assert_eq!(o.code, EXIT_NEWER, "{use_seam}: {o:?}");
                assert!(said(&o, "a newer release is available"), "{o:?}");
                assert!(started(&h).is_empty(), "{use_seam}: {:?}", started(&h));
                assert_eq!(
                    state_of(h.tmp.path()),
                    tree,
                    "{use_seam}: a check writes nothing"
                );
                assert_eq!(calls.get(), u32::from(use_seam), "the seam is reached once");
            }
        }

        #[test]
        fn ck_04_a_normal_resume_still_finishes_the_interrupted_upgrade() {
            let h = lost_after_the_flip();
            let before = started(&h).len();
            let o = run(&h, &[]);
            assert_eq!(o.code, EXIT_DONE, "{o:?}");
            assert!(
                said(
                    &o,
                    &format!("finished the interrupted upgrade {RC1} -> {RC2}")
                ),
                "{o:?}"
            );
            assert!(!marker(&h).exists(), "the marker is gone");
            let after = started(&h);
            assert!(
                after.len() > before && after[before..].iter().any(|e| e.contains(RC2)),
                "the resume started the new release: {after:?}"
            );
            assert!(
                after[before..].iter().any(|e| e.starts_with("run_roles")),
                "and ran the roles step: {after:?}"
            );
        }

        #[test]
        fn ck_05_a_normal_upgrade_still_starts_the_new_release() {
            let h = staged_harness(RC2, files(RC2), |_| {});
            let o = run(&h, &[]);
            assert_eq!(o.code, EXIT_DONE, "{o:?}");
            assert!(!marker(&h).exists(), "no marker is left");
            assert!(
                started(&h).iter().any(|e| e.contains(RC2)),
                "{:?}",
                started(&h)
            );
            assert!(
                started(&h).iter().any(|e| e.starts_with("run_roles")),
                "{:?}",
                started(&h)
            );
        }
    }

    // ---- R-4a (cox W1-65, W1-66, W1-67, W1-91): at the TOP of `restart`, before anything starts, the release directory is verified (marv's
    // `verify_release_dir_bytes`) and the roles step runs from a private STAGE built from the verified bytes.
    mod r4a {
        use super::*;
        use cortex_upgrade::layout::CommitStep;
        use std::os::unix::fs::MetadataExt;
        use std::path::{Path, PathBuf};

        /// What the roles step saw in the directory it was given, at the moment it was called.
        #[derive(Debug, Clone)]
        struct Seen {
            dir: PathBuf,
            listing: Vec<String>,
            script: Option<(u32, Vec<u8>)>,
            compose: Option<(u32, Vec<u8>)>,
            dir_mode: u32,
        }

        fn snapshot(d: &Path) -> Seen {
            let mut listing: Vec<String> = fs::read_dir(d)
                .map(|r| {
                    r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            listing.sort();
            let file = |n: &str| {
                let p = d.join(n);
                fs::symlink_metadata(&p)
                    .ok()
                    .map(|m| (m.mode() & 0o7777, fs::read(&p).unwrap_or_default()))
            };
            Seen {
                dir: d.to_path_buf(),
                listing,
                script: file("db-roles.sh"),
                compose: file("compose.yml"),
                dir_mode: fs::symlink_metadata(d)
                    .map(|m| m.mode() & 0o7777)
                    .unwrap_or(0),
            }
        }

        fn record(h: &Harness) -> Arc<Mutex<Vec<Seen>>> {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let s2 = seen.clone();
            *h.containers.on_roles.lock().unwrap() = Some(Arc::new(move |d: &Path| {
                s2.lock().unwrap().push(snapshot(d));
            }));
            seen
        }

        fn stage_dir(h: &Harness) -> PathBuf {
            h.tmp.path().join(format!(".roles-stage-{RC2}"))
        }

        fn release_dir(h: &Harness) -> PathBuf {
            h.tmp.path().join("releases").join(RC2)
        }

        /// Run with `tamper` called right after the links were flipped (the new release is committed, `restart` is next).
        fn run_tampering(h: &Harness, tamper: &dyn Fn()) -> Outcome {
            let t: &dyn Fn(CommitStep) -> bool = &|s| {
                if s == CommitStep::CurrentSet {
                    tamper();
                }
                true
            };
            run_hooked(
                h,
                &[],
                &cortex_upgrade::engine::Hooks {
                    commit_step: Some(t),
                    ..Default::default()
                },
            )
        }

        fn no_start(h: &Harness) {
            let ev = events(h);
            assert!(
                !ev.iter()
                    .any(|e| e.starts_with("compose_up db") || e.starts_with("run_roles")),
                "nothing was started and no role script ran: {ev:?}"
            );
        }

        #[test]
        fn eb_01_a_changed_roles_script_or_compose_file_or_stack_env_is_refused_before_anything_starts()
         {
            for (what, file, bytes) in [
                (
                    "script",
                    "db-roles.sh",
                    b"#!/bin/sh\necho evil\n".as_slice(),
                ),
                ("compose.yml", "compose.yml", b"name: other\n".as_slice()),
                (
                    "stack.env",
                    "stack.env",
                    b"CORTEX_VERSION=9.9.9\n".as_slice(),
                ),
            ] {
                let h = staged_harness(RC2, files(RC2), |_| {});
                let seen = record(&h);
                let dir = release_dir(&h);
                let o = run_tampering(&h, &|| fs::write(dir.join(file), bytes).unwrap());
                assert_ne!(o.code, 0, "{what}: {o:?}");
                assert!(
                    o.lines
                        .iter()
                        .any(|l| l.contains("release verification refused") && l.contains(file)),
                    "{what}: the line names the file: {o:?}"
                );
                no_start(&h);
                assert!(
                    seen.lock().unwrap().is_empty(),
                    "{what}: the roles step never ran"
                );
                assert!(
                    !stage_dir(&h).exists(),
                    "{what}: no stage was made for a refused release"
                );
            }
        }

        #[test]
        fn eb_02_the_roles_step_runs_from_the_stage_which_holds_the_verified_files_and_is_gone_afterwards()
         {
            let h = staged_harness(RC2, files(RC2), |_| {});
            let seen = record(&h);
            let o = run(&h, &[]);
            assert_eq!(o.code, 0, "{o:?}");
            let s = seen.lock().unwrap().clone();
            assert_eq!(s.len(), 1, "one roles call: {s:?}");
            assert_eq!(
                s[0].dir,
                stage_dir(&h),
                "the roles step is given the stage, not the release directory"
            );
            assert_ne!(s[0].dir, release_dir(&h));
            assert_eq!(s[0].listing, ["compose.yml", "db-roles.sh"]);
            assert_eq!(s[0].dir_mode, 0o700);
            assert_eq!(s[0].script, Some((0o500, ROLES.to_vec())));
            assert_eq!(s[0].compose, Some((0o400, COMPOSE.as_bytes().to_vec())));
            assert!(
                !stage_dir(&h).exists(),
                "the stage is removed when restart returns"
            );
        }

        #[test]
        fn eb_03_both_attempts_of_the_roles_step_run_the_staged_copy_even_if_the_original_is_replaced_between_them()
         {
            let h = staged_harness(RC2, files(RC2), |_| {});
            *h.containers.roles_seq.lock().unwrap() = vec![Err(PortError::TimedOut), Ok(())].into();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let (s2, dir) = (seen.clone(), release_dir(&h));
            *h.containers.on_roles.lock().unwrap() = Some(Arc::new(move |d: &Path| {
                s2.lock().unwrap().push(snapshot(d));
                // the ORIGINAL is replaced during every call: the next call must still see the verified bytes in the stage
                let _ = fs::write(dir.join("db-roles.sh"), b"#!/bin/sh\necho evil\n");
            }));
            let o = run(&h, &[]);
            assert_eq!(o.code, 0, "{o:?}");
            let s = seen.lock().unwrap().clone();
            assert_eq!(s.len(), 2, "the run and its confirmation: {s:?}");
            assert_eq!(s[0].dir, s[1].dir, "ONE stage serves both attempts");
            assert_eq!(s[0].dir, stage_dir(&h));
            assert_eq!(
                s[1].script,
                Some((0o500, ROLES.to_vec())),
                "the second attempt still runs the verified bytes"
            );
            assert!(!stage_dir(&h).exists());
        }

        #[test]
        fn eb_04_the_stage_is_removed_after_a_refused_role_step_and_after_two_timeouts() {
            let h = staged_harness(RC2, files(RC2), |_| {});
            *h.containers.roles.lock().unwrap() = Err(PortError::Failed(OneLine::new("no")));
            let o = run(&h, &[]);
            assert_ne!(o.code, 0, "{o:?}");
            assert!(!stage_dir(&h).exists(), "after a refusal");
            let h = staged_harness(RC2, files(RC2), |_| {});
            *h.containers.roles_seq.lock().unwrap() =
                vec![Err(PortError::TimedOut), Err(PortError::TimedOut)].into();
            let o = run(&h, &[]);
            assert_ne!(o.code, 0, "{o:?}");
            assert!(!stage_dir(&h).exists(), "after two timeouts");
        }

        #[test]
        fn eb_05_a_home_writable_by_group_is_refused_before_anything_starts() {
            use std::os::unix::fs::PermissionsExt;
            let h = staged_harness(RC2, files(RC2), |_| {});
            let seen = record(&h);
            let home = h.tmp.path().to_path_buf();
            let o = run_tampering(&h, &|| {
                fs::set_permissions(&home, fs::Permissions::from_mode(0o775)).unwrap();
            });
            fs::set_permissions(h.tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
            assert_ne!(o.code, 0, "{o:?}");
            assert!(
                o.lines
                    .iter()
                    .any(|l| l.contains("writable by group or other") && l.contains("chmod")),
                "{o:?}"
            );
            no_start(&h);
            assert!(seen.lock().unwrap().is_empty());
        }

        #[test]
        fn eb_06_a_resumed_run_verifies_and_stages_too() {
            // a run lost right after the flip leaves the marker; the next run finishes it through `restart`: with a changed script it is refused,
            // with the good one it runs from the stage
            let h = staged_harness(RC2, files(RC2), |_| {});
            let lose: &dyn Fn(CommitStep) -> bool = &|s| {
                if s == CommitStep::CurrentSet {
                    panic!("process lost")
                }
                true
            };
            lost_at(
                &h,
                &cortex_upgrade::engine::Hooks {
                    commit_step: Some(lose),
                    ..Default::default()
                },
            );
            let seen = record(&h);
            let o = run(&h, &[]);
            assert_eq!(o.code, 0, "{o:?}");
            let s = seen.lock().unwrap().clone();
            assert_eq!(s.len(), 1, "{s:?}");
            assert_eq!(s[0].dir, stage_dir(&h), "the resumed run is staged as well");
            assert_eq!(s[0].script, Some((0o500, ROLES.to_vec())));
            // and a resumed run over a changed script is refused, never started
            let h = staged_harness(RC2, files(RC2), |_| {});
            lost_at(
                &h,
                &cortex_upgrade::engine::Hooks {
                    commit_step: Some(lose),
                    ..Default::default()
                },
            );
            fs::write(
                release_dir(&h).join("db-roles.sh"),
                b"#!/bin/sh\necho evil\n",
            )
            .unwrap();
            let seen = record(&h);
            let o = run(&h, &[]);
            assert_ne!(o.code, 0, "{o:?}");
            assert!(o.lines.iter().any(|l| l.contains("db-roles.sh")), "{o:?}");
            assert!(seen.lock().unwrap().is_empty());
        }
    }

    /// R-4b (cox, his ruling on sam's CP4, 8 Nov): a release whose project is not the fixed one must not reach `compose_up`, even though
    /// `run_roles` would refuse it later. The project is read from the VERIFIED bytes (the stage's copy of `compose.yml`) by the adapter's own
    /// function and compared with `cv2-s5` BEFORE the first compose call: one plain line, nothing started, the stage removed.
    mod r4b {
        use super::*;

        /// A release whose `compose.yml` carries `name_line` instead of `name: cv2-s5` (the checksums and the manifest are made to match:
        /// verification passes, so only the project can refuse). The `db` service is unchanged, so the engine's comparison with the running
        /// release (which covers the `db` service, not the project name) passes too.
        fn with_project(name_line: &str) -> Files {
            let mut f = files(RC2);
            let text = COMPOSE.replacen("name: cv2-s5", name_line, 1);
            assert_ne!(text, COMPOSE, "the project line was replaced");
            let at = f.iter().position(|e| e.0 == "compose.yml").unwrap();
            f[at].1 = text.into_bytes();
            f
        }

        /// What was STARTED for the new release before the refusal: any compose call that names it (the first is `compose_up db <release>`) and any roles
        /// step. The rollback after the refusal is expected to bring the previous release's server back (`compose_up server <previous>`), so it is not counted.
        fn started(h: &Harness) -> Vec<String> {
            events(h)
                .into_iter()
                .filter(|e| {
                    (e.starts_with("compose_up") && e.ends_with(RC2)) || e.starts_with("run_roles")
                })
                .collect()
        }

        fn stage_of(h: &Harness) -> std::path::PathBuf {
            h.tmp.path().join(format!(".roles-stage-{RC2}"))
        }

        fn listing(h: &Harness) -> Vec<String> {
            let mut v: Vec<String> = fs::read_dir(h.tmp.path())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }

        #[test]
        fn wp_01_a_valid_but_wrong_project_is_refused_before_the_first_compose_call() {
            for line in [
                "name: other-project",
                "name: cv2-s6",
                "name: cv2-s5x",
                "name: s5",
                "name: cortexv2",
            ] {
                let h = staged_harness(RC2, with_project(line), |_| {});
                let o = run(&h, &[]);
                assert_ne!(o.code, 0, "{line}: {o:?}");
                assert!(
                    o.lines.iter().any(|l| l
                        .contains("not of the project the roles script targets")
                        && l.contains("nothing was started")),
                    "{line}: one plain line, naming the cause: {o:?}"
                );
                assert!(
                    started(&h).is_empty(),
                    "{line}: no compose call, no roles step: {:?}",
                    started(&h)
                );
                assert!(!stage_of(&h).exists(), "{line}: the stage was removed");
            }
        }

        #[test]
        fn wp_02_a_project_that_is_not_a_valid_name_is_refused_the_same_way() {
            let long = format!("name: a{}", "b".repeat(32)); // 33 characters
            for (what, line) in [
                ("no name line", "# no name here".to_string()),
                ("a name without the space", "name:cv2-s5".to_string()),
                ("an upper case letter", "name: Cv2-s5".to_string()),
                ("a leading digit", "name: 2cv2-s5".to_string()),
                ("an underscore", "name: cv2_s5".to_string()),
                ("too long", long),
                ("an empty value", "name: ".to_string()),
                (
                    "a second name line",
                    "name: cv2-s5\nname: cv2-s5".to_string(),
                ),
            ] {
                let h = staged_harness(RC2, with_project(&line), |_| {});
                let o = run(&h, &[]);
                assert_ne!(o.code, 0, "{what}: {o:?}");
                assert!(
                    o.lines
                        .iter()
                        .any(|l| (l.contains("does not name a valid project")
                            || l.contains("not of the project the roles script targets"))
                            && l.contains("nothing was started")),
                    "{what}: {o:?}"
                );
                assert!(started(&h).is_empty(), "{what}: {:?}", started(&h));
                assert!(!stage_of(&h).exists(), "{what}: the stage was removed");
            }
        }

        #[test]
        fn wp_03_the_guard_writes_nothing_more_than_a_refusal_of_the_verification_does() {
            // the two refusals sit at the top of `restart`: after the wrong-project one the home holds exactly what it holds after a refusal
            // of the verification (a changed script), apart from the nothing that the stage's removal leaves
            let wrong = staged_harness(RC2, with_project("name: other-project"), |_| {});
            let _ = run(&wrong, &[]);
            let tampered = staged_harness(RC2, files(RC2), |_| {});
            let dir = tampered.tmp.path().join("releases").join(RC2);
            let o = {
                let t: &dyn Fn(cortex_upgrade::layout::CommitStep) -> bool = &|s| {
                    if s == cortex_upgrade::layout::CommitStep::CurrentSet {
                        fs::write(dir.join("db-roles.sh"), b"#!/bin/sh\necho evil\n").unwrap();
                    }
                    true
                };
                run_hooked(
                    &tampered,
                    &[],
                    &cortex_upgrade::engine::Hooks {
                        commit_step: Some(t),
                        ..Default::default()
                    },
                )
            };
            assert_ne!(o.code, 0, "{o:?}");
            assert_eq!(
                listing(&wrong),
                listing(&tampered),
                "the same files are left by the two refusals"
            );
        }

        /// R-4b, CP1 to CP3 (sam's COMBINED-PATH-GAPS; cox 8 Nov): the REAL `Engine::run` with the REAL `RealContainers` and a RECORDING FAKE `podman`
        /// (a shell script: it logs each compose call with whether the stage exists at that moment, and answers `load`, `image inspect`, `inspect`
        /// and `compose` from control files). The staged roles script is a real script that logs where it ran from and obeys `ctl/roles_mode`.
        /// No database, no container, no network: the ports that are not the adapter are the harness's fakes. These cells exercise code that
        /// exists, so they were not RED first; they are the evidence before the first REAL run on the clean VM.
        mod combined {
            use super::*;
            use cortex_upgrade::engine::{Args, Hooks};
            use cortex_upgrade::layout::CommitStep;
            use cortex_upgrade::real::{ComposeEnv, Deadlines, RealContainers};
            use std::os::unix::fs::PermissionsExt;
            use std::path::PathBuf;
            use std::time::Duration;

            /// The staged roles script: logs `ROLES from=<its directory> args=<arguments>` and obeys `ctl/roles_mode`.
            const SCRIPT: &str = r#"#!/bin/sh
d=$(dirname "$0")
echo "ROLES from=$d args=$*" >> "@EV@"
mode=ok; [ -e "@CTL@/roles_mode" ] && mode=$(cat "@CTL@/roles_mode")
case "$mode" in
  ok) exit 0 ;;
  hang_first) if [ ! -e "@CTL@/first_done" ]; then : > "@CTL@/first_done"; exec sleep 30; fi; exit 0 ;;
  hang_always) exec sleep 60 ;;
  trap) mkdir "$d/trap" && : > "$d/trap/file" && chmod 500 "$d/trap"; exit 0 ;;
esac
exit 0
"#;

            /// The fake `podman`: every compose call is logged with `stage=present|absent`; control files in `ctl/` steer it.
            const PODMAN: &str = r#"#!/bin/sh
H="@HOME@"
case "$1" in
  load) echo "Loaded image: localhost/cortex-server@@DIGEST@" ;;
  image) echo "@DIGEST@" ;;
  compose)
    stage=absent; ls -d "$H"/.roles-stage-* >/dev/null 2>&1 && stage=present
    echo "COMPOSE stage=$stage args=$*" >> "@EV@"
    if [ -e "@CTL@/compose_fail" ]; then
      w=$(cat "@CTL@/compose_fail"); for a in "$@"; do [ "$a" = "$w" ] && { echo boom >&2; exit 1; }; done
    fi
    if [ -e "@CTL@/replace_original" ]; then
      printf '#!/bin/sh\necho EVIL >> "@EV@"\nexit 0\n' > "$H/releases/@RC2@/db-roles.sh"
    fi
    exit 0 ;;
  inspect)
    name=$4
    if [ -e "@CTL@/inspect_$name" ]; then cat "@CTL@/inspect_$name"
    else case "$name" in *-migrate) echo "$name|exited|0|" ;; *) echo "$name|running|0|healthy" ;; esac; fi ;;
esac
exit 0
"#;

            struct Rig {
                h: Harness,
                real: RealContainers,
                fx: Tmp,
            }

            impl Rig {
                fn ev_path(&self) -> PathBuf {
                    self.fx.path().join("events.log")
                }
                fn ctl(&self, name: &str, text: &str) {
                    fs::write(self.fx.path().join("ctl").join(name), text).unwrap();
                }
                fn events(&self) -> Vec<String> {
                    fs::read_to_string(self.ev_path())
                        .unwrap_or_default()
                        .lines()
                        .map(str::to_owned)
                        .collect()
                }
                fn home(&self) -> PathBuf {
                    self.h.tmp.path().to_path_buf()
                }
                fn stage(&self) -> PathBuf {
                    self.home().join(format!(".roles-stage-{RC2}"))
                }
                fn release_dir(&self) -> PathBuf {
                    self.home().join("releases").join(RC2)
                }
            }

            fn rig() -> Rig {
                rig_for(None)
            }

            /// `project`: a replacement for the release's `name: cv2-s5` line (the checksums and the manifest are made to match, so only the project can
            /// refuse); `None` is the release as it is.
            fn rig_for(project: Option<&str>) -> Rig {
                let fx = Tmp::new("r4b-fx");
                fs::create_dir_all(fx.path().join("ctl")).unwrap();
                let ev = fx.path().join("events.log").display().to_string();
                let ctl = fx.path().join("ctl").display().to_string();
                let script = SCRIPT.replace("@EV@", &ev).replace("@CTL@", &ctl);
                let mut f = files(RC2);
                let at = f.iter().position(|e| e.0 == "db-roles.sh").unwrap();
                f[at].1 = script.clone().into_bytes();
                if let Some(line) = project {
                    let at = f.iter().position(|e| e.0 == "compose.yml").unwrap();
                    let text = COMPOSE.replacen("name: cv2-s5", line, 1);
                    assert_ne!(text, COMPOSE, "the project line was replaced");
                    f[at].1 = text.into_bytes();
                }
                let digest = digest_text(script.as_bytes());
                let h = staged_harness(RC2, f, |m| {
                    m["cortex"]["roles_script_digest"] = Value::String(digest.clone());
                });
                let podman = fx.path().join("podman");
                let text = PODMAN
                    .replace("@HOME@", &h.tmp.path().display().to_string())
                    .replace("@EV@", &ev)
                    .replace("@CTL@", &ctl)
                    .replace("@DIGEST@", DIGEST_A)
                    .replace("@RC2@", RC2);
                fs::write(&podman, text).unwrap();
                fs::set_permissions(&podman, fs::Permissions::from_mode(0o755)).unwrap();
                let real = RealContainers::with(
                    podman.as_os_str(),
                    Deadlines {
                        load: Duration::from_secs(20),
                        up: Duration::from_secs(20),
                        inspect: Duration::from_secs(20),
                    },
                    ComposeEnv::new(h.tmp.path().to_path_buf(), 8601),
                );
                Rig { h, real, fx }
            }

            fn run_real(r: &Rig, a: &Args, hooks: &Hooks) -> Outcome {
                let h = &r.h;
                let fetcher = Fetcher::new(HostT(h.host.clone()), FetchConfig::default(), None);
                Engine {
                    env: &h.env,
                    fetcher: &fetcher,
                    stack: &h.stack,
                    psql: &h.psql,
                    containers: &r.real,
                    dumper: &h.dumper,
                    pace: &h.pace,
                    clock: &h.clock,
                }
                .run_hooked(a, hooks)
            }

            fn kinds(ev: &[String]) -> Vec<&str> {
                ev.iter()
                    .map(|l| l.split_whitespace().next().unwrap_or(""))
                    .collect()
            }

            /// What was started for the NEW release: any roles step and any compose call on its directory. The rollback after a refusal is expected to
            /// bring the previous release back (a compose call on the previous release's directory), so it is not counted.
            fn started_new(r: &Rig) -> Vec<String> {
                let new_dir = format!("releases/{RC2}/");
                r.events()
                    .into_iter()
                    .filter(|l| {
                        l.starts_with("ROLES") || (l.starts_with("COMPOSE") && l.contains(&new_dir))
                    })
                    .collect()
            }

            /// Every roles line says it ran from the STAGE, never from the release directory.
            fn only_from_the_stage(r: &Rig) {
                for l in r.events().iter().filter(|l| l.starts_with("ROLES")) {
                    assert!(
                        l.starts_with(&format!("ROLES from={} ", r.stage().display())),
                        "the roles script ran from the stage: {l}"
                    );
                }
                assert!(
                    !r.events().iter().any(|l| l.contains("EVIL")),
                    "the original, replaced script never ran: {:?}",
                    r.events()
                );
            }

            fn lose_after_flip() -> impl Fn(CommitStep) -> bool {
                |s| {
                    if s == CommitStep::CurrentSet {
                        panic!("process lost")
                    }
                    true
                }
            }

            #[test]
            fn cb_01_a_first_run_verifies_and_stages_before_the_first_compose_and_runs_the_staged_script()
             {
                let r = rig();
                let o = run_real(&r, &args(&[]), &Hooks::default());
                assert_eq!(o.code, 0, "{o:?}");
                let ev = r.events();
                assert_eq!(kinds(&ev), ["COMPOSE", "ROLES", "COMPOSE"], "{ev:?}");
                let rel = r.release_dir().display().to_string();
                assert!(
                    ev[0].contains("stage=present"),
                    "the stage exists at the FIRST compose call: {}",
                    ev[0]
                );
                assert!(
                    ev[0].contains(&format!("-f {rel}/compose.yml"))
                        && ev[0].ends_with(" up -d db"),
                    "compose uses the verified release directory itself: {}",
                    ev[0]
                );
                assert!(
                    ev[1].starts_with(&format!(
                        "ROLES from={} args=--deadline ",
                        r.stage().display()
                    )) && ev[1].ends_with(" audit tokens"),
                    "the roles step ran the STAGED script with the exact modules: {}",
                    ev[1]
                );
                assert!(
                    ev[2].contains("stage=present") && ev[2].ends_with(" up -d migrate server"),
                    "{}",
                    ev[2]
                );
                assert!(
                    !r.stage().exists(),
                    "the stage is gone when the run returns"
                );
            }

            #[test]
            fn cb_02_a_replacement_of_the_original_after_the_check_changes_nothing_through_the_real_adapter()
             {
                let r = rig();
                r.ctl("replace_original", "1"); // the first compose call replaces the ORIGINAL script, after the verification
                let o = run_real(&r, &args(&[]), &Hooks::default());
                assert_eq!(o.code, 0, "{o:?}");
                assert!(
                    fs::read(r.release_dir().join("db-roles.sh"))
                        .unwrap()
                        .starts_with(b"#!/bin/sh\necho EVIL"),
                    "the original really was replaced"
                );
                assert_eq!(
                    kinds(&r.events()),
                    ["COMPOSE", "ROLES", "COMPOSE"],
                    "{:?}",
                    r.events()
                );
                only_from_the_stage(&r);
            }

            #[test]
            fn cb_03_a_run_lost_after_the_flip_is_resumed_verified_and_staged_before_the_first_compose()
             {
                use std::panic::{AssertUnwindSafe, catch_unwind};
                let r = rig();
                let lose = lose_after_flip();
                let hooks = Hooks {
                    commit_step: Some(&lose),
                    ..Default::default()
                };
                let died = catch_unwind(AssertUnwindSafe(|| run_real(&r, &args(&[]), &hooks)));
                assert!(died.is_err(), "the run did not return");
                assert!(
                    r.events().is_empty(),
                    "nothing was started before the loss: {:?}",
                    r.events()
                );
                let o = run_real(&r, &args(&[]), &Hooks::default());
                assert_eq!(o.code, 0, "{o:?}");
                let ev = r.events();
                assert_eq!(kinds(&ev), ["COMPOSE", "ROLES", "COMPOSE"], "{ev:?}");
                assert!(
                    ev[0].contains("stage=present"),
                    "the resumed run staged first: {}",
                    ev[0]
                );
                only_from_the_stage(&r);
                assert!(!r.stage().exists());
            }

            #[test]
            fn cb_04_an_invalid_release_or_a_bad_ancestor_starts_nothing_through_the_real_adapter()
            {
                // a changed script
                let r = rig();
                let dir = r.release_dir();
                let tamper: &dyn Fn(CommitStep) -> bool = &|s| {
                    if s == CommitStep::CurrentSet {
                        fs::write(dir.join("db-roles.sh"), b"#!/bin/sh\necho EVIL\n").unwrap();
                    }
                    true
                };
                let o = run_real(
                    &r,
                    &args(&[]),
                    &Hooks {
                        commit_step: Some(tamper),
                        ..Default::default()
                    },
                );
                assert_ne!(o.code, 0, "{o:?}");
                assert!(
                    o.lines
                        .iter()
                        .any(|l| l.contains("release verification refused")
                            && l.contains("db-roles.sh")),
                    "{o:?}"
                );
                assert!(
                    started_new(&r).is_empty(),
                    "no compose call on the new release and no roles step: {:?}",
                    r.events()
                );
                assert!(!r.stage().exists());
                // a home writable by group
                let r = rig();
                let home = r.home();
                let loosen: &dyn Fn(CommitStep) -> bool = &|s| {
                    if s == CommitStep::CurrentSet {
                        fs::set_permissions(&home, fs::Permissions::from_mode(0o775)).unwrap();
                    }
                    true
                };
                let o = run_real(
                    &r,
                    &args(&[]),
                    &Hooks {
                        commit_step: Some(loosen),
                        ..Default::default()
                    },
                );
                fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
                assert_ne!(o.code, 0, "{o:?}");
                assert!(
                    o.lines
                        .iter()
                        .any(|l| l.contains("writable by group or other") && l.contains("chmod")),
                    "{o:?}"
                );
                assert!(started_new(&r).is_empty(), "{:?}", r.events());
            }

            #[test]
            fn cb_05_a_timeout_then_the_confirming_rerun_runs_the_same_stage_twice() {
                let r = rig();
                r.ctl("roles_mode", "hang_first");
                let a = Args {
                    roles_timeout: Duration::from_secs(1),
                    ..args(&[])
                };
                let o = run_real(&r, &a, &Hooks::default());
                assert_eq!(o.code, 0, "{o:?}");
                assert!(said(&o, "timed out once; a second run confirmed"), "{o:?}");
                let ev = r.events();
                assert_eq!(
                    kinds(&ev),
                    ["COMPOSE", "ROLES", "ROLES", "COMPOSE"],
                    "{ev:?}"
                );
                let from = |l: &str| l.split_whitespace().nth(1).unwrap().to_owned();
                assert_eq!(
                    from(&ev[1]),
                    from(&ev[2]),
                    "both attempts ran from the same stage: {ev:?}"
                );
                only_from_the_stage(&r);
                assert!(!r.stage().exists());
            }

            #[test]
            fn cb_06_two_timeouts_leave_the_outcome_unknown_the_marker_and_no_stage_and_no_rollback()
             {
                // SLOW BY DESIGN (about 50 seconds): the confirming re-run's deadline is the floor, 45 s of real time, and the engine has no clock seam
                let r = rig();
                r.ctl("roles_mode", "hang_always");
                let a = Args {
                    roles_timeout: Duration::from_secs(1),
                    ..args(&[])
                };
                let o = run_real(&r, &a, &Hooks::default());
                assert_eq!(o.code, 4, "{o:?}");
                assert!(
                    said(&o, "timed out twice") || said(&o, "roles are unknown"),
                    "{o:?}"
                );
                assert!(
                    r.home().join("upgrade.incomplete").is_file(),
                    "the marker stays"
                );
                let ev = r.events();
                assert_eq!(
                    kinds(&ev),
                    ["COMPOSE", "ROLES", "ROLES"],
                    "no rollback, no further compose: {ev:?}"
                );
                only_from_the_stage(&r);
                assert!(!r.stage().exists(), "the stage was dropped on return");
            }

            #[test]
            fn cb_07_the_stage_is_gone_after_every_way_out_and_the_original_script_never_runs() {
                // (what to set up, what the run must say). A roles script that EXITS NONZERO is not a case here: the real adapter reports any nonzero exit
                // as a timeout (it waits out the deadline, W1-49), so it ends as the confirmed-twice UNKNOWN that cb_06 covers.
                type Setup = fn(&Rig);
                let cases: Vec<(&str, Setup, &str)> = vec![
                    (
                        "compose fails on the database",
                        |r| r.ctl("compose_fail", "db"),
                        "compose failed",
                    ),
                    (
                        "the database never becomes healthy",
                        |r| r.ctl("inspect_cv2-s5-db", "cv2-s5-db|running|0|starting"),
                        "did not become healthy",
                    ),
                    (
                        "the migration fails",
                        |r| r.ctl("inspect_cv2-s5-migrate", "cv2-s5-migrate|exited|1|"),
                        "migrations did not finish",
                    ),
                    (
                        "the server never becomes healthy",
                        |r| r.ctl("inspect_cv2-s5-server", "cv2-s5-server|running|0|unhealthy"),
                        "server",
                    ),
                ];
                for (what, setup, says) in cases {
                    let r = rig();
                    // an unrelated file in the home must survive
                    fs::write(r.home().join("keep-me"), b"1").unwrap();
                    setup(&r);
                    let o = run_real(&r, &args(&[]), &Hooks::default());
                    assert_ne!(o.code, 0, "{what}: {o:?}");
                    assert!(said(&o, says), "{what}: {o:?}");
                    assert!(!r.stage().exists(), "{what}: the stage is gone");
                    assert!(
                        r.home().join("keep-me").is_file(),
                        "{what}: an unrelated file stays"
                    );
                    only_from_the_stage(&r);
                    assert!(
                        r.events().iter().filter(|l| l.starts_with("ROLES")).count() <= 2,
                        "{what}: {:?}",
                        r.events()
                    );
                }
            }

            #[test]
            fn cb_08_a_stage_that_cannot_be_removed_is_left_as_ruled_and_the_run_is_not_failed_by_it()
             {
                let r = rig();
                r.ctl("roles_mode", "trap"); // the staged script leaves a directory inside the stage that its owner cannot remove from
                let o = run_real(&r, &args(&[]), &Hooks::default());
                assert_eq!(o.code, 0, "the residue does not change the result: {o:?}");
                let stage = r.stage();
                assert!(
                    stage.is_dir(),
                    "the stage stays (one best-effort removal, a failure is swallowed)"
                );
                assert!(
                    stage.join("trap").is_dir(),
                    "what the removal could not take is still there"
                );
                // cleanup by exact path
                fs::set_permissions(stage.join("trap"), fs::Permissions::from_mode(0o700)).unwrap();
                fs::remove_dir_all(&stage).unwrap();
                assert!(!stage.exists());
            }

            // ---- sam's R4B-SAM01 (limit 9): the guard's refusal on BOTH sides of the engine's fork, through the real engine, the real adapter and the recording
            // fake `podman`. A refusal from `restart` goes to `fail_after_switch` on a first run (roll back, exit 3) and to `resume_failed` on a retry of an
            // interrupted upgrade (marker kept, nothing rolled back, exit 4). These cells exercise code that exists, so they were not RED first: their RED is
            // the same cells against the engine WITHOUT the guard, and the routing mutants GD-12 and GD-13 (receipts).

            /// The refusal profiles of the guard: (what, the replacement for the `name: cv2-s5` line, the words of the refusal): a valid but wrong
            /// project, the prefix case, and three names that are not valid.
            fn bad_projects() -> [(&'static str, &'static str, &'static str); 5] {
                const WRONG: &str = "not of the project the roles script targets";
                const INVALID: &str = "does not name a valid project";
                [
                    ("a valid but wrong project", "name: other-project", WRONG),
                    (
                        "a name that only starts with the fixed one",
                        "name: cv2-s5x",
                        WRONG,
                    ),
                    ("an upper case name", "name: Cv2-s5", INVALID),
                    ("no name line", "# no name here", INVALID),
                    ("two name lines", "name: cv2-s5\nname: cv2-s5", INVALID),
                ]
            }

            /// The refusal is ONE line: the step, the cause and "nothing was started".
            fn refused_by_the_guard(o: &Outcome, says: &str) {
                assert!(
                    o.lines
                        .iter()
                        .any(|l| l.contains("step 9 (restart) failed:")
                            && l.contains(says)
                            && l.ends_with("nothing was started")),
                    "{o:?}"
                );
            }

            /// A run lost right after the flip and before any step 9 call, then the retry: the id of the attempt (from the marker the lost run left) and the
            /// retry's outcome.
            fn lost_then_resumed(r: &Rig) -> (String, Outcome) {
                use std::panic::{AssertUnwindSafe, catch_unwind};
                let lose = lose_after_flip();
                let hooks = Hooks {
                    commit_step: Some(&lose),
                    ..Default::default()
                };
                let died = catch_unwind(AssertUnwindSafe(|| run_real(r, &args(&[]), &hooks)));
                assert!(died.is_err(), "the run did not return");
                assert!(
                    r.events().is_empty(),
                    "nothing was started before the loss: {:?}",
                    r.events()
                );
                let id = marker_id(&r.h);
                (id, run_real(r, &args(&[]), &Hooks::default()))
            }

            #[test]
            fn cb_09_a_first_run_of_another_project_is_refused_before_any_compose_and_rolled_back_exit_3()
             {
                // the control: the same fixture with the fixed project starts the release, so the project alone makes the difference
                let c = rig();
                let o = run_real(&c, &args(&[]), &Hooks::default());
                assert_eq!(o.code, 0, "{o:?}");
                assert_eq!(kinds(&c.events()), ["COMPOSE", "ROLES", "COMPOSE"]);
                for (what, line, says) in bad_projects() {
                    let r = rig_for(Some(line));
                    let o = run_real(&r, &args(&[]), &Hooks::default());
                    assert_eq!(o.code, 3, "{what}: {o:?}");
                    assert_safe(&o);
                    refused_by_the_guard(&o, says);
                    assert!(
                        said(&o, &format!("rolled back: {RC1} is running again")),
                        "{what}: {o:?}"
                    );
                    // the links (current is the old release, previous forgotten), the failed release kept, the dump set kept
                    assert_rolled_back(&r.h);
                    assert!(
                        !marker(&r.h).exists(),
                        "{what}: a successful rollback clears the marker"
                    );
                    // the only compose call is the rollback's, for the PREVIOUS release's server, made after the stage was dropped; nothing of the new release
                    let ev = r.events();
                    assert_eq!(kinds(&ev), ["COMPOSE"], "{what}: {ev:?}");
                    let old_file = format!("-f {}/releases/{RC1}/compose.yml", r.home().display());
                    assert!(
                        ev[0].contains("stage=absent")
                            && ev[0].contains(&old_file)
                            && ev[0].ends_with(" up -d server"),
                        "{what}: {}",
                        ev[0]
                    );
                    assert!(started_new(&r).is_empty(), "{what}: {ev:?}");
                    assert!(!r.stage().exists(), "{what}: the stage is gone");
                    // the record: one, rolled back, step 9 not ok
                    let recs = record_values(&r.h);
                    assert_eq!(recs.len(), 1, "{what}");
                    assert_eq!(recs[0]["result"].as_str(), Some("rolled_back"), "{what}");
                    assert_eq!(step_flags(&recs[0]).last(), Some(&(9, false)), "{what}");
                }
            }

            #[test]
            fn cb_10_a_resumed_run_of_another_project_is_refused_keeps_everything_and_rolls_nothing_back_exit_4()
             {
                // the control: the same fixture and the same loss with the fixed project; the retry finishes the upgrade
                let c = rig();
                let (_, o) = lost_then_resumed(&c);
                assert_eq!(o.code, 0, "{o:?}");
                assert_eq!(kinds(&c.events()), ["COMPOSE", "ROLES", "COMPOSE"]);
                for (what, line, says) in bad_projects() {
                    let r = rig_for(Some(line));
                    let (id, o) = lost_then_resumed(&r);
                    // exit 4, "nothing was rolled back", the two ways on, the marker, current the new release and previous the old one, the record `resume_failed`
                    assert_resume_stopped(&r.h, &o, "step 9 (restart) failed");
                    refused_by_the_guard(&o, says);
                    assert_eq!(
                        marker_id(&r.h),
                        id,
                        "{what}: the marker is still the same attempt's"
                    );
                    // nothing at all ran: no compose call (none on the new release, and no rollback compose for the previous one) and no roles step
                    assert!(r.events().is_empty(), "{what}: {:?}", r.events());
                    assert!(!r.stage().exists(), "{what}: the stage is gone");
                }
            }
        }
    }
}

/// R-6, stage D (cox W1-68, W1-91 Q1): the engine's `Env` carries the verified compose provider the entry read once; an upgrade or a rollback whose
/// provider is not an absolute path (a caller that did not read it) is refused before anything is written.
mod r6 {
    use super::*;
    use cortex_upgrade::engine::EXIT_REFUSED;
    use std::fs;
    use std::path::{Path, PathBuf};

    const LINE: &str = "no compose provider is set";

    fn listing(d: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn cp_01_an_upgrade_and_a_rollback_without_an_absolute_provider_are_refused_first_and_change_nothing()
     {
        for provider in [
            "",
            "podman-compose",
            "bin/podman-compose",
            "./podman-compose",
        ] {
            for list in [vec![], vec!["--rollback"]] {
                let mut h = harness("0.2.0", vec![]);
                h.env.compose_provider = PathBuf::from(provider);
                let before = listing(h.tmp.path());
                let o = run(&h, &list);
                assert_eq!(o.code, EXIT_REFUSED, "{provider:?} {list:?}: {o:?}");
                assert_eq!(o.lines.len(), 1, "{provider:?} {list:?}: {o:?}");
                assert!(
                    o.lines[0].starts_with(LINE) && o.lines[0].ends_with("nothing was changed"),
                    "{provider:?} {list:?}: {o:?}"
                );
                assert_eq!(
                    listing(h.tmp.path()),
                    before,
                    "{provider:?} {list:?}: no lock, no file"
                );
                assert!(
                    events(&h).is_empty(),
                    "{provider:?} {list:?}: no port was called: {:?}",
                    events(&h)
                );
            }
        }
    }

    #[test]
    fn cp_02_a_check_needs_no_provider_and_an_absolute_one_passes_the_guard_in_every_mode() {
        let mut h = harness("0.2.0", vec![]);
        h.env.compose_provider = PathBuf::new();
        let o = run(&h, &["--check"]);
        assert!(
            !o.lines.iter().any(|l| l.contains(LINE)),
            "a check starts no container: {o:?}"
        );
        for list in [vec![], vec!["--rollback"], vec!["--check"]] {
            let h = harness("0.2.0", vec![]);
            assert!(h.env.compose_provider.is_absolute());
            let o = run(&h, &list);
            assert!(!o.lines.iter().any(|l| l.contains(LINE)), "{list:?}: {o:?}");
        }
    }

    #[test]
    fn cp_03_the_provider_guard_comes_before_the_unit_directory_check() {
        let mut h = harness("0.2.0", vec![]);
        h.env.compose_provider = PathBuf::new();
        h.env.unit_dir = h.tmp.path().join("no-such-units");
        let o = run(&h, &[]);
        assert!(
            o.lines[0].starts_with(LINE),
            "the caller's omission is named first: {o:?}"
        );
        // with a provider the unit directory is what refuses (the guard did not mask it)
        let mut h = harness("0.2.0", vec![]);
        h.env.unit_dir = h.tmp.path().join("no-such-units");
        let o = run(&h, &[]);
        assert!(o.lines[0].contains("unit directory"), "{o:?}");
        assert!(!o.lines[0].starts_with(LINE), "{o:?}");
    }
}
