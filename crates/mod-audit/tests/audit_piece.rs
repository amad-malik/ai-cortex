//! W1-37: the audit provider depends on identity at the piece boundary.
use cortex_module::{manifest::Port, piece::Piece};
use mod_audit::AuditPiece;

#[test]
fn audit_piece_declares_its_provider_and_identity_dependency() {
    let piece = AuditPiece::default();
    let manifest = piece.manifest();

    assert!(manifest.provides.contains(&Port::Audit));
    assert!(manifest.uses.contains(&Port::Identity));
}
