//! RED for piece 7, DB-free: search's OWN concurrency cap (MS-15, MS-15b; S3 r2.7 R-9 case b and R-16's exception). At most two searches at
//! once; the third is refused AT ONCE with 429 `busy` and never queued (v1 incident 912073e2); a place comes back when its permit drops.

use std::time::Duration;

use mod_search::cap::Cap;

#[test]
fn ms15_the_third_search_is_refused_at_once_and_a_place_comes_back() {
    let cap = Cap::new(2);
    let a = cap.try_enter().expect("the first");
    let b = cap.try_enter().expect("the second");
    let busy = match cap.try_enter() {
        Err(p) => p,
        Ok(_) => panic!("the third must be refused"),
    };
    assert_eq!((busy.code, busy.status), ("busy", 429));
    // Refused means refused: asking again while full is still refused, nothing queues behind it.
    assert!(cap.try_enter().is_err());
    drop(a);
    let c = cap.try_enter().expect("a place came back");
    assert!(cap.try_enter().is_err(), "full again");
    drop(b);
    drop(c);
    cap.try_enter().expect("empty again");
}

#[tokio::test]
async fn ms15_a_refused_request_does_not_wait_and_a_clone_shares_the_places() {
    let cap = Cap::new(2);
    let clone = cap.clone();
    let _a = cap.try_enter().expect("one");
    let _b = clone.try_enter().expect("two, through the clone");
    // try_enter is synchronous: it answers at once even on a runtime with paused time.
    let r = tokio::time::timeout(Duration::from_millis(50), async {
        cap.try_enter().is_err()
    })
    .await;
    assert_eq!(r, Ok(true));
}

#[test]
fn ms15_a_place_is_released_when_the_holder_panics() {
    let cap = Cap::new(1);
    let c2 = cap.clone();
    let r = std::panic::catch_unwind(move || {
        let _p = c2.try_enter().expect("the place");
        panic!("the search panicked while holding it");
    });
    assert!(r.is_err());
    cap.try_enter().expect("the unwind dropped the permit");
}
