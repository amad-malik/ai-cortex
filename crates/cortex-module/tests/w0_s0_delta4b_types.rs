//! Wave 0 S0 delta 4b (cox 271405dd and f0e25a51; vera's DELTA4-REWORK-REVIEW.md `5c2acb05`): `Ctx`'s
//! identity has ONE source and no public field. `principal`, `project` and `request_id` are read through
//! accessors; assigning any of them, or reading a field directly, does not compile outside the crate, so
//! a clone cannot carry an altered principal, project or request id into `Ctx::actor` or `Ctx::begin`.
//!
//! RED: the fields are still public and the accessors read them, so the four compile-fail fixtures
//! COMPILE when they must not (trybuild); the accessor pass fixture passes before and after.
#[test]
fn d4b_t_1_ctx_identity_is_read_through_accessors_and_cannot_be_assigned_from_outside() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/d4b_accessors_pass.rs");
    t.compile_fail("tests/ui/d4b_assign_principal_fail.rs");
    t.compile_fail("tests/ui/d4b_assign_project_fail.rs");
    t.compile_fail("tests/ui/d4b_assign_request_id_fail.rs");
    t.compile_fail("tests/ui/d4b_read_field_fail.rs");
}
