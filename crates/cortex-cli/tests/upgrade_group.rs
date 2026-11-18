//! `cortex upgrade` (piece 14, the entry point; cox W1-79 and W1-80): the rules of `src/groups/upgrade.rs`, as PURE functions of what
//! the caller read. install.sh (`deploy/install.sh`, wave1/s5 6e1d4d2 lines 398-399, 5t b0390dd lines 248-249):
//! `case ${HOME:-} in /*) ;; *) die "HOME must be an absolute path ..." ;; esac; unit_dir="$HOME/.config/systemd/user"`. No
//! `XDG_CONFIG_HOME`. A refusal names a rule and never echoes the value. Nothing is read from the process and no file is looked at.
//!
//! Order of the checks (one fault at a time is the normal case; `ee_09` pins it): the units directory (HOME), the home, the port,
//! the token file, the manifest URL.

use cortex_cli::groups::upgrade::{
    Entry, Guarded, LOCAL_REQUEST_TIMEOUT, RELEASE_REQUEST_TIMEOUT, Vars, compose_provider_for,
    engine_env, engine_parts, entry_from, local_stack, local_work_pending, real_ports, run_with,
    run_with_transport, to_output, translate_offline, unit_dir_from,
};
use cortex_upgrade::engine::{EXIT_REFUSED, Mode, Outcome, parse_args};
use cortex_upgrade::fetch::{FetchError, Request, Response, Transport};
use cortex_upgrade::members::check_home;
use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const NOT_ABSOLUTE: &str =
    "HOME must be an absolute path (the units go to $HOME/.config/systemd/user)";
const NOT_UTF8: &str = "HOME is not valid UTF-8";
const ALPHABET: &str = "HOME holds a character other than letters, digits and . _ / - (it is written into systemd units)";
const EMPTY_COMPONENT: &str =
    "HOME has an empty component (a double slash, a trailing slash or the root alone)";
const DOTDOT: &str = "HOME has a .. component";

fn unit(home: &str) -> Result<PathBuf, String> {
    unit_dir_from(Some(OsStr::new(home)))
}

fn unit_bytes(home: &[u8]) -> Result<PathBuf, String> {
    unit_dir_from(Some(OsStr::from_bytes(home)))
}

#[test]
fn eu_01_the_unit_directory_is_home_plus_dot_config_systemd_user() {
    assert_eq!(
        unit("/home/dev"),
        Ok(PathBuf::from("/home/dev/.config/systemd/user"))
    );
    assert_eq!(
        unit("/home/a.b_c-d/x9"),
        Ok(PathBuf::from("/home/a.b_c-d/x9/.config/systemd/user")),
        "every allowed character, kept as written (no normalisation)"
    );
    assert_eq!(
        unit("/home/./dev"),
        Ok(PathBuf::from("/home/./dev/.config/systemd/user")),
        "a . component is allowed, as in the home rule"
    );
    assert_eq!(
        unit("/nonexistent/zz"),
        Ok(PathBuf::from("/nonexistent/zz/.config/systemd/user")),
        "pure: no file is looked at (the engine checks the directory before step 8)"
    );
}

#[test]
fn eu_02_home_missing_empty_or_relative_is_refused_with_install_shs_words() {
    assert_eq!(unit_dir_from(None), Err(NOT_ABSOLUTE.to_owned()));
    for h in [
        "",
        "home/dev",
        "./home",
        "~",
        "~/x",
        "dev",
        " /home/dev",
        "C:/x",
    ] {
        assert_eq!(unit(h), Err(NOT_ABSOLUTE.to_owned()), "{h:?}");
    }
}

#[test]
fn eu_03_a_character_outside_the_alphabet_is_refused() {
    for h in [
        "/home/de v",
        "/home/dev%h",
        "/home/$HOME",
        "/home/dev;x",
        "/home/dev\n",
        "/home/dev\t",
        "/home/dev\r",
        "/home/dev'",
        "/home/dev\"",
        "/home/dev\\x",
        "/home/dev*",
        "/home/dev?",
        "/home/dev[1]",
        "/home/dev|x",
        "/home/dev&x",
        "/home/dev`x`",
        "/home/dev:x",
        "/home/dev=x",
        "/home/dev@x",
        "/home/dev+x",
        "/home/dév",
        "/home/\u{202e}dev",
        "/home/dev\u{7f}",
    ] {
        assert_eq!(unit(h), Err(ALPHABET.to_owned()), "{h:?}");
    }
    assert_eq!(
        unit_bytes(b"/home/dev\0x"),
        Err(ALPHABET.to_owned()),
        "a NUL byte"
    );
}

#[test]
fn eu_04_a_home_that_is_not_utf8_is_refused() {
    assert_eq!(unit_bytes(b"/home/de\xffv"), Err(NOT_UTF8.to_owned()));
    assert_eq!(unit_bytes(b"/\xff"), Err(NOT_UTF8.to_owned()));
    assert_eq!(
        unit_bytes(b"\xff"),
        Err(NOT_ABSOLUTE.to_owned()),
        "install.sh's test (absolute) is made first: a first byte that is not / is a relative path"
    );
}

#[test]
fn eu_05_an_empty_component_a_trailing_slash_and_the_root_are_refused() {
    for h in [
        "/",
        "//",
        "/home//dev",
        "/home/dev/",
        "//home/dev",
        "/home/dev//",
    ] {
        assert_eq!(unit(h), Err(EMPTY_COMPONENT.to_owned()), "{h:?}");
    }
}

#[test]
fn eu_06_a_dot_dot_component_is_refused_whatever_its_place() {
    for h in [
        "/..",
        "/home/..",
        "/home/../root",
        "/../home/dev",
        "/home/dev/..",
    ] {
        assert_eq!(unit(h), Err(DOTDOT.to_owned()), "{h:?}");
    }
    for h in [
        "/home/...",
        "/home/..a",
        "/home/a..",
        "/home/.a",
        "/home/a..",
    ] {
        assert!(unit(h).is_ok(), "{h:?} is a name, not a .. component");
    }
}

