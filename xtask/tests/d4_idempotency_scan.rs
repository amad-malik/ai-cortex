//! Delta 4 (cox c025db40; kai 68cecbdb) and its rework (vera D4-V2; cox's boundary ruling, row 16d9cdd2):
//! only the chain's one wrapper (`cortex-http/src/idem.rs`) calls `idempotency::begin` and
//! `idempotency::finish`; `cortex-db` itself (its own code and tests) is exempt. This is finn's half of
//! the fitness rule (workspace-wide); mira's half is a DB-free test of `cortex-http`'s own `src/`. The
//! type system cannot stop a piece calling `cortex_db::idempotency` (it is `pub`); a piece that does
//! still cannot return a stored reply (only the chain renders one), and this rule catches the call.
//!
//! **The boundary (cox's ruling, vera reviews it):** the scanner covers PRODUCT SOURCE: the `.rs` files
//! under a `src/` directory of every crate, and `build.rs`. `tests/`, `benches/` and `examples/` are
//! EXEMPT (cargo never links them into the binary); a `#[cfg(test)]` module inside `src/` stays
//! scanned. In scanned files comments and string literals (plain, raw, byte) are removed before
//! matching. A `use ...idempotency::{..}` list naming `begin`, `finish` or `self` (on one line or
//! several), a glob import `idempotency::*` and an alias of the module (`idempotency as x`) count.
//!
//! Negative controls (cox): (1) a real call in a fixture `src/` file IS flagged; (2) the same call
//! under a fixture `tests/` is NOT; (3) the pattern inside a string literal in `src/` is NOT; (4) an
//! import with a rename (`begin as x`) in `src/` IS flagged. Mine: a multi-line list, a module alias,
//! `{self}`, a `#[cfg(test)]` module in `src/`, `build.rs`, another file of `cortex-http` than `idem.rs`,
//! and (delta 4b; vera's nonblocking limitation) the raw-identifier spelling `idempotency::r#begin(`/`r#finish(`.
//!
//! RED: the old scanner reads test files and string literals, so the `idem-ok` fixture is flagged and
//! the new forms are not found.
use std::path::{Path, PathBuf};

use xtask::{Rule, Violation, fitness};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fitness")
        .join(name)
        .join("Cargo.toml")
}

fn v(krate: &str, depends_on: &str) -> Violation {
    Violation {
        rule: Rule::IdempotencyOutsideTheChain,
        krate: krate.into(),
        depends_on: depends_on.into(),
    }
}

#[test]
fn d4_x_1_wrapper_cortex_db_tests_benches_examples_comments_and_literals_are_not_calls() {
    assert_eq!(fitness(&fixture("idem-ok")).expect("fitness runs"), vec![]);
}

#[test]
fn d4_x_2_a_piece_calling_begin_or_finish_is_found_in_every_form_in_product_source() {
    assert_eq!(
        fitness(&fixture("idem-bad")).expect("fitness runs"),
        vec![
            v(
                "cortex-http",
                "idempotency::finish (cortex-http/src/other.rs:2)"
            ),
            v(
                "mod-identity",
                "idempotency::{..} (mod-identity/src/lib.rs:1)"
            ),
            v(
                "mod-identity",
                "idempotency::{..} (mod-identity/src/multi.rs:1)"
            ),
            v(
                "mod-policy",
                "idempotency (alias) (mod-policy/src/alias.rs:1)"
            ),
            v("mod-policy", "idempotency::* (mod-policy/src/lib.rs:1)"),
            v("mod-policy", "idempotency::begin (mod-policy/build.rs:1)"),
            v("mod-tokens", "idempotency::begin (mod-tokens/src/lib.rs:3)"),
            v(
                "mod-tokens",
                "idempotency::begin (mod-tokens/src/rawident.rs:2)"
            ),
            v(
                "mod-tokens",
                "idempotency::finish (mod-tokens/src/cfgtest.rs:4)"
            ),
            v(
                "mod-tokens",
                "idempotency::finish (mod-tokens/src/rawident.rs:3)"
            ),
            v(
                "mod-tokens",
                "idempotency::{..} (mod-tokens/src/selfl.rs:1)"
            ),
        ],
        "a call, a raw-identifier call, a rename import, a multi-line list, an alias, {{self}}, a glob, build.rs, a #[cfg(test)] \
         module in src/ and another file of cortex-http are found; the comment line, cortex-db and idem.rs are not"
    );
}

#[test]
fn d4_x_3_this_workspace_still_passes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("Cargo.toml");
    assert_eq!(fitness(&root).expect("fitness runs"), vec![]);
}
