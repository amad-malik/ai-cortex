//! AUD-04 / S4: the audit piece declares its isolation boundary.
use cortex_module::{DefaultRole, Piece, Port, ScopeClass};
use mod_audit::AuditPiece;

#[test]
fn audit_piece_declares_its_own_port_and_project_lead_scope() {
    let piece = AuditPiece::new();
    let manifest = piece.manifest();
    assert_eq!(manifest.id, "audit");
    assert_eq!(manifest.schema, "audit");
    assert_eq!(manifest.provides, &[Port::Audit]);
    let scope = manifest
        .scopes
        .iter()
        .find(|s| s.name == "audit:read")
        .unwrap();
    assert_eq!(scope.class, ScopeClass::Admin);
    assert_eq!(scope.default_roles, &[DefaultRole::ProjectLead]);
    assert_eq!(piece.migrations().schema, "audit");
    assert!(!piece.migrations().steps.is_empty());
}
