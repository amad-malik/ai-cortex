//! EN-01 and DP-06 (source scans): the environment is read in ONE module; the library shells out to nothing, has no
//! build script and no unsafe. A scan is a guard, not a proof: it reads our source, not our dependencies (a dependency
//! may read its own variables; RT-05 pins the proxy case).
use std::fs;
use std::path::{Path, PathBuf};

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The code of a line: everything before a `//` comment. Naive on purpose (a `//` inside a string hides the rest of
/// that line); a violation on its own line is always seen.
fn code(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

fn own_sources() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut v = Vec::new();
    rs_files(&root.join("src"), &mut v);
    let cli = root.join("../cortex-cli/src");
    if cli.is_dir() {
        rs_files(&cli, &mut v);
    }
    v
}

#[test]
fn en_01_only_env_rs_reads_the_environment() {
    let files = own_sources();
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    for must in ["lib.rs", "env.rs", "token.rs"] {
        assert!(
            names.iter().any(|n| n == must),
            "the scan did not see {must}: {names:?}"
        );
    }
    let banned = ["env::var", "env::vars", "var_os", "set_var", "remove_var"];
    let mut env_rs_reads = false;
    for f in &files {
        let is_env_rs = f.ends_with("cortex-client/src/env.rs");
        let text = fs::read_to_string(f).unwrap();
        for (i, line) in text.lines().enumerate() {
            let c = code(line);
            for b in banned {
                if c.contains(b) {
                    assert!(
                        is_env_rs,
                        "EN-01: {}:{} reads the environment outside env.rs: {line}",
                        f.display(),
                        i + 1
                    );
                    env_rs_reads = true;
                }
            }
        }
    }
    assert!(
        env_rs_reads,
        "positive control: env.rs itself must read the environment"
    );
}

#[test]
fn dp_06_no_process_spawn_no_unsafe_no_build_script() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for f in own_sources() {
        let text = fs::read_to_string(&f).unwrap();
        for (i, line) in text.lines().enumerate() {
            let c = code(line);
            for b in [
                "std::process::Command",
                "process::Command",
                "unsafe ",
                "unsafe{",
            ] {
                assert!(
                    !c.contains(b),
                    "DP-06: {}:{} uses {b}: {line}",
                    f.display(),
                    i + 1
                );
            }
        }
    }
    assert!(
        !root.join("build.rs").exists(),
        "DP-06: cortex-client has a build script"
    );
    assert!(
        !root.join("../cortex-cli/build.rs").exists(),
        "DP-06: cortex-cli has a build script"
    );
}