/// The two rules must not drift: for every candidate, the entry accepts the home exactly when the engine's own
/// `members::check_home` accepts the unit directory it would make (the same alphabet, empty-component and `..` rules).
#[test]
fn eu_07_the_rule_is_the_one_of_the_engines_check_home() {
    let corpus: Vec<String> = [
        "/home/dev",
        "/",
        "//",
        "/home//dev",
        "/home/dev/",
        "/home/..",
        "/home/../x",
        "/home/...",
        "/home/a b",
        "/home/a%b",
        "/home/a$b",
        "/home/a;b",
        "/home/é",
        "/a",
        "/a/b/c-d_e.f",
        "/.hidden",
        "/..hidden",
        "/home/dev\n",
        "/home/a\u{202e}b",
        "/tmp/x\0y",
        "/a/./b",
        "/a/b/.",
        "/-",
        "/_",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    for h in &corpus {
        let made = format!("{h}/.config/systemd/user");
        let engine_ok = check_home(Path::new(&made)).is_ok();
        assert_eq!(
            unit(h).is_ok(),
            engine_ok,
            "{h:?}: the entry and members::check_home must agree on {made:?}"
        );
    }
}

const SENTINEL: &str = "SYNTHETIC-TOKEN-ENTRY";
const BASE: &str = "https://releases.example/cortex";
const MANIFEST: &str = "https://releases.example/cortex/releases/manifest.json";
const PORT_HINT: &str = "if you installed with --port, set CORTEX_PORT to it";
const NO_LOCATION: &str =
    "no download location is configured (set CORTEX_BASE_URL to an https URL)";

fn os(s: &str) -> Option<OsString> {
    Some(OsString::from(s))
}

/// A complete, valid set; a cell changes ONE field.
fn base() -> Vars {
    Vars {
        home: os("/home/dev"),
        cortex_home: None,
        port: None,
        token_file: os("/home/dev/cortex-v2/lead.token"),
        base_url: os(BASE),
    }
}

fn err(v: &Vars) -> String {
    match entry_from(v, Mode::Upgrade) {
        Ok(e) => panic!("expected a refusal, got {e:?}"),
        Err(m) => {
            assert!(!m.contains(SENTINEL), "the refusal echoes the value: {m}");
            assert!(!m.contains('\n') && !m.is_empty(), "one plain line: {m:?}");
            m
        }
    }
}

fn ok(v: &Vars) -> Entry {
    entry_from(v, Mode::Upgrade)
        .unwrap_or_else(|m| panic!("expected an Entry, got the refusal {m:?}"))
}

#[test]
fn ee_01_the_defaults_are_install_shs() {
    let e = ok(&base());
    assert_eq!(e.home, PathBuf::from("/home/dev/cortex-v2"));
    assert_eq!(e.port, 8601);
    assert_eq!(e.local_url, "http://127.0.0.1:8601");
    assert_eq!(
        e.token_file,
        PathBuf::from("/home/dev/cortex-v2/lead.token")
    );
    assert_eq!(e.manifest_url.as_deref(), Some(MANIFEST));
    assert_eq!(e.unit_dir, PathBuf::from("/home/dev/.config/systemd/user"));
}

#[test]
fn ee_02_cortex_home_and_the_port_override_and_an_empty_value_is_unset() {
    let mut v = base();
    v.cortex_home = os("/srv/cortex.x-1/home");
    v.port = os("8777");
    let e = ok(&v);
    assert_eq!(e.home, PathBuf::from("/srv/cortex.x-1/home"));
    assert_eq!(e.port, 8777);
    assert_eq!(e.local_url, "http://127.0.0.1:8777");
    // `${VAR:-default}`: empty is the same as unset, as in install.sh
    let mut v = base();
    v.cortex_home = os("");
    v.port = os("");
    let e = ok(&v);
    assert_eq!(e.home, PathBuf::from("/home/dev/cortex-v2"));
    assert_eq!(e.port, 8601);
}

#[test]
fn ee_03_the_home_follows_install_shs_home_rule() {
    let fault = |h: &str| {
        let mut v = base();
        v.cortex_home = os(h);
        err(&v)
    };
    for h in ["home/x", "./x", "~/x"] {
        assert_eq!(fault(h), "CORTEX_HOME must be an absolute path", "{h:?}");
    }
    for h in [
        "/srv/a b", "/srv/a%b", "/srv/$x", "/srv/a;b", "/srv/é", "/srv/a\n", "/srv/a=b",
    ] {
        assert_eq!(
            fault(h),
            "CORTEX_HOME may contain only letters, digits and . _ / - (it is written into systemd units)",
            "{h:?}"
        );
    }
    for h in ["/", "/srv//x", "/srv/x/", "/srv/..", "/srv/../x", "/../srv"] {
        assert_eq!(
            fault(h),
            "CORTEX_HOME may not hold an empty or a .. component (a double slash, a trailing slash or a ..)",
            "{h:?}"
        );
    }
    let mut v = base();
    v.cortex_home = Some(OsStr::from_bytes(b"/srv/\xff").to_owned());
    assert_eq!(err(&v), "CORTEX_HOME is not valid UTF-8");
}

#[test]
fn ee_04_the_port_is_a_plain_number_that_is_not_the_v1_stacks() {
    let fault = |p: &str| {
        let mut v = base();
        v.port = os(p);
        err(&v)
    };
    for p in [
        "0", "65536", "99999", "123456", "08601", "00", "-1", "+1", "8601 ", " 8601", "8e3",
        "0x21", "86 01", "٣",
    ] {
        assert_eq!(
            fault(p),
            format!(
                "CORTEX_PORT must be a whole number from 1 to 65535 (digits only, no leading zero); {PORT_HINT}"
            ),
            "{p:?}"
        );
    }
    for p in ["8501", "5499", "5500", "8765", "8766"] {
        assert_eq!(
            fault(p),
            format!("CORTEX_PORT belongs to the live v1 stack; {PORT_HINT}"),
            "{p:?}"
        );
    }
    for (p, n) in [
        ("1", 1u16),
        ("65535", 65535),
        ("8601", 8601),
        ("8502", 8502),
    ] {
        let mut v = base();
        v.port = os(p);
        assert_eq!(ok(&v).port, n, "{p:?}");
    }
}

#[test]
fn ee_05_the_token_file_is_required_and_absolute_and_no_file_is_read() {
    let fault = |t: Option<&str>| {
        let mut v = base();
        v.token_file = t.map(OsString::from);
        err(&v)
    };
    assert_eq!(
        fault(None),
        "CORTEX_TOKEN_FILE must be set to the operator's token file (an absolute path)"
    );
    assert_eq!(
        fault(Some("")),
        "CORTEX_TOKEN_FILE must be set to the operator's token file (an absolute path)"
    );
    for t in ["lead.token", "./lead.token", "~/lead.token"] {
        assert_eq!(
            fault(Some(t)),
            "CORTEX_TOKEN_FILE must be set to the operator's token file (an absolute path)",
            "{t:?}"
        );
    }
    let mut v = base();
    v.token_file = Some(OsStr::from_bytes(b"/x/a\0b").to_owned());
    assert_eq!(err(&v), "CORTEX_TOKEN_FILE holds a NUL byte");
    // pure: a file that does not exist is not this function's business (the engine reads it, and says so)
    let mut v = base();
    v.token_file = os("/nonexistent/dir/token");
    assert_eq!(ok(&v).token_file, PathBuf::from("/nonexistent/dir/token"));
}

#[test]
fn ee_06_the_base_url_is_required_https_and_plain_and_makes_the_manifest_url() {
    let fault = |m: Option<&str>| {
        let mut v = base();
        v.base_url = m.map(OsString::from);
        err(&v)
    };
    // no default in the tree: unset or empty is the online path OFF. It is NOT a refusal of the entry (cox W1-88: local operations must
    // work): the entry has no manifest URL, in every mode; the operations that need one refuse later (`ne_02`, `ne_03`)
    for m in [None, Some("")] {
        for mode in [Mode::Upgrade, Mode::Check, Mode::Rollback] {
            let mut v = base();
            v.base_url = m.map(OsString::from);
            let e = entry_from(&v, mode).unwrap_or_else(|m| panic!("{mode:?}: {m}"));
            assert_eq!(e.manifest_url, None, "{m:?} {mode:?}");
        }
    }
    for m in [
        "http://releases.example/cortex",
        "ftp://x/m",
        "releases.example/cortex",
        "file:///etc/passwd",
        "HTTPS://releases.example/cortex",
        "//releases.example/cortex",
        "https://",
        "https:///cortex",
    ] {
        assert_eq!(
            fault(Some(m)),
            "CORTEX_BASE_URL must be an https URL",
            "{m:?}"
        );
    }
    for m in [
        "https://releases.example/a b",
        "https://releases.example/a\tb",
        "https://releases.example/a\nb",
        "https://releases.example/a\u{7f}",
        "https://releases.example/é",
        "https://releases.example/\u{202e}",
    ] {
        assert_eq!(
            fault(Some(m)),
            "CORTEX_BASE_URL holds a space, a control or a non-ASCII character",
            "{m:?}"
        );
    }
    let long = format!("https://releases.example/{}", "a".repeat(2000));
    assert_eq!(
        fault(Some(&long)),
        "CORTEX_BASE_URL is longer than 2000 bytes"
    );
    let at_limit = format!("https://releases.example/{}", "a".repeat(2000 - 25));
    assert_eq!(at_limit.len(), 2000);
    let mut v = base();
    v.base_url = Some(OsString::from(&at_limit));
    assert_eq!(
        ok(&v).manifest_url,
        Some(format!("{at_limit}/releases/manifest.json")),
        "exactly 2000 bytes is accepted"
    );
}

/// `manifest_url = ${CORTEX_BASE_URL%/}/releases/manifest.json` (cox W1-80): ONE trailing slash is removed, as the shell's `%/`
/// does, and nothing else is normalised.
#[test]
fn ee_12_the_manifest_url_is_the_base_without_one_trailing_slash_plus_releases_manifest_json() {
    let manifest = |b: &str| {
        let mut v = base();
        v.base_url = os(b);
        ok(&v).manifest_url.expect("a base URL is set")
    };
    assert_eq!(manifest(BASE), MANIFEST);
    assert_eq!(manifest("https://releases.example/cortex/"), MANIFEST);
    assert_eq!(
        manifest("https://releases.example/cortex//"),
        "https://releases.example/cortex//releases/manifest.json",
        "only ONE slash is removed (the shell's %/)"
    );
    assert_eq!(
        manifest("https://releases.example"),
        "https://releases.example/releases/manifest.json"
    );
    assert_eq!(
        manifest("https://releases.example/"),
        "https://releases.example/releases/manifest.json"
    );
    assert_eq!(
        manifest("https://127.0.0.1:57123/a/b"),
        "https://127.0.0.1:57123/a/b/releases/manifest.json"
    );
}

#[test]
fn ee_07_xdg_config_home_cannot_move_the_unit_directory() {
    // `Vars` has no field for it, so no caller can pass it; the same HOME always gives the same directory.
    let a = ok(&base());
    let mut v = base();
    v.home = os("/home/dev");
    assert_eq!(ok(&v).unit_dir, a.unit_dir);
    assert_eq!(a.unit_dir, PathBuf::from("/home/dev/.config/systemd/user"));
}

#[test]
fn ee_08_a_refusal_never_echoes_a_value() {
    // every field carries the sentinel in a way that makes the field invalid; `err` asserts the sentinel is absent and the
    // refusal must name the field it is about
    let bad = |f: &dyn Fn(&mut Vars), names: &str| {
        let mut v = base();
        f(&mut v);
        let m = err(&v);
        assert!(m.starts_with(names), "{m:?} should be about {names}");
    };
    bad(&|v| v.home = os(&format!("{SENTINEL} x")), "HOME ");
    bad(
        &|v| v.cortex_home = os(&format!("/{SENTINEL} x")),
        "CORTEX_HOME ",
    );
    bad(&|v| v.port = os(SENTINEL), "CORTEX_PORT ");
    bad(&|v| v.token_file = os(SENTINEL), "CORTEX_TOKEN_FILE ");
    bad(
        &|v| v.base_url = os(&format!("http://{SENTINEL}")),
        "CORTEX_BASE_URL ",
    );
    bad(
        &|v| v.base_url = os(&format!("https://{SENTINEL} x")),
        "CORTEX_BASE_URL ",
    );
}

#[test]
fn ee_09_the_order_of_the_checks_is_units_home_port_token_manifest() {
    let mut v = Vars {
        home: os("relative"),
        cortex_home: os("relative"),
        port: os("0"),
        token_file: os("relative"),
        base_url: os("http://x"),
    };
    assert!(err(&v).starts_with("HOME must be an absolute path"));
    v.home = os("/home/dev");
    assert!(err(&v).starts_with("CORTEX_HOME "));
    v.cortex_home = None;
    assert!(err(&v).starts_with("CORTEX_PORT "));
    v.port = None;
    assert!(err(&v).starts_with("CORTEX_TOKEN_FILE "));
    v.token_file = os("/t");
    assert!(err(&v).starts_with("CORTEX_BASE_URL "));
    v.base_url = os(BASE);
    ok(&v);
}

#[test]
fn ee_10_the_default_home_comes_from_home_and_inherits_its_validation() {
    // HOME is validated first (units rule), so a home made from it is valid by construction
    let mut v = base();
    v.home = os("/home/a.b-c_d");
    assert_eq!(ok(&v).home, PathBuf::from("/home/a.b-c_d/cortex-v2"));
    v.home = os("/home/dev/");
    assert!(
        err(&v).starts_with("HOME "),
        "a trailing slash in HOME is refused before any home is made"
    );
}

#[test]
fn ee_11_vars_asks_the_environment_for_exactly_five_names_and_never_for_xdg() {
    let asked = RefCell::new(Vec::<String>::new());
    let lookup = |name: &str| -> Option<OsString> {
        asked.borrow_mut().push(name.to_owned());
        Some(OsString::from(format!("value-of-{name}")))
    };
    let v = Vars::from_lookup(&lookup);
    let mut names = asked.borrow().clone();
    names.sort();
    assert_eq!(
        names,
        [
            "CORTEX_BASE_URL",
            "CORTEX_HOME",
            "CORTEX_PORT",
            "CORTEX_TOKEN_FILE",
            "HOME"
        ],
        "each name once, and not XDG_CONFIG_HOME"
    );
    assert_eq!(v.home, os("value-of-HOME"));
    assert_eq!(v.cortex_home, os("value-of-CORTEX_HOME"));
    assert_eq!(v.port, os("value-of-CORTEX_PORT"));
    assert_eq!(v.token_file, os("value-of-CORTEX_TOKEN_FILE"));
    assert_eq!(v.base_url, os("value-of-CORTEX_BASE_URL"));
    // a variable that is not set stays None
    let none = Vars::from_lookup(&|_| None);
    assert!(none.home.is_none() && none.cortex_home.is_none() && none.port.is_none());
    assert!(none.token_file.is_none() && none.base_url.is_none());
}

// ---- the command's help (cox W1-80: the one-sentence install hint is in --help too)

mod process {
    use std::process::Command;

    /// The binary with a CLEARED environment plus exactly the variables named.
    pub fn cortex_env(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_cortex"))
            .args(args)
            .env_clear()
            .envs(env.iter().copied())
            .output()
            .expect("run the cortex binary");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    pub fn cortex(args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_cortex"))
            .args(args)
            .env_clear()
            .output()
            .expect("run the cortex binary");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

#[test]
fn ug_01_the_help_of_the_command_says_the_port_hint_in_one_sentence_and_lists_the_usage() {
    let (code, out, err) = process::cortex(&["upgrade", "--help"]);
    assert_eq!(code, 0, "{err}");
    assert!(err.is_empty(), "{err:?}");
    assert!(
        out.contains(PORT_HINT),
        "the help lacks the install hint: {out}"
    );
    assert_eq!(out.matches(PORT_HINT).count(), 1, "said once: {out}");
    assert!(
        out.contains("cortex upgrade [--check | --rollback] [--to VERSION]"),
        "the usage line of the engine: {out}"
    );
    assert!(
        out.contains("CORTEX_BASE_URL") && out.contains("CORTEX_PORT"),
        "{out}"
    );
    assert!(
        out.contains("Global options such as --token-file go before the engine's own options"),
        "the help says where the global options go: {out}"
    );
    assert!(!out.contains('\u{1b}'), "no escape byte in the help");
    // the top-level help lists the command (sorted with the others)
    let (code, top, _) = process::cortex(&["--help"]);
    assert_eq!(code, 0);
    assert!(top.contains("upgrade"), "{top}");
}

// ---- the whole call (`run_with`): the arguments first, then the environment, then the real ports and the engine. DB-free and
// network-free: every cell ends at a refusal made BEFORE the engine touches a container, the database or the network.

fn s(list: &[&str]) -> Vec<String> {
    list.iter().map(|x| (*x).to_owned()).collect()
}

fn rvars(home_dir: &Path) -> Vars {
    Vars {
        home: os("/home/dev"),
        cortex_home: Some(home_dir.as_os_str().to_owned()),
        port: os("8777"),
        token_file: os("/home/dev/cortex-v2/lead.token"),
        base_url: os(BASE),
    }
}

/// A fresh, empty directory under the system temp directory; removed by the caller.
fn fresh_dir(tag: &str) -> PathBuf {
    // under cargo's target tmp directory, never `/tmp`: since R-6b the entry reads the compose provider, and the ancestor rule of that read
    // refuses a directory writable by group or other (the system temp directory is one); the mode is set, so that no umask changes the answer
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("cortex-upgrade-group-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make the directory");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("mode");
    dir
}

fn refused(line: &str) -> Outcome {
    Outcome {
        code: EXIT_REFUSED,
        lines: vec![line.to_owned()],
    }
}

const NO_RELEASE: &str = "there is no running release (no current link in the home directory)";

#[test]
fn er_01_a_usage_error_is_the_engines_own_line_with_the_refusal_code_and_comes_first() {
    let want = parse_args(&s(&["--bogus"])).expect_err("parse_args refuses an unknown flag");
    // the environment is empty AND invalid: the arguments are looked at first
    assert_eq!(run_with(&s(&["--bogus"]), &Vars::default()), refused(&want));
    for bad in [
        s(&["--check", "--rollback"]),
        s(&["--to"]),
        s(&["--to", "not-a-version"]),
        s(&["--roles-timeout", "44"]),
        s(&["extra"]),
        s(&["--check", "--check"]),
    ] {
        let want = parse_args(&bad).expect_err("refused by parse_args");
        assert_eq!(run_with(&bad, &Vars::default()), refused(&want), "{bad:?}");
    }
}

#[test]
fn er_02_a_bad_environment_is_one_plain_line_with_the_refusal_code() {
    let mut v = Vars::default();
    let want = entry_from(&v, Mode::Check).expect_err("HOME is not set");
    assert_eq!(run_with(&s(&["--check"]), &v), refused(&want));
    v.home = os("/home/dev");
    let want = entry_from(&v, Mode::Check).expect_err("the token file is not set");
    assert_eq!(run_with(&s(&[]), &v), refused(&want));
}

#[test]
fn er_03_a_valid_call_reaches_the_engine_and_a_home_without_a_release_is_refused_there_and_left_empty()
 {
    let home = fresh_dir("er03");
    let out = run_with(&s(&["--check"]), &rvars(&home));
    let left: Vec<_> = std::fs::read_dir(&home).unwrap().collect();
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out, refused(NO_RELEASE));
    assert!(
        left.is_empty(),
        "nothing was written into the home: {left:?}"
    );
}

#[test]
fn er_04_the_real_adapters_get_the_home_the_port_and_the_current_release() {
    let entry: Entry =
        entry_from(&rvars(Path::new("/srv/ent-home")), Mode::Upgrade).expect("a valid environment");
    let ports = real_ports(&entry, None);
    let c = format!("{:?}", ports.containers);
    assert!(c.contains("\"/srv/ent-home\""), "containers: {c}");
    assert!(c.contains("8777"), "containers: {c}");
    assert!(!c.contains("8601"), "containers: {c}");
    let p = format!("{:?}", ports.psql);
    assert!(p.contains("\"/srv/ent-home/current\""), "psql: {p}");
    let d = format!("{:?}", ports.dumper);
    assert!(d.contains("home: \"/srv/ent-home\""), "dumper: {d}");
    assert!(d.contains("\"/srv/ent-home/current\""), "dumper: {d}");
}

#[test]
fn er_05_the_request_bounds_are_the_stated_ones() {
    // one release-host request (the manifest, or the image archive, the large asset): 30 minutes; one local request: 10 seconds
    assert_eq!(RELEASE_REQUEST_TIMEOUT, Duration::from_secs(1800));
    assert_eq!(LOCAL_REQUEST_TIMEOUT, Duration::from_secs(10));
}

#[test]
fn er_06_the_local_stack_talks_to_the_derived_loopback_address_and_to_nothing_else() {
    let mut v = rvars(Path::new("/srv/ent-home"));
    let entry = entry_from(&v, Mode::Upgrade).expect("a valid environment");
    assert_eq!(local_stack(&entry).base, "http://127.0.0.1:8777");
    assert_eq!(local_stack(&entry).base, entry.local_url);
    v.port = None;
    let entry = entry_from(&v, Mode::Upgrade).expect("the default port");
    assert_eq!(local_stack(&entry).base, "http://127.0.0.1:8601");
}

// ---- what the command prints: stdout for the result (exit 0 and 10), stderr for a refusal (cox W1-80)

#[test]
fn ug_02_the_result_goes_to_stdout_and_a_refusal_to_stderr_and_the_exit_code_is_the_engines() {
    let lines = vec!["one".to_owned(), "two".to_owned()];
    for code in [0u8, 10] {
        let o = to_output(&Outcome {
            code,
            lines: lines.clone(),
        });
        assert_eq!(
            (o.exit, o.stdout.as_str(), o.stderr.as_str()),
            (code, "one\ntwo\n", ""),
            "{code}"
        );
    }
    for code in [2u8, 3, 4, 1, 255] {
        let o = to_output(&Outcome {
            code,
            lines: lines.clone(),
        });
        assert_eq!(
            (o.exit, o.stdout.as_str(), o.stderr.as_str()),
            (code, "", "one\ntwo\n"),
            "{code}"
        );
    }
    let o = to_output(&Outcome {
        code: 0,
        lines: vec![],
    });
    assert_eq!((o.exit, o.stdout.as_str(), o.stderr.as_str()), (0, "", ""));
}

fn token_ok() -> &'static str {
    "/home/dev/cortex-v2/lead.token"
}

fn env_for<'a>(home_dir: &'a str, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut v = vec![
        ("HOME", "/home/dev"),
        ("CORTEX_HOME", home_dir),
        ("CORTEX_PORT", "8777"),
        ("CORTEX_BASE_URL", BASE),
    ];
    v.extend_from_slice(extra);
    v
}

#[test]
fn ug_03_through_the_binary_a_usage_error_and_a_bad_environment_are_one_stderr_line_with_exit_2() {
    let want = parse_args(&s(&["--bogus"])).expect_err("usage");
    let (code, out, err) = process::cortex_env(&["upgrade", "--bogus"], &[]);
    assert_eq!((code, out.as_str(), err), (2, "", format!("{want}\n")));
    let want = unit_dir_from(None).expect_err("HOME");
    let (code, out, err) = process::cortex_env(&["upgrade", "--check"], &[]);
    assert_eq!((code, out.as_str(), err), (2, "", format!("{want}\n")));
    // the engine's own usage refusals are the engine's words
    let want = parse_args(&s(&["--check", "--rollback"])).expect_err("usage");
    let (code, out, err) = process::cortex_env(&["upgrade", "--check", "--rollback"], &[]);
    assert_eq!((code, out.as_str(), err), (2, "", format!("{want}\n")));
}

#[test]
fn ug_04_through_the_binary_a_home_without_a_release_is_the_engines_refusal_and_stays_empty() {
    let home = fresh_dir("ug04");
    let h = home.to_str().unwrap().to_owned();
    let env = env_for(&h, &[("CORTEX_TOKEN_FILE", token_ok())]);
    let (code, out, err) = process::cortex_env(&["upgrade", "--check"], &env);
    let left: Vec<_> = std::fs::read_dir(&home).unwrap().collect();
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(
        (code, out.as_str(), err),
        (2, "", format!("{NO_RELEASE}\n"))
    );
    assert!(
        left.is_empty(),
        "nothing was written into the home: {left:?}"
    );
}

#[test]
fn ug_05_the_token_flag_beats_the_environment_variable_when_the_global_flag_stands_before_the_engines_options()
 {
    let home = fresh_dir("ug05");
    let h = home.to_str().unwrap().to_owned();
    // a BAD variable and a GOOD flag: the flag decides, so the call reaches the engine
    let env = env_for(&h, &[("CORTEX_TOKEN_FILE", "relative.token")]);
    for args in [
        vec!["--token-file", token_ok(), "upgrade", "--check"],
        vec!["upgrade", "--token-file", token_ok(), "--check"],
    ] {
        let (code, out, err) = process::cortex_env(&args, &env);
        assert_eq!(
            (code, out.as_str(), err),
            (2, "", format!("{NO_RELEASE}\n")),
            "{args:?}"
        );
    }
    // AFTER an engine option the global flag is part of the engine's own arguments and is refused in the engine's words (the help says
    // where the global options go): never silently ignored
    let want = parse_args(&s(&["--check", "--token-file", token_ok()]))
        .expect_err("the engine refuses it");
    let (code, out, err) =
        process::cortex_env(&["upgrade", "--check", "--token-file", token_ok()], &env);
    assert_eq!((code, out.as_str(), err), (2, "", format!("{want}\n")));
    // a GOOD variable and a BAD flag: the flag still decides, and it is refused
    let env = env_for(&h, &[("CORTEX_TOKEN_FILE", token_ok())]);
    let (code, out, err) = process::cortex_env(
        &["--token-file", "relative.token", "upgrade", "--check"],
        &env,
    );
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(code, 2);
    assert_eq!(out, "");
    assert!(err.starts_with("CORTEX_TOKEN_FILE must be set"), "{err}");
}

#[test]
fn er_07_the_engines_env_gets_each_value_of_the_entry_in_its_own_field() {
    let mut v = rvars(Path::new("/srv/ent-home"));
    v.port = os("8777");
    v.token_file = os("/home/dev/cortex-v2/lead.token");
    v.base_url = os("https://releases.example/cortex/");
    let entry = entry_from(&v, Mode::Upgrade).expect("a valid environment");
    let env = engine_env(&entry, None);
    assert_eq!(env.home, PathBuf::from("/srv/ent-home"));
    assert_eq!(
        env.manifest_url,
        "https://releases.example/cortex/releases/manifest.json"
    );
    assert_eq!(env.local_url, "http://127.0.0.1:8777");
    assert_eq!(
        env.operator_token_file,
        PathBuf::from("/home/dev/cortex-v2/lead.token")
    );
    assert_eq!(
        env.unit_dir,
        PathBuf::from("/home/dev/.config/systemd/user")
    );
}

// ---- W1-88 (cox row c8f09a05, mike's REWORK of the entry-point pin): with no CORTEX_BASE_URL the ONLINE path is off, and everything local
// still works: `--rollback`, and the recovery of an interrupted rollback or upgrade, enter the engine; an operation that needs the manifest
// refuses BEFORE any fetch or change with the one line and its fix; a set but unusable base URL still refuses for upgrade and check.

/// A release-host transport that counts how often it is asked and never answers.
#[derive(Clone)]
struct Counting(Arc<AtomicUsize>);

impl Counting {
    fn new() -> (Counting, Arc<AtomicUsize>) {
        let n = Arc::new(AtomicUsize::new(0));
        (Counting(n.clone()), n)
    }
}

impl Transport for Counting {
    fn get(&self, _request: &Request<'_>) -> Result<Response, FetchError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(FetchError::Transport)
    }
}

/// `rvars` without a base URL: the online path is off.
fn offline_vars(home_dir: &Path) -> Vars {
    let mut v = rvars(home_dir);
    v.base_url = None;
    v
}

/// A fresh home that holds exactly the named files (content is garbage on purpose: the engine must not need it to be valid to answer).
fn home_with(tag: &str, files: &[&str]) -> PathBuf {
    let dir = fresh_dir(tag);
    for f in files {
        std::fs::write(dir.join(f), "not a marker\n").expect("write the file");
    }
    provider_record(&dir);
    dir
}

/// R-6b: a VALID compose provider and its record in `home`: `<home>/provider-bin/podman-compose` (an executable regular file in a directory of
/// its own) and `<home>/compose-provider` naming it, every mode set explicitly. An upgrade or a rollback reads the record before the engine is
/// entered, so a cell that means to REACH the engine has one. Returns the canonical path of the provider (what the read must answer).
fn provider_record(home: &Path) -> PathBuf {
    let bin = home.join("provider-bin");
    std::fs::create_dir(&bin).expect("make the provider directory");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("mode");
    let p = bin.join("podman-compose");
    std::fs::write(&p, "#!/bin/sh\nexit 0\n").expect("write the provider");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("mode");
    let record = home.join("compose-provider");
    std::fs::write(&record, format!("{}\n", p.display())).expect("write the record");
    std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o644)).expect("mode");
    std::fs::canonicalize(&p).expect("canonical provider")
}

