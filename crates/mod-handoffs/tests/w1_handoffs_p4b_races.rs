//! Wave 1, mod-handoffs P4b (row 2a86598d): the races of `mvp/tests/mod-handoffs.md` r3.4 that need several
//! requests in flight at once: HAND-48 (two returns), HAND-59 (accept against rework, one key twice, two keys),
//! HAND-74 (cancel against return, 50 rounds), HAND-84 (one key at once) and HAND-101 (the log is a chain, 20
//! mixed verbs at once, 20 rounds). HAND-30 (eight claims) is in `w1_handoffs_p2_read_claim.rs`.
//!
//! The requests go through the real chain on separate pool connections (pool of 16, multi-thread runtime). A 429
//! `busy` is retried with the SAME key, up to five times (R-9). Run ONLY under `crates/cortex-testkit/supervise.sh`
//! through `cv2-admit`, `CV2_DB=1`, one run at a time.
mod common;

use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request};
use common::{Seed, Who, key, project_a, seed_return, world};
use cortex_types::HandoffId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

fn path(id: HandoffId, verb: &str) -> String {
    format!("/v2/handoffs/{id}/{verb}")
}

fn report() -> Value {
    let sha: String = Sha256::digest(b"r")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({ "summary": "done", "risks": "none", "tests": ["cargo test"],
            "evidence": [{ "kind": "link", "ref": "r", "sha256": sha }] })
}

#[derive(Debug, Clone)]
struct Shot {
    status: u16,
    ty: Option<String>,
    replayed: bool,
    json: Value,
    tries: u32,
}

impl Shot {
    fn is(&self, status: u16, ty: &str) -> bool {
        self.status == status && self.ty.as_deref() == Some(ty)
    }
}

/// One POST through the chain; a 429 is retried with the same key, at most five times.
async fn fire(app: Router, who: Who, p: String, k: String, body: Option<Value>) -> Shot {
    let mut tries = 0;
    loop {
        tries += 1;
        let mut req = Request::builder()
            .method(Method::POST)
            .uri(&p)
            .header("authorization", format!("Bearer {}", who.token()))
            .header("idempotency-key", &k);
        let text = match &body {
            Some(b) => {
                req = req.header("content-type", "application/json");
                b.to_string()
            }
            None => String::new(),
        };
        let resp = app
            .clone()
            .oneshot(req.body(Body::from(text)).expect("a request"))
            .await
            .expect("a response");
        let status = resp.status().as_u16();
        let replayed = resp.headers().contains_key("cortex-idempotent-replay");
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.expect("a body");
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if status == 429 && tries < 6 {
            tokio::time::sleep(Duration::from_millis(15 * u64::from(tries))).await;
            continue;
        }
        return Shot {
            status,
            ty: json["type"].as_str().map(str::to_owned),
            replayed,
            json,
            tries,
        };
    }
}

/// One request of a race: who, path, key, body.
type Job = (Router, Who, String, String, Option<Value>);

/// Fire all of them at once (spawned, so they truly overlap) and return the answers in input order.
async fn together(jobs: Vec<Job>) -> Vec<Shot> {
    together_lagged(jobs.into_iter().map(|j| (j, 0)).collect()).await
}

/// The same, with a start delay in milliseconds per job (to make either side of a race go first).
async fn together_lagged(jobs: Vec<(Job, u64)>) -> Vec<Shot> {
    let mut set = tokio::task::JoinSet::new();
    for (i, ((app, who, p, k, body), delay_ms)) in jobs.into_iter().enumerate() {
        set.spawn(async move {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            (i, fire(app, who, p, k, body).await)
        });
    }
    let mut out: Vec<(usize, Shot)> = Vec::new();
    while let Some(r) = set.join_next().await {
        out.push(r.expect("a task"));
    }
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, s)| s).collect()
}

async fn claimed(w: &common::World, creator: Who, holder: Who) -> HandoffId {
    let mut admin = w.admin().await;
    Seed::pending(creator)
        .claimed_by(holder)
        .insert(&mut admin)
        .await
}

async fn returned(w: &common::World, creator: Who, author: Who) -> HandoffId {
    let mut admin = w.admin().await;
    let mut s = Seed::pending(creator).claimed_by(author);
    s.state = "returned";
    let id = s.insert(&mut admin).await;
    seed_return(&mut admin, id, project_a(), 1, author, "report").await;
    id
}

async fn scalar(w: &common::World, sql: &'static str, id: HandoffId) -> i64 {
    let mut admin = w.admin().await;
    sqlx::query_scalar(sql)
        .bind(id.to_string())
        .fetch_one(&mut admin)
        .await
        .expect("a count")
}

