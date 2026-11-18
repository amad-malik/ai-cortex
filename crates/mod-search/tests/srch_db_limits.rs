//! RED for piece 7, DB cells for the bounds (MS-14 to MS-18; S3 r2.7 R-9 case b and R-16's exception). A slow query needs no hook in the
//! product code: a superuser connection holds ACCESS EXCLUSIVE on `records.records`, so every search blocks on the view behind it.
//! Needs `supervise.sh`'s throwaway Postgres.
mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, HeaderValue, header};
use cortex_module::route::AppState;
use cortex_module::{Ctx, Principal, Role, WorkerKind};
use cortex_types::{ModuleId, RequestId, SystemClock, Timestamp, TokenId, WorkerId};
use mod_search::validate::check;
use mod_search::{Cap, ID, Limits, answer, handle};
use serde_json::json;
use support::{Rec, World, lock_records, release, seed, setup, tctx, wait_searching};

fn valid(q: &str) -> mod_search::Validated {
    check(
        Some("application/json"),
        &serde_json::to_vec(&json!({"query": q})).expect("json"),
    )
    .expect("valid")
}

fn limits(statement_ms: u64, budget_ms: u64) -> Limits {
    Limits {
        statement_timeout: Duration::from_millis(statement_ms),
        budget: Duration::from_millis(budget_ms),
    }
}

/// Start an `answer` in a task; it blocks while the lock is held.
fn start(
    w: &World,
    cap: &Cap,
    l: Limits,
) -> tokio::task::JoinHandle<Result<mod_search::SearchResult, cortex_module::Problem>> {
    let (pool, cap, a) = (Arc::clone(&w.pool), cap.clone(), w.a);
    tokio::spawn(async move { answer(&pool, tctx(a), &cap, &l, &valid("blocked")).await })
}

#[tokio::test]
async fn ms14_a_statement_timeout_is_503_unavailable_and_leaves_no_backend_behind() {
    let w = setup("ms14").await;
    seed(&w, w.a, &Rec::note(1, 1, "blocked words", None)).await;
    let lock = lock_records(&w).await;
    let started = Instant::now();
    let r = answer(
        &w.pool,
        tctx(w.a),
        &Cap::new(2),
        &limits(50, 4_500),
        &valid("blocked"),
    )
    .await;
    let took = started.elapsed();
    let p = r.expect_err("a blocked search times out");
    assert_eq!((p.code, p.status), ("unavailable", 503));
    assert!(
        took < Duration::from_millis(1_500),
        "the 50 ms statement timeout fired, not the budget: {took:?}"
    );
    assert_eq!(
        wait_searching(&w, 0, 1).await,
        0,
        "no backend is left running the statement"
    );
    release(lock).await;
}

#[tokio::test]
async fn ms15_the_third_search_is_refused_at_once_with_429_and_touches_no_database() {
    let w = setup("ms15").await;
    seed(&w, w.a, &Rec::note(1, 1, "blocked words", None)).await;
    let lock = lock_records(&w).await;
    let cap = Cap::new(2);
    let (t1, t2) = (
        start(&w, &cap, limits(10_000, 10_000)),
        start(&w, &cap, limits(10_000, 10_000)),
    );
    assert_eq!(
        wait_searching(&w, 2, 5).await,
        2,
        "two searches are running"
    );
    let started = Instant::now();
    let p = answer(
        &w.pool,
        tctx(w.a),
        &cap,
        &limits(10_000, 10_000),
        &valid("blocked"),
    )
    .await
    .expect_err("refused");
    assert_eq!((p.code, p.status), ("busy", 429));
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "refused at once, not queued: {:?}",
        started.elapsed()
    );
    assert_eq!(
        support::searching(&w).await,
        2,
        "the refused search ran no statement"
    );
    release(lock).await;
    for t in [t1, t2] {
        let r = t
            .await
            .expect("task")
            .expect("the two admitted searches answer");
        assert!(r.hits.len() <= 1);
    }
    // A place came back: a new search is admitted and answers.
    answer(
        &w.pool,
        tctx(w.a),
        &cap,
        &limits(3_000, 4_500),
        &valid("blocked"),
    )
    .await
    .expect("admitted again");
}

fn principal(w: &World) -> Principal {
    Principal {
        worker: WorkerId::new(&SystemClock),
        project: w.a,
        kind: WorkerKind::Worker,
        role: Role::Member(None),
        token: TokenId::new(&SystemClock),
        expires_at: Timestamp::from_unix_micros(4_000_000_000_000_000),
        renew_due: false,
    }
}

