//! Wave 1, mod-handoffs rework of hale's F-1 (review r0 of piece 8; cox W1-39): ONE rule for every date-time input of
//! a body, a query or a cursor, refused with 422 `invalid_request`, never a database error and never a 500.
//!
//! The rule (W1-39): RFC 3339 `YYYY-MM-DDTHH:MM:SS[.f{1,9}](Z|+HH:MM|-HH:MM)`, upper-case `T` and `Z`, a real calendar
//! date, year 1970 to 9999, hour 0-23, minute 0-59, second 0-59, offset minutes 0-59 and the offset within
//! -12:00 .. +14:00; NUL or any other character anywhere is a reject. Not normalised to UTC.
//!
//! The shared vectors are cox's `mvp/api/rfc3339-vectors.json` (14 accept, 48 reject), copied into this crate
//! (copied under `tests/data`, hash in the file name). This file is DB-free and runs the create body's `due_at` through
//! this piece's validator; `w1_handoffs_p5_rfc3339_db.rs` runs the same lists through the routes and the list cursor.

use cortex_module::Problem;
use mod_handoffs::body::CreateBody;
use mod_handoffs::validate::validate_create;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Cox's `mvp/api/rfc3339-vectors.json`, copied here so the cells are hermetic (a runner without `/path/to/docs` runs
/// them, and a later edit of cox's file cannot change them silently); the hash is in the file name.
const RAW: &str = include_str!("data/rfc3339-vectors.4125a4b4.json");

fn vectors() -> (Vec<String>, Vec<String>) {
    let v: Value = serde_json::from_str(RAW).expect("the vectors are JSON");
    let list = |k: &str| -> Vec<String> {
        v[k].as_array()
            .unwrap_or_else(|| panic!("{k} is a list"))
            .iter()
            .map(|s| s.as_str().expect("a string").to_owned())
            .collect()
    };
    (list("accept"), list("reject"))
}

fn body(due_at: &str) -> CreateBody {
    serde_json::from_value(json!({
        "kind": "task",
        "to": { "kind": "job_role", "ref": "dev" },
        "summary": "s",
        "next_action": "n",
        "acceptance": "a",
        "due_at": due_at
    }))
    .expect("a body that decodes")
}

fn pair(p: &Problem) -> (u16, &'static str) {
    (p.status, p.code)
}

/// The copy is the file cox published: its sha256 starts with the hash in its name.
#[test]
fn the_vendored_vectors_are_the_ones_cox_published() {
    let hex: String = Sha256::digest(RAW.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(hex.starts_with("4125a4b4"), "{hex}");
}

#[test]
fn the_vector_file_has_the_lists_cox_published() {
    let (accept, reject) = vectors();
    assert_eq!((accept.len(), reject.len()), (14, 48));
}

/// Every accept vector passes the validator.
#[test]
fn due_at_accepts_every_shared_accept_vector() {
    let (accept, _) = vectors();
    let wrong: Vec<String> = accept
        .iter()
        .filter_map(|s| {
            validate_create(body(s))
                .err()
                .map(|p| format!("{s:?} -> {p:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "refused but valid:\n{}", wrong.join("\n"));
}

/// Every reject vector is 422 `invalid_request` at step 0.
#[test]
fn due_at_refuses_every_shared_reject_vector_with_422() {
    let (_, reject) = vectors();
    let wrong: Vec<String> = reject
        .iter()
        .filter_map(|s| match validate_create(body(s)) {
            Err(p) if pair(&p) == (422, "invalid_request") => None,
            Err(p) => Some(format!("{s:?} -> {}", p.status)),
            Ok(_) => Some(format!("{s:?} -> accepted")),
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} of {} reject vectors were not refused with 422:\n{}",
        wrong.len(),
        reject.len(),
        wrong.join("\n")
    );
}

/// hale's measured cases, by name: the offsets and the year Postgres refused (22009, 22008) and the edges of the
/// rule that Postgres would take (so the rule, not the database, is what refuses them).
#[test]
fn due_at_edges_of_the_rule() {
    for ok in [
        "2030-01-01T00:00:00+14:00",
        "2030-01-01T00:00:00-12:00",
        "1970-01-01T00:00:00Z",
        "9999-12-31T23:59:59.999999999Z",
    ] {
        validate_create(body(ok)).unwrap_or_else(|p| panic!("{ok}: {}", p.status));
    }
    for bad in [
        "2030-01-01T00:00:00+16:00", // Postgres: 22009
        "2030-01-01T00:00:00-16:00", // Postgres: 22009
        "2030-01-01T00:00:00+23:59", // Postgres: 22009
        "2030-01-01T00:00:00-23:59", // Postgres: 22009
        "0000-01-01T00:00:00Z",      // Postgres: 22008
        "2030-01-01T00:00:00+15:59", // Postgres would take it; the rule does not
        "2030-01-01T00:00:00-12:01",
        "0001-01-01T00:00:00Z", // Postgres would take it; the rule starts at 1970
    ] {
        assert_eq!(
            validate_create(body(bad)).err().map(|p| pair(&p)),
            Some((422, "invalid_request")),
            "{bad}"
        );
    }
}
