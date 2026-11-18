//! Wave 0 S0 delta 5, part 2: `AuditDetails`, the closed `details` of an audit row (cox 825a9022;
//! kai's design rule: `details` is a closed type; a token-shaped string must be unrecordable).
//! DB-free.
//!
//! The only value types are ids, codes and booleans; no string, number or nested value can be built
//! into one. `code` is the only way a string gets in, and it validates:
//! - **key** and **code** grammar: `^[a-z][a-z0-9_]{0,31}$` (no dot, colon, hyphen, space or case);
//! - at most 16 entries, no duplicate key;
//! - anything else is `PortError::Invalid`, never truncated, escaped or echoed.
//!
//! A token-shaped string (`ctx2_`, 32 hex, a dot, 43 characters: 81 in all) fails twice, on length and
//! on the dot. Because the grammar excludes quotes, backslashes and control characters, the JSON the
//! sink stores needs no escaping, and the worst case is far under the table's 16 KiB check.
//!
//! RED: the builders are `todo!()` (runtime), and the types are open, so the compile-fail fixtures in
//! `tests/ui/` compile when they must not (trybuild).
use cortex_module::{AuditDetails, DetailRef, DetailValue, PortError};
use cortex_types::{HandoffId, ProjectId, RecordId, SystemClock, TeamId, TokenId, WorkerId};

fn invalid(r: Result<AuditDetails, PortError>) -> bool {
    matches!(r, Err(PortError::Invalid(_)))
}

fn json(d: &AuditDetails) -> serde_json::Value {
    serde_json::from_str(&d.render_json()).expect("render_json is JSON")
}

/// The token shape: `ctx2_` + 32 hex + `.` + 43 characters = 81.
fn token_shaped() -> String {
    format!(
        "ctx2_{}.{}",
        "a1".repeat(16),
        "Zy_".repeat(15).get(..43).expect("45 characters")
    )
}

const KEYS: [&str; 17] = [
    "k00", "k01", "k02", "k03", "k04", "k05", "k06", "k07", "k08", "k09", "k10", "k11", "k12",
    "k13", "k14", "k15", "k16",
];

#[test]
fn d5_ad_1_an_empty_details_is_an_empty_object() {
    let d = AuditDetails::new();
    assert!(d.is_empty());
    assert_eq!(d.len(), 0);
    assert_eq!(d.render_json(), "{}");
    assert_eq!(json(&d), serde_json::json!({}));
}

#[test]
fn d5_ad_2_each_kind_of_id_is_recorded_as_its_lowercase_hyphenated_text() {
    let c = &SystemClock;
    let (p, t, w, k, r, h) = (
        ProjectId::new(c),
        TeamId::new(c),
        WorkerId::new(c),
        TokenId::new(c),
        RecordId::new(c),
        HandoffId::new(c),
    );
    let d = AuditDetails::new()
        .id("project", p)
        .and_then(|d| d.id("team", t))
        .and_then(|d| d.id("worker", w))
        .and_then(|d| d.id("token", k))
        .and_then(|d| d.id("record", r))
        .and_then(|d| d.id("handoff", h))
        .expect("six ids");
    assert_eq!(d.len(), 6);
    assert_eq!(
        json(&d),
        serde_json::json!({
            "project": p.to_string(), "team": t.to_string(), "worker": w.to_string(),
            "token": k.to_string(), "record": r.to_string(), "handoff": h.to_string(),
        })
    );
    // The entries keep the order they were added in, and name the kind of id.
    let keys: Vec<&str> = d.entries().iter().map(|(k, _)| *k).collect();
    assert_eq!(
        keys,
        ["project", "team", "worker", "token", "record", "handoff"]
    );
    assert_eq!(
        d.entries()[2].1,
        DetailValue::Ref(DetailRef::Worker(w)),
        "an entry reads back as the id it was given"
    );
}

#[test]
fn d5_ad_3_codes_and_flags_are_recorded_as_a_string_and_a_boolean() {
    let d = AuditDetails::new()
        .code("reason", "lost_key")
        .and_then(|d| d.flag("self_review", true))
        .and_then(|d| d.flag("forced", false))
        .expect("a code and two flags");
    assert_eq!(
        json(&d),
        serde_json::json!({"reason": "lost_key", "self_review": true, "forced": false})
    );
    assert_eq!(
        d.render_json(),
        r#"{"reason":"lost_key","self_review":true,"forced":false}"#
    );
}

#[test]
fn d5_ad_4_a_code_that_fits_the_grammar_is_accepted_at_its_boundaries() {
    for ok in ["a", "z9", "lost_key", "a_1_b_2", &"x".repeat(32)] {
        let d = AuditDetails::new()
            .code("reason", ok)
            .unwrap_or_else(|e| panic!("{ok:?} must be accepted, got {e:?}"));
        assert_eq!(json(&d), serde_json::json!({"reason": ok}));
    }
}

