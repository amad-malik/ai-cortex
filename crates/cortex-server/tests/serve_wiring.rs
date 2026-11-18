//! The wiring the server binds once its pool exists (piece 11 `serve`, mira; found 7 Nov 13:0xZ by `serve_real.rs` sr3: on the real set EVERY
//! authenticated request was 503 `unavailable` at the verify step, because nothing handed the pool to `mod-tokens`' verifier, which the
//! piece documents as "Wiring for the server (piece 11)" and which fails CLOSED until bound). DB-free: what the compiled list carries.
//! The binding itself, against a real database, is `serve_real.rs` (sr3). RED first.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cortex_server::assembly::{Wiring, compiled, compiled_with_wiring};

#[test]
fn ww1_the_compiled_list_comes_with_the_wiring_of_the_tokens_verifier_and_nothing_else() {
    let (pieces, wiring) = compiled_with_wiring();
    let with: Vec<&str> = pieces.iter().map(|p| p.manifest().id).collect();
    let plain: Vec<&str> = compiled().iter().map(|p| p.manifest().id).collect();
    assert_eq!(with, plain, "the same pieces in the same order");
    assert!(with.contains(&"tokens"), "tokens is compiled in: {with:?}");
    assert_eq!(
        wiring.len(),
        1,
        "mod-tokens' verifier is the one piece that needs the pool bound after assembly"
    );
}

#[test]
fn ww2_a_fixture_run_has_no_wiring_and_with_adds_one_binder_per_call() {
    assert!(Wiring::none().is_empty());
    assert_eq!(Wiring::none().len(), 0);
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let w = Wiring::none()
        .with(move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        })
        .with(|_| {});
    assert_eq!(w.len(), 2);
    assert!(!w.is_empty());
    assert_eq!(
        w.clone().len(),
        2,
        "it is cheap to clone: the binders are shared"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "nothing runs until a pool is bound"
    );
}
