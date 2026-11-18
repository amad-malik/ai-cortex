//! Static checks over the crate's own source and manifest (STA-1 to STA-3, TOK-20) and the randomness condition of
//! cox's W1-32. DB-free. RED: `verify_lookup` has no call site yet (the store is stage B), so TOK-20 fails.
mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::crate_dir;

/// Every `.rs` file under `src/`, with comments removed (a line comment and the text of a `//!` or `///` doc).
fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        for e in std::fs::read_dir(dir).expect("read_dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).expect("read");
                let code: String = text
                    .lines()
                    .map(|l| match l.find("//") {
                        Some(i) if !l[..i].contains('"') => &l[..i],
                        _ => l,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                out.push((p, code));
            }
        }
    }
    let mut out = Vec::new();
    walk(&crate_dir().join("src"), &mut out);
    out
}

/// STA-1: the crate depends only on what the cells name. (`sqlx` and `axum` are here beyond the r5 list, as in
/// mod-identity: the handlers run SQL on the transaction's connection and name HTTP methods.)
#[test]
fn sta_1_the_dependencies_are_the_allowed_ones() {
    let text = std::fs::read_to_string(crate_dir().join("Cargo.toml")).expect("manifest");
    let normal: BTreeSet<String> = text
        .split("[dependencies]")
        .nth(1)
        .expect("a dependencies table")
        .split("\n[")
        .next()
        .expect("table")
        .lines()
        .filter_map(|l| l.split('=').next())
        .map(|k| k.trim().trim_end_matches(".workspace").to_owned())
        .filter(|k| !k.is_empty() && !k.starts_with('#'))
        .collect();
    let allowed: BTreeSet<String> = [
        "async-trait",
        "axum",
        "base64",
        "cortex-db",
        "cortex-module",
        "cortex-types",
        "getrandom",
        "serde",
        "serde_json",
        "sha2",
        "sqlx",
        "subtle",
        "thiserror",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    assert_eq!(normal, allowed);
}

/// STA-2 and cox's W1-32: `getrandom` is the only source of secret bytes, named in `token.rs` and nowhere else; no
/// `rand`, no time-seeded generator, no `Uuid::new_v4`, no `fastrand`/`oorandom`; the OS source is called once per
/// token, with no second source behind it.
#[test]
fn sta_2_the_operating_system_is_the_only_source_of_randomness() {
    let forbidden = [
        "rand::",
        "thread_rng",
        "fastrand",
        "oorandom",
        "new_v4",
        "SystemTime",
        "Instant::now",
        "RandomState",
    ];
    let mut getrandom_files = BTreeSet::new();
    for (path, code) in sources() {
        for f in forbidden {
            assert!(!code.contains(f), "{} contains `{f}`", path.display());
        }
        if code.contains("getrandom::") {
            getrandom_files.insert(
                path.file_name()
                    .expect("name")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    assert_eq!(
        getrandom_files,
        BTreeSet::from(["token.rs".to_owned()]),
        "getrandom is named in token.rs only"
    );
    let token = std::fs::read_to_string(crate_dir().join("src/token.rs")).expect("token.rs");
    let calls = token
        .lines()
        .filter(|l| !l.trim_start().starts_with("//") && l.contains("getrandom::fill("))
        .count();
    assert_eq!(
        calls, 1,
        "one call of the OS source in the crate, behind `generate`"
    );
}

/// STA-3: the crate reaches the `identity` and `audit` schemas only through `IdentityApi` and `AuditSink` (S1 2.3).
#[test]
fn sta_3_no_sql_on_the_identity_or_audit_schemas() {
    for (path, code) in sources() {
        for schema in ["identity.", "audit."] {
            for (n, line) in code.lines().enumerate() {
                let quoted = line.contains('"');
                assert!(
                    !(quoted && line.contains(schema)),
                    "{}:{} names the `{schema}` schema",
                    path.display(),
                    n + 1
                );
            }
        }
    }
}

/// TOK-20: `tokens.verify_lookup` has exactly ONE call site, in the store of `verify`; a handler that calls it
/// fails the scan.
#[test]
fn tok_20_verify_lookup_has_one_call_site_and_it_is_the_verifiers_store() {
    let mut sites = Vec::new();
    for (path, code) in sources() {
        for (n, line) in code.lines().enumerate() {
            if line.contains("verify_lookup") {
                sites.push(format!(
                    "{}:{}",
                    path.file_name().expect("name").to_string_lossy(),
                    n + 1
                ));
            }
        }
    }
    assert_eq!(sites.len(), 1, "exactly one call site: {sites:?}");
    assert!(
        sites[0].starts_with("store.rs:"),
        "inside the verifier's store: {sites:?}"
    );
}
