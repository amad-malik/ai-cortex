//! RED for piece 7, DB-free: decoding and step 0 of `POST /v2/search` (mvp/pieces/mod-search-TEST-DEFINITIONS-r0.md MS-09, MS-10, MS-11,
//! MS-19, MS-22, MS-24; S3 r2.7 R-1 to R-3 and R-16; the date-time rule W1-39). The bounds come BEFORE any database read and before the
//! route's own cap: an invalid request takes no slot (MS-15b).

use cortex_module::Problem;
use mod_search::validate::{check, is_rfc3339};
use mod_search::{RecordType, Validated};
use serde_json::{Value, json};

const JSON: Option<&str> = Some("application/json");

fn run(body: Value) -> Result<Validated, Problem> {
    check(JSON, &serde_json::to_vec(&body).expect("serialises"))
}

fn ok(body: Value) -> Validated {
    run(body).unwrap_or_else(|p| panic!("expected Ok, got {} {} ({})", p.status, p.code, p.detail))
}

fn refused(body: Value) -> (&'static str, u16) {
    match run(body) {
        Err(p) => (p.code, p.status),
        Ok(v) => panic!("expected a refusal, got {v:?}"),
    }
}

fn refused_raw(ct: Option<&str>, raw: &[u8]) -> (&'static str, u16) {
    match check(ct, raw) {
        Err(p) => (p.code, p.status),
        Ok(v) => panic!("expected a refusal, got {v:?}"),
    }
}

const INVALID: (&str, u16) = ("invalid_request", 422);

// ---- MS-09: the query, in CHARACTERS (R-1: the one limit counted in characters, nix D-11) ----

#[test]
fn ms09_query_is_1_to_1000_characters_and_the_over_long_code_is_its_own() {
    assert_eq!(ok(json!({"query": "a"})).query, "a");
    ok(json!({"query": "x".repeat(1000)}));
    // 1,000 four-byte characters are 4,000 bytes and still allowed.
    ok(json!({"query": "\u{1F600}".repeat(1000)}));
    assert_eq!(
        refused(json!({"query": "x".repeat(1001)})),
        ("query_too_long", 422)
    );
    assert_eq!(
        refused(json!({"query": "\u{1F600}".repeat(1001)})),
        ("query_too_long", 422)
    );
    // Empty is the generic code (minLength 1).
    assert_eq!(refused(json!({"query": ""})), INVALID);
    // A missing query, a non-string query.
    assert_eq!(refused(json!({})), INVALID);
    assert_eq!(refused(json!({"query": 7})), INVALID);
    assert_eq!(refused(json!({"query": null})), INVALID);
    assert_eq!(refused(json!({"query": ["a"]})), INVALID);
}

