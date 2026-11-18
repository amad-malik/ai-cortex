//! RED for piece 6, DB-free: step 0 of `records.log` (tests/mod-records.md REC-02, 03, 05, 06, 08, 13, 25 to 27, 32).
//! The per-field checks come BEFORE the cross-field rules (CHANGES section 17); an enum or shape violation never reaches
//! the validator (serde refuses it, 422 `invalid_request`, R-3).

use cortex_module::{Problem, SectionName};
use mod_records::request::RecordCreate;
use mod_records::store::is_input_error_code;
use mod_records::validate::{Validated, is_rfc3339, validate_create};
use serde_json::{Value, json};

fn decode(v: Value) -> Result<RecordCreate, serde_json::Error> {
    serde_json::from_value(v)
}

/// Decode (must succeed) then validate.
fn run(v: Value) -> Result<Validated, Problem> {
    validate_create(&decode(v).expect("the body decodes"))
}

fn refused(v: Value) -> (&'static str, u16) {
    match run(v) {
        Err(p) => (p.code, p.status),
        Ok(ok) => panic!("expected a refusal, got {ok:?}"),
    }
}

fn accepted(v: Value) -> Validated {
    run(v).unwrap_or_else(|p| panic!("expected Ok, got {} {} ({})", p.status, p.code, p.detail))
}

const WORKER: &str = "01930000-0000-7000-8000-0000000000aa";

fn paths(n: usize) -> Value {
    Value::Array(
        (0..n)
            .map(|i| json!(format!("/docs/page-{i}.md")))
            .collect(),
    )
}

fn orientation(body: &str, reading_path: Value) -> Value {
    json!({"type":"knowledge","summary":"orientation","section":"orientation","body":body,"fields":{"reading_path":reading_path}})
}

// ---- REC-02 / REC-25: reserved and unknown types ----

#[test]
fn rec02_reserved_types_are_type_reserved() {
    for t in ["human-message", "drill-canary", "drill", "conversation"] {
        assert_eq!(
            refused(json!({"type":t,"summary":"s"})),
            ("type_reserved", 422),
            "type {t}"
        );
    }
}

#[test]
fn rec02_unknown_type_and_underscore_spelling_never_decode() {
    assert!(decode(json!({"type":"work_product","summary":"s"})).is_err());
    assert!(decode(json!({"type":"gossip","summary":"s"})).is_err());
    assert!(decode(json!({"type":"note","summary":"s","surprise":1})).is_err());
}

#[test]
fn rec01_every_writable_type_passes_step_zero() {
    for body in [
        json!({"type":"decision","summary":"s","fields":{"why":"because"}}),
        json!({"type":"lesson","summary":"s"}),
        json!({"type":"note","summary":"s"}),
        json!({"type":"status","summary":"s"}),
        json!({"type":"question","summary":"s"}),
        json!({"type":"blocker","summary":"s"}),
        json!({"type":"knowledge","summary":"s","source":{"kind":"file","ref":"/x"}}),
        json!({"type":"work-product","summary":"s","fields":{"files":["/a/file.rs"]}}),
    ] {
        accepted(body);
    }
}

// ---- REC-03: per-field before cross-field ----

#[test]
fn rec03_note_with_empty_source_ref_is_invalid_request_not_source_not_allowed() {
    assert_eq!(
        refused(json!({"type":"note","summary":"s","source":{"kind":"file","ref":""}})),
        ("invalid_request", 422)
    );
}

#[test]
fn rec03_knowledge_without_source_is_source_required() {
    assert_eq!(
        refused(json!({"type":"knowledge","summary":"s"})),
        ("source_required", 422)
    );
}

#[test]
fn rec03_note_with_a_valid_source_is_source_not_allowed() {
    assert_eq!(
        refused(json!({"type":"note","summary":"s","source":{"kind":"file","ref":"/x"}})),
        ("source_not_allowed", 422)
    );
}