#[test]
fn d5_ad_5_a_code_outside_the_grammar_is_refused_never_truncated_or_escaped() {
    let long = "x".repeat(33);
    let token = token_shaped();
    assert_eq!(token.len(), 81, "the shape under test is 81 characters");
    let uuid = WorkerId::new(&SystemClock).to_string();
    let cases: Vec<(&str, &str)> = vec![
        ("empty", ""),
        ("33 characters", &long),
        ("a token-shaped string", &token),
        ("a UUID", &uuid),
        (
            "32 hex starting with a digit",
            "0123456789abcdef0123456789abcdef",
        ),
        ("a space (a sentence)", "lost it"),
        ("a NUL", "lost\0key"),
        ("a newline", "lost\nkey"),
        ("a double quote", "lost\"key"),
        ("a backslash", "lost\\key"),
        ("a capital", "Lost"),
        ("a dot", "lost.key"),
        ("a colon", "lost:key"),
        ("a hyphen", "lost-key"),
        ("a leading underscore", "_lost"),
        ("a leading digit", "1lost"),
        ("non-ASCII letters", "perdu\u{e9}"),
        ("full-width letters", "\u{ff4c}ost"),
    ];
    for (what, v) in cases {
        assert!(
            invalid(AuditDetails::new().code("reason", v)),
            "{what} must be refused as PortError::Invalid"
        );
    }
    // The control inside the case: the same call with a legal code succeeds.
    assert!(AuditDetails::new().code("reason", "lost_key").is_ok());
}

#[test]
fn d5_ad_6_a_refusal_carries_a_fixed_message_never_the_value() {
    let token = token_shaped();
    let err = AuditDetails::new()
        .code("reason", &token)
        .expect_err("a token-shaped string is refused");
    let shown = format!("{err} | {err:?}");
    assert!(matches!(err, PortError::Invalid(_)));
    assert!(
        !shown.contains("ctx2_"),
        "the refusal must not echo the value: {shown}"
    );
    assert!(
        !shown.contains("a1a1"),
        "the refusal must not echo the value: {shown}"
    );
}

#[test]
fn d5_ad_7_a_key_follows_the_same_grammar_for_every_kind_of_entry() {
    let id = WorkerId::new(&SystemClock);
    let long = "k".repeat(33);
    // The key is a `&'static str`; leak a few to test the grammar (a test, not a caller).
    let leak = |s: &str| -> &'static str { Box::leak(s.to_owned().into_boxed_str()) };
    let bad: Vec<(&str, &'static str)> = vec![
        ("empty", ""),
        ("capital", "Key"),
        ("leading digit", "1key"),
        ("hyphen", "a-b"),
        ("dot", "a.b"),
        ("space", "a b"),
        ("33 characters", leak(&long)),
    ];
    for (what, k) in bad {
        assert!(invalid(AuditDetails::new().id(k, id)), "id key {what}");
        assert!(
            invalid(AuditDetails::new().code(k, "lost_key")),
            "code key {what}"
        );
        assert!(
            invalid(AuditDetails::new().flag(k, true)),
            "flag key {what}"
        );
    }
    // Controls: the shortest and the longest legal keys.
    assert!(AuditDetails::new().flag("k", true).is_ok());
    assert!(
        AuditDetails::new()
            .flag(leak(&"k".repeat(32)), true)
            .is_ok()
    );
}

#[test]
fn d5_ad_8_a_key_is_used_once_whatever_the_kind() {
    let id = WorkerId::new(&SystemClock);
    let first = AuditDetails::new().flag("who", true).expect("first");
    assert!(invalid(first.clone().flag("who", false)), "flag then flag");
    assert!(
        invalid(first.clone().code("who", "lost_key")),
        "flag then code"
    );
    assert!(invalid(first.id("who", id)), "flag then id");
    // A different key is fine.
    assert!(
        AuditDetails::new()
            .flag("who", true)
            .and_then(|d| d.flag("whom", true))
            .is_ok()
    );
}

#[test]
fn d5_ad_9_at_most_sixteen_entries() {
    let mut d = AuditDetails::new();
    for k in &KEYS[..16] {
        d = d.flag(k, true).expect("the first sixteen fit");
    }
    assert_eq!(d.len(), 16);
    assert_eq!(AuditDetails::MAX_ENTRIES, 16);
    assert!(invalid(d.flag(KEYS[16], true)), "the 17th entry is refused");
}

#[test]
fn d5_ad_10_the_worst_case_is_valid_json_far_under_the_tables_16_kib_check() {
    let long_key =
        |i: usize| -> &'static str { Box::leak(format!("{:k<30}{i:02}", "").into_boxed_str()) };
    let uuid = WorkerId::new(&SystemClock);
    let mut d = AuditDetails::new();
    for i in 0..16 {
        let k = long_key(i);
        assert_eq!(k.len(), 32);
        d = match i % 3 {
            0 => d.id(k, uuid),
            1 => d.code(k, &"c".repeat(32)),
            _ => d.flag(k, true),
        }
        .expect("a maximal entry");
    }
    let text = d.render_json();
    assert!(text.len() < 16 * 1024, "{} bytes", text.len());
    assert!(
        text.len() < 2048,
        "in practice about 1 KiB: {} bytes",
        text.len()
    );
    assert_eq!(json(&d).as_object().expect("an object").len(), 16);
}

/// The compile-fail guard (trybuild): outside the module a string cannot be built into a detail.
/// Pairs: the legal public use compiles and runs; each forgery is refused with the pinned
/// diagnostic. `id_string_fail` is a shape pin (a `&str` is not an id) that holds in RED too.
#[test]
fn d5_ad_11_no_string_can_be_built_into_a_detail_outside_the_module() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/audit_details_closed_pass.rs");
    t.compile_fail("tests/ui/audit_details_literal_fail.rs");
    t.compile_fail("tests/ui/detail_value_forge_fail.rs");
    t.compile_fail("tests/ui/detail_id_string_fail.rs");
}
