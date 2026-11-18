//! PO-05 (compile-fail, trybuild): a route cannot be built without a scope or without a `TargetRule`,
//! and a `match` on `TargetRule` with a variant missing does not compile. The pass fixture is the
//! control: it proves the fixture setup compiles what it should, so a failure above it means something.
//! DB-free. Expected compiler output lives next to each fixture (`*.stderr`).
#[test]
fn po05_a_route_needs_a_scope_and_a_rule_and_a_match_needs_every_arm() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/po05_complete_match_pass.rs");
    t.compile_fail("tests/ui/po05_route_without_a_target_rule_fail.rs");
    t.compile_fail("tests/ui/po05_route_without_a_scope_fail.rs");
    t.compile_fail("tests/ui/po05_match_missing_an_arm_fail.rs");
}