#[test]
fn ms09_a_nul_character_is_422_and_the_syntax_and_encoding_edges_are_400() {
    assert_eq!(refused(json!({"query": "a\u{0}b"})), INVALID);
    // Raw bytes: invalid UTF-8, a lone surrogate escape and bad syntax are 400 (R-2, R-3).
    let bad_utf8 = b"{\"query\":\"a\xff\xfeb\"}";
    assert_eq!(refused_raw(JSON, bad_utf8), ("invalid_request", 400));
    assert_eq!(
        refused_raw(JSON, br#"{"query":"a\ud800b"}"#),
        ("invalid_request", 400)
    );
    assert_eq!(
        refused_raw(JSON, br#"{"query":"a""#),
        ("invalid_request", 400)
    );
    assert_eq!(refused_raw(JSON, b""), ("invalid_request", 400));
    assert_eq!(
        refused_raw(JSON, br#"{"query":"a"} trailing"#),
        ("invalid_request", 400)
    );
    // A NUL written as an escape in the JSON text is the same NUL.
    assert_eq!(
        refused_raw(JSON, br#"{"query":"a\u0000b"}"#),
        ("invalid_request", 422)
    );
}

#[test]
fn ms09_fuzzed_queries_are_accepted_as_text_and_never_executed_here() {
    for q in [
        "\"unbalanced quote",
        "& | ! ( ) : * < > ' \" \\",
        "SELECT 1; DROP TABLE records.records; --",
        "\u{1F600}\u{1F4A5}",
        "\u{05E9}\u{05DC}\u{05D5}\u{05DD} \u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        "! ! ! ! ! !",
        "the",
        "...",
    ] {
        assert_eq!(ok(json!({"query": q})).query, q, "query {q:?}");
    }
    ok(json!({"query": "&|!".repeat(333)}));
}

// ---- R-3: protocol edges (MS-24, the part the handler owns; the 405 and the 413 are the chain's) ----

#[test]
fn ms24_the_body_must_be_a_json_object_sent_as_application_json() {
    let obj = br#"{"query":"a"}"#;
    assert_eq!(refused_raw(None, obj), ("invalid_request", 415));
    assert_eq!(
        refused_raw(Some("text/plain"), obj),
        ("invalid_request", 415)
    );
    assert_eq!(
        refused_raw(Some("application/x-www-form-urlencoded"), obj),
        ("invalid_request", 415)
    );
    check(Some("application/json; charset=utf-8"), obj).expect("a charset parameter is fine");
    check(Some("Application/JSON"), obj).expect("the media type is case-insensitive");
    // Valid JSON that is not an object is 422; syntax errors are 400.
    for raw in [
        &b"[]"[..],
        br#"["query"]"#,
        b"null",
        b"7",
        br#""query""#,
        b"true",
    ] {
        assert_eq!(
            refused_raw(JSON, raw),
            INVALID,
            "body {:?}",
            String::from_utf8_lossy(raw)
        );
    }
    assert_eq!(refused_raw(JSON, b"{"), ("invalid_request", 400));
    assert_eq!(refused_raw(JSON, b"not json"), ("invalid_request", 400));
}

// ---- MS-11: the schema is closed ----

#[test]
fn ms11_an_unknown_field_is_422_never_dropped() {
    for field in ["room", "hall", "graph", "no_rerank", "surprise", "Query"] {
        assert_eq!(
            refused(json!({"query": "a", field: true})),
            INVALID,
            "field {field}"
        );
    }
    // The same field twice is a shape error, not a silent last-wins.
    assert_eq!(
        refused_raw(JSON, br#"{"query":"a","limit":1,"limit":2}"#),
        INVALID
    );
    // A null for a property that is not nullable is a wrong type.
    for f in [
        "mode",
        "types",
        "since",
        "until",
        "limit",
        "include_history",
    ] {
        assert_eq!(refused(json!({"query": "a", f: null})), INVALID, "null {f}");
    }
}

// ---- MS-10: limit ----

#[test]
fn ms10_limit_is_1_to_50_default_10_and_never_clamped() {
    assert_eq!(ok(json!({"query": "a"})).limit, 10);
    assert_eq!(ok(json!({"query": "a", "limit": 1})).limit, 1);
    assert_eq!(ok(json!({"query": "a", "limit": 50})).limit, 50);
    for bad in [
        json!(51),
        json!(0),
        json!(-1),
        json!(1000),
        json!(10.5),
        json!("10"),
        json!(true),
    ] {
        assert_eq!(
            refused(json!({"query": "a", "limit": bad.clone()})),
            INVALID,
            "limit {bad}"
        );
    }
}

// ---- MS-19: mode ----

#[test]
fn ms19_only_the_keyword_mode_exists() {
    ok(json!({"query": "a", "mode": "keyword"}));
    for bad in [
        json!("hybrid"),
        json!("vector"),
        json!("fts"),
        json!(""),
        json!("Keyword"),
        json!(1),
        json!(true),
    ] {
        assert_eq!(
            refused(json!({"query": "a", "mode": bad.clone()})),
            INVALID,
            "mode {bad}"
        );
    }
}

// ---- MS-22: the filters ----

#[test]
fn ms22_types_are_the_twelve_enum_values_once_each() {
    let all = [
        "decision",
        "lesson",
        "note",
        "status",
        "question",
        "blocker",
        "knowledge",
        "conversation",
        "work-product",
        "human-message",
        "drill-canary",
        "drill",
    ];
    let v = ok(json!({"query": "a", "types": all}));
    assert_eq!(v.types.len(), 12);
    let v = ok(json!({"query": "a", "types": ["decision", "work-product"]}));
    assert_eq!(v.types, vec![RecordType::Decision, RecordType::WorkProduct]);
    assert_eq!(
        ok(json!({"query": "a", "types": []})).types,
        Vec::<RecordType>::new(),
        "an empty list is no filter"
    );
    assert_eq!(ok(json!({"query": "a"})).types, Vec::<RecordType>::new());
    // A duplicate (uniqueItems), an unknown value, the underscore spelling, a wrong case, a non-string.
    for bad in [
        json!(["note", "note"]),
        json!(["nope"]),
        json!(["work_product"]),
        json!(["Note"]),
        json!([1]),
        json!("note"),
        json!(["note", null]),
    ] {
        assert_eq!(
            refused(json!({"query": "a", "types": bad.clone()})),
            INVALID,
            "types {bad}"
        );
    }
}

#[test]
fn ms22_include_history_is_a_boolean() {
    assert!(!ok(json!({"query": "a"})).history);
    assert!(ok(json!({"query": "a", "include_history": true})).history);
    assert!(!ok(json!({"query": "a", "include_history": false})).history);
    for bad in [json!("true"), json!(1), json!([true])] {
        assert_eq!(
            refused(json!({"query": "a", "include_history": bad.clone()})),
            INVALID
        );
    }
}

#[test]
fn ms22_the_interval_is_since_inclusive_until_exclusive_and_compared_as_instants() {
    let v =
        ok(json!({"query": "a", "since": "2026-11-06T13:00:00Z", "until": "2026-11-06T14:00:00Z"}));
    assert!(!v.empty_interval());
    assert_eq!(
        v.since.as_deref(),
        Some("2026-11-06T13:00:00Z"),
        "the value is checked and kept, not normalised"
    );
    ok(json!({"query": "a", "since": "2026-11-06T13:00:00Z"}));
    ok(json!({"query": "a", "until": "2026-11-06T13:00:00Z"}));
    // since equal to until (as instants, whatever the offsets) is an empty interval, still a valid request.
    let same = ok(
        json!({"query": "a", "since": "2026-11-06T13:00:00+01:00", "until": "2026-11-06T12:00:00Z"}),
    );
    assert!(same.empty_interval());
    let same_ns = ok(
        json!({"query": "a", "since": "2026-11-06T13:00:00.123456789Z", "until": "2026-11-06T13:00:00.123456789Z"}),
    );
    assert!(same_ns.empty_interval());
    // since after until is 422; one nanosecond is enough.
    assert_eq!(
        refused(
            json!({"query": "a", "since": "2026-11-06T14:00:00Z", "until": "2026-11-06T13:00:00Z"})
        ),
        INVALID
    );
    assert_eq!(
        refused(
            json!({"query": "a", "since": "2026-11-06T13:00:00.000000001Z", "until": "2026-11-06T13:00:00Z"})
        ),
        INVALID
    );
    // Different offsets, the instant decides: 13:00+01:00 is 12:00Z, which is BEFORE 12:30Z.
    ok(
        json!({"query": "a", "since": "2026-11-06T13:00:00+01:00", "until": "2026-11-06T12:30:00Z"}),
    );
    assert!(!ok(json!({"query": "a", "since": "2026-11-06T13:00:00+01:00", "until": "2026-11-06T12:30:00Z"})).empty_interval());
    // Not a string.
    assert_eq!(refused(json!({"query": "a", "since": 5})), INVALID);
    assert_eq!(refused(json!({"query": "a", "until": true})), INVALID);
}

// ---- W1-39: the shared vectors, for the validator and for since and until ----

fn vectors() -> (Vec<String>, Vec<String>) {
    let v: Value = serde_json::from_str(include_str!("data/rfc3339-vectors.4125a4b4.json"))
        .expect("the vectors parse");
    let list = |k: &str| -> Vec<String> {
        v[k].as_array()
            .unwrap_or_else(|| panic!("{k} is a list"))
            .iter()
            .map(|x| x.as_str().expect("a string").to_owned())
            .collect()
    };
    (list("accept"), list("reject"))
}

#[test]
fn w139_the_shared_rfc3339_vectors_pass_for_since_and_until() {
    let (accept, reject) = vectors();
    assert_eq!((accept.len(), reject.len()), (14, 48), "the pinned file");
    for s in &accept {
        assert!(is_rfc3339(s), "accept {s:?}");
        ok(json!({"query": "a", "since": s}));
        ok(json!({"query": "a", "until": s}));
    }
    for s in &reject {
        assert!(!is_rfc3339(s), "reject {s:?}");
        assert_eq!(
            refused(json!({"query": "a", "since": s})),
            INVALID,
            "since {s:?}"
        );
        assert_eq!(
            refused(json!({"query": "a", "until": s})),
            INVALID,
            "until {s:?}"
        );
    }
}

// ---- R-16: step 0 is per field in schema order, before any cross-field rule ----

#[test]
fn r16_the_first_refusal_wins_in_schema_order_and_the_field_code_comes_with_its_field() {
    // `query` is the first property: its own code wins over a later bad field.
    assert_eq!(
        refused(json!({"query": "x".repeat(1001), "limit": 0, "since": "nope"})),
        ("query_too_long", 422)
    );
    assert_eq!(refused(json!({"query": "", "limit": 0})), INVALID);
    // A per-field refusal comes before the cross-field one (since after until).
    assert_eq!(
        refused(
            json!({"query": "a", "since": "2026-11-06T14:00:00Z", "until": "2026-11-06T13:00:00Z", "limit": 0})
        ),
        INVALID
    );
}

// ---- sam MS7-01: an enum property is a STRING; serde's default enum decoder also takes `{"variant": null}`, which S3 does not allow ----

#[test]
fn ms7_01_an_enum_property_given_as_an_object_is_422_not_a_valid_value() {
    for (name, body) in [
        (
            "mode as {keyword: null}",
            json!({"query": "a", "mode": {"keyword": null}}),
        ),
        (
            "mode as {keyword: {}}",
            json!({"query": "a", "mode": {"keyword": {}}}),
        ),
        (
            "mode as {hybrid: null}",
            json!({"query": "a", "mode": {"hybrid": null}}),
        ),
        (
            "types as [{note: null}]",
            json!({"query": "a", "types": [{"note": null}]}),
        ),
        (
            "types as [{work-product: null}]",
            json!({"query": "a", "types": [{"work-product": null}]}),
        ),
        (
            "types with a string and an object",
            json!({"query": "a", "types": ["decision", {"note": null}]}),
        ),
        (
            "types as [[\"note\"]]",
            json!({"query": "a", "types": [["note"]]}),
        ),
    ] {
        assert_eq!(refused(body), INVALID, "{name}");
    }
    // The controls: the plain strings still work.
    ok(json!({"query": "a", "mode": "keyword", "types": ["note", "work-product"]}));
}
