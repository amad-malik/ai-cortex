//! Wave 1, mod-handoffs P4c (row 2a86598d): what the DATABASE guarantees, whatever the code does. Test
//! definitions: `mvp/tests/mod-handoffs.md` r3.4, HAND-90 and HAND-91 (row-level security by project), HAND-92
//! (grants and append-only), HAND-95 (the packet is frozen: by column grant for the piece role, by trigger for
//! everyone else, and untouched by every verb), HAND-93 and HAND-94 (the canary and secret sweeps).
//!
//! Each probe runs in its own transaction on a superuser connection, switched to the role under test with
//! `set_config('role', ..)` and given `cortex.project_id` the way a module transaction is; it is always rolled
//! back, so one probe cannot change the next. The tables are FORCEd row-level security, so even the owner sees
//! only the project in `cortex.project_id`.
//!
//! Run ONLY under `crates/cortex-testkit/supervise.sh` through `cv2-admit`, `CV2_DB=1`, one run at a time.
mod common;

use axum::http::Method;
use common::{Answer, Seed, Who, key, project_a, project_b, seed_return, seed_review, world};
use cortex_types::HandoffId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Connection;

const PIECE: &str = "cortex_m_handoffs";

/// `(table, count all, count rows of the project in $1)`.
const TABLES: [(&str, &str, &str); 5] = [
    (
        "handoffs",
        "SELECT count(*) FROM handoffs.handoffs",
        "SELECT count(*) FROM handoffs.handoffs WHERE project_id = $1::uuid",
    ),
    (
        "returns",
        "SELECT count(*) FROM handoffs.returns",
        "SELECT count(*) FROM handoffs.returns WHERE project_id = $1::uuid",
    ),
    (
        "evidence",
        "SELECT count(*) FROM handoffs.evidence",
        "SELECT count(*) FROM handoffs.evidence WHERE project_id = $1::uuid",
    ),
    (
        "reviews",
        "SELECT count(*) FROM handoffs.reviews",
        "SELECT count(*) FROM handoffs.reviews WHERE project_id = $1::uuid",
    ),
    (
        "transitions",
        "SELECT count(*) FROM handoffs.transitions",
        "SELECT count(*) FROM handoffs.transitions WHERE project_id = $1::uuid",
    ),
];

/// One INSERT per table. `$1` is the row's project; `$2` is a handoff (a return, for `evidence`).
const INSERTS: [(&str, &str); 5] = [
    (
        "handoffs",
        "INSERT INTO handoffs.handoffs (id, project_id, kind, created_by, to_kind, to_ref, summary, next_action, acceptance) \
         VALUES (gen_random_uuid(), $1::uuid, 'task', gen_random_uuid(), 'job_role', 'dev', 's', 'n', 'a')",
    ),
    (
        "returns",
        "INSERT INTO handoffs.returns (id, handoff_id, project_id, round, author, incarnation, summary, risks) \
         VALUES (gen_random_uuid(), $2::uuid, $1::uuid, 9, gen_random_uuid(), 1, 's', 'none')",
    ),
    (
        "evidence",
        "INSERT INTO handoffs.evidence (id, return_id, project_id, kind, ref, sha256, size) \
         VALUES (gen_random_uuid(), $2::uuid, $1::uuid, 'link', 'r', decode(repeat('00', 32), 'hex'), 0)",
    ),
    (
        "reviews",
        "INSERT INTO handoffs.reviews (id, handoff_id, project_id, round, reviewer, verdict) \
         VALUES (gen_random_uuid(), $2::uuid, $1::uuid, 9, gen_random_uuid(), 'accept')",
    ),
    (
        "transitions",
        "INSERT INTO handoffs.transitions (id, handoff_id, project_id, seq, from_state, to_state, actor_kind, actor_id, incarnation) \
         VALUES (gen_random_uuid(), $2::uuid, $1::uuid, 99, 'pending', 'claimed', 'worker', 'someone', 1)",
    ),
];

/// A refusal: SQLSTATE and message.
type Refused = (String, String);