async fn state_of(w: &common::World, id: HandoffId) -> String {
    let mut admin = w.admin().await;
    sqlx::query_scalar("SELECT state FROM handoffs.handoffs WHERE id = $1::uuid")
        .bind(id.to_string())
        .fetch_one(&mut admin)
        .await
        .expect("a state")
}

// ---- HAND-48 ---------------------------------------------------------------------------------------------

/// HAND-48: two returns at once from the holder. With one key one executes and the other replays the stored
/// reply; with two keys exactly one `returns` row exists and the loser is 409 `invalid_transition`, never 500.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_returns_at_once_leave_one_return() {
    let w = world("p4b_two_returns").await;
    for round in 0..10 {
        // One key.
        let id = claimed(&w, Who::L, Who::W1).await;
        let k = key();
        let jobs = (0..2)
            .map(|_| {
                (
                    w.app.clone(),
                    Who::W1,
                    path(id, "return"),
                    k.clone(),
                    Some(report()),
                )
            })
            .collect();
        let s = together(jobs).await;
        assert!(
            s.iter().all(|x| x.status == 200) && s[0].json == s[1].json,
            "round {round} same key: {s:?}"
        );
        assert_eq!(
            s.iter().filter(|x| x.replayed).count(),
            1,
            "one executed, one replayed: {s:?}"
        );
        assert_eq!(
            scalar(
                &w,
                "SELECT count(*) FROM handoffs.returns WHERE handoff_id = $1::uuid",
                id
            )
            .await,
            1
        );

        // Two keys.
        let id = claimed(&w, Who::L, Who::W1).await;
        let jobs = (0..2)
            .map(|_| {
                (
                    w.app.clone(),
                    Who::W1,
                    path(id, "return"),
                    key(),
                    Some(report()),
                )
            })
            .collect();
        let s = together(jobs).await;
        let wins = s.iter().filter(|x| x.status == 200).count();
        let losses = s.iter().filter(|x| x.is(409, "invalid_transition")).count();
        assert_eq!((wins, losses), (1, 1), "round {round} two keys: {s:?}");
        assert_eq!(
            scalar(
                &w,
                "SELECT count(*) FROM handoffs.returns WHERE handoff_id = $1::uuid",
                id
            )
            .await,
            1,
            "exactly one returns row"
        );
        assert_eq!(state_of(&w, id).await, "returned");
    }
}

// ---- HAND-59 ---------------------------------------------------------------------------------------------

/// HAND-59: accept against rework by the lead on one returned handoff: one review row for the round, the loser
/// 409 `invalid_transition`. The same accept twice with one key replays; with two keys the second is 409.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accept_against_rework_leaves_one_review() {
    let w = world("p4b_review_race").await;
    for round in 0..10 {
        let id = returned(&w, Who::L, Who::W1).await;
        let s = together(vec![
            (
                w.app.clone(),
                Who::L,
                path(id, "accept"),
                key(),
                Some(json!({})),
            ),
            (
                w.app.clone(),
                Who::L,
                path(id, "rework"),
                key(),
                Some(json!({ "notes": "again" })),
            ),
        ])
        .await;
        let wins = s.iter().filter(|x| x.status == 200).count();
        let losses = s.iter().filter(|x| x.is(409, "invalid_transition")).count();
        assert_eq!((wins, losses), (1, 1), "round {round}: {s:?}");
        assert_eq!(
            scalar(
                &w,
                "SELECT count(*) FROM handoffs.reviews WHERE handoff_id = $1::uuid",
                id
            )
            .await,
            1,
            "the UNIQUE (handoff_id, round) is the guard"
        );
        let want = if s[0].status == 200 {
            "accepted"
        } else {
            "pending"
        };
        assert_eq!(state_of(&w, id).await, want, "the winner decides the state");
    }
}

/// HAND-59 (continued): one accept twice with one key is a replay; with two keys the second is 409.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_same_accept_twice_replays_or_conflicts() {
    let w = world("p4b_accept_twice").await;
    for round in 0..10 {
        let id = returned(&w, Who::L, Who::W1).await;
        let k = key();
        let s = together(
            (0..2)
                .map(|_| {
                    (
                        w.app.clone(),
                        Who::L,
                        path(id, "accept"),
                        k.clone(),
                        Some(json!({})),
                    )
                })
                .collect(),
        )
        .await;
        assert!(
            s.iter().all(|x| x.status == 200) && s[0].json == s[1].json,
            "round {round} one key: {s:?}"
        );
        assert_eq!(s.iter().filter(|x| x.replayed).count(), 1, "{s:?}");

        let id = returned(&w, Who::L, Who::W1).await;
        let s = together(
            (0..2)
                .map(|_| {
                    (
                        w.app.clone(),
                        Who::L,
                        path(id, "accept"),
                        key(),
                        Some(json!({})),
                    )
                })
                .collect(),
        )
        .await;
        let wins = s.iter().filter(|x| x.status == 200).count();
        let losses = s.iter().filter(|x| x.is(409, "invalid_transition")).count();
        assert_eq!((wins, losses), (1, 1), "round {round} two keys: {s:?}");
        assert_eq!(
            scalar(
                &w,
                "SELECT count(*) FROM handoffs.reviews WHERE handoff_id = $1::uuid",
                id
            )
            .await,
            1
        );
    }
}

