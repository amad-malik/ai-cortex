//! F1-05, r3 (mike's F1A-R2-02): the token directory is always absolute. Per the XDG base
//! directory rule, a relative `XDG_CONFIG_HOME` is invalid and ignored (falling back to HOME); a
//! relative `HOME` is an error, never a path relative to the working directory. Absolute, relative
//! and empty cases for both. RED (baseline ca217ac): relative bases are accepted.
//! `load` carries the same rule (mike: "load of an otherwise empty config file then succeeds with
//! that path"); that case was added after the fix and its RED is a run of this file against the
//! pre-fix source (RED-R2-load-72088c5.txt).
//! vera's F1A-SE-01 (the same defect): `f1_05_token_dir_and_load_agree_on_every_base` runs her
//! regression design's cases through BOTH `token_dir` and `load`, requires every success to be
//! absolute, and requires a refusal to name the key only, never the relative value. It was added
//! after the fix; its RED is a run against the pre-fix source (RED-R2-matrix-72088c5.txt).
use std::path::{Path, PathBuf};

use cortex_config::{ConfigError, Env, load, token_dir};

fn env(home: Option<&str>, xdg: Option<&str>) -> Env {
    let mut pairs = Vec::new();
    if let Some(h) = home {
        pairs.push(("HOME", h));
    }
    if let Some(x) = xdg {
        pairs.push(("XDG_CONFIG_HOME", x));
    }
    Env::from_pairs(&pairs)
}

#[test]
fn f1_05_an_absolute_xdg_base_is_used() {
    assert_eq!(
        token_dir(&env(Some("/home/w"), Some("/xdg"))).expect("absolute XDG"),
        PathBuf::from("/xdg/cortex")
    );
}

#[test]
fn f1_05_a_relative_or_empty_xdg_base_is_ignored() {
    for xdg in ["relative", "./x", "", "x/../y"] {
        assert_eq!(
            token_dir(&env(Some("/home/w"), Some(xdg))).expect("falls back to HOME"),
            PathBuf::from("/home/w/.config/cortex"),
            "XDG_CONFIG_HOME={xdg:?} must be ignored"
        );
    }
}

#[test]
fn f1_05_a_relative_home_is_an_error() {
    for home in ["relative", "./h", "h/w"] {
        assert!(
            matches!(token_dir(&env(Some(home), None)), Err(ConfigError::Invalid { ref key, .. }) if key == "HOME"),
            "HOME={home:?} must be refused"
        );
        // A relative XDG base does not rescue a relative HOME.
        assert!(
            token_dir(&env(Some(home), Some("rel"))).is_err(),
            "HOME={home:?} with a relative XDG must be refused"
        );
    }
}

#[test]
fn f1_05_an_empty_or_missing_home_is_missing() {
    for e in [
        env(None, None),
        env(Some(""), None),
        env(Some(""), Some("rel")),
    ] {
        assert!(matches!(token_dir(&e), Err(ConfigError::Missing(ref m)) if m == "HOME"));
    }
}

#[test]
fn f1_05_load_refuses_a_relative_home_and_ignores_a_relative_xdg() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("f1_05_token_dir_paths");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let empty = dir.join(format!("empty-{}.toml", std::process::id()));
    std::fs::write(&empty, "").expect("write config");
    assert!(
        matches!(load(&empty, &env(Some("relative"), None)), Err(ConfigError::Invalid { ref key, .. }) if key == "HOME"),
        "load with HOME=\"relative\" must be refused"
    );
    assert_eq!(
        load(&empty, &env(Some("/home/w"), Some("relative")))
            .expect("a relative XDG_CONFIG_HOME falls back to HOME")
            .paths
            .token_dir,
        PathBuf::from("/home/w/.config/cortex")
    );
}

/// What one environment must give, from `token_dir` and from `load` alike.
#[derive(Debug, Clone, Copy)]
enum Want {
    Dir(&'static str),
    Invalid(&'static str),
    Missing(&'static str),
}

/// Synthetic relative values. A refusal must never print them.
const REL: &str = "rel-sentinel-5f1a";
const REL2: &str = "rel-sentinel-9c2e/sub";

fn outcome(r: Result<PathBuf, ConfigError>) -> Result<PathBuf, (String, String, String)> {
    r.map_err(|e| {
        let key = match &e {
            ConfigError::Invalid { key, .. } => format!("Invalid({key})"),
            ConfigError::Missing(k) => format!("Missing({k})"),
            other => format!("other: {other}"),
        };
        (key, format!("{e}"), format!("{e:?}"))
    })
}

#[test]
fn f1_05_token_dir_and_load_agree_on_every_base() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("f1_05_token_dir_paths");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let empty = dir.join(format!("matrix-{}.toml", std::process::id()));
    std::fs::write(&empty, "").expect("write config");

    let cases: [(&str, Option<&str>, Option<&str>, Want); 11] = [
        // vera's fault cases
        (
            "relative XDG, absolute HOME",
            Some("/home/w"),
            Some(REL),
            Want::Dir("/home/w/.config/cortex"),
        ),
        (
            "relative HOME, no XDG",
            Some(REL),
            None,
            Want::Invalid("HOME"),
        ),
        (
            "relative HOME, relative XDG",
            Some(REL),
            Some(REL2),
            Want::Invalid("HOME"),
        ),
        (
            "relative XDG, no HOME",
            None,
            Some(REL),
            Want::Missing("HOME"),
        ),
        (
            "relative XDG, empty HOME",
            Some(""),
            Some(REL),
            Want::Missing("HOME"),
        ),
        // her positive partners
        (
            "absolute XDG, no HOME",
            None,
            Some("/xdg"),
            Want::Dir("/xdg/cortex"),
        ),
        (
            "absolute XDG, relative HOME not selected",
            Some(REL),
            Some("/xdg"),
            Want::Dir("/xdg/cortex"),
        ),
        (
            "absolute HOME, XDG unset",
            Some("/home/w"),
            None,
            Want::Dir("/home/w/.config/cortex"),
        ),
        (
            "absolute HOME, XDG empty",
            Some("/home/w"),
            Some(""),
            Want::Dir("/home/w/.config/cortex"),
        ),
        ("both absent", None, None, Want::Missing("HOME")),
        ("XDG empty, no HOME", None, Some(""), Want::Missing("HOME")),
    ];

    let mut failures = Vec::new();
    for (name, home, xdg, want) in cases {
        let e = env(home, xdg);
        let via_helper = outcome(token_dir(&e));
        let via_load = outcome(load(&empty, &e).map(|c| c.paths.token_dir));
        for (path, got) in [("token_dir", &via_helper), ("load", &via_load)] {
            let ok = match (want, got) {
                (Want::Dir(d), Ok(p)) => p.is_absolute() && p.as_path() == Path::new(d),
                (Want::Invalid(k), Err((key, display, debug))) => {
                    *key == format!("Invalid({k})")
                        && !display.contains("rel-sentinel")
                        && !debug.contains("rel-sentinel")
                }
                (Want::Missing(k), Err((key, _, _))) => *key == format!("Missing({k})"),
                _ => false,
            };
            if !ok {
                failures.push(format!("{name}: {path} gave {got:?}, want {want:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of 22 checks failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
