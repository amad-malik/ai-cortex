//! RED for piece 7, DB cells for what a search returns (test definitions MS-04 to MS-08, MS-12, MS-13, MS-22), against the S2 revision 8 views
//! on a real Postgres. Needs `supervise.sh`'s throwaway Postgres. Run ONLY as:
//!
//! ```text
//! CV2_DB=1 CV2_ADMIT_LOG=1 /path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit \
//!     timeout --kill-after=120 780 crates/cortex-testkit/supervise.sh cargo test -p mod-search --test srch_db_semantics --no-fail-fast
//! ```
mod support;

use cortex_types::ProjectId;
use mod_search::Hit;
use mod_search::query::run;
use mod_search::validate::check;
use serde_json::{Value, json};
use support::{BASE_MS, Rec, World, seed, set_state, setup, uuid7};

async fn search(w: &World, project: ProjectId, body: Value) -> Vec<Hit> {
    let v = check(
        Some("application/json"),
        &serde_json::to_vec(&body).expect("json"),
    )
    .unwrap_or_else(|p| panic!("the request is valid: {} {}", p.code, p.detail));
    let mut tx = support::search_tx(w, Some(project)).await;
    let hits = run(&mut tx, &v)
        .await
        .unwrap_or_else(|p| panic!("the search answers: {} {} {}", p.status, p.code, p.detail));
    tx.commit().await.expect("commit");
    hits
}

fn ids(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|h| h.id.clone()).collect()
}

/// `2026-11-06T00:MM:SSZ` for `secs` after the base (below an hour).
fn at(secs: u64) -> String {
    format!("2026-11-06T00:{:02}:{:02}Z", secs / 60, secs % 60)
}

#[tokio::test]
async fn ms04_only_active_records_by_default_and_the_history_flag_adds_the_others_with_their_state()
{
    let w = setup("ms04").await;
    let (r1, r2, r3) = (
        seed(&w, w.a, &Rec::note(1, 1, "rollout plan", None)).await,
        seed(&w, w.a, &Rec::note(2, 2, "rollout plan", None)).await,
        seed(&w, w.a, &Rec::note(3, 3, "rollout plan", None)).await,
    );
    set_state(&w, w.a, &r2, "superseded", Some(&r1)).await;
    set_state(&w, w.a, &r3, "invalidated", None).await;
    let hits = search(&w, w.a, json!({"query": "rollout"})).await;
    assert_eq!(ids(&hits), vec![r1.clone()]);
    assert_eq!(hits[0].state, "active");
    // History: all three, equal rank, so newest first; each with its state.
    let hits = search(
        &w,
        w.a,
        json!({"query": "rollout", "include_history": true}),
    )
    .await;
    assert_eq!(ids(&hits), vec![r3, r2, r1]);
    let states: Vec<_> = hits.iter().map(|h| h.state.as_str()).collect();
    assert_eq!(states, vec!["invalidated", "superseded", "active"]);
    // An undone record (a state row that says active) is an ordinary active hit.
    let r4 = seed(&w, w.a, &Rec::note(4, 4, "rollout plan", None)).await;
    set_state(&w, w.a, &r4, "active", None).await;
    let hits = search(&w, w.a, json!({"query": "rollout"})).await;
    assert!(ids(&hits).contains(&r4));
}

#[tokio::test]
async fn ms05_expired_and_drill_canary_records_are_hidden_whatever_the_flags() {
    let w = setup("ms05").await;
    let plain = seed(&w, w.a, &Rec::note(1, 1, "kestrel notes", None)).await;
    let future = seed(
        &w,
        w.a,
        &Rec {
            valid_until: Some("2099-01-01T00:00:00Z"),
            ..Rec::note(2, 2, "kestrel future", None)
        },
    )
    .await;
    let _expired = seed(
        &w,
        w.a,
        &Rec {
            valid_until: Some("2020-01-01T00:00:00Z"),
            ..Rec::note(3, 3, "kestrel expired", None)
        },
    )
    .await;
    let _canary = seed(
        &w,
        w.a,
        &Rec {
            ty: "drill-canary",
            ..Rec::note(4, 4, "kestrel canary", None)
        },
    )
    .await;
    for body in [
        json!({"query": "kestrel"}),
        json!({"query": "kestrel", "include_history": true}),
    ] {
        let got = ids(&search(&w, w.a, body).await);
        assert_eq!(
            got,
            vec![future.clone(), plain.clone()],
            "newest first among equal ranks"
        );
    }
}