fn refused(e: &sqlx::Error) -> Refused {
    match e.as_database_error() {
        Some(d) => (
            d.code().map(|c| c.into_owned()).unwrap_or_default(),
            d.message().to_owned(),
        ),
        None => ("-".into(), e.to_string()),
    }
}

/// Switch a transaction to `role` and give it `cortex.project_id` (`None`: never set).
async fn begin_as<'c>(
    conn: &'c mut sqlx::PgConnection,
    role: &str,
    project: Option<&str>,
) -> sqlx::Transaction<'c, sqlx::Postgres> {
    let mut tx = conn.begin().await.expect("begin");
    sqlx::query("SELECT set_config('role', $1, true)")
        .bind(role)
        .execute(&mut *tx)
        .await
        .expect("switch role");
    if let Some(p) = project {
        sqlx::query("SELECT set_config('cortex.project_id', $1, true)")
            .bind(p)
            .execute(&mut *tx)
            .await
            .expect("set project");
    }
    tx
}

/// Rows affected by `sql` as `role`, or why it was refused. Always rolled back.
async fn run_as(
    w: &common::World,
    role: &str,
    project: Option<&str>,
    sql: &'static str,
    binds: &[String],
) -> Result<u64, Refused> {
    let mut conn = w.admin().await;
    let mut tx = begin_as(&mut conn, role, project).await;
    let mut q = sqlx::query(sql);
    for b in binds {
        q = q.bind(b.as_str());
    }
    let out = q
        .execute(&mut *tx)
        .await
        .map(|r| r.rows_affected())
        .map_err(|e| refused(&e));
    let _ = tx.rollback().await;
    out
}

/// A `count(*)` as `role`, or why it was refused.
async fn count_as(
    w: &common::World,
    role: &str,
    project: Option<&str>,
    sql: &'static str,
    binds: &[String],
) -> Result<i64, Refused> {
    let mut conn = w.admin().await;
    let mut tx = begin_as(&mut conn, role, project).await;
    let mut q = sqlx::query_scalar::<_, i64>(sql);
    for b in binds {
        q = q.bind(b.as_str());
    }
    let out = q.fetch_one(&mut *tx).await.map_err(|e| refused(&e));
    let _ = tx.rollback().await;
    out
}

fn code(r: &Result<u64, Refused>) -> String {
    match r {
        Ok(n) => format!("ok:{n}"),
        Err((c, _)) => c.clone(),
    }
}

/// The owner of the piece's tables.
async fn owner(w: &common::World) -> String {
    let mut admin = w.admin().await;
    sqlx::query_scalar(
        "SELECT tableowner::text FROM pg_tables WHERE schemaname = 'handoffs' AND tablename = 'handoffs'",
    )
    .fetch_one(&mut admin)
    .await
    .expect("the owner")
}

struct Fixture {
    a: HandoffId,
    a_return: String,
}

/// One returned handoff with a return, one evidence entry, a review and a creation row in A, and the same in B.
async fn two_projects(w: &common::World) -> Fixture {
    let mut admin = w.admin().await;
    let mut a = Seed::pending(Who::L).claimed_by(Who::W1);
    a.state = "returned";
    let a = a.insert(&mut admin).await;
    seed_return(&mut admin, a, project_a(), 1, Who::W1, "a").await;
    seed_review(&mut admin, a, project_a(), 1, Who::L, "accept", None, false).await;
    let mut b = Seed::pending(Who::B1)
        .summary("CANARY-B-SUMMARY-6b1d0e")
        .claimed_by(Who::B2);
    b.state = "returned";
    let b = b.insert(&mut admin).await;
    seed_return(&mut admin, b, project_b(), 1, Who::B2, "b").await;
    seed_review(
        &mut admin,
        b,
        project_b(),
        1,
        Who::B1,
        "accept",
        None,
        false,
    )
    .await;
    let a_return: String =
        sqlx::query_scalar("SELECT id::text FROM handoffs.returns WHERE handoff_id = $1::uuid")
            .bind(a.to_string())
            .fetch_one(&mut admin)
            .await
            .expect("a's return");
    Fixture { a, a_return }
}