/// A fresh HOME that holds a VALID unit directory (`.config/systemd/user`: a real directory of the user's own, mode 0700). Since the engine of
/// packet 19 (W1-74, W1-75) a plain upgrade validates the destination of the ten member files FIRST, before the lock and before it recovers
/// anything, so the cells that mean to reach the engine's recovery in that mode need one; the caller removes it.
fn unit_home(tag: &str) -> PathBuf {
    let h = fresh_dir(tag);
    let u = h.join(".config").join("systemd").join("user");
    std::fs::create_dir_all(&u).expect("make the unit directory");
    std::fs::set_permissions(&u, std::fs::Permissions::from_mode(0o700))
        .expect("mode of the unit directory");
    h
}

/// `v` with its HOME (and so the unit directory) taken from `unit_home`.
fn at_unit_home(mut v: Vars, uh: &Path) -> Vars {
    v.home = Some(uh.as_os_str().to_owned());
    v
}

/// The names in `dir`, without the two the FIXTURE adds (`compose-provider` and `provider-bin`: see [`provider_record`]); the engine creates neither.
fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "compose-provider" && n != "provider-bin")
        .collect();
    v.sort();
    v
}

fn call(args: &[&str], vars: &Vars) -> (Outcome, usize) {
    let (t, n) = Counting::new();
    let out = run_with_transport(&s(args), vars, t);
    (out, n.load(Ordering::SeqCst))
}

