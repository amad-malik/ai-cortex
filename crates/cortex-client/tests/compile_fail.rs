//! TF-11 (trybuild): the token type is ZeroizeOnDrop and is not Clone, Display or Serialize. The pass fixture is the
//! control: a failure above it means something. DB-free.
#[test]
fn tf_11_the_token_type_has_the_right_traits() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/tf11_token_zeroize_on_drop_pass.rs");
    t.compile_fail("tests/ui/tf11_token_clone_fail.rs");
    t.compile_fail("tests/ui/tf11_token_display_fail.rs");
    t.compile_fail("tests/ui/tf11_token_serialize_fail.rs");
}
