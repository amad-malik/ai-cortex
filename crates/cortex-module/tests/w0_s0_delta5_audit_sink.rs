//! Wave 0 S0 delta 5, part 2: the shape of `AuditSink::record` (cox 825a9022; DV-3 kai e351c8ca).
//! DB-free: a `ModuleTx` needs a database, so what is pinned here is the SIGNATURE, by a spy that
//! implements the trait, and that the trait stays object-safe and usable behind `Arc<dyn …>`.
//!
//! The signature is `record(tx, action, target, details)`: **no `Outcome` parameter** (DV-3: 0.2.001
//! audits committed operations only, so the outcome is always `allowed`; refusals are a log line, not
//! a row; a later denial log is a separate method, so this one does not change twice).
//!
//! RED: the trait has three parameters; the spy's four do not match (compile error E0050).
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_db::ModuleTx;
use cortex_module::{AuditDetails, AuditSink, AuditTarget, PortError};

/// Keeps what it was given, as text; nothing else.
#[derive(Default)]
struct Spy {
    seen: Mutex<Vec<(String, AuditTarget, String)>>,
}

#[async_trait]
impl AuditSink for Spy {
    async fn record(
        &self,
        _tx: &mut ModuleTx,
        action: &'static str,
        target: AuditTarget,
        details: AuditDetails,
    ) -> Result<(), PortError> {
        self.seen.lock().expect("spy lock").push((
            action.to_owned(),
            target,
            details.render_json(),
        ));
        Ok(())
    }
}

#[test]
fn d5_as_1_the_sink_takes_tx_action_target_and_details() {
    // The impl above IS the check: a fifth parameter (an `Outcome`) or a missing `details` would
    // not match the trait, and this file would not compile.
    let sink: Arc<dyn AuditSink> = Arc::new(Spy::default());
    // Object safety and the `Send + Sync` supertraits: the sink is shared across tasks.
    fn shared<T: Send + Sync + ?Sized>(_: &T) {}
    shared(&*sink);
}

#[test]
fn d5_as_2_the_details_value_can_cross_a_task_boundary() {
    // `async_trait` makes the returned future `Send`; that needs `AuditDetails: Send`.
    fn send<T: Send + Sync + 'static>() {}
    send::<AuditDetails>();
}

#[test]
fn d5_as_3_an_empty_details_is_the_no_detail_case() {
    // Most audited operations have nothing to add. `AuditDetails::new()` is the one call for that.
    let d = AuditDetails::new();
    assert!(d.is_empty());
    assert_eq!(d.len(), 0);
    assert_eq!(d.render_json(), "{}");
    assert_eq!(d, AuditDetails::default());
}
