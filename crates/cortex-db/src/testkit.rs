//! Test-only switches, compiled only with the `testkit` feature (F1-16). Each is a process-wide
//! flag, so a test that sets one runs in a test binary of its own.
use std::sync::atomic::{AtomicBool, Ordering};

static NONLOCAL: AtomicBool = AtomicBool::new(false);
static SEAL_OFF: AtomicBool = AtomicBool::new(false);

/// Z-2's nonlocal control (F1-06, vera's Z2-ROLE-N01): `begin` sets the role and project with
/// session scope instead of transaction scope.
pub fn set_nonlocal(on: bool) {
    NONLOCAL.store(on, Ordering::SeqCst);
}

/// F1-32's and I1-07's inverted-order control: the audit seal is not enforced.
pub fn set_seal_off(on: bool) {
    SEAL_OFF.store(on, Ordering::SeqCst);
}

pub(crate) fn nonlocal() -> bool {
    NONLOCAL.load(Ordering::SeqCst)
}

pub(crate) fn seal_off() -> bool {
    SEAL_OFF.load(Ordering::SeqCst)
}

/// F1-06's session-reuse oracle (TESTS r7 F1-06, Z-2's Z2-2): under this feature, every new pool
/// connection sets this setting to a fresh random UUID with session scope, before its first use.
/// The oracle is the backend pid plus this token. A custom setting stays readable under
/// `SET LOCAL ROLE`; `pg_stat_activity.backend_start` does not (it reads NULL for a session whose
/// user the current role cannot act as), so it cannot be the oracle inside a `ModuleTx`.
pub const SESSION_TOKEN_GUC: &str = "cortex_test.session_token";

/// The pool's own connections, for a test's "next transaction" that sets no role (the leak
/// checks of F1-06). An accessor, not a behaviour under test.
pub fn raw_pool(pool: &crate::DbPool) -> &sqlx::PgPool {
    &pool.pool
}

static CANCELLED: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

/// The cancel-on-drop oracle (lane A; cox 14:48Z, nemo's finding): the backend pids a dropped `ModuleTx` sent a
/// cancel for, in order. A normal drop (nothing in flight) never appears here.
pub fn cancelled_backends() -> Vec<i32> {
    CANCELLED.lock().expect("cancelled lock").clone()
}

pub(crate) fn note_cancel(pid: i32) {
    CANCELLED.lock().expect("cancelled lock").push(pid);
}

// ---- vera's DBCANCEL-L1 knobs ------------------------------------------------------------------------------------

static CANCEL_REFUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static DRAIN_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static FINISHED: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

/// Makes the cancel connection of an abandoned transaction fail as if the database refused it. Process-wide: a test
/// that sets it runs in a test binary of its own.
pub fn set_cancel_refused(on: bool) {
    CANCEL_REFUSED.store(on, std::sync::atomic::Ordering::SeqCst);
}

/// Shortens the wait for the rollback after the cancel (0 restores the default). Process-wide, like the above.
pub fn set_drain_timeout_ms(ms: u64) {
    DRAIN_MS.store(ms, std::sync::atomic::Ordering::SeqCst);
}

/// The backend pids whose abandoned-transaction cleanup task has ended (successfully or by its drain timeout).
pub fn finished_cleanups() -> Vec<i32> {
    FINISHED.lock().expect("finished lock").clone()
}

pub(crate) fn cancel_refused() -> bool {
    CANCEL_REFUSED.load(std::sync::atomic::Ordering::SeqCst)
}

pub(crate) fn drain_override() -> Option<std::time::Duration> {
    match DRAIN_MS.load(std::sync::atomic::Ordering::SeqCst) {
        0 => None,
        ms => Some(std::time::Duration::from_millis(ms)),
    }
}

pub(crate) fn note_finished(pid: i32) {
    FINISHED.lock().expect("finished lock").push(pid);
}
