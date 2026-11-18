//! Wave 1 `wave1/ports2` item 2 (cox W1-33; sam's BOOT12-01; tate 4056cbbe): `HandoffsApi::open_for` is REPLACED by
//! two methods, because a claimed and a pending list answer two different questions with two different mandatory
//! behaviours. A flood of high-priority pending work must never push the worker's newest claimed handoff out of a
//! capped list.
//!
//! - `claimed_by(worker, limit)`: the worker's CLAIMED handoffs, newest claim first, then id.
//! - `pending_for(worker, limit)`: the handoffs addressed to the worker (directly or by job role) that are PENDING,
//!   priority descending, `created_at` ascending, id.
//! - The cap, 1000, is per method: [`HANDOFF_LIST_CAP`]. The ordering and the cap are the provider's (bob's
//!   HAND-110 in two cells, and a flood cell); the trait fixes the shape, and the doc says what each call returns.
//!
//! DB-free. RED: the trait still has `open_for`, and `HANDOFF_LIST_CAP` does not exist, so this file does not compile.
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_db::ModuleTx;
use cortex_module::{HANDOFF_LIST_CAP, HandoffView, HandoffsApi, PortError};
use cortex_types::WorkerId;

/// Records which method was called, and answers empty.
struct Recorder {
    calls: Mutex<Vec<&'static str>>,
}

#[async_trait]
impl HandoffsApi for Recorder {
    async fn claimed_by(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        _limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        self.calls.lock().expect("lock").push("claimed_by");
        Ok(vec![])
    }
    async fn pending_for(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        _limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        self.calls.lock().expect("lock").push("pending_for");
        Ok(vec![])
    }
}

#[test]
fn w1_hs_1_a_provider_with_the_two_methods_is_a_handoffs_port() {
    let r = Arc::new(Recorder {
        calls: Mutex::new(vec![]),
    });
    // Object safe: this is how `PortsBuilder::handoffs` takes it (the registry cells cover the registration).
    let port: Arc<dyn HandoffsApi> = r.clone();
    drop(port);
    assert!(r.calls.lock().expect("lock").is_empty());
}

#[test]
fn w1_hs_2_the_cap_is_one_thousand_per_method() {
    assert_eq!(HANDOFF_LIST_CAP, 1000);
}

/// `open_for` is gone: a provider that still defines it does not compile (the trait has no such method), so no
/// consumer can keep calling the mixed, capped list.
#[test]
fn w1_hs_3_open_for_is_removed() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/w1_open_for_removed_fail.rs");
}