#[test]
fn ee_13_the_base_url_is_validated_and_used_only_by_the_operations_that_fetch() {
    // an unusable value is a refusal for upgrade and check, and is ignored by a rollback (it never fetches: the value is neither validated nor used)
    for bad in [
        "http://releases.example/cortex",
        "https://",
        "ftp://x",
        "not a url",
    ] {
        let mut v = base();
        v.base_url = os(bad);
        for mode in [Mode::Upgrade, Mode::Check] {
            let m =
                entry_from(&v, mode).expect_err("upgrade and check refuse an unusable base URL");
            assert!(m.starts_with("CORTEX_BASE_URL "), "{mode:?} {bad:?}: {m}");
        }
        let e = entry_from(&v, Mode::Rollback).unwrap_or_else(|m| {
            panic!("a rollback must not be refused by the base URL {bad:?}: {m}")
        });
        assert_eq!(
            e.manifest_url, None,
            "{bad:?}: a rollback has no manifest URL"
        );
    }
    // a good one is used by upgrade and check, and ignored by a rollback
    let v = base();
    assert_eq!(
        entry_from(&v, Mode::Check).unwrap().manifest_url.as_deref(),
        Some(MANIFEST)
    );
    assert_eq!(entry_from(&v, Mode::Rollback).unwrap().manifest_url, None);
}