#[tokio::test]
async fn ms06_words_are_anded_a_quoted_phrase_keeps_its_order_minus_excludes_and_or_widens() {
    let w = setup("ms06").await;
    let ab = seed(&w, w.a, &Rec::note(1, 1, "alpha beta", None)).await;
    let a = seed(&w, w.a, &Rec::note(2, 2, "alpha", None)).await;
    let bg = seed(&w, w.a, &Rec::note(3, 3, "beta gamma", None)).await;
    let ba = seed(&w, w.a, &Rec::note(4, 4, "beta alpha", None)).await;
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    let want = |v: &[&String]| sorted(v.iter().map(|s| (*s).clone()).collect());
    // AND: only the records with both words.
    assert_eq!(
        sorted(ids(&search(&w, w.a, json!({"query": "alpha beta"})).await)),
        want(&[&ab, &ba])
    );
    // A quoted phrase: in order only.
    assert_eq!(
        ids(&search(&w, w.a, json!({"query": "\"alpha beta\""})).await),
        vec![ab.clone()]
    );
    // `-word` excludes.
    assert_eq!(
        ids(&search(&w, w.a, json!({"query": "alpha -beta"})).await),
        vec![a.clone()]
    );
    // `or` returns either.
    assert_eq!(
        sorted(ids(
            &search(&w, w.a, json!({"query": "gamma or alpha"})).await
        )),
        want(&[&ab, &a, &bg, &ba])
    );
}

#[tokio::test]
async fn ms07_stems_match_and_stop_words_or_punctuation_alone_are_an_empty_list_not_an_error() {
    let w = setup("ms07").await;
    let d1 = seed(&w, w.a, &Rec::note(1, 1, "deploy the service", None)).await;
    let d2 = seed(&w, w.a, &Rec::note(2, 2, "we deployed it", None)).await;
    let mut got = ids(&search(&w, w.a, json!({"query": "deploying"})).await);
    got.sort();
    let mut want = vec![d1, d2];
    want.sort();
    assert_eq!(got, want);
    for q in [
        "the",
        "...",
        "!!! ???",
        "a an the of",
        "\"\"",
        "-",
        "or",
        "&|!()",
    ] {
        assert!(
            search(&w, w.a, json!({"query": q})).await.is_empty(),
            "query {q:?}"
        );
    }
}

