//! The real `cortex-server serve` process: exit code and stderr (piece 11 `serve` S4c, mira). DB-free: every case is refused before a
//! database is touched.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn run(config: &PathBuf, fault: Option<&str>) -> (Option<i32>, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex-server"));
    c.arg("serve").arg("--config").arg(config);
    // The config loader reads HOME; nothing else of the environment matters here.
    c.env_clear().env("HOME", "/home/x");
    if let Some(f) = fault {
        c.env("CORTEX_TEST_FAULT", f);
    }
    let out = c.output().expect("the binary runs");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn config(name: &str, text: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("sb-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    let f = d.join("cortex.toml");
    fs::write(&f, text).expect("config");
    f
}

#[test]
fn sb1_a_config_that_cannot_be_read_exits_1_with_one_stderr_line_and_nothing_on_stdout() {
    let missing = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("sb-no-such-dir/canary-bin-dir/cortex.toml");
    let (code, stdout, stderr) = run(&missing, None);
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert!(
        stderr.starts_with("serve refused: configuration: "),
        "{stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(!stderr.contains("canary-bin-dir"), "{stderr}");
}

#[test]
fn sb2_a_module_enabled_twice_exits_1_naming_it() {
    let p = config(
        "twice",
        "[modules]\nenabled = [\"identity\", \"identity\"]\n",
    );
    let (code, _, stderr) = run(&p, None);
    assert_eq!(code, Some(1));
    assert_eq!(stderr, "serve refused: module identity is enabled twice\n");
}

#[test]
fn sb3_a_test_fault_variable_this_build_cannot_use_is_warned_about_before_the_refusal() {
    let p = config(
        "fault",
        "[modules]\nenabled = [\"identity\", \"identity\"]\n",
    );
    let (code, _, stderr) = run(&p, Some("bogus-canary-fault"));
    assert_eq!(code, Some(1));
    let warning = cortex_server::fault::warning(Some("bogus-canary-fault"))
        .expect("a value no build knows is always warned about");
    assert_eq!(
        stderr,
        format!("serve: {warning}\nserve refused: module identity is enabled twice\n")
    );
    assert!(
        !stderr.contains("bogus-canary-fault"),
        "the value is never echoed"
    );
}
