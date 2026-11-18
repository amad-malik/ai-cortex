//! Delta 4b (cox 271405dd; vera's DELTA4-REWORK-REVIEW.md `5c2acb05`): "private fields alone do not stop
//! arbitrary trusted Rust from calling the public `Ctx::new`". So `Ctx::new` and `with_route` may be
//! used only in the PRODUCT SOURCE of `cortex-http` (the chain, which makes the one `Ctx` per request)
//! and in `cortex-module` (which defines them). Same boundary and same scanner as the idempotency rule:
//! `src/` of every crate and `build.rs`; `tests/`, `benches/`, `examples/` exempt; a `#[cfg(test)]`
//! module in `src/` stays scanned; comments and literals stripped.
//!
//! Negative controls: a call from a piece's `src/` is flagged (also spaced, split over two lines, in a
//! `#[cfg(test)]` module, in `build.rs`, and `.with_route(` on a clone); the same under `tests/`,
//! `benches/` and `examples/` is NOT; the text in a comment or a string literal is NOT; `cortex-http`'s
//! and `cortex-module`'s own source is NOT; another type that merely ends in `Ctx` is NOT.
//!
//! RED: the rule does not exist, so the bad fixture reports nothing.
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
        rule: Rule::CtxConstructedOutsideTheChain,
        krate: krate.into(),
        depends_on: depends_on.into(),
    }
}

#[test]
fn d4b_x_1_the_chain_the_definer_tests_comments_literals_and_other_ctx_types_are_not_flagged() {
    assert_eq!(fitness(&fixture("ctx-ok")).expect("fitness runs"), vec![]);
}

#[test]
fn d4b_x_2_a_piece_making_or_replacing_a_ctx_is_found_in_every_form() {
    assert_eq!(
        fitness(&fixture("ctx-bad")).expect("fitness runs"),
        vec![
            v("mod-identity", "with_route (mod-identity/src/lib.rs:2)"),
            v("mod-policy", "Ctx::new (mod-policy/src/cfgtest.rs:3)"),
            v("mod-policy", "Ctx::new (mod-policy/src/lib.rs:1)"),
            v("mod-policy", "Ctx::new (mod-policy/src/split.rs:2)"),
            v("mod-tokens", "Ctx::new (mod-tokens/build.rs:1)"),
            v("mod-tokens", "Ctx::new (mod-tokens/src/lib.rs:3)"),
        ],
        "a call, a spaced call, a call split over two lines, a #[cfg(test)] module in src/, build.rs and \
         .with_route( on a clone are found; the comment, cortex-http and cortex-module are not"
    );
}

#[test]
fn d4b_x_3_this_workspace_makes_no_ctx_outside_the_chain() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("Cargo.toml");
    assert_eq!(fitness(&root).expect("fitness runs"), vec![]);
}