#[test]
fn rec03_source_bounds_ref_2048_span_512() {
    let ok_ref = "r".repeat(2048);
    accepted(json!({"type":"knowledge","summary":"s","source":{"kind":"url","ref":ok_ref}}));
    let long = "r".repeat(2049);
    assert_eq!(
        refused(json!({"type":"knowledge","summary":"s","source":{"kind":"url","ref":long}})),
        ("invalid_request", 422)
    );
    assert_eq!(
        refused(
            json!({"type":"knowledge","summary":"s","source":{"kind":"file","ref":"/x","span":""}})
        ),
        ("invalid_request", 422)
    );
}

// ---- REC-05: type rules ----

#[test]
fn rec05_decision_needs_why() {
    assert_eq!(
        refused(json!({"type":"decision","summary":"s"})),
        ("why_required", 422)
    );
    assert_eq!(
        refused(json!({"type":"decision","summary":"s","fields":{"why":""}})),
        ("why_required", 422)
    );
    assert_eq!(
        refused(json!({"type":"decision","summary":"s","fields":{"why":7}})),
        ("why_required", 422)
    );
}

#[test]
fn rec05_work_product_needs_files() {
    assert_eq!(
        refused(json!({"type":"work-product","summary":"s"})),
        ("files_required", 422)
    );
    assert_eq!(
        refused(json!({"type":"work-product","summary":"s","fields":{"files":[]}})),
        ("files_required", 422)
    );
}

#[test]
fn rec05_knowledge_in_a_reserved_section_waives_the_source() {
    accepted(json!({"type":"knowledge","summary":"s","section":"rules-project"}));
    accepted(json!({"type":"knowledge","summary":"s","section":format!("rules-worker:{WORKER}")}));
}

// ---- REC-06: bounds are bytes ----

#[test]
fn rec06_summary_bound_is_512_bytes_not_characters() {
    accepted(json!({"type":"note","summary":"s".repeat(512)}));
    assert_eq!(
        refused(json!({"type":"note","summary":"s".repeat(513)})),
        ("invalid_request", 422)
    );
    // 257 two-byte characters: 257 characters, 514 bytes.
    assert_eq!(
        refused(json!({"type":"note","summary":"\u{e9}".repeat(257)})),
        ("invalid_request", 422)
    );
    accepted(json!({"type":"note","summary":"\u{e9}".repeat(256)}));
    assert_eq!(
        refused(json!({"type":"note","summary":""})),
        ("invalid_request", 422)
    );
}

#[test]
fn rec06_body_bound_is_262144_bytes() {
    accepted(json!({"type":"note","summary":"s","body":"b".repeat(262_144)}));
    assert_eq!(
        refused(json!({"type":"note","summary":"s","body":"b".repeat(262_145)})),
        ("invalid_request", 422)
    );
}

#[test]
fn rec06_fields_bound_is_65536_bytes_as_stored() {
    // jsonb text puts a space after ':' and after ',' : the bound is on that form.
    let fits = json!({"type":"note","summary":"s","fields":{"k":"v".repeat(65_536 - 9)}});
    accepted(fits);
    let over = json!({"type":"note","summary":"s","fields":{"k":"v".repeat(65_536)}});
    assert_eq!(refused(over), ("invalid_request", 422));
    // Many small members: compact JSON is about 60,000 bytes, the stored form with its separators about 72,000.
    let many: serde_json::Map<String, Value> =
        (0..6000).map(|i| (format!("k{i:04}"), json!(1))).collect();
    assert_eq!(
        refused(json!({"type":"note","summary":"s","fields":many})),
        ("invalid_request", 422)
    );
}

// ---- REC-08: NUL ----

