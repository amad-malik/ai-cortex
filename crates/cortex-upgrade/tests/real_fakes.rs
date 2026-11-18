//! Fakes and the allow-list for the real-ports cells (brief r1 sections 2, 3 and 5). Pulled into the other `real_*.rs`
//! files with `#[path = "real_fakes.rs"] mod fakes;`. No test calls a real `podman`, `podman compose`, `cortex-dump` or
//! script: every program is a fake shell script in a temporary directory that LOGS what it was started with (its
//! argument vector, one element per line, and the environment it was started with, read from `/proc/$$/environ`, which
//! is exactly what the wrapper passed).
#![allow(dead_code)]

#[path = "common/mod.rs"]
pub mod common;

use cortex_upgrade::real::ComposeEnv;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const SQL: &str =
    "select count(*) from pg_hba_file_rules where auth_method = 'trust' or error is not null";
pub const INSPECT_FORMAT: &str =
    "{{.Name}}|{{.State.Status}}|{{.State.ExitCode}}|{{with .State.Health}}{{.Status}}{{end}}";
pub const PROJECT: &str = "cortexv2";
/// The project `db-roles.sh` targets (its default container is `cv2-s5-db`); `run_roles` runs only for this one.
pub const ROLES_PROJECT: &str = "cv2-s5";
pub const PORT: u16 = 8611;
pub const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// One start of a fake, as it logged itself.
#[derive(Debug, Clone)]
pub struct Call {
    pub program: String,
    pub argv: Vec<String>,
    /// `NAME=value` lines of the environment the fake was started with.
    pub env: Vec<String>,
    /// The working directory it was started in.
    pub cwd: String,
}

impl Call {
    pub fn env_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .env
            .iter()
            .map(|l| l.split('=').next().unwrap_or("").to_string())
            .collect();
        v.sort();
        v
    }
}

/// The compose variables the cells give `RealContainers`: the home and the local port.
pub fn compose_env(home: &Path) -> ComposeEnv {
    ComposeEnv::new(home.to_path_buf(), PORT)
}