#[test]
fn lw_01_local_work_is_pending_when_a_rollback_or_an_upgrade_marker_is_there() {
    let empty = home_with("lw-empty", &[]);
    let unrelated = home_with("lw-unrelated", &["cortex.toml"]);
    let intent = home_with("lw-intent", &["rollback.intent"]);
    let marker = home_with("lw-marker", &["rollback.unfinished"]);
    let upgrade = home_with("lw-upgrade", &["upgrade.incomplete"]);
    let got = [
        local_work_pending(&empty),
        local_work_pending(&unrelated),
        local_work_pending(&intent),
        local_work_pending(&marker),
        local_work_pending(&upgrade),
        local_work_pending(&empty.join("no-such-directory")),
    ];
    for d in [&empty, &unrelated, &intent, &marker, &upgrade] {
        let _ = std::fs::remove_dir_all(d);
    }
    assert_eq!(
        got,
        [false, false, true, true, true, false],
        "empty, unrelated file, rollback.intent, rollback.unfinished, upgrade.incomplete, a home that does not exist"
    );
}

#[test]
fn gd_01_with_the_online_path_off_the_release_host_is_never_asked() {
    let req = Request {
        url: "https://releases.example/cortex/releases/manifest.json",
        bearer: None,
    };
    let (t, n) = Counting::new();
    let off = Guarded {
        inner: t,
        online: false,
    };
    for _ in 0..3 {
        assert_eq!(off.get(&req).err(), Some(FetchError::BadUrl));
    }
    assert_eq!(
        n.load(Ordering::SeqCst),
        0,
        "offline: the inner transport is never reached"
    );
    let (t, n) = Counting::new();
    let on = Guarded {
        inner: t,
        online: true,
    };
    assert_eq!(
        on.get(&req).err(),
        Some(FetchError::Transport),
        "online: the inner answer is passed on"
    );
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[test]
fn tr_01_only_the_engines_refusal_of_the_empty_manifest_url_becomes_the_no_location_line() {
    let bad_url = FetchError::BadUrl.to_string();
    let out = |code: u8, lines: &[&str]| Outcome {
        code,
        lines: lines.iter().map(|l| (*l).to_owned()).collect(),
    };
    // offline, a refusal whose LAST line is the fetcher's bad-URL line: that line (only) is replaced
    assert_eq!(
        translate_offline(
            out(
                EXIT_REFUSED,
                &[
                    "completed an interrupted rollback (2.0.0 to 1.0.0)",
                    &bad_url
                ]
            ),
            false
        ),
        out(
            EXIT_REFUSED,
            &[
                "completed an interrupted rollback (2.0.0 to 1.0.0)",
                NO_LOCATION
            ]
        )
    );
    assert_eq!(
        translate_offline(out(EXIT_REFUSED, &[&bad_url]), false),
        out(EXIT_REFUSED, &[NO_LOCATION])
    );
    // online: never touched
    assert_eq!(
        translate_offline(out(EXIT_REFUSED, &[&bad_url]), true),
        out(EXIT_REFUSED, &[&bad_url])
    );
    // another last line, another code, an empty outcome, the bad-URL line not last: never touched
    for o in [
        out(
            EXIT_REFUSED,
            &["the dump tool bin/cortex-dump is not installed"],
        ),
        out(3, &[&bad_url]),
        out(0, &[&bad_url]),
        out(EXIT_REFUSED, &[]),
        out(EXIT_REFUSED, &[&bad_url, "something after it"]),
    ] {
        assert_eq!(translate_offline(o.clone(), false), o);
    }
}

#[test]
fn ne_01_a_rollback_without_a_base_url_enters_the_engine_and_asks_the_release_host_nothing() {
    let home = fresh_dir("ne01");
    provider_record(&home); // a rollback reads the compose provider's record before the engine is entered (R-6b)
    let (out, asked) = call(&["--rollback"], &offline_vars(&home));
    let left = files_in(&home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out.code, EXIT_REFUSED, "{out:?}");
    assert_eq!(out.lines.len(), 1, "{out:?}");
    assert!(
        out.lines[0].starts_with("rollback refused"),
        "the engine's own line, not the group's: {out:?}"
    );
    assert_ne!(out.lines, vec![NO_LOCATION.to_owned()]);
    assert_eq!(asked, 0, "the release host was asked {asked} times");
    assert!(
        left.iter()
            .all(|f| f != "rollback.intent" && f != "rollback.unfinished"),
        "{left:?}"
    );
}

#[test]
fn ne_02_an_upgrade_without_a_base_url_refuses_before_the_engine_with_the_exact_line_and_changes_nothing()
 {
    let home = fresh_dir("ne02");
    let (out, asked) = call(&[], &offline_vars(&home));
    let left = files_in(&home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out, refused(NO_LOCATION));
    assert_eq!(asked, 0);
    assert!(
        left.is_empty(),
        "no lock, no file: the engine was not entered: {left:?}"
    );
}

#[test]
fn ne_03_a_check_without_a_base_url_refuses_the_same_way() {
    let home = fresh_dir("ne03");
    let (out, asked) = call(&["--check"], &offline_vars(&home));
    let left = files_in(&home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out, refused(NO_LOCATION));
    assert_eq!(asked, 0);
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn ne_04_an_interrupted_rollback_is_recovered_by_the_engine_in_every_mode_without_a_base_url() {
    // the engine's own refusal of an intent it cannot use proves it was entered (a valid intent would go on to start the old release
    // with the REAL podman, which no cell may do); the group must not stand in front of it
    for args in [vec!["--rollback"], vec![]] {
        let home = home_with("ne04", &["rollback.intent"]);
        let uh = unit_home("ne04u");
        let (out, asked) = call(&args, &at_unit_home(offline_vars(&home), &uh));
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&uh);
        assert_eq!(out.code, EXIT_REFUSED, "{args:?}: {out:?}");
        assert!(
            out.lines[0].starts_with("an earlier rollback was interrupted and cannot be completed"),
            "{args:?}: {out:?}"
        );
        assert_eq!(asked, 0, "{args:?}");
    }
}

#[test]
fn ne_05_an_unfinished_or_unreadable_upgrade_marker_reaches_the_engine_in_every_mode_without_a_base_url()
 {
    for args in [vec!["--rollback"], vec![], vec!["--check"]] {
        let home = home_with("ne05", &["upgrade.incomplete"]);
        let uh = unit_home("ne05u");
        let (out, asked) = call(&args, &at_unit_home(offline_vars(&home), &uh));
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&uh);
        assert_eq!(out.code, EXIT_REFUSED, "{args:?}: {out:?}");
        assert!(
            out.lines[0].starts_with("the marker of an unfinished upgrade cannot be used"),
            "{args:?}: the engine's own line about the marker, not an earlier refusal: {out:?}"
        );
        assert_ne!(
            out.lines,
            vec![NO_LOCATION.to_owned()],
            "{args:?}: the group stood in front of the engine"
        );
        assert_eq!(asked, 0, "{args:?}");
    }
}