#[test]
fn rec08_nul_is_refused_in_summary_body_fields_and_source() {
    for v in [
        json!({"type":"note","summary":"a\u{0}b"}),
        json!({"type":"note","summary":"s","body":"a\u{0}b"}),
        json!({"type":"note","summary":"s","fields":{"k":"a\u{0}b"}}),
        json!({"type":"note","summary":"s","fields":{"a\u{0}b":1}}),
        json!({"type":"knowledge","summary":"s","source":{"kind":"file","ref":"/a\u{0}b"}}),
    ] {
        assert_eq!(refused(v), ("invalid_request", 422));
    }
}

// ---- REC-13 (validation part): the section grammar ----

#[test]
fn rec13_section_grammar() {
    let w = format!("rules-worker:{WORKER}");
    assert_eq!(
        accepted(json!({"type":"knowledge","summary":"s","section":"rules-project"})).section,
        Some(SectionName::RulesProject)
    );
    assert!(matches!(
        accepted(json!({"type":"knowledge","summary":"s","section":w})).section,
        Some(SectionName::RulesWorker(_))
    ));
    let long = "x".repeat(129);
    for bad in [
        "scratch",
        "project-brief",
        "rules-worker:",
        "rules-worker:not-an-id",
        "rules-worker:01930000-0000-7000-8000-0000000000AA",
        "Rules-Project",
        long.as_str(),
    ] {
        assert_eq!(
            refused(json!({"type":"knowledge","summary":"s","section":bad})),
            ("section_invalid", 422),
            "section {bad:?}"
        );
    }
}

// ---- REC-26 / REC-27: orientation ----

#[test]
fn rec26_orientation_accepts_a_valid_body_and_reading_path_without_a_source() {
    let v = accepted(orientation(&"o".repeat(3072), paths(5)));
    assert_eq!(v.section, Some(SectionName::Orientation));
}

#[test]
fn rec26_orientation_body_is_at_most_3072_bytes_and_required() {
    assert_eq!(
        refused(orientation(&"o".repeat(3073), paths(5))),
        ("invalid_request", 422)
    );
    let no_body = json!({"type":"knowledge","summary":"o","section":"orientation","fields":{"reading_path":paths(5)}});
    assert_eq!(refused(no_body), ("invalid_request", 422));
    let no_fields = json!({"type":"knowledge","summary":"o","section":"orientation","body":"x"});
    assert_eq!(refused(no_fields), ("invalid_request", 422));
}

#[test]
fn rec27_reading_path_counts() {
    for n in [5, 6, 10] {
        accepted(orientation("b", paths(n)));
    }
    for n in [0, 1, 4, 11] {
        assert_eq!(
            refused(orientation("b", paths(n))),
            ("invalid_request", 422),
            "{n} entries"
        );
    }
    let missing =
        json!({"type":"knowledge","summary":"o","section":"orientation","body":"b","fields":{}});
    assert_eq!(refused(missing), ("invalid_request", 422));
    let not_array = orientation("b", json!("/a"));
    assert_eq!(refused(not_array), ("invalid_request", 422));
}

fn with_entry(entry: Value) -> Value {
    let mut p = paths(5);
    p.as_array_mut().unwrap()[0] = entry;
    orientation("b", p)
}

#[test]
fn rec27_reading_path_entry_rules() {
    accepted(with_entry(json!(format!("/{}", "e".repeat(255)))));
    assert_eq!(
        refused(with_entry(json!(format!("/{}", "e".repeat(256))))),
        ("locator_too_long", 422)
    );
    // Length is checked before the pattern: 257 bytes AND relative is still locator_too_long.
    assert_eq!(
        refused(with_entry(json!("e".repeat(257)))),
        ("locator_too_long", 422)
    );
    for bad in [
        json!("docs/x"),
        json!(""),
        json!("/a\nb"),
        json!("/a\u{0}b"),
        json!("/a\u{7f}b"),
        json!(7),
    ] {
        assert_eq!(
            refused(with_entry(bad.clone())),
            ("invalid_request", 422),
            "entry {bad}"
        );
    }
}

// ---- REC-32: a section needs the type knowledge ----