/// Writes an executable fake at `path`: it logs to `log`, then runs `body` (shell).
pub fn script(path: &Path, log: &Path, body: &str) {
    let text = format!(
        "#!/bin/sh\nLOG='{}'\n{{\n  echo CALL\n  printf 'CWD\\t%s\\n' \"$(pwd)\"\n  printf 'PROG\\t%s\\n' \"$0\"\n  for a in \"$@\"; do printf 'ARG\\t%s\\n' \"$a\"; done\n  tr '\\0' '\\n' < /proc/$$/environ | while IFS= read -r l; do printf 'ENV\\t%s\\n' \"$l\"; done\n  echo END\n}} >> \"$LOG\"\n{}\n",
        log.display(),
        body
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Every start the log holds, in order.
pub fn calls(log: &Path) -> Vec<Call> {
    let text = fs::read_to_string(log).unwrap_or_default();
    let mut out = Vec::new();
    let mut cur: Option<Call> = None;
    for line in text.lines() {
        if line == "CALL" {
            cur = Some(Call {
                program: String::new(),
                argv: Vec::new(),
                env: Vec::new(),
                cwd: String::new(),
            });
        } else if line == "END" {
            out.extend(cur.take());
        } else if let Some(c) = cur.as_mut() {
            if let Some(v) = line.strip_prefix("CWD\t") {
                c.cwd = v.to_string();
            } else if let Some(v) = line.strip_prefix("PROG\t") {
                c.program = v.to_string();
            } else if let Some(v) = line.strip_prefix("ARG\t") {
                c.argv.push(v.to_string());
            } else if let Some(v) = line.strip_prefix("ENV\t") {
                c.env.push(v.to_string());
            }
        }
    }
    out
}

/// A lone fake (no closed-list check): for the cells of the runner itself, which start a program with any vector.
pub fn standalone(tag: &str, body: &str) -> (common::Tmp, PathBuf, PathBuf) {
    let t = common::Tmp::new(tag);
    let log = t.path().join("calls.log");
    let p = t.path().join("fake");
    script(&p, &log, body);
    (t, p, log)
}

/// The paths of a `Harness` made by the parent test, for a CHILD test process that re-runs the test (it must not create
/// or remove anything).
pub struct Paths {
    pub base: PathBuf,
    pub home: PathBuf,
    pub release: PathBuf,
    pub podman: PathBuf,
    pub log: PathBuf,
}

impl Paths {
    pub fn at(base: &str) -> Paths {
        let base = PathBuf::from(base);
        Paths {
            home: base.join("home"),
            release: base.join("home/releases/0.2.0"),
            podman: base.join("fakes/podman"),
            log: base.join("calls.log"),
            base,
        }
    }
}

/// A temporary home with a release, a fake `podman`, and (on request) the fake dump tool and roles script.
pub struct Harness {
    pub tmp: common::Tmp,
    pub home: PathBuf,
    pub release: PathBuf,
    pub podman: PathBuf,
    pub log: PathBuf,
    /// Set by the ONE cell (`re_03`) that passes an archive path full of shell characters on purpose, to show that no
    /// shell reads it (R1): the allow-list then asks of that path only that it be absolute.
    pub hostile_archive_path: std::cell::Cell<bool>,
}

impl Harness {
    /// `podman_body` is the shell that decides what the fake `podman` answers.
    pub fn new(tag: &str, podman_body: &str) -> Harness {
        let tmp = common::Tmp::new(tag);
        let home = tmp.path().join("home");
        let release = home.join("releases/0.2.0");
        let log = tmp.path().join("calls.log");
        fs::create_dir_all(&release).unwrap();
        fs::write(
            release.join("compose.yml"),
            format!("name: {PROJECT}\nservices:\n  db:\n    image: x\n"),
        )
        .unwrap();
        fs::write(release.join("stack.env"), "CORTEX_VERSION=0.2.0\n").unwrap();
        let podman = tmp.path().join("fakes/podman");
        script(&podman, &log, podman_body);
        Harness {
            tmp,
            home,
            release,
            podman,
            log,
            hostile_archive_path: std::cell::Cell::new(false),
        }
    }
    pub fn calls(&self) -> Vec<Call> {
        calls(&self.log)
    }
    /// The fake dump tool at `<home>/bin/cortex-dump`.
    pub fn dump_tool(&self, body: &str) -> PathBuf {
        let p = self.home.join("bin/cortex-dump");
        script(&p, &self.log, body);
        p
    }
    /// The fake roles script at `<release>/db-roles.sh`.
    /// A release with a roles script is a release of the roles project (`cv2-s5`): the compose file is rewritten so.
    pub fn roles_script(&self, body: &str) -> PathBuf {
        self.set_compose(&format!(
            "name: {ROLES_PROJECT}\nservices:\n  db:\n    image: x\n"
        ));
        let p = self.release.join("db-roles.sh");
        script(&p, &self.log, body);
        p
    }
    pub fn set_compose(&self, text: &str) {
        fs::write(self.release.join("compose.yml"), text).unwrap();
    }
    pub fn ctr(&self, service: &str) -> String {
        format!("{PROJECT}-{service}")
    }
}

impl Drop for Harness {
    /// R3: every vector ANY fake of ANY test logged must fit the closed list.
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        for c in self.calls() {
            check_closed_with(
                &c,
                &self.home,
                &self.release,
                self.hostile_archive_path.get(),
            );
        }
    }
}

const FORBIDDEN: [&str; 14] = [
    "prune", "rm", "kill", "stop", "down", "--all", "-a", "--force", "volume", "system", "network",
    "secret", "pod", "image",
];

fn ref_ok(r: &str) -> bool {
    let b = r.as_bytes();
    !b.is_empty()
        && b.len() <= 256
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || b"._:/@-".contains(c))
}