#[test]
fn ne_06_a_check_with_a_rollback_pending_gets_the_engines_own_explanation_not_the_no_location_line()
{
    let home = home_with("ne06", &["rollback.unfinished"]);
    let (out, asked) = call(&["--check"], &offline_vars(&home));
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out.code, EXIT_REFUSED);
    assert!(
        out.lines[0].starts_with("an unfinished rollback is pending"),
        "{out:?}"
    );
    assert_eq!(asked, 0);
}

#[test]
fn ne_08_a_plain_upgrade_over_local_work_meets_the_engines_unit_directory_check_first_and_a_rollback_and_a_check_do_not()
 {
    // packet 19 (W1-74, W1-75): in an UPGRADE the engine validates the destination of the ten member files before the lock and before it recovers
    // anything. The entry only passes the engine's own refusal on: the line, the code, nothing asked, nothing changed (no lock, the marker stays).
    // `--rollback` and `--check` have no such check first: they reach their own lines with the very same (unusable) HOME.
    // The HOME is the CELL's OWN: a fresh empty directory, so its `.config/systemd/user` is ABSENT by construction, whatever the host has at
    // `/home/dev` (vera's VE7FE-01: the cell used to assume that ambient directory was missing), and the expected path is DERIVED from it.
    let home = home_with("ne08", &["rollback.intent"]);
    let uh = fresh_dir("ne08h");
    let v = at_unit_home(offline_vars(&home), &uh);
    let want = format!(
        "cannot look at the unit directory {}",
        uh.join(".config").join("systemd").join("user").display()
    );
    assert!(!uh.join(".config").exists(), "the unit directory is absent");
    let (plain, asked) = call(&[], &v);
    let left = files_in(&home);
    assert_eq!(plain.code, EXIT_REFUSED, "{plain:?}");
    assert!(
        plain.lines[0].starts_with(&want) && plain.lines[0].ends_with("nothing was changed"),
        "{plain:?}"
    );
    assert_eq!(asked, 0);
    assert_eq!(
        left,
        ["rollback.intent"],
        "no lock, the marker stays: {left:?}"
    );
    let (rollback, asked) = call(&["--rollback"], &v);
    assert!(
        rollback.lines[0]
            .starts_with("an earlier rollback was interrupted and cannot be completed"),
        "{rollback:?}"
    );
    assert_eq!(asked, 0);
    let (check, asked) = call(&["--check"], &v);
    assert!(
        check.lines[0].starts_with("an unfinished rollback is pending"),
        "{check:?}"
    );
    assert_eq!(asked, 0);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&uh);
}

