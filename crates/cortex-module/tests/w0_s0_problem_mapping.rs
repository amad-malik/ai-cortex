//! Wave 0 S0: `DbError` and `PortError` as a `Problem`, with mira's mapping (signatures r2
//! section 3), so a read handler that opens its own transaction refuses as the chain does.
//! DB-free. Written with the mapping in one commit (not RED-first).
use cortex_db::{DbError, LockStep};
use cortex_module::{PortError, Problem};

fn code_status(p: Problem) -> (&'static str, u16) {
    (p.code, p.status)
}

#[test]
fn w0_s0_a_database_error_maps_by_kind_only() {
    let cases = [
        (DbError::Busy, ("busy", 429)),
        (DbError::StatementTimeout, ("unavailable", 503)),
        (DbError::SessionEnded, ("unavailable", 503)),
        (DbError::Unavailable, ("unavailable", 503)),
        (
            DbError::Constraint {
                name: "secret_constraint_name".into(),
            },
            ("conflict", 409),
        ),
        (DbError::PermissionDenied, ("internal_error", 500)),
        (DbError::DelegationFailed, ("internal_error", 500)),
        (
            DbError::LockOrder {
                held: LockStep::OwnerRows,
                wanted: LockStep::OwnerRows,
            },
            ("internal_error", 500),
        ),
        (DbError::UnknownModule, ("internal_error", 500)),
        (DbError::Isolation, ("internal_error", 500)),
        (
            DbError::Other("sqlstate XX000 row data".into()),
            ("internal_error", 500),
        ),
    ];
    for (e, want) in cases {
        let name = format!("{e:?}");
        let p = Problem::from(e);
        assert_eq!(code_status(p.clone()), want, "{name}");
        // No message, constraint name or row text reaches the problem.
        let text = format!("{} {} {}", p.title, p.detail, p.fix);
        for leak in ["secret_constraint_name", "XX000", "row data"] {
            assert!(!text.contains(leak), "{name} leaks {leak}: {text}");
        }
        assert_eq!(p.scope, None);
    }
}

#[test]
fn w0_s0_a_port_error_maps_to_the_chains_codes() {
    let cases = [
        (PortError::NotFound, ("not_found", 404)),
        (PortError::Invalid("reason_code"), ("invalid_request", 422)),
        (PortError::Forbidden, ("target_not_allowed", 403)),
        (PortError::Conflict("reason_code"), ("conflict", 409)),
        (PortError::Unavailable, ("unavailable", 503)),
        (PortError::Db(DbError::Busy), ("busy", 429)),
    ];
    for (e, want) in cases {
        let p = Problem::from(e);
        let text = format!("{} {} {}", p.title, p.detail, p.fix);
        assert!(
            !text.contains("reason_code"),
            "the port's reason is not sent: {text}"
        );
        assert_eq!(code_status(p), want);
    }
}
