//! F1-01 Crate graph. [BASE] (lane A TESTS r5 §2, LA-05; S1 §2.1; strategy P8.)
//!
//! `fitness` passes this workspace and a clean fixture, and fails a fixture for each rule:
//! a `mod-*` crate on another `mod-*` crate (`mod-tokens` -> `mod-identity`); `cortex-http` on a
//! `mod-*` crate; a shipped crate on `cortex-testkit` outside `[dev-dependencies]`. The clean
//! fixture uses `cortex-testkit` as a dev-dependency, which is allowed.
//!
//! RED: `xtask::fitness` is `todo!()`. Control (GREEN): with the `mod-*` rule removed, the
//! `mod-mod` fixture passes and this test fails.
use std::path::{Path, PathBuf};

use xtask::{Rule, Violation, fitness};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fitness")
        .join(name)
        .join("Cargo.toml")
}

fn v(rule: Rule, krate: &str, depends_on: &str) -> Violation {
    Violation {
        rule,
        krate: krate.into(),
        depends_on: depends_on.into(),
    }
}

#[test]
fn f1_01_this_workspace_passes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("Cargo.toml");
    assert_eq!(fitness(&root).expect("fitness runs"), vec![]);
}

#[test]
fn f1_01_clean_fixture_passes() {
    assert_eq!(fitness(&fixture("clean")).expect("fitness runs"), vec![]);
}

#[test]
fn f1_01_a_mod_crate_on_another_mod_crate_fails() {
    assert_eq!(
        fitness(&fixture("mod-mod")).expect("fitness runs"),
        vec![v(Rule::ModDependsOnMod, "mod-tokens", "mod-identity")]
    );
}

#[test]
fn f1_01_cortex_http_on_a_mod_crate_fails() {
    assert_eq!(
        fitness(&fixture("http-mod")).expect("fitness runs"),
        vec![v(Rule::HttpDependsOnMod, "cortex-http", "mod-identity")]
    );
}

#[test]
fn f1_01_a_shipped_crate_on_testkit_outside_dev_dependencies_fails() {
    assert_eq!(
        fitness(&fixture("testkit-normal")).expect("fitness runs"),
        vec![v(
            Rule::ShippedDependsOnTestkit,
            "cortex-server",
            "cortex-testkit"
        )]
    );
}