#[tokio::test]
async fn ms08_a_summary_match_outranks_a_body_match_and_ties_go_newest_first_then_by_id_the_same_every_time()
 {
    let w = setup("ms08").await;
    let body_only = seed(
        &w,
        w.a,
        &Rec::note(1, 1, "weekly report", Some("zanzibar was mentioned")),
    )
    .await;
    let in_summary = seed(&w, w.a, &Rec::note(2, 2, "zanzibar report", None)).await;
    assert_eq!(
        ids(&search(&w, w.a, json!({"query": "zanzibar"})).await),
        vec![in_summary, body_only]
    );
    // Equal rank: newest first; the same instant: the smaller id first.
    let old = seed(&w, w.a, &Rec::note(10, 10, "quokka", None)).await;
    let new_hi = seed(&w, w.a, &Rec::note(12, 20, "quokka", None)).await;
    let new_lo = seed(&w, w.a, &Rec::note(11, 20, "quokka", None)).await;
    let want = vec![new_lo, new_hi, old];
    for _ in 0..3 {
        let hits = search(&w, w.a, json!({"query": "quokka"})).await;
        assert_eq!(ids(&hits), want);
        assert!(
            hits.windows(2).all(|p| p[0].rank >= p[1].rank),
            "rank descending"
        );
    }
    let hit = &search(&w, w.a, json!({"query": "quokka", "limit": 1})).await[0];
    assert_eq!(hit.id, want[0]);
    assert_eq!(hit.r#type, "note");
    assert_eq!(hit.summary, "quokka");
    assert!(hit.rank > 0.0);
    assert_eq!(
        hit.created_at, "2026-11-06T00:00:20.000000Z",
        "RFC 3339 with microseconds and a Z"
    );
}

#[tokio::test]
async fn ms12_a_word_at_100000_characters_is_found_one_at_250000_is_not_and_a_long_record_says_it_was_cut()
 {
    let w = setup("ms12").await;
    let filler = "lorem ".repeat(16_000); // 96,000 characters
    let mut long = String::new();
    long.push_str(&filler);
    long.push_str(" needleone ");
    long.push_str(&"lorem ".repeat(27_000)); // pushes needletwo past 250,000 characters
    long.push_str(" needletwo");
    assert!(long.chars().count() > 250_000 && long.len() <= 262_144);
    let long_id = seed(&w, w.a, &Rec::note(1, 1, "long record", Some(&long))).await;
    let short_id = seed(
        &w,
        w.a,
        &Rec::note(2, 2, "short record", Some("needleone is here too")),
    )
    .await;
    let hits = search(&w, w.a, json!({"query": "needleone"})).await;
    let mut got = ids(&hits);
    got.sort();
    let mut want = vec![long_id.clone(), short_id.clone()];
    want.sort();
    assert_eq!(got, want);
    let by_id = |id: &String| hits.iter().find(|h| &h.id == id).expect("a hit");
    assert!(
        by_id(&long_id).truncated,
        "a record over 200,000 characters carries truncated: true"
    );
    assert!(!by_id(&short_id).truncated);
    // The known gap, stated: a word only in the tail is invisible.
    assert!(
        search(&w, w.a, json!({"query": "needletwo"}))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn ms13_adversarial_bodies_insert_and_are_searchable_by_their_summary() {
    let w = setup("ms13").await;
    // (summary word, type, body)
    let prose = "the quick brown fox jumps over the lazy dog ".repeat(5_957); // about 256 KiB
    let one_token = "x".repeat(200_000);
    let mut short_tokens = String::new();
    for i in 0..100_000u32 {
        // 2-character tokens from a pool of 400 letters squared: distinct, 3 bytes per token with the space.
        let (a, b) = (i / 400, i % 400);
        short_tokens.push(char::from_u32(0x4E00 + a).expect("cjk"));
        short_tokens.push(char::from_u32(0x4E00 + b).expect("cjk"));
        short_tokens.push(' ');
    }
    let cjk: String = (0..90_000u32)
        .map(|i| format!("{} ", char::from_u32(0x4E00 + i % 20_000).expect("cjk")))
        .collect();
    let hex: String = (0..131_072u32)
        .map(|i| format!("{:02x}", i % 251))
        .collect();
    let corpora: Vec<(&str, &str, String)> = vec![
        ("corpusprose", "note", prose),
        (
            "corpusconversation",
            "conversation",
            "word ".repeat(838_000),
        ), // about 4 MiB
        ("corpusonetoken", "note", one_token),
        ("corpusshorttokens", "conversation", short_tokens),
        ("corpuscjk", "conversation", cjk),
        ("corpushex", "note", hex),
    ];
    for (n, (word, ty, body)) in corpora.iter().enumerate() {
        let rec = Rec {
            ty,
            ..Rec::note(100 + n as u64, 1 + n as u64, word, Some(body))
        };
        let id = seed(&w, w.a, &rec).await; // an insert that errors out of the generated column fails here
        let hits = search(&w, w.a, json!({"query": word})).await;
        assert_eq!(
            ids(&hits),
            vec![id],
            "the summary word {word} finds its record"
        );
    }
}

#[tokio::test]
async fn ms22_types_since_until_and_limit_narrow_the_answer() {
    let w = setup("ms22").await;
    let d1 = seed(
        &w,
        w.a,
        &Rec {
            ty: "decision",
            ..Rec::note(1, 10, "gamma one", None)
        },
    )
    .await;
    let n2 = seed(&w, w.a, &Rec::note(2, 20, "gamma two", None)).await;
    let l3 = seed(
        &w,
        w.a,
        &Rec {
            ty: "lesson",
            ..Rec::note(3, 30, "gamma three", None)
        },
    )
    .await;
    let all = ids(&search(&w, w.a, json!({"query": "gamma"})).await);
    assert_eq!(all.len(), 3);
    // types
    assert_eq!(
        ids(&search(&w, w.a, json!({"query": "gamma", "types": ["decision"]})).await),
        vec![d1.clone()]
    );
    let mut two = ids(&search(
        &w,
        w.a,
        json!({"query": "gamma", "types": ["decision", "lesson"]}),
    )
    .await);
    two.sort();
    let mut want = vec![d1.clone(), l3.clone()];
    want.sort();
    assert_eq!(two, want);
    // [since, until): since inclusive, until exclusive.
    let s = at(20);
    assert_eq!(
        ids(&search(&w, w.a, json!({"query": "gamma", "since": s})).await),
        vec![l3.clone(), n2.clone()]
    );
    assert_eq!(
        ids(&search(&w, w.a, json!({"query": "gamma", "until": at(30)})).await),
        vec![n2.clone(), d1.clone()]
    );
    assert_eq!(
        ids(&search(
            &w,
            w.a,
            json!({"query": "gamma", "since": at(20), "until": at(30)})
        )
        .await),
        vec![n2.clone()]
    );
    // An offset is honoured as an instant: 01:00:20+01:00 is 00:00:20Z.
    assert_eq!(
        ids(&search(&w, w.a, json!({"query": "gamma", "since": "2026-11-06T01:00:20+01:00", "until": "2026-11-06T01:00:30+01:00"})).await),
        vec![n2.clone()]
    );
    // limit takes the first rows of the order.
    assert_eq!(
        ids(&search(&w, w.a, json!({"query": "gamma", "limit": 2})).await),
        all[..2].to_vec()
    );
    let _ = (BASE_MS, uuid7(0, 0));
}