#[test]
fn ne_07_a_set_but_unusable_base_url_still_refuses_upgrade_and_check_and_does_not_stop_a_rollback()
{
    for url in ["http://releases.example/cortex", "https://"] {
        let mut v = rvars(&fresh_dir("ne07"));
        v.base_url = os(url);
        let home = PathBuf::from(v.cortex_home.clone().unwrap());
        provider_record(&home); // the rollback below reads the record (R-6b); the upgrade and the check refuse before it
        for args in [vec![], vec!["--check"]] {
            let (out, asked) = call(&args, &v);
            assert_eq!(
                out,
                refused("CORTEX_BASE_URL must be an https URL"),
                "{url} {args:?}"
            );
            assert_eq!(asked, 0);
            assert!(files_in(&home).is_empty(), "the engine was not entered");
        }
        let (out, asked) = call(&["--rollback"], &v);
        let _ = std::fs::remove_dir_all(&home);
        assert!(
            out.lines[0].starts_with("rollback refused"),
            "{url}: {out:?}"
        );
        assert_eq!(asked, 0);
    }
}

#[test]
fn ug_06_through_the_binary_a_rollback_without_a_base_url_reaches_the_engine() {
    let home = fresh_dir("ug06");
    provider_record(&home);
    let h = home.to_str().unwrap().to_owned();
    let env = env_for_offline(&h);
    let (code, out, err) = process::cortex_env(&["upgrade", "--rollback"], &env);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(code, 2, "{err}");
    assert_eq!(out, "");
    assert!(
        err.starts_with("rollback refused"),
        "the engine's line: {err}"
    );
    assert!(!err.contains("no download location"), "{err}");
}

#[test]
fn ug_07_through_the_binary_an_upgrade_or_a_check_without_a_base_url_refuses_with_the_exact_line() {
    for args in [vec!["upgrade"], vec!["upgrade", "--check"]] {
        let home = fresh_dir("ug07");
        let h = home.to_str().unwrap().to_owned();
        let env = env_for_offline(&h);
        let (code, out, err) = process::cortex_env(&args, &env);
        let left = files_in(&home);
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(
            (code, out.as_str(), err),
            (2, "", format!("{NO_LOCATION}\n")),
            "{args:?}"
        );
        assert!(left.is_empty(), "{args:?}: {left:?}");
    }
}

fn env_for_offline(home_dir: &str) -> Vec<(&str, &str)> {
    vec![
        ("HOME", "/home/dev"),
        ("CORTEX_HOME", home_dir),
        ("CORTEX_PORT", "8777"),
        ("CORTEX_TOKEN_FILE", "/home/dev/cortex-v2/lead.token"),
    ]
}

#[test]
fn er_08_the_engines_env_gets_an_empty_manifest_url_when_the_online_path_is_off_never_a_stand_in() {
    let entry = entry_from(&offline_vars(Path::new("/srv/ent-home")), Mode::Upgrade)
        .expect("the entry exists");
    assert_eq!(entry.manifest_url, None);
    let env = engine_env(&entry, None);
    assert_eq!(
        env.manifest_url, "",
        "an empty URL, so that nothing can be fetched; never a dummy endpoint"
    );
    assert_eq!(
        env.local_url, "http://127.0.0.1:8777",
        "the local operations keep their values"
    );
}

// ---- R-6b (cox W1-68, W1-91 Q1, his row d02a11df): the entry reads the verified compose provider ONCE and hands the SAME path to the adapter
// and to the engine's `Env`; a refusal is the library's own line, with the refusal code, before the engine and before anything is written.

#[test]
fn pr_01_the_provider_is_read_for_an_upgrade_and_a_rollback_and_not_at_all_for_a_check() {
    let home = fresh_dir("pr01");
    let want = provider_record(&home);
    let entry = entry_from(&rvars(&home), Mode::Upgrade).expect("a valid environment");
    for mode in [Mode::Upgrade, Mode::Rollback] {
        assert_eq!(
            compose_provider_for(&entry, mode),
            Ok(Some(want.clone())),
            "{mode:?}"
        );
    }
    assert_eq!(compose_provider_for(&entry, Mode::Check), Ok(None));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn pr_02_without_a_record_an_upgrade_and_a_rollback_are_refused_and_a_check_is_not() {
    let home = fresh_dir("pr02");
    let entry = entry_from(&rvars(&home), Mode::Upgrade).expect("a valid environment");
    for mode in [Mode::Upgrade, Mode::Rollback] {
        let e = compose_provider_for(&entry, mode).expect_err("there is no record");
        assert!(
            e.contains("no compose provider is recorded") && e.contains("fix:"),
            "{mode:?}: {e}"
        );
    }
    assert_eq!(
        compose_provider_for(&entry, Mode::Check),
        Ok(None),
        "a check needs no record"
    );
    assert!(files_in(&home).is_empty(), "reading writes nothing");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn pr_03_the_refusal_comes_before_the_engine_with_the_refusal_code_nothing_written_and_no_request()
{
    for args in [vec![], vec!["--rollback"]] {
        let home = fresh_dir("pr03");
        let (out, asked) = call(&args, &rvars(&home));
        let left = files_in(&home);
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(out.code, EXIT_REFUSED, "{args:?}: {out:?}");
        assert_eq!(out.lines.len(), 1, "{args:?}: {out:?}");
        assert!(
            out.lines[0].contains("no compose provider is recorded")
                && out.lines[0].contains("fix:"),
            "{args:?}: the library's own line: {out:?}"
        );
        assert_eq!(asked, 0, "{args:?}");
        assert!(left.is_empty(), "{args:?}: no lock, no file: {left:?}");
    }
    // a check is not stopped by it: it reaches the engine's own line (a home without a release)
    let home = fresh_dir("pr03c");
    let (out, _) = call(&["--check"], &rvars(&home));
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out, refused(NO_RELEASE));
}

#[test]
fn pr_04_the_order_of_the_refusals_is_arguments_environment_download_location_then_the_provider() {
    // the download location first: an offline upgrade with no local work and no record says NO_LOCATION
    let home = fresh_dir("pr04a");
    let (out, asked) = call(&[], &offline_vars(&home));
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out, refused(NO_LOCATION));
    assert_eq!(asked, 0);
    // a bad environment first: a port that is not a number
    let home = fresh_dir("pr04b");
    let mut v = rvars(&home);
    v.port = os("not-a-port");
    let (out, _) = call(&[], &v);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out.code, EXIT_REFUSED);
    assert!(!out.lines[0].contains("compose provider"), "{out:?}");
    // a usage error first
    let home = fresh_dir("pr04c");
    let (out, _) = call(&["--no-such-option"], &rvars(&home));
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out.code, EXIT_REFUSED);
    assert!(!out.lines[0].contains("compose provider"), "{out:?}");
}

