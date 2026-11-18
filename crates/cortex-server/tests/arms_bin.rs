//! The real `cortex-server migrate` and `cortex-server bootstrap` processes: exit code, stdout and stderr (piece 11, cox). DB-free: every
//! case is refused before a database is touched. The full path (assembly of the real pieces, a database) is `tests/arms.rs` and, with
//! the real five pieces, Wave 2. RED first.
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn run(args: &[&str]) -> (Option<i32>, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex-server"));
    c.args(args);
    // The config loader reads HOME; nothing else of the environment matters here.
    c.env_clear().env("HOME", "/home/x");
    let out = c.output().expect("the binary runs");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn config(name: &str, text: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("ab-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    let f = d.join("cortex.toml");
    fs::write(&f, text).expect("config");
    f
}

fn missing() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ab-no-such-dir/canary-arm-dir/cortex.toml")
}

#[test]
fn ab1_migrate_with_a_config_that_cannot_be_read_exits_1_with_one_stderr_line_and_nothing_on_stdout()
 {
    let m = missing();
    let (code, stdout, stderr) = run(&["migrate", "--config", m.to_str().expect("utf8")]);
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert!(
        stderr.starts_with("migrate refused: configuration: "),
        "{stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(!stderr.contains("canary-arm-dir"), "{stderr}");
}

#[test]
fn ab2_bootstrap_with_a_config_that_cannot_be_read_exits_1_with_one_stderr_line_and_writes_no_token_file()
 {
    let m = missing();
    let token = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("ab-never-{}.token", std::process::id()));
    let _ = fs::remove_file(&token);
    let (code, stdout, stderr) = run(&[
        "bootstrap",
        "--config",
        m.to_str().expect("utf8"),
        "--project",
        "zz-p",
        "--lead",
        "zz-l",
        "--token-file",
        token.to_str().expect("utf8"),
    ]);
    assert_eq!(code, Some(1));
    assert_eq!(stdout, "");
    assert!(
        stderr.starts_with("bootstrap refused: configuration: "),
        "{stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(!stderr.contains("canary-arm-dir"), "{stderr}");
    assert!(!token.exists(), "no token file");
}

#[test]
fn ab3_a_module_enabled_twice_is_refused_by_both_commands_naming_it() {
    let p = config(
        "twice",
        "[modules]\nenabled = [\"identity\", \"identity\"]\n",
    );
    let path = p.to_str().expect("utf8");
    let (code, _, stderr) = run(&["migrate", "--config", path]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr,
        "migrate refused: module identity is enabled twice\n"
    );
    let (code, _, stderr) = run(&[
        "bootstrap",
        "--config",
        path,
        "--project",
        "zz-p",
        "--lead",
        "zz-l",
        "--token-file",
        "/nonexistent-dir/never.token",
    ]);
    assert_eq!(code, Some(1));
    assert_eq!(
        stderr,
        "bootstrap refused: module identity is enabled twice\n"
    );
}