// ---- HAND-90 ---------------------------------------------------------------------------------------------

/// HAND-90: as the piece role with project A, on each of the five tables: a SELECT shows only A's rows; an INSERT
/// of a row of project B is refused by the policy's WITH CHECK (while the same INSERT for A is accepted, so the
/// refusal is the project and not the statement); an UPDATE or DELETE of B's rows touches none or is refused.
#[tokio::test]
async fn a_project_sees_and_writes_only_its_own_rows() {
    let w = world("p4c_rls").await;
    let f = two_projects(&w).await;
    let (a, b) = (project_a().to_string(), project_b().to_string());
    for (name, all, by_project) in TABLES {
        assert_eq!(
            count_as(&w, PIECE, Some(&a), all, &[]).await.ok(),
            Some(1),
            "{name}: A's one row"
        );
        assert_eq!(
            count_as(&w, PIECE, Some(&a), by_project, std::slice::from_ref(&b))
                .await
                .ok(),
            Some(0),
            "{name}: no row of B is returned to A"
        );
        assert_eq!(
            count_as(&w, PIECE, Some(&b), by_project, std::slice::from_ref(&a))
                .await
                .ok(),
            Some(0),
            "{name}: and none of A's to B"
        );
    }
    for (name, sql) in INSERTS {
        let parent = if name == "evidence" {
            f.a_return.clone()
        } else {
            f.a.to_string()
        };
        let control = run_as(&w, PIECE, Some(&a), sql, &[a.clone(), parent.clone()]).await;
        assert_eq!(
            code(&control),
            "ok:1",
            "{name}: the statement itself is good: {control:?}"
        );
        let crossing = run_as(&w, PIECE, Some(&a), sql, &[b.clone(), parent]).await;
        assert_eq!(
            code(&crossing),
            "42501",
            "{name}: a row of B from A: {crossing:?}"
        );
        assert!(
            crossing
                .as_ref()
                .err()
                .is_some_and(|(_, m)| m.contains("row-level security")),
            "{name}: refused by the policy: {crossing:?}"
        );
    }
    // UPDATE and DELETE of B's rows: none touched, or refused.
    let bs = [b.clone()];
    let touch = run_as(
        &w,
        PIECE,
        Some(&a),
        "UPDATE handoffs.handoffs SET priority = 1 WHERE project_id = $1::uuid",
        &bs,
    )
    .await;
    assert_eq!(code(&touch), "ok:0", "{touch:?}");
    for (name, sql) in [
        (
            "handoffs",
            "DELETE FROM handoffs.handoffs WHERE project_id = $1::uuid",
        ),
        (
            "returns",
            "DELETE FROM handoffs.returns WHERE project_id = $1::uuid",
        ),
        (
            "evidence",
            "DELETE FROM handoffs.evidence WHERE project_id = $1::uuid",
        ),
        (
            "reviews",
            "DELETE FROM handoffs.reviews WHERE project_id = $1::uuid",
        ),
        (
            "transitions",
            "DELETE FROM handoffs.transitions WHERE project_id = $1::uuid",
        ),
    ] {
        let r = run_as(&w, PIECE, Some(&a), sql, &bs).await;
        assert!(
            matches!(code(&r).as_str(), "ok:0" | "42501"),
            "{name}: {r:?}"
        );
    }
    // B's rows are all still there.
    for (name, _, by_project) in TABLES {
        assert_eq!(
            count_as(&w, PIECE, Some(&b), by_project, std::slice::from_ref(&b))
                .await
                .ok(),
            Some(1),
            "{name}: B's row is untouched"
        );
    }
}

// ---- HAND-91 ---------------------------------------------------------------------------------------------

