//! Wave 1, mod-handoffs P4d (row 2a86598d): the piece's problem codes against the S3 r2.7 twin (cox W1-31: "add a
//! cell that your manifest's entry equals the twin's"). DB-free.
//!
//! The data file `data/s3-r2.7-handoffs-codes.1d2c4b2f.json` is an extract of the pinned JSON twin
//! (`x-error-codes.pieces`, sha256 `1d2c4b2f...`): the thirteen codes this piece mints and their statuses. The
//! twin gives a STATUS per code and no title or fix text; the text lives in each piece. A code that several pieces
//! declare (`reason_required`, `target_retired`) must have identical code, status, title and fix in all of them,
//! which the registry enforces (finn, W1-31). Cox ruled the text in W1-34: ONE source, `mvp/api/shared-problem-codes.json`
//! (read-only), copied under `tests/data` with its hash in the name.

use std::collections::BTreeSet;

use mod_handoffs::PROBLEMS;
use serde_json::Value;

fn twin() -> Value {
    serde_json::from_str(include_str!("data/s3-r2.7-handoffs-codes.1d2c4b2f.json"))
        .expect("the vendored extract is JSON")
}

#[test]
fn the_extract_is_of_the_pinned_twin() {
    let t = twin();
    assert!(
        t["sha256"].as_str().expect("sha").starts_with("1d2c4b2f"),
        "r2.7 JSON twin"
    );
}

/// Every code the piece mints has the status the twin gives it, and the twin lists no code of this piece that the
/// piece does not mint.
#[test]
fn each_problem_code_has_the_twins_status_and_the_sets_are_equal() {
    let t = twin();
    let codes = t["codes"].as_object().expect("codes");
    for spec in PROBLEMS {
        let status = codes
            .get(spec.code)
            .unwrap_or_else(|| panic!("{} is not in the twin", spec.code))
            .as_u64()
            .expect("a status");
        assert_eq!(u64::from(spec.status), status, "{}", spec.code);
    }
    let mine: BTreeSet<&str> = PROBLEMS.iter().map(|s| s.code).collect();
    let theirs: BTreeSet<&str> = codes.keys().map(String::as_str).collect();
    assert_eq!(mine, theirs, "the piece mints exactly the twin's thirteen");
    assert_eq!(PROBLEMS.len(), 13, "no code twice in the piece's own list");
}

/// A refusal carries a fix (S3 rule: "every refusal carries fix"); a title and a fix are never empty and never
/// the same words.
#[test]
fn each_problem_code_has_a_title_and_a_distinct_fix() {
    for spec in PROBLEMS {
        assert!(!spec.title.trim().is_empty(), "{} title", spec.code);
        assert!(!spec.fix.trim().is_empty(), "{} fix", spec.code);
        assert_ne!(spec.title, spec.fix, "{}", spec.code);
        assert!(
            spec.code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{} is a bare snake_case code",
            spec.code
        );
    }
}

/// Cox's `mvp/api/shared-problem-codes.json` (W1-34), copied here so the cell is hermetic; the hash is in the file name.
const SHARED: &str = include_str!("data/shared-problem-codes.13573a0e.json");

fn shared() -> Value {
    serde_json::from_str(SHARED).expect("the shared codes file is JSON")
}

/// The copy is the file cox published: its sha256 starts with the hash in its name.
#[test]
fn the_vendored_shared_codes_are_the_ones_cox_published() {
    use sha2::{Digest, Sha256};
    let hex: String = Sha256::digest(SHARED.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(hex.starts_with("13573a0e"), "{hex}");
}

/// W1-34: every code in the one source that this piece declares is declared with EXACTLY its four fields, and
/// the piece declares both of the codes the source lists (the source says handoffs is one of the pieces).
#[test]
fn the_shared_codes_are_declared_exactly_as_the_one_source_gives_them() {
    let shared = shared();
    let entries = shared["codes"].as_array().expect("codes");
    assert!(entries.len() >= 2, "the source lists the shared codes");
    let mut seen = BTreeSet::new();
    for e in entries {
        let code = e["code"].as_str().expect("code");
        let spec = PROBLEMS
            .iter()
            .find(|s| s.code == code)
            .unwrap_or_else(|| panic!("{code} is shared and this piece must declare it"));
        assert_eq!(
            u64::from(spec.status),
            e["status"].as_u64().expect("status"),
            "{code} status"
        );
        assert_eq!(
            spec.title,
            e["title"].as_str().expect("title"),
            "{code} title"
        );
        assert_eq!(spec.fix, e["fix"].as_str().expect("fix"), "{code} fix");
        seen.insert(code.to_owned());
    }
    assert!(seen.contains("reason_required") && seen.contains("target_retired"));
}
