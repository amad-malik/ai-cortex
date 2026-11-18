//! After the Part A freeze: busy is 429, not 409 (cox's ruling 45e33f74 Q3; freeze note r1
//! `mvp/WAVE0-FREEZE-NOTE.md`, "busy"; row c36ee928). A lock held by another request is a reason
//! to retry later, not a conflict with the state. The chain adds `Retry-After` (mira). S0 owns the
//! status. The code, the title, the detail and the fix stay as they were. DB-free.
//!
//! RED: S0 still maps `DbError::Busy` to 409.
use cortex_db::DbError;
use cortex_module::{PortError, Problem};

#[test]
fn w0_s0_busy_is_429_and_keeps_its_code_and_fix() {
    for p in [
        Problem::from(DbError::Busy),
        Problem::from(PortError::Db(DbError::Busy)),
    ] {
        assert_eq!((p.code, p.status), ("busy", 429));
        assert_eq!(p.fix, "Retry with the same Idempotency-Key.");
        assert_eq!(p.scope, None);
    }
}

#[test]
fn w0_s0_a_conflict_stays_409() {
    // Control: only busy moves; a constraint or a port conflict is still 409 `conflict`.
    let c = Problem::from(DbError::Constraint {
        name: String::new(),
    });
    assert_eq!((c.code, c.status), ("conflict", 409));
    let p = Problem::from(PortError::Conflict("reason"));
    assert_eq!((p.code, p.status), ("conflict", 409));
}