#[test]
fn rec32_a_non_knowledge_type_with_a_section_is_invalid_request() {
    let w = format!("rules-worker:{WORKER}");
    for t in ["note", "decision", "lesson"] {
        for sec in ["orientation", "rules-project", w.as_str()] {
            let mut v = json!({"type":t,"summary":"s","section":sec,"body":"b","fields":{"why":"w","reading_path":paths(5)}});
            if t == "decision" {
                v["fields"]["why"] = json!("because");
            }
            assert_eq!(refused(v), ("invalid_request", 422), "{t} in {sec}");
        }
    }
}

// ---- REC-05 (work-product files, cox W1-15): a NON-EMPTY ARRAY OF STRINGS, each 1 to 1024 bytes, no control character, no NUL ----

fn wp(files: Value) -> Value {
    json!({"type":"work-product","summary":"s","fields":{"files":files}})
}

#[test]
fn rec05_files_absent_or_empty_is_files_required() {
    assert_eq!(
        refused(json!({"type":"work-product","summary":"s"})),
        ("files_required", 422)
    );
    assert_eq!(
        refused(json!({"type":"work-product","summary":"s","fields":{}})),
        ("files_required", 422)
    );
    assert_eq!(refused(wp(json!([]))), ("files_required", 422));
}

#[test]
fn rec05_files_must_be_an_array_of_non_empty_strings_without_control_characters() {
    accepted(wp(json!(["a"])));
    accepted(wp(json!(["a", "b", "/c/d.md", "dup", "dup"]))); // no uniqueness rule, no item-count limit of its own
    for bad in [
        json!("a"),
        json!({}),
        json!(null),
        json!(7),
        json!([7]),
        json!([null]),
        json!([["a"]]),
        json!([{"path":"/a"}]),
        json!(["a", ""]),
        json!(["/x\nstill"]),
        json!(["tab\there"]),
        json!(["del\u{7f}char"]),
        json!(["a\u{0}b"]),
    ] {
        assert_eq!(
            refused(wp(bad.clone())),
            ("invalid_request", 422),
            "files {bad}"
        );
    }
}

#[test]
fn rec05_files_item_bound_is_1024_bytes_not_characters() {
    accepted(wp(json!(["f"]))); // 1 byte passes
    accepted(wp(json!(["f".repeat(1024)]))); // 1024 bytes pass
    assert_eq!(
        refused(wp(json!(["f".repeat(1025)]))),
        ("locator_too_long", 422)
    ); // 1025 fail
    // Multibyte at the edge: 512 two-byte characters are 1024 bytes; 513 are 1026.
    accepted(wp(json!(["\u{e9}".repeat(512)])));
    assert_eq!(
        refused(wp(json!(["\u{e9}".repeat(513)]))),
        ("locator_too_long", 422)
    );
    // 1022 ASCII bytes plus one two-byte character is 1024 (passes); 1023 plus one is 1025 (fails).
    accepted(wp(json!([format!("{}\u{e9}", "f".repeat(1022))])));
    assert_eq!(
        refused(wp(json!([format!("{}\u{e9}", "f".repeat(1023))]))),
        ("locator_too_long", 422)
    );
    // A three-byte and a four-byte character at the edge.
    accepted(wp(json!([format!("{}\u{20ac}", "f".repeat(1021))]))); // 1021 + 3 = 1024
    assert_eq!(
        refused(wp(json!([format!("{}\u{1f600}", "f".repeat(1021))]))),
        ("locator_too_long", 422)
    ); // 1021 + 4 = 1025
}

#[test]
fn rec05_files_length_is_checked_before_the_pattern() {
    // In-field order: length, then pattern. An item that is both over 1024 bytes and holds a control character is too long.
    let both = format!("{}\n", "f".repeat(1025));
    assert_eq!(refused(wp(json!([both]))), ("locator_too_long", 422));
    // Another item's problem does not hide this one, and the first failing item decides.
    assert_eq!(
        refused(wp(json!(["ok", "f".repeat(1025), ""]))),
        ("locator_too_long", 422)
    );
}

