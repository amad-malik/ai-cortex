//! AUD-07: action-specific detail validation, without a database.
use cortex_module::ports::AuditDetails;
use cortex_types::{ProjectId, WorkerId};
use mod_audit::validate_details;

#[test]
fn audit_rejects_unknown_action_and_unexpected_detail() {
    assert!(validate_details("not.registered", &AuditDetails::new()).is_err());
    let unexpected = AuditDetails::new().flag("self_review", true).unwrap();
    assert!(validate_details("handoff.created", &unexpected).is_err());
    assert!(validate_details("handoff.accepted", &unexpected).is_ok());
}

#[test]
fn token_reason_is_bounded_by_contract_not_only_socket_grammar() {
    let good = AuditDetails::new().code("reason", "lost_key").unwrap();
    let numeric = AuditDetails::new().code("reason", "lost1").unwrap();
    assert!(validate_details("token.revoked", &good).is_ok());
    assert!(validate_details("token.revoked", &numeric).is_err());
    assert!(
        AuditDetails::new()
            .code("reason", "ctx2_01234567890123456789012345678901.secret")
            .is_err()
    );
}

#[test]
fn team_assignment_always_records_change_flag() {
    let empty = AuditDetails::new();
    assert!(validate_details("worker.team_assigned", &empty).is_err());
}

#[test]
fn bootstrap_has_exactly_typed_facts_and_created_action_is_reserved() {
    let project = ProjectId::parse("01930000-0000-7000-8000-000000000001").unwrap();
    let lead = WorkerId::parse("01930000-0000-7000-8000-000000000002").unwrap();
    let details = AuditDetails::new()
        .id("project", project)
        .unwrap()
        .id("lead", lead)
        .unwrap()
        .flag("created", true)
        .unwrap()
        .flag("reissued", false)
        .unwrap();
    assert!(validate_details("project.bootstrapped", &details).is_ok());
    assert!(validate_details("project.bootstrapped", &AuditDetails::new()).is_err());
    assert!(validate_details("project.created", &AuditDetails::new()).is_err());
}
