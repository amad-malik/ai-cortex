//! Wave 1, mod-handoffs rework of hale's F-1 (cox W1-39), the database half. The same shared vectors as
//! `w1_handoffs_p5_rfc3339.rs`, now through the real routes on a real database: `due_at` on create and the `after`
//! cursor on list. An accept vector is 201 (or 200); a reject vector is 422 `invalid_request`; NOTHING is ever a
//! 500, and a refused create stores nothing. A grid of values the rule accepts, across the edges of the calendar and
//! of the offset range, is run against Postgres itself: if the rule lets one through that Postgres rejects, the
//! answer would be a 500 and this cell fails.
//!
//! Run ONLY under `crates/cortex-testkit/supervise.sh` through `cv2-admit`, `CV2_DB=1`, one run at a time.
mod common;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use common::{Seed, Who, world};
use cortex_types::{HandoffId, SystemClock};
use serde_json::{Value, json};

/// Cox's `mvp/api/rfc3339-vectors.json`, copied here so the cells are hermetic (a runner without `/path/to/docs` runs
/// them, and a later edit of cox's file cannot change them silently); the hash is in the file name.
const RAW: &str = include_str!("data/rfc3339-vectors.4125a4b4.json");

fn vectors() -> (Vec<String>, Vec<String>) {
    let v: Value = serde_json::from_str(RAW).expect("the vectors are JSON");
    let list = |k: &str| -> Vec<String> {
        v[k].as_array()
            .unwrap_or_else(|| panic!("{k} is a list"))
            .iter()
            .map(|s| s.as_str().expect("a string").to_owned())
            .collect()
    };
    (list("accept"), list("reject"))
}

fn create_body(due_at: &str) -> Value {
    json!({ "kind": "task", "to": { "kind": "job_role", "ref": "dev" }, "summary": "s",
            "next_action": "n", "acceptance": "a", "due_at": due_at })
}

fn code(a: &common::Answer) -> (u16, Option<String>) {
    (
        a.status.as_u16(),
        a.json["type"].as_str().map(str::to_owned),
    )
}

/// `due_at` on create: every accept vector 201, every reject vector 422, never a 500, nothing stored for a reject.
#[tokio::test]
async fn due_at_follows_the_shared_vectors_through_the_route() {
    let w = world("p5_due_at").await;
    let (accept, reject) = vectors();
    let mut wrong = Vec::new();
    for s in &accept {
        let a = w.post(Who::L, "/v2/handoffs", create_body(s)).await;
        if a.status.as_u16() != 201 {
            wrong.push(format!("accept {s:?} -> {} {}", a.status, a.text));
        }
    }
    let stored = w.count("SELECT count(*) FROM handoffs.handoffs").await;
    for s in &reject {
        let a = w.post(Who::L, "/v2/handoffs", create_body(s)).await;
        if code(&a) != (422, Some("invalid_request".into())) {
            wrong.push(format!("reject {s:?} -> {} {}", a.status, a.text));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong answers:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    assert_eq!(
        stored,
        i64::try_from(accept.len()).expect("small"),
        "one row per accept vector"
    );
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.handoffs").await,
        stored,
        "a refused create stores nothing"
    );
}

/// The `after` cursor takes the same rule for the time it carries: a well-formed cursor whose timestamp is an accept
/// vector is a 200 page; one whose timestamp is a reject vector is 422 `invalid_request`, never a 500.
#[tokio::test]
async fn the_list_cursor_follows_the_shared_vectors() {
    let w = world("p5_cursor").await;
    // A row must exist: with none, Postgres never evaluates the cursor's timestamp and a bad one goes unnoticed.
    Seed::pending(Who::L).insert(&mut w.admin().await).await;
    let (accept, reject) = vectors();
    let id = HandoffId::new(&SystemClock);
    let cursor = |ts: &str| B64URL.encode(format!("h1|50|{ts}|{id}"));
    let mut wrong = Vec::new();
    for s in &accept {
        let a = w
            .get(Who::L, &format!("/v2/handoffs?after={}", cursor(s)))
            .await;
        if a.status.as_u16() != 200 {
            wrong.push(format!("accept {s:?} -> {} {}", a.status, a.text));
        }
    }
    for s in &reject {
        let a = w
            .get(Who::L, &format!("/v2/handoffs?after={}", cursor(s)))
            .await;
        if code(&a) != (422, Some("invalid_request".into())) {
            wrong.push(format!("reject {s:?} -> {} {}", a.status, a.text));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong answers:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// hale's measured cursor case, by name: a cursor with `+23:00` was a 500.
#[tokio::test]
async fn a_cursor_with_an_offset_postgres_refuses_is_a_422() {
    let w = world("p5_cursor_offset").await;
    // A row must exist: with none, Postgres never evaluates the cursor's timestamp and a bad one goes unnoticed.
    Seed::pending(Who::L).insert(&mut w.admin().await).await;
    let id = HandoffId::new(&SystemClock);
    for ts in ["2030-01-01T00:00:00+23:00", "0000-01-01T00:00:00Z"] {
        let c = B64URL.encode(format!("h1|50|{ts}|{id}"));
        let a = w.get(Who::L, &format!("/v2/handoffs?after={c}")).await;
        assert_eq!(
            code(&a),
            (422, Some("invalid_request".into())),
            "{ts}: {}",
            a.text
        );
    }
}

/// Nothing the rule accepts is refused by Postgres: a grid over the ends of the range of years, the last days of the
/// months (leap and not), the extreme offsets and the longest fraction. Each is 201 on create and a 200 page as a
/// cursor. A 500 here means the rule let through a value the database cannot hold.
#[tokio::test]
async fn nothing_the_rule_accepts_reaches_a_database_error() {
    let w = world("p5_grid").await;
    let years = [1970, 1971, 2000, 2024, 2026, 2100, 9999];
    let days: [(u32, u32); 6] = [(1, 1), (2, 28), (2, 29), (6, 30), (12, 31), (7, 31)];
    let leap = |y: u32| (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400);
    let offsets = [
        "Z", "-12:00", "-09:30", "+00:00", "+05:30", "+13:45", "+14:00",
    ];
    let times = ["00:00:00", "23:59:59.999999999"];
    let id = HandoffId::new(&SystemClock);
    let mut tried = 0;
    let mut wrong = Vec::new();
    for y in years {
        for (m, d) in days {
            if m == 2 && d == 29 && !leap(y) {
                continue;
            }
            for off in offsets {
                for t in times {
                    let ts = format!("{y:04}-{m:02}-{d:02}T{t}{off}");
                    tried += 1;
                    let a = w.post(Who::L, "/v2/handoffs", create_body(&ts)).await;
                    if a.status.as_u16() != 201 {
                        wrong.push(format!("create {ts} -> {} {}", a.status, a.text));
                    }
                    let c = B64URL.encode(format!("h1|50|{ts}|{id}"));
                    let l = w.get(Who::L, &format!("/v2/handoffs?after={c}")).await;
                    if l.status.as_u16() != 200 {
                        wrong.push(format!("cursor {ts} -> {} {}", l.status, l.text));
                    }
                }
            }
        }
    }
    assert!(tried > 400, "the grid is not trivial: {tried}");
    assert!(
        wrong.is_empty(),
        "{} of {tried} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
