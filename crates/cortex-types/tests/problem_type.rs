//! mike's F1A-R0-02 public-contract fixture (kai's ruling in row 82054725): a platform code and a
//! module-owned code reach `ProblemType` with their own, distinct wire text.
//!
//! RED: `code`, `is_platform`, `ProblemCodeMinter::take` and `mint` are `todo!()`. The forge
//! guards already hold on the baseline (the field is private); they keep it so.
use cortex_types::{ProblemCodeError, ProblemCodeMinter, ProblemType};

/// CONTRACTS §5's stable platform codes, in its order.
const PLATFORM: [&str; 13] = [
    "token_missing",
    "token_invalid",
    "missing_scope",
    "target_not_allowed",
    "not_found",
    "restore_in_progress",
    "maintenance",
    "capability_unavailable",
    "idempotency_key_required",
    "idempotency_key_reused",
    "already_issued",
    "body_too_large",
    "invalid_request",
];

#[test]
fn platform_codes_are_stable_and_marked_platform() {
    let got: Vec<&str> = ProblemType::PLATFORM.iter().map(|t| t.code()).collect();
    assert_eq!(got, PLATFORM);
    assert_eq!(ProblemType::MISSING_SCOPE.code(), "missing_scope");
    assert!(ProblemType::PLATFORM.iter().all(|t| t.is_platform()));
}

/// The only test in this binary that takes the minter: `take` succeeds once per process.
#[test]
fn a_module_code_is_minted_by_the_one_minter_and_keeps_its_own_text() {
    let minter = ProblemCodeMinter::take().expect("the first take succeeds");
    assert!(
        ProblemCodeMinter::take().is_none(),
        "a second minter was handed out"
    );
    let retired = minter.mint("target_retired").expect("I-2's module code");
    assert_eq!(retired.code(), "target_retired");
    assert!(!retired.is_platform());
    assert_ne!(
        retired,
        ProblemType::INVALID_REQUEST,
        "a module code must not collapse into invalid_request"
    );
    let expired = minter.mint("token_expired").expect("S1 §5.4's token code");
    assert_eq!(expired.code(), "token_expired");
    assert_ne!(expired, retired);
    assert_eq!(
        minter.mint("Bad-Code"),
        Err(ProblemCodeError::BadCode("Bad-Code".into()))
    );
    assert_eq!(
        minter.mint(""),
        Err(ProblemCodeError::BadCode(String::new()))
    );
    assert_eq!(
        minter.mint("missing_scope"),
        Err(ProblemCodeError::PlatformCode("missing_scope"))
    );
}

#[test]
fn a_problem_type_cannot_be_forged() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/problem_type_const_pass.rs");
    t.compile_fail("tests/ui/problem_type_forge_fail.rs");
    t.compile_fail("tests/ui/problem_minter_forge_fail.rs");
}