// ---- HAND-74 ---------------------------------------------------------------------------------------------

/// HAND-74: a claimed handoff is cancelled while its holder returns it, 50 rounds. The lead's cancel is allowed
/// in `claimed` and in `returned`, so it always succeeds. The return either ran first (200, one `returns` row,
/// the log reads claimed to returned to cancelled) or lost (409 `invalid_transition`, no `returns` row, the log
/// reads claimed to cancelled). Never a half state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_against_return_never_leaves_a_half_state() {
    let w = world("p4b_cancel_return").await;
    let (mut return_first, mut cancel_first) = (0, 0);
    for round in 0..50 {
        let id = claimed(&w, Who::L, Who::W1).await;
        // Alternate which side starts late, by 0 to 4 ms, so both orders occur.
        let lag = u64::try_from(round % 5).expect("small");
        let (lag_return, lag_cancel) = if round % 2 == 0 { (lag, 0) } else { (0, lag) };
        let s = together_lagged(vec![
            (
                (
                    w.app.clone(),
                    Who::W1,
                    path(id, "return"),
                    key(),
                    Some(report()),
                ),
                lag_return,
            ),
            (
                (
                    w.app.clone(),
                    Who::L,
                    path(id, "cancel"),
                    key(),
                    Some(json!({ "reason": "stop" })),
                ),
                lag_cancel,
            ),
        ])
        .await;
        let (ret, cancel) = (&s[0], &s[1]);
        assert_eq!(cancel.status, 200, "round {round}: {s:?}");
        assert_eq!(state_of(&w, id).await, "cancelled", "round {round}");
        let returns = scalar(
            &w,
            "SELECT count(*) FROM handoffs.returns WHERE handoff_id = $1::uuid",
            id,
        )
        .await;
        let mut admin = w.admin().await;
        let log: Vec<(i32, String)> = sqlx::query_as(
            "SELECT seq, to_state FROM handoffs.transitions WHERE handoff_id = $1::uuid ORDER BY seq",
        )
        .bind(id.to_string())
        .fetch_all(&mut admin)
        .await
        .expect("the log");
        if ret.status == 200 {
            return_first += 1;
            assert_eq!(returns, 1, "round {round}");
            let to: Vec<&str> = log.iter().map(|(_, t)| t.as_str()).collect();
            assert_eq!(to, ["pending", "returned", "cancelled"], "round {round}");
        } else {
            cancel_first += 1;
            assert!(ret.is(409, "invalid_transition"), "round {round}: {s:?}");
            assert_eq!(returns, 0, "no returns row from the loser, round {round}");
            let to: Vec<&str> = log.iter().map(|(_, t)| t.as_str()).collect();
            assert_eq!(to, ["pending", "cancelled"], "round {round}");
        }
        assert_eq!(
            log.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            (1..=i32::try_from(log.len()).expect("small")).collect::<Vec<_>>(),
            "seq has no gap"
        );
    }
    eprintln!("HAND-74: return first {return_first}, cancel first {cancel_first}");
}

// ---- HAND-84 ---------------------------------------------------------------------------------------------

/// HAND-84: two identical requests with one key at once: one executes, the other waits and replays; one
/// transition row.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_key_at_once_executes_once() {
    let w = world("p4b_key_at_once").await;
    let mut admin = w.admin().await;
    for round in 0..20 {
        let id = Seed::pending(Who::L)
            .addressed_to_worker(Who::W1)
            .insert(&mut admin)
            .await;
        let k = key();
        let s = together(
            (0..2)
                .map(|_| (w.app.clone(), Who::W1, path(id, "claim"), k.clone(), None))
                .collect(),
        )
        .await;
        assert!(
            s.iter().all(|x| x.status == 200) && s[0].json == s[1].json,
            "round {round}: {s:?}"
        );
        assert_eq!(s.iter().filter(|x| x.replayed).count(), 1, "{s:?}");
        assert_eq!(
            scalar(
                &w,
                "SELECT count(*) FROM handoffs.transitions WHERE handoff_id = $1::uuid AND to_state = 'claimed'",
                id
            )
            .await,
            1
        );
    }
}

// ---- HAND-101 --------------------------------------------------------------------------------------------

