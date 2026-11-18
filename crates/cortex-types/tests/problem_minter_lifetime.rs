//! mike's F-1a R0 r2 carry item 3: the minter's lifetime, in a process of its own (this test
//! binary takes the minter once, and only this test runs here).
//!
//! Atomic once-only acquisition under concurrent callers; no reacquisition after the holder is
//! dropped; and the capability is neither `Clone` nor `Copy` (trybuild guards).
use std::sync::{Arc, Barrier};

use cortex_types::ProblemCodeMinter;

#[test]
fn exactly_one_of_many_concurrent_takers_gets_the_minter_and_dropping_it_frees_nothing() {
    const TAKERS: usize = 16;
    let start = Arc::new(Barrier::new(TAKERS));
    let handles: Vec<_> = (0..TAKERS)
        .map(|_| {
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                ProblemCodeMinter::take()
            })
        })
        .collect();
    let mut won: Vec<ProblemCodeMinter> = handles
        .into_iter()
        .filter_map(|h| h.join().expect("a taker panicked"))
        .collect();
    assert_eq!(won.len(), 1, "{} takers got a minter", won.len());
    let minter = won.pop().expect("one winner");
    assert!(
        minter.mint("target_retired").is_ok(),
        "the winner's minter works"
    );
    drop(minter);
    assert!(
        ProblemCodeMinter::take().is_none(),
        "the minter was handed out again after its holder was dropped"
    );
}

#[test]
fn the_minter_is_neither_clone_nor_copy() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/problem_minter_clone_fail.rs");
    t.compile_fail("tests/ui/problem_minter_copy_fail.rs");
}
