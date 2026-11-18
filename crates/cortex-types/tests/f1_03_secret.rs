//! F1-03 Secrets never print. [BASE] (lane A TESTS r5 §2, LA-05; S1 §2.1, T5.)
//!
//! trybuild pairs: the pass fixtures build and read a Secret and assert `ZeroizeOnDrop`; the
//! compile-fail fixtures format a Secret with `{:?}` and `{}` and must fail with the pinned
//! diagnostics in their `.stderr` files.
//!
//! RED: the baseline `Secret` derives Debug, so `secret_debug_fail` compiles and this test fails
//! at run time; `Secret::new`/`expose` are `todo!()` and `ZeroizeOnDrop` is not implemented.
//! Controls (GREEN): adding `impl Display for Secret` makes the `{}` case fail; removing `Secret`
//! makes the pass fixtures fail, so a missing type cannot pass.

#[test]
fn f1_03_secret_cannot_be_formatted_and_is_zeroised() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/secret_expose_pass.rs");
    t.pass("tests/ui/secret_zeroize_on_drop_pass.rs");
    t.compile_fail("tests/ui/secret_debug_fail.rs");
    t.compile_fail("tests/ui/secret_display_fail.rs");
}