// ---- sam's 18 definition-only cells for the work-product files ruling (cox W1-15; cells sha256 e27fec1a2047e723...) ----

/// Each cell's body goes through step 0: a 201 cell must pass step 0 (the write itself is the DB tests'), every other cell must be
/// refused with exactly its expected status and code. The data file is a byte-for-byte copy of sam's cells.
#[test]
fn rec05_sams_work_product_files_cells_hold() {
    let cells: Value =
        serde_json::from_str(include_str!("data/work-product-files-cells.e27fec1a.json"))
            .expect("cells json");
    let cells = cells["cells"].as_array().expect("a cells array");
    assert_eq!(cells.len(), 18);
    for cell in cells {
        let id = cell["case_id"].as_str().expect("case_id");
        let status = cell["expected_status"].as_u64().expect("status");
        let result = run(cell["body"].clone());
        if status == 201 {
            result.unwrap_or_else(|p| {
                panic!("{id}: expected step 0 to pass, got {} {}", p.status, p.code)
            });
        } else {
            let want = cell["expected_code"].as_str().expect("code");
            let p = result.expect_err(id);
            assert_eq!((u64::from(p.status), p.code), (status, want), "{id}");
        }
    }
}

// ---- affects (cox W1-19): at most 256 items, no duplicates; count first, then uuid format (serde), then uniqueness ----

fn affects(n: usize) -> Value {
    let ids: Vec<String> = (0..n)
        .map(|i| format!("01930000-0000-7000-8000-{:012x}", i + 1))
        .collect();
    json!({"type":"decision","summary":"s","fields":{"why":"w"},"affects":ids})
}

#[test]
fn rec05_affects_is_bounded_at_256_and_unique() {
    accepted(json!({"type":"decision","summary":"s","fields":{"why":"w"},"affects":[]}));
    accepted(affects(1));
    accepted(affects(256));
    assert_eq!(refused(affects(257)), ("invalid_request", 422));
    // One duplicate fails, anywhere in the list.
    let mut dup = affects(3);
    dup["affects"][2] = dup["affects"][0].clone();
    assert_eq!(refused(dup), ("invalid_request", 422));
    // Count is checked first: 257 items that also hold a duplicate are the same refusal.
    let mut both = affects(257);
    both["affects"][1] = both["affects"][0].clone();
    assert_eq!(refused(both), ("invalid_request", 422));
    // A malformed id never decodes (serde, before the validator).
    assert!(decode(json!({"type":"note","summary":"s","affects":["not-a-uuid"]})).is_err());
}

// ---- valid_until (hale F-1): RFC 3339 date-time only; "infinity", "now", a date alone and the rest of Postgres' input forms are 422 ----

fn vu(value: Value) -> Value {
    json!({"type":"note","summary":"s","valid_until":value})
}

#[test]
fn rec06_valid_until_accepts_rfc3339_date_time_only() {
    for ok in [
        "2030-01-01T00:00:00Z",
        "2030-01-01T00:00:00.1Z",
        "2030-01-01T00:00:00.123456Z",
        "2030-01-01T00:00:00.123456789Z",
        "2030-01-01T12:34:56+05:30",
        "2030-02-28T23:59:59-08:00",
        "2028-02-29T00:00:00Z", // a leap day
        "9999-12-31T23:59:59Z",
        "1970-01-01T00:00:00Z",
        "2030-01-01T00:00:00+14:00",
        "2030-01-01T00:00:00-12:00",
    ] {
        accepted(vu(json!(ok)));
    }
    accepted(json!({"type":"note","summary":"s"})); // absent is fine
    accepted(vu(json!(null))); // null is the absent value (S3: string or null)
}