fn ctr_ok(c: &str) -> bool {
    ["db", "migrate", "server"].iter().any(|s| {
        c.strip_suffix(&format!("-{s}")).is_some_and(|p| {
            !p.is_empty()
                && p.len() <= 32
                && p.as_bytes()[0].is_ascii_lowercase()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
    })
}

/// A dump label as `DumpLabel` makes it, `pre-upgrade-<from>-to-<to>` from two checked versions: letters, digits and `. - +`,
/// at most 144 characters (12 + 64 + 4 + 64), a version on each side of a `-to-` (vera's UPREAL-L4: not only the prefix).
fn label_ok(l: &str) -> bool {
    let Some(rest) = l.strip_prefix("pre-upgrade-") else {
        return false;
    };
    l.len() <= 144
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
        && rest.match_indices("-to-").any(|(i, _)| {
            let (from, to) = (&rest[..i], &rest[i + 4..]);
            !from.is_empty() && !to.is_empty()
        })
}

/// An archive path as our own code can produce it: absolute, and made of letters, digits and `. _ / - +` (the install's
/// home is limited to letters, digits and `. _ / -` by install.sh, a version to SemVer characters). A glob character, a
/// space or a shell character cannot come from a good install, so it is refused (hue, brief r3).
fn path_ok(p: &str) -> bool {
    p.starts_with('/')
        && p.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._/-+".contains(&b))
}

/// The indexes of the VALUE positions of a vector: the data in it (paths, names, references, ids, the SQL), as opposed
/// to the subcommands and flags. A position that is not listed is a command or operator position.
fn value_slots(name: &str, a: &[&str]) -> Vec<usize> {
    match (name, a) {
        ("podman", ["load", "--input", _]) => vec![2],
        ("podman", ["image", "inspect", "--format", "{{.Digest}}", _]) => vec![3, 4],
        ("podman", ["compose", "-f", _, "--env-file", _, "up", "-d", rest @ ..]) => {
            let mut v = vec![2, 4];
            v.extend(7..7 + rest.len());
            v
        }
        ("podman", ["inspect", "--format", _, _]) => vec![2, 3],
        (
            "podman",
            [
                "exec",
                "-i",
                "--user",
                "postgres",
                _,
                "psql",
                "-U",
                "postgres",
                "-d",
                "cortex",
                "-tAc",
                _,
            ],
        ) => {
            vec![4, 11]
        }
        ("cortex-dump", ["--target", _, "--out", _, "--label", _, "--timeout", _]) => {
            vec![1, 3, 5, 7]
        }
        ("db-roles.sh", ["--deadline", _, ids @ ..]) => (1..2 + ids.len()).collect(),
        _ => vec![],
    }
}

/// The allow-list of brief section 3, as ONE helper. Panics with the offending vector.
pub fn check_closed(c: &Call, home: &Path, dir: &Path) {
    check_closed_with(c, home, dir, false)
}

/// [`check_closed`]; `hostile_archive_path` loosens ONE value shape (the archive path need only be absolute), for the cell
/// that passes shell characters on purpose.
pub fn check_closed_with(c: &Call, home: &Path, dir: &Path, hostile_archive_path: bool) {
    let a: Vec<&str> = c.argv.iter().map(String::as_str).collect();
    let fail = |why: &str| -> ! { panic!("R3: {why}: {} {:?}", c.program, c.argv) };
    check_env(c, home);
    let prog = Path::new(&c.program);
    let name = prog.file_name().and_then(|n| n.to_str()).unwrap_or("");
    // R3 constrains the COMMAND and OPERATOR positions (hue, brief r3): no forbidden word, no glob character there
    // (`image` only in its one shape). A VALUE position is data and is held to its own shape in the match below.
    let image_shape = name == "podman" && a.len() == 5 && a[0] == "image" && a[1] == "inspect";
    let values = value_slots(name, &a);
    for (i, e) in a.iter().enumerate() {
        let value = values.contains(&i);
        if !value && e.contains(['*', '?', '[']) {
            fail("a wildcard element");
        }
        if !value && FORBIDDEN.contains(e) && !(image_shape && i == 0 && *e == "image") {
            fail("a forbidden element");
        }
    }
    let cs = |s: &str| format!("{}/{}", dir.display(), s);
    match name {
        "podman" => {
            let ok = match a.as_slice() {
                ["load", "--input", f] => {
                    if hostile_archive_path {
                        f.starts_with('/')
                    } else {
                        path_ok(f)
                    }
                }
                ["image", "inspect", "--format", "{{.Digest}}", r] => ref_ok(r),
                ["compose", "-f", f, "--env-file", e, "up", "-d", svcs @ ..] => {
                    *f == cs("compose.yml")
                        && *e == cs("stack.env")
                        && !svcs.is_empty()
                        && svcs.iter().all(|s| ["db", "migrate", "server"].contains(s))
                }
                ["inspect", "--format", fmt, ctr] => *fmt == INSPECT_FORMAT && ctr_ok(ctr),
                [
                    "exec",
                    "-i",
                    "--user",
                    "postgres",
                    ctr,
                    "psql",
                    "-U",
                    "postgres",
                    "-d",
                    "cortex",
                    "-tAc",
                    sql,
                ] => ctr_ok(ctr) && *sql == SQL,
                _ => false,
            };
            if !ok {
                fail("not in the closed list (podman)");
            }
        }
        "cortex-dump" => {
            let ok = c.program == format!("{}/bin/cortex-dump", home.display())
                && match a.as_slice() {
                    [
                        "--target",
                        ctr,
                        "--out",
                        out,
                        "--label",
                        label,
                        "--timeout",
                        n,
                    ] => {
                        ctr_ok(ctr)
                            && *out == format!("{}/dumps", home.display())
                            && label_ok(label)
                            && !n.is_empty()
                            && n.bytes().all(|b| b.is_ascii_digit())
                    }
                    _ => false,
                };
            if !ok {
                fail("not in the closed list (cortex-dump)");
            }
        }
        "db-roles.sh" => {
            // db-roles.sh --deadline <E> <module id>...  (hue's design r1, cox W1-49): E is the engine's absolute deadline in
            // epoch seconds, digits only, no leading zero, at most 11; at least one module id.
            let ok = c.program == cs("db-roles.sh")
                && match a.as_slice() {
                    ["--deadline", e, ids @ ..] => {
                        !e.is_empty()
                            && e.len() <= 11
                            && !e.starts_with('0')
                            && e.bytes().all(|b| b.is_ascii_digit())
                            && !ids.is_empty()
                            && ids.iter().all(|m| {
                                let b = m.as_bytes();
                                !b.is_empty()
                                    && b.len() <= 32
                                    && b[0].is_ascii_lowercase()
                                    && b.iter().all(|c| {
                                        c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_'
                                    })
                            })
                    }
                    _ => false,
                };
            if !ok {
                fail("not in the closed list (db-roles.sh)");
            }
        }
        // The fake `sh` of the R1 cell must never be started at all.
        _ => fail("a program outside the closed list of three"),
    }
}

/// R2 (+ hue's compose variables), as one helper too: a child's environment is the four fixed names (`XDG_RUNTIME_DIR`
/// only when the test process had it), and the COMPOSE child alone has exactly three more, with the values the
/// constructor was given.
pub fn check_env(c: &Call, home: &Path) {
    let names = c.env_names();
    let fail = |why: &str| -> ! { panic!("R2: {why}: {} {:?}: {:?}", c.program, c.argv, c.env) };
    let is_compose = c.argv.first().map(String::as_str) == Some("compose");
    // R-6: a compose child may ALSO carry the verified provider, `PODMAN_COMPOSE_PROVIDER=<absolute path>`; no other child may
    let provider_ok = is_compose
        && c.env
            .iter()
            .filter(|l| l.starts_with("PODMAN_COMPOSE_PROVIDER="))
            .count()
            <= 1;
    let mut base: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| !n.starts_with("CORTEX_"))
        .filter(|n| !(provider_ok && *n == "PODMAN_COMPOSE_PROVIDER"))
        .collect();
    base.sort();
    let allowed_base = |b: &[&str]| {
        b.iter()
            .all(|n| ["HOME", "LANG", "PATH", "XDG_RUNTIME_DIR"].contains(n))
            && b.contains(&"LANG")
            && b.contains(&"PATH")
            && b.contains(&"HOME")
    };
    if !allowed_base(&base) {
        fail("a name outside the four fixed ones");
    }
    let cortex: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| n.starts_with("CORTEX_"))
        .collect();
    if !is_compose {
        if !cortex.is_empty() {
            fail("a CORTEX_ name in a child that is not compose");
        }
        return;
    }
    if cortex != ["CORTEX_CONFIG", "CORTEX_DUMP_STATUS", "CORTEX_PORT"] {
        fail("the compose child does not have exactly the three variables");
    }
    if let Some(v) = c
        .env
        .iter()
        .find_map(|l| l.strip_prefix("PODMAN_COMPOSE_PROVIDER="))
        && (!v.starts_with('/') || v.chars().any(char::is_control))
    {
        fail("the compose provider is not an absolute path without control characters");
    }
    let get = |k: &str| {
        c.env
            .iter()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or("")
    };
    // The cells that give another home or port (rc_19, rc_22) check their own values; the names are always checked.
    let ours = get("CORTEX_CONFIG").starts_with(&home.display().to_string());
    if ours
        && (get("CORTEX_CONFIG") != format!("{}/cortex.toml", home.display())
            || get("CORTEX_DUMP_STATUS") != format!("{}/dump-status", home.display())
            || get("CORTEX_PORT") != PORT.to_string())
    {
        fail("a compose variable does not hold the constructor's value");
    }
}
