//! The `test-hooks` feature (cox W1-56, mike's HOOKS finding on packet 15 r1): `Engine::run_hooked` and `Hooks` are test machinery in a production crate. The
//! feature is declared, is empty, is NOT a default feature, gates both items, and no release recipe names it. DB-free. RED first.

use std::path::Path;

fn manifest() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).expect("manifest")
}

fn features(toml: &str) -> Vec<String> {
    toml.split("\n[")
        .find(|t| t.starts_with("features]"))
        .map(|t| {
            t.lines()
                .skip(1)
                .take_while(|l| !l.starts_with('['))
                .map(|l| l.trim().to_string())
                .collect()
        })
        .expect("a [features] table")
}

#[test]
fn tf_01_test_hooks_is_declared_empty_and_not_default() {
    let f = features(&manifest());
    assert!(f.iter().any(|l| l == "test-hooks = []"), "{f:?}");
    assert!(
        !f.iter().any(|l| l.starts_with("default")),
        "no default feature may exist (it could turn test-hooks on): {f:?}"
    );
}

#[test]
fn tf_02_the_hooks_and_run_hooked_exist_only_behind_the_feature() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/engine.rs"))
        .expect("engine.rs");
    for item in ["pub struct Hooks<'a> {", "pub fn run_hooked(&self"] {
        let at = src.find(item).unwrap_or_else(|| panic!("{item} is gone"));
        let before: Vec<&str> = src[..at].lines().rev().take(4).collect();
        assert!(
            before
                .iter()
                .any(|l| l.trim() == "#[cfg(feature = \"test-hooks\")]"),
            "{item} must sit directly under #[cfg(feature = \"test-hooks\")]: {before:?}"
        );
    }
}

#[test]
fn tf_03_no_release_recipe_names_the_feature() {
    // the release recipe: the Containerfile, every shell script under deploy/ and scripts/, and the cargo configuration
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["deploy", "scripts", ".cargo"] {
        let d = root.join(dir);
        if let Ok(rd) = std::fs::read_dir(&d) {
            files.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_file()));
        }
    }
    assert!(!files.is_empty(), "no recipe file found under {root:?}");
    for f in files {
        // only text recipes; the test-hooks word may appear in a comment that forbids it, but never as a feature to build with
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let l = line.trim_start();
            if l.starts_with('#') {
                continue;
            }
            assert!(
                !(l.contains("test-hooks")
                    && (l.contains("--features") || l.contains("features="))),
                "{}:{}: a recipe names the test-hooks feature: {line}",
                f.display(),
                n + 1
            );
        }
    }
}
