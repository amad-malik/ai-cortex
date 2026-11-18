//! `cortex-testkit`: test-only support (LAYOUT §7, r7). Only ever a `[dev-dependency]`; `xtask
//! fitness` refuses it anywhere else (F1-01).
//!
//! **The test database (kai's L-9).** `supervise.sh`, an admitted supervisor (Z-2's accepted r3
//! `run.sh`, renamed), owns one throwaway Postgres 18 per admitted run, passes its port and
//! superuser password in `CV2_PG_PORT` and `CV2_PG_PASSWORD`, and stops, removes and verifies
//! the container on every exit path. **Test binaries never start containers.** `test_db()`
//! connects to it and gives each test binary a database of its own; the roles (S1 §9) are
//! cluster-wide and created idempotently.
//!
//! Run: `cv2-admit timeout --kill-after=120 780 crates/cortex-testkit/supervise.sh cargo test …`

/// The F1-31 maintenance fixtures; banked for v0.2.001 with `cortex-db`'s `maintenance` feature.
#[cfg(feature = "maintenance")]
pub mod maint;
pub mod pg;
pub mod spy;

pub use cortex_types::TestClock;
pub use pg::{
    PIECE_ROLES, SessionId, TestDb, ctx, empty_db, isolated_db, module, same_session,
    session_identity, sqlstate, test_db, within,
};
