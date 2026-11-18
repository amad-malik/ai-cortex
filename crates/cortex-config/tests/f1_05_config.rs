//! F1-05 Config fails closed. [BASE] (lane A TESTS r5 §2; S1 §2.1 `cortex-config`, §4.5.)
//!
//! An unknown key stops startup and names the key; `XDG_CONFIG_HOME` moves the token directory;
//! the state, socket and token paths come from one place. RED: `load` and `token_dir` are
//! `todo!()`.
use std::path::{Path, PathBuf};

use cortex_config::{ConfigError, Env, load, token_dir};

/// Writes `body` to a file of its own under cargo's per-test temporary directory.
fn config_file(name: &str, body: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("f1_05");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let p = dir.join(format!("{name}-{}.toml", std::process::id()));
    std::fs::write(&p, body).expect("write config");
    p
}

fn home() -> Env {
    Env::from_pairs(&[("HOME", "/home/w")])
}

#[test]
fn f1_05_an_unknown_key_stops_startup_naming_the_key() {
    let typo = config_file("typo", "[paths]\nstate_dirr = \"/srv/x\"\n");
    match load(&typo, &home()) {
        Err(ConfigError::UnknownKey(k)) => assert_eq!(k, "paths.state_dirr"),
        other => panic!("want UnknownKey(paths.state_dirr), got {other:?}"),
    }
    let top = config_file("top", "colour = \"blue\"\n");
    match load(&top, &home()) {
        Err(ConfigError::UnknownKey(k)) => assert_eq!(k, "colour"),
        other => panic!("want UnknownKey(colour), got {other:?}"),
    }
    // Control: the same key spelled right loads.
    let ok = config_file("ok", "[paths]\nstate_dir = \"/srv/x\"\n");
    let cfg = load(&ok, &home()).expect("a known key loads");
    assert_eq!(cfg.paths.state_dir, PathBuf::from("/srv/x"));
}

#[test]
fn f1_05_xdg_config_home_moves_the_token_dir() {
    let empty = config_file("empty-xdg", "");
    let xdg = Env::from_pairs(&[("HOME", "/home/w"), ("XDG_CONFIG_HOME", "/xdg")]);
    assert_eq!(
        load(&empty, &xdg).expect("loads").paths.token_dir,
        PathBuf::from("/xdg/cortex")
    );
    assert_eq!(
        load(&empty, &home()).expect("loads").paths.token_dir,
        PathBuf::from("/home/w/.config/cortex")
    );
    // An empty XDG_CONFIG_HOME counts as unset (XDG base directory rules).
    let blank = Env::from_pairs(&[("HOME", "/home/w"), ("XDG_CONFIG_HOME", "")]);
    assert_eq!(
        load(&empty, &blank).expect("loads").paths.token_dir,
        PathBuf::from("/home/w/.config/cortex")
    );
    // Neither variable: fail closed, naming what is missing.
    assert!(matches!(load(&empty, &Env::default()), Err(ConfigError::Missing(m)) if m == "HOME"));
}

#[test]
fn f1_05_state_socket_and_token_paths_come_from_one_place() {
    let empty = config_file("defaults", "");
    let env = home();
    let p = load(&empty, &env).expect("loads").paths;
    // The defaults are P1 §2's and S1 §7.1's.
    assert_eq!(p.state_dir, PathBuf::from("/var/lib/cortex"));
    assert_eq!(p.host_socket, PathBuf::from("/run/cortex/admin.sock"));
    assert_eq!(p.worker_socket, PathBuf::from("/run/cortex/api.sock"));
    // The token dir in Config is the same function every other crate calls.
    assert_eq!(p.token_dir, token_dir(&env).expect("token dir"));
    // An override in [paths] changes that path and nothing else.
    let one = config_file("one", "[paths]\nhost_socket = \"/tmp/a.sock\"\n");
    let q = load(&one, &env).expect("loads").paths;
    assert_eq!(q.host_socket, PathBuf::from("/tmp/a.sock"));
    assert_eq!(
        (q.state_dir, q.worker_socket, q.token_dir),
        (p.state_dir, p.worker_socket, p.token_dir)
    );
}
