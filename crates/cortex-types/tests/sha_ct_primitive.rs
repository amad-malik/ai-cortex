//! mike's F1A-R2-03: `Sha256`'s constant-time equality goes through a reviewed primitive,
//! `subtle::ConstantTimeEq`, not a hand-written fold. This is a type-level check of WHICH
//! primitive is used; it is not timing evidence, and no timing guarantee is claimed beyond the
//! `subtle` crate's documented best effort (optimisation barriers; no guarantee from the compiler).
//! RED (baseline ca217ac): `Sha256` does not implement `subtle::ConstantTimeEq`.
#[test]
fn sha256_equality_uses_subtle() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/sha256_constant_time_eq_pass.rs");
}
