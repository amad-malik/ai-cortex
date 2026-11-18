//! F1-05, r2: `Env` never formats a value (mike's F1A-R0-01; kai's ruling in row 82054725).
//!
//! `Env` holds only `ENV_KEYS`; `get` is the one way to read a value; `Debug` names the keys and
//! prints every value as `<redacted>`; there is no iterator over values (the trybuild guards).
//! All values here are synthetic sentinels; no failure message prints a formatted `Env`.
//!
//! RED: the baseline derives `Debug` and keeps every key, so the sentinels appear in the output,
//! a key outside `ENV_KEYS` is kept, and `from_process` captures `PATH`.
use cortex_config::{ENV_KEYS, Env};

const SENTINELS: [(&str, &str); 3] = [
    ("HOME", "F1A_SENTINEL_HOME"),
    ("XDG_CONFIG_HOME", "F1A_SENTINEL_XDG"),
    ("EXAMPLE_API_TOKEN", "F1A_SENTINEL_TOKEN"),
];

#[test]
fn f1_05_env_debug_names_the_keys_and_redacts_every_value() {
    let env = Env::from_pairs(&SENTINELS);
    let shown = format!("{env:?}");
    assert!(
        !shown.contains("F1A_SENTINEL"),
        "Env's Debug printed a value"
    );
    for k in ENV_KEYS {
        assert!(shown.contains(k), "Env's Debug does not name the key {k}");
    }
    assert!(
        shown.contains("<redacted>"),
        "Env's Debug does not mark its values as redacted"
    );
    // Explicit retrieval is unchanged.
    assert_eq!(env.get("HOME"), Some("F1A_SENTINEL_HOME"));
    assert_eq!(env.get("XDG_CONFIG_HOME"), Some("F1A_SENTINEL_XDG"));
}

#[test]
fn f1_05_env_captures_only_the_named_keys() {
    assert_eq!(ENV_KEYS, &["HOME", "XDG_CONFIG_HOME"]);
    let env = Env::from_pairs(&SENTINELS);
    // assert!, not assert_eq!: a failure must not print a value.
    assert!(
        env.get("EXAMPLE_API_TOKEN").is_none(),
        "a key outside ENV_KEYS was kept"
    );
    // GREEN refinement (mike's carry item 2): the process half runs hermetically, in the child
    // below, under an environment this test controls. The old scan over the real environment
    // could reject a correct redaction when an unrelated value happened to be "<redacted>", and
    // it passed an empty capture.
}

const CHILD: &str = "F1A_ENV_HERMETIC_CHILD";

/// Re-runs this test binary with a cleared environment holding only the variables below, and
/// runs the child test there. The child prints nothing but pass or fail.
#[test]
fn f1_05_env_from_process_reads_only_the_named_keys_hermetic() {
    let exe = std::env::current_exe().expect("the test binary");
    let out = std::process::Command::new(exe)
        .env_clear()
        .env(CHILD, "1")
        .env("HOME", "F1A_PROC_HOME_SENTINEL")
        .env("XDG_CONFIG_HOME", "F1A_PROC_XDG_SENTINEL")
        .env("EXAMPLE_API_TOKEN", "F1A_PROC_TOKEN_SENTINEL")
        .env("PATH", "/usr/bin:/bin")
        .env("UNRELATED_MARKER", "<redacted>")
        .args([
            "--exact",
            "f1_05_env_from_process_child",
            "--ignored",
            "--test-threads=1",
        ])
        .output()
        .expect("the child runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "the hermetic child failed (its assertion names the property, never a value)"
    );
}

/// The child: runs only when the parent above starts it.
#[test]
#[ignore = "run by f1_05_env_from_process_reads_only_the_named_keys_hermetic in a controlled environment"]
fn f1_05_env_from_process_child() {
    assert_eq!(
        std::env::var(CHILD).as_deref(),
        Ok("1"),
        "run only by the parent test"
    );
    let p = Env::from_process();
    // Positive: the named keys are captured, with their values.
    assert!(
        p.get("HOME") == Some("F1A_PROC_HOME_SENTINEL"),
        "HOME was not captured"
    );
    assert!(
        p.get("XDG_CONFIG_HOME") == Some("F1A_PROC_XDG_SENTINEL"),
        "XDG_CONFIG_HOME was not captured"
    );
    // Negative: nothing else is captured, and no value is printed.
    assert!(p.get("PATH").is_none(), "from_process captured PATH");
    assert!(
        p.get("EXAMPLE_API_TOKEN").is_none(),
        "from_process captured EXAMPLE_API_TOKEN"
    );
    let shown = format!("{p:?}");
    assert!(
        !shown.contains("F1A_PROC_"),
        "from_process's Debug printed a value"
    );
    assert!(
        shown.contains("HOME") && shown.contains("XDG_CONFIG_HOME"),
        "Debug does not name the keys"
    );
}

#[test]
fn f1_05_env_has_no_iterator_over_values() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/env_get_pass.rs");
    t.compile_fail("tests/ui/env_iter_fail.rs");
    t.compile_fail("tests/ui/env_into_iter_fail.rs");
}