/// HAND-101: the log is a chain. Twenty mixed verbs at once on one handoff, in waves, twenty rounds. A round is
/// eight waves of twenty requests drawn from the verbs that do not end the handoff (claim, release, return,
/// rework, by the people who may and by the people who may not), then a last wave that adds `accept` and
/// `cancel`. So the chain grows long under contention instead of ending at the first terminal move. `seq` runs
/// 1 to n with no gap or duplicate, every `from_state` equals the previous `to_state`, the number of 200s equals
/// the number of log rows after the creation row, no answer is a 5xx, and none is still 429 after five retries.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twenty_mixed_verbs_at_once_leave_a_chain() {
    let w = world("p4b_chain").await;
    // Who, verb, body. Eight verbs that keep the handoff open.
    let open: Vec<(Who, &str, Option<Value>)> = vec![
        (Who::W1, "claim", None),
        (Who::W2, "claim", None),
        (Who::W1, "release", Some(json!({}))),
        (Who::L, "release", Some(json!({ "reason": "lead" }))),
        (Who::W1, "return", Some(report())),
        (Who::W2, "return", Some(report())),
        (Who::L, "rework", Some(json!({ "notes": "again" }))),
        (Who::W4, "claim", None),
    ];
    let ending: Vec<(Who, &str, Option<Value>)> = vec![
        (Who::L, "accept", Some(json!({}))),
        (Who::L, "cancel", Some(json!({ "reason": "stop" }))),
    ];
    let (mut total_ok, mut longest, mut deepest_round, mut most_tries) = (0, 0, 0, 0);
    for round in 0..20 {
        let mut admin = w.admin().await;
        let id = Seed::pending(Who::L).insert(&mut admin).await;
        let mut ok = 0;
        for wave in 0..9 {
            let jobs = (0..20)
                .map(|i| {
                    // The last wave swaps two of the twenty for the verbs that end the handoff.
                    let (who, verb, body) = if wave == 8 && i < ending.len() {
                        &ending[i]
                    } else {
                        &open[(i + round + wave) % open.len()]
                    };
                    (w.app.clone(), *who, path(id, verb), key(), body.clone())
                })
                .collect();
            let s = together(jobs).await;
            for x in &s {
                assert!(x.status < 500, "round {round} wave {wave}: a 5xx: {x:?}");
                assert_ne!(
                    x.status, 429,
                    "round {round} wave {wave}: still busy after retries: {x:?}"
                );
            }
            ok += s.iter().filter(|x| x.status == 200).count();
            most_tries = most_tries.max(s.iter().map(|x| x.tries).max().unwrap_or(0));
        }
        total_ok += ok;
        let log: Vec<(i32, Option<String>, String)> = sqlx::query_as(
            "SELECT seq, from_state, to_state FROM handoffs.transitions WHERE handoff_id = $1::uuid ORDER BY seq",
        )
        .bind(id.to_string())
        .fetch_all(&mut admin)
        .await
        .expect("the log");
        let n = i32::try_from(log.len()).expect("small");
        assert_eq!(
            log.iter().map(|(q, _, _)| *q).collect::<Vec<_>>(),
            (1..=n).collect::<Vec<_>>(),
            "round {round}: seq is 1 to n"
        );
        assert_eq!(log[0].1, None);
        for pair in log.windows(2) {
            assert_eq!(
                pair[1].1.as_deref(),
                Some(pair[0].2.as_str()),
                "round {round}: from_state is the previous to_state: {log:?}"
            );
        }
        assert_eq!(
            ok,
            log.len() - 1,
            "round {round}: allowed calls equal log rows after the creation row"
        );
        if log.len() > longest {
            (longest, deepest_round) = (log.len(), round);
        }
        let (state, holder, closed, seq): (String, Option<String>, bool, i32) = sqlx::query_as(
            "SELECT state, holder::text, closed_at IS NOT NULL, transition_seq FROM handoffs.handoffs WHERE id = $1::uuid",
        )
        .bind(id.to_string())
        .fetch_one(&mut admin)
        .await
        .expect("row");
        assert_eq!(
            state,
            log.last().expect("a row").2,
            "the row is the log's end"
        );
        assert_eq!(seq, n, "the counter is the log's end");
        assert!(
            state != "pending" || holder.is_none(),
            "pending implies no holder"
        );
        assert!(
            state != "claimed" || holder.is_some(),
            "claimed implies a holder"
        );
        assert_eq!(
            closed,
            state == "accepted" || state == "cancelled",
            "closed_at iff terminal"
        );
    }
    eprintln!(
        "HAND-101: {total_ok} allowed moves over 20 rounds of 9 waves of 20 requests; longest log {longest} rows (round {deepest_round}); most tries of one request {most_tries}"
    );
}
