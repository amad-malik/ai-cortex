//! S3 r2.7 conformance (cox: "point them at the r2.7 JSON", 13:57Z; it was r2.6 before), DB-free. The data file is an extract of the
//! pinned JSON twin (1d2c4b2f..., `mvp/wave1-pins-20261106-S3r27`): the records operations and their schemas. What this piece accepts and answers must match it, property for
//! property; and the problem codes S3 lists per operation must be codes this piece (or the platform) can return.

use cortex_module::{Role, SectionName};
use cortex_types::{RecordId, SystemClock};
use mod_records::problems;
use mod_records::request::{Invalidate, RecordCreate, Supersede};
use mod_records::store::{Created, Lineage, LineageState, RecordOut, SectionPointer, SourceOut};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn s3() -> Value {
    serde_json::from_str(include_str!("data/s3-r2.7-records.1d2c4b2f.json"))
        .expect("the extract parses")
}

fn props(schema: &str) -> BTreeSet<String> {
    s3()["schemas"][schema]["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .cloned()
        .collect()
}

fn required(schema: &str) -> BTreeSet<String> {
    s3()["schemas"][schema]["required"]
        .as_array()
        .expect("required")
        .iter()
        .map(|v| v.as_str().expect("name").to_owned())
        .collect()
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().expect("an object").keys().cloned().collect()
}

fn id() -> RecordId {
    RecordId::new(&SystemClock)
}

/// An output object is valid against its S3 schema when it has no property S3 does not know (additionalProperties: false) and every
/// required property is present.
fn conforms(schema: &str, v: &Value) {
    let got = keys(v);
    let known = props(schema);
    let unknown: Vec<_> = got.difference(&known).collect();
    assert!(
        unknown.is_empty(),
        "{schema}: properties S3 does not know: {unknown:?}"
    );
    let missing: Vec<_> = required(schema).difference(&got).cloned().collect();
    assert!(
        missing.is_empty(),
        "{schema}: required properties missing: {missing:?}"
    );
}

#[test]
fn c01_every_s3_property_of_the_requests_decodes_and_an_unknown_one_does_not() {
    let (reply, handoff) = (id().to_string(), "01930000-0000-7000-8000-0000000000cc");
    let full = json!({"type":"knowledge","summary":"s","body":"b","fields":{"x":1},
        "source":{"kind":"file","ref":"/x","span":"sp"},"affects":[id().to_string()],"reply_to":reply,"handoff_id":handoff,
        "section":"rules-project","valid_until":"2030-01-01T00:00:00Z"});
    assert_eq!(
        keys(&full),
        props("RecordCreate"),
        "the body used here names every S3 property"
    );
    serde_json::from_value::<RecordCreate>(full.clone()).expect("every S3 property decodes");
    let mut extra = full;
    extra["surprise"] = json!(1);
    assert!(
        serde_json::from_value::<RecordCreate>(extra).is_err(),
        "additionalProperties: false"
    );

    let sup = json!({"by": id().to_string(), "reason": "r"});
    assert_eq!(keys(&sup), props("Supersede"));
    serde_json::from_value::<Supersede>(sup).expect("supersede decodes");
    let inv = json!({"reason": "r", "undo": true});
    assert_eq!(keys(&inv), props("Invalidate"));
    serde_json::from_value::<Invalidate>(inv).expect("invalidate decodes");
    assert!(serde_json::from_value::<Invalidate>(json!({"reason":"r","surprise":1})).is_err());
    // required: `type` and `summary` for a record, `by` for a supersede, nothing for an invalidation.
    assert_eq!(
        required("RecordCreate"),
        ["summary", "type"].map(String::from).into()
    );
    assert!(serde_json::from_value::<RecordCreate>(json!({"summary":"s"})).is_err());
    assert!(serde_json::from_value::<Supersede>(json!({"reason":"r"})).is_err());
    serde_json::from_value::<Invalidate>(json!({}))
        .expect("an invalidation needs no field to decode (reason_required is the handler's)");
}

#[test]
fn c02_every_answer_has_only_s3_properties_and_all_the_required_ones() {
    let created = serde_json::to_value(Created {
        id: id(),
        created_at: "2026-11-06T12:00:00.000000Z".into(),
        section_revision: Some(2),
    })
    .expect("json");
    conforms("RecordRef", &created);
    let lineage = Lineage {
        reason: "r".into(),
        by_kind: "worker".into(),
        by_id: "w".into(),
        at: "2026-11-06T12:00:00.000000Z".into(),
    };
    let full = RecordOut {
        id: id(),
        kind: "knowledge".into(),
        summary: "s".into(),
        body: Some("b".into()),
        fields: json!({}),
        source: Some(SourceOut {
            kind: "file".into(),
            reference: "/x".into(),
            span: Some("s".into()),
        }),
        state: "superseded".into(),
        superseded_by: Some(id()),
        lineage: Some(lineage),
        created_at: "2026-11-06T12:00:00.000000Z".into(),
        author_kind: "worker".into(),
        author: "w".into(),
        provenance: "worker_written".into(),
        reply_to: Some(id()),
        handoff_id: Some(id().to_string()),
        valid_until: Some("2030-01-01T00:00:00.000000Z".into()),
        section: Some("rules-project".into()),
        section_revision: Some(1),
    };
    let v = serde_json::to_value(&full).expect("json");
    conforms("Record", &v);
    assert_eq!(
        keys(&v),
        props("Record"),
        "a fully populated record names every S3 property"
    );
    conforms("Source", &v["source"]);
    assert_eq!(keys(&v["source"]), props("Source"));
    conforms("Lineage", &v["lineage"]);
    // A minimal record omits the optional properties (they are absent, never null).
    let min = RecordOut {
        body: None,
        source: None,
        superseded_by: None,
        lineage: None,
        reply_to: None,
        handoff_id: None,
        valid_until: None,
        section: None,
        section_revision: None,
        state: "active".into(),
        ..full
    };
    let v = serde_json::to_value(&min).expect("json");
    conforms("Record", &v);
    assert!(
        v.as_object()
            .expect("object")
            .values()
            .all(|x| !x.is_null()),
        "no null property"
    );
    let state = serde_json::to_value(LineageState {
        id: id(),
        state: "active".into(),
    })
    .expect("json");
    conforms("LineageState", &state);
    let ptr = serde_json::to_value(SectionPointer {
        section: "orientation".into(),
        record_id: id(),
        revision: 1,
        sha256: "0".repeat(64),
        source_path: Some("/p".into()),
    })
    .expect("json");
    conforms("SectionPointer", &ptr);
    assert_eq!(keys(&ptr), props("SectionPointer"));
    let ptr = serde_json::to_value(SectionPointer {
        section: "orientation".into(),
        record_id: id(),
        revision: 1,
        sha256: "0".repeat(64),
        source_path: None,
    })
    .expect("json");
    conforms("SectionPointer", &ptr);
}

#[test]
fn c03_the_problem_codes_s3_lists_are_codes_this_piece_or_the_platform_returns_and_the_converse() {
    let platform = ["not_found", "invalid_request", "target_not_allowed"];
    let mine: Vec<(&str, u16)> = problems::ALL.iter().map(|p| (p.code, p.status)).collect();
    let mut listed = BTreeSet::new();
    for (name, op) in s3()["operations"].as_object().expect("operations") {
        for e in op["x_errors"].as_array().expect("x_errors") {
            let e = e.as_str().expect("entry");
            let (status, code) = e.split_once(' ').expect("'<status> <code>'");
            let status: u16 = status.parse().expect("status");
            listed.insert(code.to_owned());
            if platform.contains(&code) {
                continue;
            }
            let found = mine.iter().find(|(c, _)| *c == code);
            assert!(
                found.is_some(),
                "{name}: S3 lists {code}, which this piece does not mint"
            );
            assert_eq!(
                found.map(|f| f.1),
                Some(status),
                "{name}: {code} has another status in S3"
            );
        }
    }
    for (code, _) in &mine {
        assert!(
            listed.contains(*code),
            "{code} is minted but no S3 operation of this piece lists it"
        );
    }
}

#[test]
fn c04_the_section_names_s3_accepts_are_the_names_the_piece_parses() {
    // S3: rules-project | rules-worker:<36 characters> | orientation; the pattern lives in the RecordCreate.section schema.
    let pattern = s3()["schemas"]["RecordCreate"]["properties"]["section"]["pattern"]
        .as_str()
        .expect("pattern")
        .to_owned();
    assert_eq!(
        pattern,
        "^(rules-project|rules-worker:[0-9a-f-]{36}|orientation)$"
    );
    for ok in [
        "rules-project",
        "orientation",
        "rules-worker:01930000-0000-7000-8000-0000000000aa",
    ] {
        assert!(SectionName::parse(ok).is_some(), "{ok}");
    }
    for bad in ["project-brief", "scratch", "rules-worker:short", ""] {
        assert!(SectionName::parse(bad).is_none(), "{bad}");
    }
    let _ = Role::ProjectLead;
}

/// cox W1-34: `reason_required` is declared by four pieces and its title and fix come from ONE file (api/shared-problem-codes.json,
/// copied byte for byte into tests/data). A piece declares the code with exactly those four fields.
#[test]
fn c05_reason_required_equals_the_shared_problem_codes_file() {
    let shared: Value =
        serde_json::from_str(include_str!("data/shared-problem-codes.13573a0e.json"))
            .expect("shared codes");
    let entry = shared["codes"]
        .as_array()
        .expect("codes")
        .iter()
        .find(|c| c["code"] == "reason_required")
        .expect("reason_required is in the shared file");
    let mine = problems::REASON_REQUIRED;
    assert_eq!(mine.code, entry["code"].as_str().expect("code"));
    assert_eq!(
        u64::from(mine.status),
        entry["status"].as_u64().expect("status")
    );
    assert_eq!(mine.title, entry["title"].as_str().expect("title"));
    assert_eq!(mine.fix, entry["fix"].as_str().expect("fix"));
}