#[test]
fn rec06_valid_until_refuses_everything_else() {
    for bad in [
        "infinity",
        "-infinity",
        "epoch",
        "now",
        "today",
        "tomorrow",
        "allballs",
        "2030-01-01",
        "2030-01-01 00:00:00",
        "2030-01-01 00:00:00Z",
        "2030-01-01T00:00:00",
        "2030-01-01T00:00:00+0530",
        "2030-01-01T00:00:00+05",
        "2030-01-01T00:00:00+24:00",
        "2030-01-01T00:00:00+05:60",
        "2030-01-01T24:00:00Z",
        "2030-01-01T00:60:00Z",
        "2030-01-01T00:00:60Z", // a leap second: Postgres has none, so it is refused
        "2030-01-01T00:00:61Z",
        "2030-13-45T00:00:00Z",
        "2030-00-10T00:00:00Z",
        "2030-02-30T00:00:00Z",
        "2029-02-29T00:00:00Z", // not a leap year
        "0000-01-01T00:00:00Z",
        "0001-01-01T00:00:00Z", // before 1970 (cox W1-39)
        "1969-12-31T23:59:59Z",
        "2030-01-01T00:00:00+14:01", // the real offset range is -12:00 to +14:00
        "2030-01-01T00:00:00+15:00",
        "2030-01-01T00:00:00-12:01",
        "2030-01-01T00:00:00-13:00",
        "2030-01-01T00:00:00+23:59",
        "2030-01-01t00:00:00Z", // upper-case T and Z only
        "2030-01-01T00:00:00z",
        "2030-01-01T00:00:00.Z",
        "2030-01-01T00:00:00.1234567890Z", // ten fraction digits: more than a nanosecond
        "Nov 6 2030",
        "",
        " 2030-01-01T00:00:00Z",
        "2030-01-01T00:00:00Z ",
        "2030-01-01T00:00:00Z\n",
        "2030-01-01T00:00:00Z\u{0}",
        "20300101T000000Z",
    ] {
        assert_eq!(
            refused(vu(json!(bad))),
            ("invalid_request", 422),
            "valid_until {bad:?}"
        );
    }
    // Not a string: serde refuses it before the validator.
    assert!(decode(vu(json!(7))).is_err());
    assert!(decode(vu(json!(true))).is_err());
}

// ---- cox W1-39: ONE rule for every date-time input; the shared vectors (mvp/api/rfc3339-vectors.json, sha256 4125a4b4...) ----

#[test]
fn rec06_valid_until_passes_the_shared_rfc3339_vectors() {
    let raw = include_str!("data/rfc3339-vectors.4125a4b4.json");
    let v: Value = serde_json::from_str(raw).expect("the vectors parse");
    let list = |k: &str| -> Vec<String> {
        v[k].as_array()
            .unwrap_or_else(|| panic!("{k} is a list"))
            .iter()
            .map(|x| x.as_str().expect("a string").to_owned())
            .collect()
    };
    let (accept, reject) = (list("accept"), list("reject"));
    assert_eq!((accept.len(), reject.len()), (14, 48), "the pinned file");
    for ok in &accept {
        assert!(is_rfc3339(ok), "accept {ok:?}");
        accepted(vu(json!(ok)));
    }
    for bad in &reject {
        assert!(!is_rfc3339(bad), "reject {bad:?}");
        assert_eq!(
            refused(vu(json!(bad))),
            ("invalid_request", 422),
            "valid_until {bad:?}"
        );
    }
}

/// The second line (cox W1-39): a date-time the database itself refuses is still a 422, never a 500. 22009 is a time zone
/// displacement outside the range.
#[test]
fn rec06_database_date_time_errors_are_422_never_500() {
    for code in ["22007", "22008", "22009", "22P02", "23514"] {
        assert!(is_input_error_code(code), "{code} is the caller's input");
    }
    for code in ["23505", "40P01", "40001", "42501", "57014", "XX000"] {
        assert!(
            !is_input_error_code(code),
            "{code} is not the caller's input"
        );
    }
}
