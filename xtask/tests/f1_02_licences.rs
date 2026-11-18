//! F1-02 Licence gate. [BASE] (lane A TESTS r5 §2, LA-05; SPEC §2.4; G10; D86.)
//!
//! `cargo deny` accepts this workspace under the committed `deny.toml`, and `xtask licences`,
//! run with that same file, refuses fixture crates declaring SSPL-1.0, AGPL-3.0-only and
//! BUSL-1.1 and accepts one declaring MIT.
//!
//! Scaffold, not RED: the tool versions (`cargo deny --version`, `cargo audit --version`) are
//! recorded in the RED receipt; a missing tool is a blocker. The workspace case checks licences,
//! bans and sources; advisories, which change over time, are the R1 `cargo deny check` gate.
//! RED: `xtask::licences` is `todo!()`. Control (GREEN): a `deny.toml` that allows SSPL-1.0 makes
//! the SSPL fixture pass, and this test fails.
use std::path::{Path, PathBuf};
use std::process::Command;

use xtask::{LicenceVerdict, licences};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/licences")
        .join(name)
        .join("Cargo.toml")
}

#[test]
fn f1_02_cargo_deny_accepts_this_workspace() {
    // cargo-deny 0.20 takes --manifest-path and --config before the subcommand.
    let out = Command::new("cargo")
        .args(["deny", "--manifest-path"])
        .arg(root().join("Cargo.toml"))
        .arg("--config")
        .arg(root().join("deny.toml"))
        .args(["check", "licenses", "bans", "sources"])
        .output()
        .expect("cargo deny runs");
    assert!(
        out.status.success(),
        "cargo deny refused the workspace:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn f1_02_xtask_licences_refuses_sspl_agpl_busl_and_accepts_mit() {
    let deny = root().join("deny.toml");
    for refused in ["sspl", "agpl", "busl"] {
        let verdict = licences(&fixture(refused), &deny).expect("licences runs");
        assert!(
            matches!(verdict, LicenceVerdict::Refused { .. }),
            "{refused}: want Refused, got {verdict:?}"
        );
    }
    assert_eq!(
        licences(&fixture("mit"), &deny).expect("licences runs"),
        LicenceVerdict::Accepted
    );
}