/// HAND-91: with `cortex.project_id` unset, empty or not a UUID there is no fallback to "all projects": no row
/// is visible and an insert is refused.
#[tokio::test]
async fn no_project_means_no_rows_and_no_inserts() {
    let w = world("p4c_noproject").await;
    let f = two_projects(&w).await;
    let a = project_a().to_string();
    for (label, project) in [
        ("unset", None),
        ("empty", Some("")),
        ("not a uuid", Some("not-a-uuid")),
    ] {
        for (name, all, _) in TABLES {
            let c = count_as(&w, PIECE, project, all, &[]).await;
            match (&c, label) {
                (Ok(n), _) => assert_eq!(*n, 0, "{label} {name}: zero rows"),
                (Err((state, _)), "not a uuid") => {
                    assert_eq!(state, "22P02", "{label} {name}: the cast refuses")
                }
                (Err(e), _) => panic!("{label} {name}: {e:?}"),
            }
        }
        for (name, sql) in INSERTS {
            let parent = if name == "evidence" {
                f.a_return.clone()
            } else {
                f.a.to_string()
            };
            let r = run_as(&w, PIECE, project, sql, &[a.clone(), parent]).await;
            assert!(
                r.is_err(),
                "{label} {name}: an insert must be refused: {r:?}"
            );
            if label != "not a uuid" {
                assert_eq!(code(&r), "42501", "{label} {name}: {r:?}");
            }
        }
    }
}

// ---- HAND-92 ---------------------------------------------------------------------------------------------

/// HAND-92: no DELETE on any table for the piece role (no grant); no UPDATE on the four history tables for the
/// piece role; and the four history tables refuse UPDATE and DELETE even for the owner (trigger, SQLSTATE 42501,
/// "is append-only").
#[tokio::test]
async fn deletes_are_not_granted_and_the_history_refuses_change_even_for_the_owner() {
    let w = world("p4c_grants").await;
    two_projects(&w).await;
    let a = project_a().to_string();
    for (name, sql) in [
        ("handoffs", "DELETE FROM handoffs.handoffs"),
        ("returns", "DELETE FROM handoffs.returns"),
        ("evidence", "DELETE FROM handoffs.evidence"),
        ("reviews", "DELETE FROM handoffs.reviews"),
        ("transitions", "DELETE FROM handoffs.transitions"),
    ] {
        let r = run_as(&w, PIECE, Some(&a), sql, &[]).await;
        assert_eq!(code(&r), "42501", "{name}: no DELETE grant: {r:?}");
    }
    let updates = [
        (
            "returns",
            "UPDATE handoffs.returns SET summary = 'x'",
            "UPDATE handoffs.returns SET summary = 'x' WHERE true",
        ),
        (
            "evidence",
            "UPDATE handoffs.evidence SET ref = 'x'",
            "UPDATE handoffs.evidence SET ref = 'x' WHERE true",
        ),
        (
            "reviews",
            "UPDATE handoffs.reviews SET notes = 'x'",
            "UPDATE handoffs.reviews SET notes = 'x' WHERE true",
        ),
        (
            "transitions",
            "UPDATE handoffs.transitions SET reason = 'x'",
            "UPDATE handoffs.transitions SET reason = 'x' WHERE true",
        ),
    ];
    for (name, sql, _) in updates {
        let r = run_as(&w, PIECE, Some(&a), sql, &[]).await;
        assert_eq!(code(&r), "42501", "{name}: no UPDATE grant: {r:?}");
    }
    // The owner: no grant is in the way, so it is the trigger that refuses.
    let owner = owner(&w).await;
    for (name, sql, _) in updates {
        let r = run_as(&w, &owner, Some(&a), sql, &[]).await;
        assert!(
            r.as_ref()
                .err()
                .is_some_and(|(c, m)| c == "42501" && m.contains("append-only")),
            "{name} UPDATE as owner {owner}: {r:?}"
        );
    }
    for (name, sql) in [
        ("returns", "DELETE FROM handoffs.returns"),
        ("evidence", "DELETE FROM handoffs.evidence"),
        ("reviews", "DELETE FROM handoffs.reviews"),
        ("transitions", "DELETE FROM handoffs.transitions"),
    ] {
        let r = run_as(&w, &owner, Some(&a), sql, &[]).await;
        assert!(
            r.as_ref()
                .err()
                .is_some_and(|(c, m)| c == "42501" && m.contains("append-only")),
            "{name} DELETE as owner {owner}: {r:?}"
        );
    }
}

