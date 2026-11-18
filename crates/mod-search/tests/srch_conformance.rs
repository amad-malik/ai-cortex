//! RED for piece 7, DB-free: S3 r2.7 conformance. The data file is an extract of the pinned JSON twin (1d2c4b2f...): the `search.query`
//! operation and the schemas it uses (`SearchRequest`, `SearchResult`, `SearchHit`). What the piece accepts and answers must match it,
//! property for property, and every error code S3 lists for the operation must be a code this piece mints or the platform has.

use std::collections::BTreeSet;

use mod_search::validate::check;
use mod_search::{MANIFEST, MAX_BODY, PATH, SCOPE};
use serde_json::{Value, json};

fn s3() -> Value {
    serde_json::from_str(include_str!("data/s3-r2.7-search.1d2c4b2f.json"))
        .expect("the extract parses")
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("an object").keys().cloned().collect()
}

fn strings(v: &Value) -> BTreeSet<String> {
    v.as_array()
        .expect("an array")
        .iter()
        .map(|x| x.as_str().expect("a string").to_owned())
        .collect()
}

#[test]
fn c01_the_route_is_the_one_s3_declares() {
    let s = s3();
    let op = &s["operations"]["search.query"];
    assert_eq!(op["method"], "POST");
    assert_eq!(op["path"], PATH);
    assert_eq!(op["scope"], SCOPE);
    assert_eq!(op["max_body"], MAX_BODY);
    assert_eq!(op["target"], "None");
    assert_eq!(s["version"], "0.2.1-skeleton-draft.2.7");
}

#[test]
fn c02_every_request_property_decodes_and_an_unknown_one_does_not() {
    let s = s3();
    let schema = &s["schemas"]["SearchRequest"];
    assert_eq!(schema["additionalProperties"], false);
    let props = keys(&schema["properties"]);
    assert_eq!(
        props,
        [
            "query",
            "mode",
            "types",
            "since",
            "until",
            "limit",
            "include_history"
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
    );
    assert_eq!(strings(&schema["required"]), ["query".to_owned()].into());
    // A body with every property decodes; each property's schema bounds are the ones the validator enforces.
    check(
        Some("application/json"),
        &serde_json::to_vec(&json!({
            "query": "a", "mode": "keyword", "types": ["decision"], "since": "2026-11-06T13:00:00Z",
            "until": "2026-11-06T14:00:00Z", "limit": 50, "include_history": true
        }))
        .expect("json"),
    )
    .expect("every property is read");
    let p = &schema["properties"];
    assert_eq!(p["query"]["minLength"], 1);
    assert_eq!(p["query"]["maxLength"], 1000);
    assert_eq!(p["query"]["x-code"], "query_too_long");
    assert_eq!(p["limit"]["minimum"], 1);
    assert_eq!(p["limit"]["maximum"], 50);
    assert_eq!(p["limit"]["default"], 10);
    assert_eq!(strings(&p["mode"]["enum"]), ["keyword".to_owned()].into());
    assert_eq!(p["types"]["uniqueItems"], true);
    assert_eq!(strings(&p["types"]["items"]["enum"]).len(), 12);
}

#[test]
fn c03_the_answer_carries_only_s3_properties_and_all_the_required_ones() {
    let s = s3();
    let result = &s["schemas"]["SearchResult"];
    let hit = &s["schemas"]["SearchHit"];
    assert_eq!(
        strings(&result["required"]),
        ["hits".to_owned(), "mode".to_owned()].into()
    );
    assert_eq!(keys(&result["properties"]), strings(&result["required"]));
    assert_eq!(result["additionalProperties"], false);
    assert_eq!(hit["additionalProperties"], false);
    // A serialised hit has exactly the SearchHit properties; a serialised result exactly the SearchResult ones.
    let h = mod_search::Hit {
        id: "01930000-0000-7000-8000-000000000001".into(),
        r#type: "note".into(),
        summary: "s".into(),
        rank: 0.5,
        created_at: "2026-11-06T13:00:00.123456Z".into(),
        state: "active".into(),
        truncated: false,
    };
    let hv = serde_json::to_value(&h).expect("hit json");
    assert_eq!(keys(&hv), keys(&hit["properties"]));
    assert_eq!(keys(&hv), strings(&hit["required"]));
    let r = serde_json::to_value(mod_search::SearchResult {
        hits: vec![h],
        mode: "keyword",
    })
    .expect("result json");
    assert_eq!(keys(&r), keys(&result["properties"]));
    assert_eq!(r["mode"], "keyword");
}

#[test]
fn c04_every_error_code_s3_lists_is_minted_here_or_is_a_platform_code() {
    let s = s3();
    let listed: Vec<String> = s["operations"]["search.query"]["x_errors"]
        .as_array()
        .expect("x_errors")
        .iter()
        .map(|e| e.as_str().expect("text").to_owned())
        .collect();
    let minted: BTreeSet<&str> = MANIFEST.problems.iter().map(|p| p.code).collect();
    // The platform's own codes (the chain mints them): the piece must not mint these again.
    let platform = ["invalid_request", "busy", "unavailable"];
    for e in &listed {
        let (status, code) = e.split_once(' ').expect("status and code");
        assert!(
            minted.contains(code) || platform.contains(&code),
            "{e}: neither minted here nor a platform code"
        );
        if let Some(p) = MANIFEST.problems.iter().find(|p| p.code == code) {
            assert_eq!(p.status.to_string(), status, "{e}: the status");
        }
    }
    assert_eq!(
        minted,
        ["query_too_long"].into(),
        "and every code the piece mints is listed"
    );
}