#[test]
fn pr_05_a_record_that_is_unusable_is_refused_with_the_librarys_line_before_the_engine() {
    // a relative path in the record
    let home = fresh_dir("pr05a");
    provider_record(&home);
    std::fs::write(home.join("compose-provider"), "podman-compose\n").unwrap();
    let (out, asked) = call(&[], &rvars(&home));
    let left = files_in(&home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out.code, EXIT_REFUSED, "{out:?}");
    assert!(out.lines[0].contains("not an absolute path"), "{out:?}");
    assert_eq!(asked, 0);
    assert!(left.is_empty(), "{left:?}");
    // a provider that is not executable
    let home = fresh_dir("pr05b");
    provider_record(&home);
    std::fs::set_permissions(
        home.join("provider-bin").join("podman-compose"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let (out, asked) = call(&["--rollback"], &rvars(&home));
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(out.code, EXIT_REFUSED, "{out:?}");
    assert!(out.lines[0].contains("not executable"), "{out:?}");
    assert_eq!(asked, 0);
}

#[test]
fn pr_06_the_engines_env_and_the_adapter_carry_the_same_path_and_a_check_carries_none() {
    let home = fresh_dir("pr06");
    let want = provider_record(&home);
    let entry = entry_from(&rvars(&home), Mode::Upgrade).expect("a valid environment");
    let env = engine_env(&entry, Some(&want));
    assert_eq!(
        env.compose_provider, want,
        "the engine's Env has the path read"
    );
    let shown = format!("{:?}", real_ports(&entry, Some(&want)).containers);
    assert!(
        shown.contains(&want.display().to_string()),
        "the adapter was built with the SAME path: {shown}"
    );
    assert_eq!(
        engine_env(&entry, None).compose_provider,
        PathBuf::new(),
        "a check: an empty path, the engine's guard does not apply to it"
    );
    let shown = format!("{:?}", real_ports(&entry, None).containers);
    assert!(shown.contains("provider: None"), "{shown}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn pr_07_through_the_binary_a_rollback_without_a_record_is_the_librarys_line_and_with_one_reaches_the_engine()
 {
    let home = fresh_dir("pr07");
    let h = home.to_str().unwrap().to_owned();
    let env = env_for_offline(&h);
    let (code, out, err) = process::cortex_env(&["upgrade", "--rollback"], &env);
    assert_eq!(code, 2, "{err}");
    assert_eq!(out, "");
    assert!(err.starts_with("no compose provider is recorded"), "{err}");
    assert!(files_in(&home).is_empty(), "nothing was written");
    provider_record(&home);
    let (code, _, err) = process::cortex_env(&["upgrade", "--rollback"], &env);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(code, 2, "{err}");
    assert!(
        err.starts_with("rollback refused"),
        "now the engine's own line: {err}"
    );
}

#[test]
fn pr_08_one_function_builds_the_engines_env_and_the_adapters_from_the_same_provider() {
    // found by the mutant run (RB-14): `run_engine` could build the adapters WITHOUT the provider and no cell saw it; the two are now built by ONE
    // function that a cell can look at
    let home = fresh_dir("pr08");
    let want = provider_record(&home);
    let entry = entry_from(&rvars(&home), Mode::Upgrade).expect("a valid environment");
    let (env, ports) = engine_parts(&entry, Some(&want));
    assert_eq!(env.compose_provider, want, "the engine's Env");
    let shown = format!("{:?}", ports.containers);
    assert!(
        shown.contains(&want.display().to_string()) && !shown.contains("provider: None"),
        "the adapter got the SAME path: {shown}"
    );
    let (env, ports) = engine_parts(&entry, None);
    assert_eq!(
        env.compose_provider,
        PathBuf::new(),
        "a check: none in the Env"
    );
    assert!(
        format!("{:?}", ports.containers).contains("provider: None"),
        "a check: none in the adapter"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn pr_09_a_record_whose_canonical_provider_path_holds_a_control_byte_is_refused_at_the_entry_before_the_engine()
 {
    // R6A-01 (vera 9 Nov, found by mike): the record is plain text and names the provider through a directory alias (the fixture's own `provider-bin`) whose
    // REAL name holds a control byte; the library refuses it at the entry, in one line, before any request and before anything is written (it used to be
    // returned, to fail at `compose_up` after the switch and the member steps), for an upgrade and for a rollback
    let home = fresh_dir("pr09");
    let real_parent = fresh_dir("pr09real");
    let odd = real_parent.join("bin\u{1}dir");
    std::fs::create_dir(&odd).expect("make the directory with a control byte in its name");
    std::fs::set_permissions(&odd, std::fs::Permissions::from_mode(0o755)).expect("mode");
    let provider = odd.join("podman-compose");
    std::fs::write(&provider, "#!/bin/sh\nexit 0\n").expect("write the provider");
    std::fs::set_permissions(&provider, std::fs::Permissions::from_mode(0o755)).expect("mode");
    std::os::unix::fs::symlink(&odd, home.join("provider-bin")).expect("the alias");
    let record = home.join("compose-provider");
    let named = home.join("provider-bin").join("podman-compose");
    std::fs::write(&record, format!("{}\n", named.display())).expect("write the record");
    std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o644)).expect("mode");
    assert!(
        !named.to_string_lossy().chars().any(char::is_control),
        "the record's text is plain"
    );
    for args in [&[][..], &["--rollback"][..]] {
        let (out, asked) = call(args, &rvars(&home));
        assert_eq!(out.code, EXIT_REFUSED, "{args:?}: {out:?}");
        assert!(
            out.lines[0].contains("canonical path holds a control character"),
            "{args:?}: {out:?}"
        );
        assert!(
            !out.lines[0].chars().any(char::is_control),
            "{args:?}: the line never echoes the bytes: {out:?}"
        );
        assert_eq!(asked, 0, "{args:?}: no request was made");
        assert!(
            files_in(&home).is_empty(),
            "{args:?}: nothing was written: {:?}",
            files_in(&home)
        );
    }
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&real_parent);
}

#[test]
fn pr_10_a_home_alias_to_a_directory_with_a_control_character_never_puts_it_raw_into_the_entrys_refusal()
 {
    // R6A-01, mike's second finding (kai's row f3654d2b): the entry's refusal line names the CANONICAL record path; a plain CORTEX_HOME alias whose real directory
    // name holds a control byte used to put that byte, raw, into the line the operator reads (ESC would move a terminal's cursor); now it is escaped
    for c in ['\u{1}', '\u{1b}', '\u{9b}'] {
        let parent = fresh_dir(&format!("pr10-{}", c as u32));
        let real = parent.join(format!("odd{c}home"));
        std::fs::create_dir(&real).expect("make the directory with the character in its name");
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).expect("mode");
        let alias = parent.join("plain-home");
        std::os::unix::fs::symlink(&real, &alias).expect("the alias");
        let (out, asked) = call(&[], &rvars(&alias));
        assert_eq!(out.code, EXIT_REFUSED, "{c:?}: {out:?}");
        assert!(
            out.lines[0].contains("no compose provider is recorded"),
            "{c:?}: {out:?}"
        );
        assert!(
            !out.lines[0].chars().any(char::is_control),
            "{c:?}: the line carries no raw control character: {:?}",
            out.lines[0]
        );
        assert!(
            out.lines[0].contains(&c.escape_debug().to_string()),
            "{c:?}: and shows it escaped: {out:?}"
        );
        assert_eq!(asked, 0, "{c:?}: no request was made");
        let _ = std::fs::remove_dir_all(&parent);
    }
}