// ---- HAND-95 ---------------------------------------------------------------------------------------------

/// The fourteen frozen columns: an UPDATE that names one, and an UPDATE that changes it.
const FROZEN: [(&str, &str, &str); 14] = [
    (
        "id",
        "UPDATE handoffs.handoffs SET id = id",
        "UPDATE handoffs.handoffs SET id = gen_random_uuid()",
    ),
    (
        "project_id",
        "UPDATE handoffs.handoffs SET project_id = project_id",
        "UPDATE handoffs.handoffs SET project_id = gen_random_uuid()",
    ),
    (
        "kind",
        "UPDATE handoffs.handoffs SET kind = kind",
        "UPDATE handoffs.handoffs SET kind = CASE WHEN kind = 'task' THEN 'consult' ELSE 'task' END",
    ),
    (
        "created_by",
        "UPDATE handoffs.handoffs SET created_by = created_by",
        "UPDATE handoffs.handoffs SET created_by = gen_random_uuid()",
    ),
    (
        "task_id",
        "UPDATE handoffs.handoffs SET task_id = task_id",
        "UPDATE handoffs.handoffs SET task_id = gen_random_uuid()",
    ),
    (
        "summary",
        "UPDATE handoffs.handoffs SET summary = summary",
        "UPDATE handoffs.handoffs SET summary = summary || 'x'",
    ),
    (
        "next_action",
        "UPDATE handoffs.handoffs SET next_action = next_action",
        "UPDATE handoffs.handoffs SET next_action = next_action || 'x'",
    ),
    (
        "acceptance",
        "UPDATE handoffs.handoffs SET acceptance = acceptance",
        "UPDATE handoffs.handoffs SET acceptance = acceptance || 'x'",
    ),
    (
        "verify",
        "UPDATE handoffs.handoffs SET verify = verify",
        "UPDATE handoffs.handoffs SET verify = coalesce(verify, '') || 'x'",
    ),
    (
        "context",
        "UPDATE handoffs.handoffs SET context = context",
        "UPDATE handoffs.handoffs SET context = coalesce(context, '') || 'x'",
    ),
    (
        "files",
        "UPDATE handoffs.handoffs SET files = files",
        "UPDATE handoffs.handoffs SET files = '[\"x\"]'::jsonb",
    ),
    (
        "blockers",
        "UPDATE handoffs.handoffs SET blockers = blockers",
        "UPDATE handoffs.handoffs SET blockers = '[\"x\"]'::jsonb",
    ),
    (
        "decisions",
        "UPDATE handoffs.handoffs SET decisions = decisions",
        "UPDATE handoffs.handoffs SET decisions = '[\"x\"]'::jsonb",
    ),
    (
        "created_at",
        "UPDATE handoffs.handoffs SET created_at = created_at",
        "UPDATE handoffs.handoffs SET created_at = created_at + interval '1 second'",
    ),
];

/// HAND-95 (a), (b), (d): the piece role cannot name a frozen column (no column privilege, 42501); the owner, to
/// whom no column grant applies, is stopped by the trigger `handoffs_packet_frozen` (23514); the working columns
/// are open to the piece role.
#[tokio::test]
async fn the_packet_is_frozen_by_grant_for_the_piece_and_by_trigger_for_the_owner() {
    let w = world("p4c_frozen").await;
    two_projects(&w).await;
    let a = project_a().to_string();
    for (name, names_it, changes_it) in FROZEN {
        let r = run_as(&w, PIECE, Some(&a), names_it, &[]).await;
        assert_eq!(code(&r), "42501", "(a) {name}: {r:?}");
        let r = run_as(&w, PIECE, Some(&a), changes_it, &[]).await;
        assert_eq!(code(&r), "42501", "(a) {name} changed: {r:?}");
    }
    let owner = owner(&w).await;
    for (name, _, changes_it) in FROZEN {
        let r = run_as(&w, &owner, Some(&a), changes_it, &[]).await;
        assert!(
            r.as_ref()
                .err()
                .is_some_and(|(c, m)| c == "23514" && m.contains("frozen")),
            "(b) {name} as owner {owner}: {r:?}"
        );
    }
    // (d) the working columns, open to the piece role (the A handoff is the one row it can see).
    let open = run_as(
        &w,
        PIECE,
        Some(&a),
        "UPDATE handoffs.handoffs SET state = state, holder = holder, rework_holder = rework_holder, round = round, \
         closed_at = closed_at, close_reason = close_reason, priority = priority, incarnation = incarnation",
        &[],
    )
    .await;
    assert_eq!(code(&open), "ok:1", "(d) {open:?}");
}

