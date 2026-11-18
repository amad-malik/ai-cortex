//! S3 r2.7: the project-scoped audit list is an authenticated read.
use axum::http::Method;
use cortex_module::{Piece, Ports, TargetRule};
use mod_audit::AuditPiece;

#[test]
fn audit_list_registers_as_a_strict_query_read_for_audit_scope() {
    let piece = AuditPiece::new();
    let routes = piece.routes(&Ports::default());
    let entries: Vec<_> = routes.entries().collect();
    assert_eq!(entries.len(), 1);
    let route = entries[0];
    assert_eq!(route.method, Method::GET);
    assert_eq!(route.path, "/v2/audit");
    assert_eq!(route.scope, "audit:read");
    assert_eq!(route.target, TargetRule::None);
    assert!(!route.write);
    assert!(route.query);
}
