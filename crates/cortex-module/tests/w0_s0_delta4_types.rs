//! Wave 0 S0 delta 4 (cox c025db40; kai 68cecbdb): the type-level proofs, as trybuild pairs.
//!
//! What the types guarantee (the compile-fail fixtures in `tests/ui/d4_*`):
//! - an `Authorized<T>` cannot be made except by `Ctx::authorize` (no literal, no constructor);
//! - an `Authorization` (the erased proof the replay lookup takes by `&`) cannot be made outside the
//!   crate, so a lookup that takes one **cannot run before `authorize`**: this is the type-level
//!   counterpart of the "lookup before authorize" mutation (kai's condition), whose BEHAVIOURAL twin
//!   needs the lookup and lives in mira's chain half (sam's R-19 cells);
//! - `Ctx::authorize` takes no rule; `execute` takes the `Authorized<T>`; `authorize` returns one;
//! - a `Ctx` cannot be written by hand.
//!
//! RED: the stubs are OPEN (`Authorized` and `Authorization` have public fields), so the two
//! literal fixtures COMPILE when they must not; `Routes::write` has one closure, so the pass
//! fixture does not compile. `d4_authorize_with_a_rule_fail`, `d4_execute_without_authorized_fail`,
//! `d4_authorize_returns_plain_fail` and `d4_ctx_literal_fail` fail to compile already (they are
//! shape pins, not RED for the intended reason); GREEN pins their diagnostics and the mutation
//! receipts show each one has teeth.
#[test]
fn d4_t_1_the_proof_cannot_be_made_or_skipped_outside_ctx_authorize() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/d4_two_closures_pass.rs");
    t.compile_fail("tests/ui/d4_authorized_literal_fail.rs");
    t.compile_fail("tests/ui/d4_authorization_literal_fail.rs");
    t.compile_fail("tests/ui/d4_authorize_with_a_rule_fail.rs");
    t.compile_fail("tests/ui/d4_authorize_returns_plain_fail.rs");
    t.compile_fail("tests/ui/d4_execute_without_authorized_fail.rs");
    t.compile_fail("tests/ui/d4_ctx_literal_fail.rs");
}