async fn in_state(w: &common::World, state: &'static str) -> HandoffId {
    let mut admin = w.admin().await;
    let mut s = Seed::pending(Who::L);
    s.state = state;
    if state != "pending" {
        s = s.claimed_by(Who::W1);
        s.state = state;
    }
    if state == "accepted" || state == "cancelled" {
        s.closed_at = Some("2026-11-02 00:00:00+00".to_owned());
    }
    if state == "cancelled" {
        s.close_reason = Some("no longer needed".to_owned());
    }
    let id = s.insert(&mut admin).await;
    if state == "returned" {
        seed_return(&mut admin, id, project_a(), 1, Who::W1, "report").await;
    }
    id
}

fn link(r: &str) -> Value {
    let sha: String = Sha256::digest(r.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({ "kind": "link", "ref": r, "sha256": sha })
}

/// HAND-95 (c): the packet columns, the creator and the address, hashed before and after every one of the 180
/// cells (5 states, 6 verbs, 6 actors), allowed or refused, are identical. A rework never rewrites `to_kind` and
/// `to_ref`.
#[tokio::test]
async fn no_verb_by_anyone_in_any_state_changes_the_packet_or_the_address() {
    let w = world("p4c_hash").await;
    let verbs: Vec<(&str, Option<Value>)> = vec![
        ("claim", None),
        ("release", Some(json!({ "reason": "r" }))),
        (
            "return",
            Some(
                json!({ "summary": "done", "evidence": [link("r")], "risks": "none", "tests": ["t"] }),
            ),
        ),
        ("accept", Some(json!({}))),
        ("rework", Some(json!({ "notes": "again" }))),
        ("cancel", Some(json!({ "reason": "stop" }))),
    ];
    let actors = [Who::L, Who::TL, Who::W1, Who::W2, Who::W3, Who::W4];
    let (mut cells, mut allowed) = (0, 0);
    for state in ["pending", "claimed", "returned", "accepted", "cancelled"] {
        for (verb, body) in &verbs {
            for who in actors {
                let id = in_state(&w, state).await;
                let hash = |label: &'static str| {
                    let w = &w;
                    async move {
                        let mut admin = w.admin().await;
                        let h: String = sqlx::query_scalar(
                            "SELECT md5(row(id, project_id, kind, created_by, task_id, summary, next_action, acceptance, \
                             verify, context, files, blockers, decisions, created_at, to_kind, to_ref)::text) \
                             FROM handoffs.handoffs WHERE id = $1::uuid",
                        )
                        .bind(id.to_string())
                        .fetch_one(&mut admin)
                        .await
                        .unwrap_or_else(|e| panic!("{label} hash: {e}"));
                        h
                    }
                };
                let before = hash("before").await;
                let a: Answer = w
                    .send(
                        Method::POST,
                        &format!("/v2/handoffs/{id}/{verb}"),
                        Some(who),
                        Some(&key()),
                        body.clone(),
                    )
                    .await;
                assert!(
                    a.status.as_u16() < 500,
                    "{state} {verb} {who:?}: {}",
                    a.text
                );
                if a.status.as_u16() == 200 {
                    allowed += 1;
                }
                assert_eq!(
                    before,
                    hash("after").await,
                    "{state} {verb} {who:?} -> {}: the packet or the address changed",
                    a.status
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 180);
    eprintln!("HAND-95 (c): {cells} cells, {allowed} of them allowed moves, all hashes identical");
    // The rules allow exactly these (creator L is also the lead; the handoff is addressed to role `dev`, held by W1):
    // pending: claim by W1 and W2, cancel by L = 3; claimed: release by W1 and by L (with a reason), return by W1,
    // cancel by L = 4; returned: accept, rework and cancel by L (the author W1 may not review) = 3; the two closed
    // states allow nothing. The team lead TL may cancel none of them (a job-role address has no team).
    assert_eq!(
        allowed, 10,
        "the matrix holds the real moves, and only those"
    );
}

// ---- HAND-93, HAND-94 ------------------------------------------------------------------------------------

/// HAND-93 and HAND-94: every verb in A while B holds canaries, and every refusal along the way; every response
/// (body and headers) and every audit row is searched for B's canary, B's project and B's workers (HAND-93), and
/// every table of the piece, the audit table, the idempotency store and every response for the fixture tokens
/// and the Authorization header value (HAND-94).
#[tokio::test]
async fn nothing_of_another_project_and_no_secret_appears_anywhere() {
    let w = world("p4c_sweep").await;
    // B holds canaries in every state the piece has.
    let b_ids: Vec<HandoffId> = {
        let mut admin = w.admin().await;
        let mut ids = Vec::new();
        for state in ["pending", "claimed", "returned"] {
            let mut s = Seed::pending(Who::B1)
                .summary("CANARY-B-SUMMARY-6b1d0e")
                .addressed_to_worker(Who::B2);
            if state != "pending" {
                s = s.claimed_by(Who::B2);
            }
            s.state = state;
            let id = s.insert(&mut admin).await;
            if state == "returned" {
                seed_return(
                    &mut admin,
                    id,
                    project_b(),
                    1,
                    Who::B2,
                    "CANARY-B-RETURN-6b1d0e",
                )
                .await;
            }
            ids.push(id);
        }
        ids
    };
    let mut captured: Vec<String> = Vec::new();
    let mut sent = |a: &Answer| captured.push(format!("{} {:?} {}", a.status, a.headers, a.text));

    let body = |to: Value| json!({ "kind": "task", "to": to, "summary": "s", "next_action": "n", "acceptance": "a" });
    let created = w
        .post(
            Who::L,
            "/v2/handoffs",
            body(json!({ "kind": "job_role", "ref": "dev" })),
        )
        .await;
    assert_eq!(created.status.as_u16(), 201, "{}", created.text);
    sent(&created);
    let id = HandoffId::parse(created.json["id"].as_str().expect("id")).expect("an id");
    let p = |verb: &str| format!("/v2/handoffs/{id}/{verb}");
    let report =
        json!({ "summary": "done", "evidence": [link("r")], "risks": "none", "tests": ["t"] });
    let steps: Vec<(Who, String, Option<Value>)> = vec![
        (Who::W4, p("claim"), None),
        (Who::W1, p("claim"), None),
        (Who::W2, p("claim"), None),
        (Who::W2, p("release"), Some(json!({}))),
        (Who::L, p("accept"), Some(json!({}))),
        (Who::W1, p("release"), Some(json!({}))),
        (Who::W1, p("claim"), None),
        (Who::W1, p("return"), Some(report.clone())),
        (Who::W1, p("return"), Some(report.clone())),
        (Who::L, p("rework"), Some(json!({ "notes": "again" }))),
        (Who::W1, p("claim"), None),
        (Who::W1, p("return"), Some(report)),
        (Who::L, p("accept"), Some(json!({}))),
        (Who::L, p("cancel"), Some(json!({ "reason": "late" }))),
        (Who::L, p("cancel"), Some(json!({}))),
    ];
    for (who, path, body) in steps {
        let a = w
            .send(Method::POST, &path, Some(who), Some(&key()), body)
            .await;
        sent(&a);
    }
    // Reads, and every probe at B's ids from A.
    for who in [Who::L, Who::W1, Who::W4] {
        sent(&w.get(who, &format!("/v2/handoffs/{id}")).await);
        sent(&w.get(who, "/v2/handoffs?mine=true").await);
        sent(&w.get(who, "/v2/handoffs?state=all&limit=200").await);
    }
    for b in &b_ids {
        sent(&w.get(Who::L, &format!("/v2/handoffs/{b}")).await);
        for (verb, body) in [
            ("claim", None),
            ("release", Some(json!({}))),
            (
                "return",
                Some(
                    json!({ "summary": "s", "evidence": [link("r")], "risks": "none", "tests": ["t"] }),
                ),
            ),
            ("accept", Some(json!({}))),
            ("rework", Some(json!({ "notes": "n" }))),
            ("cancel", Some(json!({ "reason": "r" }))),
        ] {
            let a = w
                .send(
                    Method::POST,
                    &format!("/v2/handoffs/{b}/{verb}"),
                    Some(Who::L),
                    Some(&key()),
                    body,
                )
                .await;
            assert_eq!(a.status.as_u16(), 404, "{verb} on B's handoff: {}", a.text);
            sent(&a);
        }
    }
    // A create that names B's worker, and a malformed one.
    sent(
        &w.post(
            Who::L,
            "/v2/handoffs",
            body(json!({ "kind": "worker", "ref": Who::B2.id().to_string() })),
        )
        .await,
    );
    sent(
        &w.post(Who::L, "/v2/handoffs", json!({ "kind": "task" }))
            .await,
    );

    let mut admin = w.admin().await;
    let audit: String =
        sqlx::query_scalar("SELECT coalesce(string_agg(t::text, ' '), '') FROM fx_audit.events t")
            .fetch_one(&mut admin)
            .await
            .expect("audit");
    let idem: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(t::text, ' '), '') FROM platform.idempotency t",
    )
    .fetch_one(&mut admin)
    .await
    .expect("idempotency");
    let mut tables = String::new();
    for sql in [
        "SELECT coalesce(string_agg(t::text, ' '), '') FROM handoffs.handoffs t WHERE project_id = $1::uuid",
        "SELECT coalesce(string_agg(t::text, ' '), '') FROM handoffs.returns t WHERE project_id = $1::uuid",
        "SELECT coalesce(string_agg(t::text, ' '), '') FROM handoffs.evidence t WHERE project_id = $1::uuid",
        "SELECT coalesce(string_agg(t::text, ' '), '') FROM handoffs.reviews t WHERE project_id = $1::uuid",
        "SELECT coalesce(string_agg(t::text, ' '), '') FROM handoffs.transitions t WHERE project_id = $1::uuid",
    ] {
        let t: String = sqlx::query_scalar(sql)
            .bind(project_a().to_string())
            .fetch_one(&mut admin)
            .await
            .expect("dump");
        tables.push_str(&t);
        tables.push(' ');
    }
    let all_a_side = format!("{} {audit} {idem} {tables}", captured.join(" "));
    assert!(!audit.is_empty() && !idem.is_empty() && !tables.trim().is_empty());
    assert!(captured.len() > 40, "the sweep must have captured a lot");

    // HAND-93: nothing of B in anything A saw, or in what A's side of the database holds.
    let mut canaries: Vec<String> = vec![
        "CANARY-B-SUMMARY".into(),
        "CANARY-B-RETURN".into(),
        project_b().to_string(),
        Who::B1.id().to_string(),
        Who::B2.id().to_string(),
    ];
    canaries.extend(b_ids.iter().map(|i| i.to_string()));
    for needle in &canaries {
        assert!(!all_a_side.contains(needle), "{needle} appears on A's side");
    }
    // HAND-94: no token and no Authorization value anywhere.
    let mut secrets: Vec<String> = Who::ALL.iter().map(|x| x.token()).collect();
    secrets.push("Bearer".into());
    secrets.push("authorization".into());
    for needle in &secrets {
        assert!(
            !all_a_side.to_lowercase().contains(&needle.to_lowercase()),
            "{needle} appears"
        );
    }
}