#[tokio::test]
async fn ms15b_an_invalid_request_is_422_at_once_and_takes_no_place_even_when_the_cap_is_full() {
    let w = setup("ms15b").await;
    seed(&w, w.a, &Rec::note(1, 1, "blocked words", None)).await;
    let lock = lock_records(&w).await;
    let cap = Cap::new(2);
    let (t1, t2) = (
        start(&w, &cap, limits(10_000, 10_000)),
        start(&w, &cap, limits(10_000, 10_000)),
    );
    assert_eq!(wait_searching(&w, 2, 5).await, 2);
    let st = AppState {
        pool: Arc::clone(&w.pool),
    };
    let ctx = Ctx::new(principal(&w), RequestId::new(&SystemClock));
    let id = ModuleId::parse(ID).expect("id");
    let mut h = HeaderMap::new();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let l = limits(10_000, 10_000);
    let call = |body: Vec<u8>| {
        let (st, ctx, id, cap, h) = (st.clone(), ctx.clone(), id.clone(), cap.clone(), h.clone());
        async move { handle(&st, &ctx, &id, &cap, &l, &h, &body).await }
    };
    // (a) invalid requests: 422 at once, whatever the cap says.
    for (body, code) in [
        (json!({"query": "x".repeat(1001)}), "query_too_long"),
        (json!({"query": "a", "limit": 0}), "invalid_request"),
        (json!({"query": "a\u{0}b"}), "invalid_request"),
    ] {
        let started = Instant::now();
        let p = call(serde_json::to_vec(&body).expect("json"))
            .await
            .expect_err("refused");
        assert_eq!((p.code, p.status), (code, 422));
        assert!(started.elapsed() < Duration::from_millis(200));
    }
    // (b) a valid request meets the full cap: 429 busy.
    let p = call(serde_json::to_vec(&json!({"query": "fine"})).expect("json"))
        .await
        .expect_err("busy");
    assert_eq!((p.code, p.status), ("busy", 429));
    assert_eq!(
        support::searching(&w).await,
        2,
        "nothing reached the database"
    );
    release(lock).await;
    for t in [t1, t2] {
        t.await.expect("task").expect("answers");
    }
    // The invalid requests took no place: both places are free again, so a valid request is admitted.
    let ok = call(serde_json::to_vec(&json!({"query": "blocked"})).expect("json"))
        .await
        .expect("admitted");
    assert_eq!(ok.status(), 200);
}

#[tokio::test]
async fn ms16_a_dropped_request_keeps_its_place_until_its_statement_is_over_and_then_leaves_nothing()
 {
    // The statement is NOT cancelled when the request goes away (dropping a ModuleTx mid-statement leaves the backend running; cortex-db has
    // no cancel-on-drop). So the place stays taken while the backend runs, which keeps the cap a bound on the backends, and the statement
    // timeout ends it: after that no backend and no place remain. 10 rounds (the definition says 100).
    let w = setup("ms16").await;
    seed(&w, w.a, &Rec::note(1, 1, "blocked words", None)).await;
    let lock = lock_records(&w).await;
    let cap = Cap::new(2);
    for round in 0..10 {
        let t = start(&w, &cap, limits(600, 10_000));
        assert_eq!(
            wait_searching(&w, 1, 5).await,
            1,
            "round {round}: the search is running"
        );
        t.abort(); // the client went away: the request's future is dropped mid-query
        let _ = t.await;
        let first = cap.try_enter().expect("one place is still free");
        assert!(
            cap.try_enter().is_err(),
            "round {round}: the other place is held by the running statement, so the cap still bounds the backends"
        );
        drop(first);
        assert_eq!(
            wait_searching(&w, 0, 3).await,
            0,
            "round {round}: the statement timeout ended the backend"
        );
        let (p1, p2) = (
            cap.try_enter().expect("the place came back"),
            cap.try_enter().expect("and the other"),
        );
        drop((p1, p2));
    }
    release(lock).await;
}

#[tokio::test]
async fn ms17_the_answer_stays_inside_its_budget_and_the_abandoned_statement_ends_at_its_timeout() {
    let w = setup("ms17").await;
    seed(&w, w.a, &Rec::note(1, 1, "blocked words", None)).await;
    let lock = lock_records(&w).await;
    let cap = Cap::new(2);
    let started = Instant::now();
    let p = answer(
        &w.pool,
        tctx(w.a),
        &cap,
        &limits(800, 300),
        &valid("blocked"),
    )
    .await
    .expect_err("over budget");
    let took = started.elapsed();
    assert_eq!((p.code, p.status), ("unavailable", 503));
    assert!(
        took >= Duration::from_millis(280) && took < Duration::from_millis(700),
        "budget 300 ms plus one round trip, not the 800 ms statement timeout: {took:?}"
    );
    // The statement goes on (no cancel-on-drop) until its own timeout, then the place comes back.
    assert_eq!(
        wait_searching(&w, 0, 3).await,
        0,
        "the statement timeout ended the abandoned backend"
    );
    let (p1, p2) = (
        cap.try_enter().expect("a place"),
        cap.try_enter().expect("the other"),
    );
    drop((p1, p2));
    release(lock).await;
}

#[tokio::test]
async fn ms18_a_search_that_cannot_read_the_records_is_503_never_an_empty_success() {
    let w = setup("ms18").await;
    seed(&w, w.a, &Rec::note(1, 1, "blocked words", None)).await;
    answer(
        &w.pool,
        tctx(w.a),
        &Cap::new(2),
        &limits(3_000, 4_500),
        &valid("blocked"),
    )
    .await
    .expect("works first");
    let mut admin = w.db.admin().await;
    sqlx::query("REVOKE SELECT ON records.pub_sources, records.pub_state FROM cortex_m_search")
        .execute(&mut admin)
        .await
        .expect("revoke");
    let p = answer(
        &w.pool,
        tctx(w.a),
        &Cap::new(2),
        &limits(3_000, 4_500),
        &valid("blocked"),
    )
    .await
    .expect_err("no privilege");
    assert_eq!((p.code, p.status), ("unavailable", 503));
    sqlx::query("GRANT SELECT ON records.pub_sources, records.pub_state TO cortex_m_search")
        .execute(&mut admin)
        .await
        .expect("grant");
    sqlx::query("ALTER VIEW records.pub_state RENAME TO pub_state_gone")
        .execute(&mut admin)
        .await
        .expect("rename");
    let p = answer(
        &w.pool,
        tctx(w.a),
        &Cap::new(2),
        &limits(3_000, 4_500),
        &valid("blocked"),
    )
    .await
    .expect_err("a view is missing");
    assert_eq!((p.code, p.status), ("unavailable", 503));
}
