//! Wave 1, mod-handoffs P1 (row 2a86598d): the step-0 validators and the decoding of the request bodies.
//! DB-free. Test definitions: `mvp/tests/mod-handoffs.md` r3.3, HAND-02 (create bounds), HAND-04 and
//! HAND-104 (the addressee grammar), HAND-28, HAND-41 to HAND-44 (return), HAND-53 (rework notes),
//! HAND-62 and HAND-71 (reasons), HAND-120 (the order inside step 0), HAND-121 (c) and HAND-06 (decoding).
//!
//! Step 0 is S3's R-16 stateless group: byte limits, NUL, patterns, enums, the fields' own codes, then the
//! cross-field rules (section 17 of `S3-r2-CHANGES.md`: per-field first, schema order inside a group).
//! RED against the stub: the validators are `todo!()`. The decoding tests (the last section) pass on
//! arrival because the body types already exist; they are listed in the commit message.
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use cortex_module::Problem;
use cortex_types::Sha256 as Digest;
use mod_handoffs::body::{
    CancelBody, CreateBody, EvidenceKind, Kind, ReleaseBody, ReturnBody, ReviewBody,
};
use mod_handoffs::validate::{
    CreateOk, ReturnOk, To, is_job_role_label, validate_accept, validate_cancel, validate_create,
    validate_release, validate_return, validate_rework,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

type Pair = (&'static str, u16);

const INVALID: Pair = ("invalid_request", 422);
const PACKET: Pair = ("packet_incomplete", 422);

fn pair(p: &Problem) -> Pair {
    (p.code, p.status)
}

fn create_body(v: Value) -> CreateBody {
    serde_json::from_value(v).expect("a body that decodes")
}

fn return_body(v: Value) -> ReturnBody {
    serde_json::from_value(v).expect("a body that decodes")
}

fn refused<T: std::fmt::Debug>(r: Result<T, Problem>) -> Pair {
    pair(&r.expect_err("the validator refuses"))
}

fn base() -> Value {
    json!({
        "kind": "task",
        "to": { "kind": "job_role", "ref": "dev" },
        "summary": "s",
        "next_action": "n",
        "acceptance": "a"
    })
}

fn with(mut v: Value, k: &str, x: Value) -> Value {
    v[k] = x;
    v
}

fn without(mut v: Value, k: &str) -> Value {
    v.as_object_mut().expect("an object").remove(k);
    v
}

fn bytes(n: usize) -> String {
    "x".repeat(n)
}

fn hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn inline_entry(data: &[u8]) -> Value {
    json!({ "kind": "output", "inline_b64": B64.encode(data), "sha256": hex(data) })
}

fn ref_entry(r: &str) -> Value {
    json!({ "kind": "link", "ref": r, "sha256": hex(r.as_bytes()) })
}

fn ret(evidence: Value) -> Value {
    json!({ "summary": "done", "evidence": evidence, "risks": "none" })
}

// ---- create: the packet (HAND-01, HAND-02) ----

#[test]
fn create_accepts_a_complete_packet_and_defaults_the_priority() {
    let ok: CreateOk = validate_create(create_body(base())).expect("a complete packet");
    assert_eq!(ok.kind, Kind::Task);
    assert_eq!(ok.to, To::JobRole("dev".to_owned()));
    assert_eq!(
        (
            ok.summary.as_str(),
            ok.next_action.as_str(),
            ok.acceptance.as_str()
        ),
        ("s", "n", "a")
    );
    assert_eq!(ok.priority, 50, "S3: the default priority is 50");
    assert!(ok.verify.is_none() && ok.context.is_none() && ok.files.is_empty());
}

#[test]
fn create_keeps_the_optional_fields_exactly_as_sent() {
    let v = with(
        with(
            with(
                with(base(), "verify", json!("run the suite")),
                "context",
                json!("c"),
            ),
            "files",
            json!(["a.rs", "b.rs"]),
        ),
        "priority",
        json!(90),
    );
    let ok = validate_create(create_body(v)).expect("ok");
    assert_eq!(ok.verify.as_deref(), Some("run the suite"));
    assert_eq!(ok.context.as_deref(), Some("c"));
    assert_eq!(ok.files, ["a.rs", "b.rs"]);
    assert_eq!(ok.priority, 90);
}

/// HAND-02: every text field is accepted at its limit in BYTES and refused one byte over.
#[test]
fn create_text_limits_are_in_bytes() {
    for (field, max) in [
        ("summary", 2048),
        ("next_action", 8192),
        ("acceptance", 8192),
        ("verify", 8192),
        ("context", 8192),
    ] {
        validate_create(create_body(with(base(), field, json!(bytes(max)))))
            .unwrap_or_else(|p| panic!("{field} at {max} bytes: {}", p.code));
        assert_eq!(
            refused(validate_create(create_body(with(
                base(),
                field,
                json!(bytes(max + 1))
            )))),
            INVALID,
            "{field} at {} bytes",
            max + 1
        );
    }
    // 1,025 two-byte characters are 2,050 bytes: refused although only 1,025 characters.
    let two_byte = "é".repeat(1025);
    assert_eq!(
        refused(validate_create(create_body(with(
            base(),
            "summary",
            json!(two_byte)
        )))),
        INVALID
    );
}

/// HAND-02, R-6: an absent, empty or whitespace-only handler-required field is `packet_incomplete`.
#[test]
fn create_required_text_is_packet_incomplete_when_absent_empty_or_blank() {
    for field in ["summary", "next_action", "acceptance"] {
        assert_eq!(
            refused(validate_create(create_body(without(base(), field)))),
            PACKET,
            "{field} absent"
        );
        for blank in ["", "   ", "\t\n "] {
            assert_eq!(
                refused(validate_create(create_body(with(
                    base(),
                    field,
                    json!(blank)
                )))),
                PACKET,
                "{field} = {blank:?}"
            );
        }
    }
}

/// HAND-02: a NUL in any text is 422, never a database error.
#[test]
fn create_refuses_a_nul_in_any_text() {
    for field in ["summary", "next_action", "acceptance", "verify", "context"] {
        assert_eq!(
            refused(validate_create(create_body(with(
                base(),
                field,
                json!("a\u{0}b")
            )))),
            INVALID,
            "NUL in {field}"
        );
    }
    assert_eq!(
        refused(validate_create(create_body(with(
            base(),
            "files",
            json!(["ok", "a\u{0}b"])
        )))),
        INVALID
    );
}

#[test]
fn create_priority_is_1_to_100() {
    for p in [1, 50, 100] {
        assert_eq!(
            validate_create(create_body(with(base(), "priority", json!(p))))
                .expect("ok")
                .priority,
            p
        );
    }
    for p in [0, 101, -1, 100_000] {
        assert_eq!(
            refused(validate_create(create_body(with(
                base(),
                "priority",
                json!(p)
            )))),
            INVALID,
            "{p}"
        );
    }
}

/// HAND-02: `files` + `blockers` + `decisions` together are at most 65,536 bytes, counted as S2's table
/// CHECK counts them: the `jsonb` text of each non-empty array (`["x"]` is the element plus 4 bytes).
#[test]
fn create_aggregate_limit_counts_the_jsonb_text() {
    let one = |n: usize| json!([bytes(n)]);
    validate_create(create_body(with(base(), "files", one(65_532))))
        .expect("65,536 bytes of jsonb text");
    assert_eq!(
        refused(validate_create(create_body(with(
            base(),
            "files",
            one(65_533)
        )))),
        INVALID
    );
}

#[test]
fn create_aggregate_limit_spans_the_three_arrays() {
    let one = |n: usize| json!([bytes(n)]);
    let spread = |n: usize| {
        with(
            with(with(base(), "files", one(n)), "blockers", one(n)),
            "decisions",
            one(n),
        )
    };
    // Each array is n + 4 bytes of jsonb text; three of them are 3n + 12.
    validate_create(create_body(spread(21_841))).expect("3 x 21,841 + 12 = 65,535 bytes");
    assert_eq!(
        refused(validate_create(create_body(spread(21_842)))),
        INVALID,
        "3 x 21,842 + 12 = 65,538"
    );
}

#[test]
fn create_due_at_must_be_an_rfc3339_date_time() {
    for ok in ["2026-11-06T12:00:00Z", "2026-11-06T12:00:00.123456+01:00"] {
        validate_create(create_body(with(base(), "due_at", json!(ok))))
            .unwrap_or_else(|p| panic!("{ok}: {}", p.code));
    }
    for bad in [
        "tomorrow",
        "2026-13-01T00:00:00Z",
        "2026-11-06",
        "2026-11-06T25:00:00Z",
        "",
    ] {
        assert_eq!(
            refused(validate_create(create_body(with(
                base(),
                "due_at",
                json!(bad)
            )))),
            INVALID,
            "{bad:?}"
        );
    }
}

// ---- create: the addressee (HAND-04, HAND-28, HAND-104) ----

/// HAND-104, HAND-28: a job-role ref outside `^[a-z][a-z0-9-]{0,31}$` is 422 `invalid_request` at step 0.
#[test]
fn job_role_label_grammar_is_anchored_on_the_whole_string() {
    for ok in ["dev", "qa", "a", "a-b", "x1", &"a".repeat(32)] {
        assert!(is_job_role_label(ok), "{ok:?} fits");
    }
    for bad in [
        "",
        "Dev",
        "QA",
        "1qa",
        "a b",
        "-a",
        "a_b",
        "dev\n",
        "dev ",
        &"a".repeat(33),
        "dév",
        "\u{212a}",
    ] {
        assert!(!is_job_role_label(bad), "{bad:?} does not fit");
    }
}

#[test]
fn create_job_role_refs_follow_the_grammar() {
    let to = |r: &str| with(base(), "to", json!({ "kind": "job_role", "ref": r }));
    assert_eq!(
        validate_create(create_body(to("dev"))).expect("ok").to,
        To::JobRole("dev".to_owned())
    );
    validate_create(create_body(to(&"a".repeat(32)))).expect("32 characters");
    for bad in [
        "Dev",
        "1qa",
        "a b",
        "dev\n",
        "",
        &"a".repeat(33),
        "\u{212a}",
    ] {
        assert_eq!(
            refused(validate_create(create_body(to(bad)))),
            INVALID,
            "{bad:?}"
        );
    }
}

/// HAND-04: a worker ref is a UUID; a malformed one is 422 `invalid_request` at step 0 (R-5: a malformed
/// id in the BODY fails decoding), which tells a caller nothing about which workers exist.
#[test]
fn create_worker_refs_are_uuids() {
    let id = "01930000-0000-7000-8000-0000000000aa";
    let to = |r: &str| with(base(), "to", json!({ "kind": "worker", "ref": r }));
    match validate_create(create_body(to(id))).expect("a UUID").to {
        To::Worker(w) => assert_eq!(w.to_string(), id),
        other => panic!("a worker address, got {other:?}"),
    }
    for bad in [
        "abc",
        "",
        "01930000-0000-7000-8000-0000000000aa\n",
        "not-a-uuid-at-all-not-a-uuid-at-all-xx",
    ] {
        assert_eq!(
            refused(validate_create(create_body(to(bad)))),
            INVALID,
            "{bad:?}"
        );
    }
}

// ---- create: the order inside step 0 (HAND-120) ----

#[test]
fn create_step_0_order_is_per_field_then_cross_field_in_schema_order() {
    // (a) summary absent AND the aggregate over the limit: the field's own check first.
    let aggregate = with(without(base(), "summary"), "files", json!([bytes(65_533)]));
    assert_eq!(refused(validate_create(create_body(aggregate))), PACKET);
    // (b) summary over its limit AND next_action absent: summary comes first in schema order.
    let b = with(
        without(base(), "next_action"),
        "summary",
        json!(bytes(2049)),
    );
    assert_eq!(refused(validate_create(create_body(b))), INVALID);
    // (c) summary absent AND next_action over its limit.
    let c = with(
        without(base(), "summary"),
        "next_action",
        json!(bytes(8193)),
    );
    assert_eq!(refused(validate_create(create_body(c))), PACKET);
    // (d) acceptance absent AND priority 101.
    let d = with(without(base(), "acceptance"), "priority", json!(101));
    assert_eq!(refused(validate_create(create_body(d))), PACKET);
    // And the reverse of (d): priority is checked after acceptance, so a bad priority alone is invalid_request.
    assert_eq!(
        refused(validate_create(create_body(with(
            base(),
            "priority",
            json!(101)
        )))),
        INVALID
    );
}

// ---- return (HAND-40 to HAND-44) ----

#[test]
fn return_accepts_inline_and_reference_evidence() {
    let data = b"hello evidence";
    let v = ret(json!([
        inline_entry(data),
        ref_entry("https://example.test/run/1")
    ]));
    let ok: ReturnOk = validate_return(return_body(v)).expect("two good entries");
    assert_eq!(ok.risks, "none");
    assert_eq!(ok.evidence.len(), 2);
    let first = &ok.evidence[0];
    assert_eq!(first.kind, EvidenceKind::Output);
    assert_eq!(first.inline.as_deref(), Some(&data[..]));
    assert_eq!(first.size, data.len());
    assert_eq!(
        first.sha256.to_hex(),
        hex(data),
        "the digest is the one computed over the bytes"
    );
    let second = &ok.evidence[1];
    assert_eq!(
        (second.reference.as_deref(), second.size),
        (Some("https://example.test/run/1"), 0)
    );
    assert!(second.inline.is_none());
    assert_eq!(
        second.sha256.to_hex(),
        hex(b"https://example.test/run/1"),
        "a reference keeps the digest it was sent"
    );
}

/// HAND-41, R-6: absent or empty evidence is `evidence_required`.
#[test]
fn return_needs_at_least_one_evidence_entry() {
    let absent = json!({ "summary": "done", "risks": "none" });
    assert_eq!(
        refused(validate_return(return_body(absent))),
        ("evidence_required", 422)
    );
    assert_eq!(
        refused(validate_return(return_body(ret(json!([]))))),
        ("evidence_required", 422)
    );
}

/// HAND-42, R-6: risks absent, empty or whitespace-only is `risks_required`; `none` and `None` are text.
#[test]
fn return_needs_risks_and_accepts_none() {
    let ev = json!([ref_entry("r")]);
    let no_risks = json!({ "summary": "done", "evidence": ev });
    assert_eq!(
        refused(validate_return(return_body(no_risks))),
        ("risks_required", 422)
    );
    for blank in ["", "  ", "\n\t"] {
        let v = json!({ "summary": "done", "evidence": ev, "risks": blank });
        assert_eq!(
            refused(validate_return(return_body(v))),
            ("risks_required", 422),
            "{blank:?}"
        );
    }
    for ok in ["none", "None", &bytes(8192)] {
        let v = json!({ "summary": "done", "evidence": ev, "risks": ok });
        assert_eq!(validate_return(return_body(v)).expect("ok").risks, ok);
    }
    let v = json!({ "summary": "done", "evidence": ev, "risks": bytes(8193) });
    assert_eq!(refused(validate_return(return_body(v))), INVALID);
}

/// HAND-41, section 17: the order of the evidence codes.
#[test]
fn return_evidence_source_rules() {
    let h = hex(b"x");
    let entry = |extra: Value| {
        let mut e = json!({ "kind": "link", "sha256": h });
        for (k, v) in extra.as_object().expect("an object") {
            e[k] = v.clone();
        }
        e
    };
    // Neither ref nor inline_b64: evidence_source_invalid.
    assert_eq!(
        refused(validate_return(return_body(ret(json!([entry(json!({}))]))))),
        ("evidence_source_invalid", 422)
    );
    // Both, each valid: evidence_source_invalid.
    let both = entry(json!({ "ref": "r", "inline_b64": B64.encode(b"x") }));
    assert_eq!(
        refused(validate_return(return_body(ret(json!([both]))))),
        ("evidence_source_invalid", 422)
    );
    // ref "" alone: the per-field bound on ref (minLength 1), invalid_request.
    assert_eq!(
        refused(validate_return(return_body(ret(json!([entry(
            json!({ "ref": "" })
        )]))))),
        INVALID
    );
    // ref "" PLUS a valid inline_b64: the per-field check fires first, NOT evidence_source_invalid.
    let empty_and_inline = entry(json!({ "ref": "", "inline_b64": B64.encode(b"x") }));
    assert_eq!(
        refused(validate_return(return_body(ret(json!([empty_and_inline]))))),
        INVALID
    );
    // Across entries: a per-field fault in entry 1 beats a cross-field fault in entry 0.
    let two = json!([both, entry(json!({ "ref": "ok", "sha256": "ABC" }))]);
    assert_eq!(refused(validate_return(return_body(ret(two)))), INVALID);
}

/// HAND-41, HAND-43: the digest and the encoding.
#[test]
fn return_checks_the_digest_and_the_encoding() {
    let data = b"abc";
    let good = inline_entry(data);
    validate_return(return_body(ret(json!([good.clone()])))).expect("a matching digest");
    // A digest that does not match the decoded bytes.
    let mut wrong = good.clone();
    wrong["sha256"] = json!(hex(b"other"));
    assert_eq!(
        refused(validate_return(return_body(ret(json!([wrong]))))),
        ("evidence_hash_mismatch", 422)
    );
    // The digest must be 64 lowercase hex characters.
    for bad in [
        hex(data).to_uppercase(),
        hex(data)[..63].to_owned(),
        "g".repeat(64),
    ] {
        let mut e = good.clone();
        e["sha256"] = json!(bad);
        assert_eq!(
            refused(validate_return(return_body(ret(json!([e]))))),
            INVALID,
            "{bad}"
        );
    }
    // Invalid base64.
    let mut e = good.clone();
    e["inline_b64"] = json!("not base64 !!");
    assert_eq!(
        refused(validate_return(return_body(ret(json!([e]))))),
        INVALID
    );
    // A reference entry's digest is stored as sent, whatever it is (there are no bytes to check).
    let r = json!({ "kind": "artifact", "ref": "s3://b/k", "sha256": "0".repeat(64) });
    let ok = validate_return(return_body(ret(json!([r])))).expect("a reference with a digest");
    assert_eq!(ok.evidence[0].sha256.to_hex(), "0".repeat(64));
}

/// HAND-44: the size bounds.
#[test]
fn return_size_bounds() {
    let ev = json!([ref_entry("r")]);
    let with_summary = |n: usize| json!({ "summary": bytes(n), "evidence": ev, "risks": "none" });
    validate_return(return_body(with_summary(32_768))).expect("32,768 bytes");
    assert_eq!(
        refused(validate_return(return_body(with_summary(32_769)))),
        INVALID
    );
    assert_eq!(
        refused(validate_return(return_body(with_summary(0)))),
        INVALID,
        "summary has minLength 1"
    );
    // ref: 2,048 bytes ok, 2,049 not.
    validate_return(return_body(ret(json!([ref_entry(&bytes(2048))])))).expect("2,048 bytes");
    assert_eq!(
        refused(validate_return(return_body(ret(json!([ref_entry(
            &bytes(2049)
        )]))))),
        INVALID
    );
    // Inline: 1 MiB decoded per entry.
    let mib = 1024 * 1024;
    validate_return(return_body(ret(json!([inline_entry(&vec![7u8; mib])])))).expect("1 MiB");
    assert_eq!(
        refused(validate_return(return_body(ret(json!([inline_entry(
            &vec![7u8; mib + 1]
        )]))))),
        INVALID
    );
    // 1 MiB in total over many small entries.
    let chunk = vec![1u8; mib / 64];
    let many: Vec<Value> = (0..64).map(|_| inline_entry(&chunk)).collect();
    validate_return(return_body(ret(json!(many)))).expect("64 entries, 1 MiB in all");
    let mut over: Vec<Value> = (0..63).map(|_| inline_entry(&chunk)).collect();
    over.push(inline_entry(&vec![1u8; mib / 64 + 1]));
    assert_eq!(
        refused(validate_return(return_body(ret(json!(over))))),
        INVALID,
        "1 MiB + 1 in all"
    );
    // 64 entries are the most.
    let sixty_five: Vec<Value> = (0..65).map(|_| ref_entry("r")).collect();
    assert_eq!(
        refused(validate_return(return_body(ret(json!(sixty_five))))),
        INVALID
    );
    // NUL.
    for bad in [
        json!({ "summary": "a\u{0}b", "evidence": ev, "risks": "none" }),
        json!({ "summary": "s", "evidence": ev, "risks": "a\u{0}b" }),
        json!({ "summary": "s", "evidence": [ref_entry("a\u{0}b")], "risks": "none" }),
    ] {
        assert_eq!(refused(validate_return(return_body(bad))), INVALID);
    }
}

/// Section 17 / R-16 on the return body: the first refusal in schema order is the one reported.
#[test]
fn return_reports_the_first_refusal_in_schema_order() {
    // summary (schema order: summary, evidence, risks, tests): an over-long summary beats a missing evidence.
    let v = json!({ "summary": bytes(32_769), "risks": "none" });
    assert_eq!(refused(validate_return(return_body(v))), INVALID);
    // evidence before risks: both missing gives evidence_required.
    let v = json!({ "summary": "s" });
    assert_eq!(
        refused(validate_return(return_body(v))),
        ("evidence_required", 422)
    );
}

// ---- reasons and notes (HAND-53, HAND-62, HAND-71) ----

/// HAND-71: cancel's reason is required, 1 to 512 bytes; it is checked before anything is read.
#[test]
fn cancel_reason_is_required_and_bounded_in_bytes() {
    let cancel = |r: Value| CancelBody {
        reason: r.as_str().map(str::to_owned),
    };
    for missing in [Value::Null, json!(""), json!("   "), json!("\n")] {
        assert_eq!(
            refused(validate_cancel(cancel(missing))),
            ("reason_required", 422)
        );
    }
    assert_eq!(validate_cancel(cancel(json!("x"))).expect("1 byte"), "x");
    assert_eq!(
        validate_cancel(cancel(json!(bytes(512))))
            .expect("512 bytes")
            .len(),
        512
    );
    assert_eq!(refused(validate_cancel(cancel(json!(bytes(513))))), INVALID);
    assert_eq!(
        refused(validate_cancel(cancel(json!("é".repeat(300))))),
        INVALID,
        "600 bytes of 300 characters"
    );
    assert_eq!(refused(validate_cancel(cancel(json!("a\u{0}b")))), INVALID);
}

/// HAND-62: release's reason is optional for the holder; its bounds hold when present. A blank one counts as none.
#[test]
fn release_reason_is_optional_and_bounded() {
    let rel = |r: Option<&str>| ReleaseBody {
        reason: r.map(str::to_owned),
    };
    assert_eq!(validate_release(rel(None)).expect("none"), None);
    assert_eq!(
        validate_release(rel(Some("   "))).expect("blank is none"),
        None
    );
    assert_eq!(
        validate_release(rel(Some("moving on"))).expect("text"),
        Some("moving on".to_owned())
    );
    assert_eq!(
        validate_release(rel(Some(&bytes(512))))
            .expect("512")
            .map(|s| s.len()),
        Some(512)
    );
    assert_eq!(refused(validate_release(rel(Some(&bytes(513))))), INVALID);
    assert_eq!(refused(validate_release(rel(Some("a\u{0}b")))), INVALID);
}

/// HAND-53: accept's notes are optional; rework's are required (`notes_required`); both at most 8,192 bytes.
#[test]
fn review_notes() {
    let notes = |n: Option<&str>| ReviewBody {
        notes: n.map(str::to_owned),
    };
    assert_eq!(validate_accept(notes(None)).expect("none"), None);
    assert_eq!(
        validate_accept(notes(Some(" "))).expect("blank is none"),
        None
    );
    assert_eq!(
        validate_accept(notes(Some("well done"))).expect("text"),
        Some("well done".to_owned())
    );
    assert_eq!(refused(validate_accept(notes(Some(&bytes(8193))))), INVALID);
    validate_accept(notes(Some(&bytes(8192)))).expect("8,192");

    for missing in [None, Some(""), Some("  "), Some("\n\t")] {
        assert_eq!(
            refused(validate_rework(notes(missing))),
            ("notes_required", 422),
            "{missing:?}"
        );
    }
    assert_eq!(
        validate_rework(notes(Some("change the test"))).expect("text"),
        "change the test"
    );
    validate_rework(notes(Some(&bytes(8192)))).expect("8,192");
    assert_eq!(refused(validate_rework(notes(Some(&bytes(8193))))), INVALID);
    assert_eq!(refused(validate_rework(notes(Some("a\u{0}b")))), INVALID);
}

// ---- decoding (HAND-06, HAND-121 (c), HAND-03): types only, GREEN on arrival ----

fn decodes_create(v: Value) -> bool {
    serde_json::from_value::<CreateBody>(v).is_ok()
}

/// HAND-121 (c): an absent or unknown `kind` fails decoding; it never defaults to a consult.
#[test]
fn decoding_kind_never_defaults_to_consult() {
    assert!(decodes_create(base()));
    assert!(decodes_create(with(base(), "kind", json!("consult"))));
    assert!(!decodes_create(without(base(), "kind")), "absent");
    for bad in [
        json!("Consult"),
        json!("consult "),
        json!("note"),
        json!(null),
        json!(["consult"]),
        json!(1),
    ] {
        assert!(!decodes_create(with(base(), "kind", bad.clone())), "{bad}");
    }
}

/// HAND-121 (a) and (b): the addressee's own `kind` is a different field and decodes on its own.
#[test]
fn decoding_the_addressee_kind_is_separate_from_the_handoff_kind() {
    for to_kind in ["worker", "job_role"] {
        let v = with(
            with(base(), "kind", json!("consult")),
            "to",
            json!({ "kind": to_kind, "ref": "x" }),
        );
        let b = create_body(v);
        assert_eq!(
            b.kind,
            Kind::Consult,
            "to.kind = {to_kind} does not change the handoff kind"
        );
    }
}

/// HAND-06: `team` and `human` addressees fail decoding in 0.2.001.
#[test]
fn decoding_refuses_team_and_human_addressees() {
    for k in ["team", "human", "Worker", "role"] {
        assert!(
            !decodes_create(with(base(), "to", json!({ "kind": k, "ref": "x" }))),
            "{k}"
        );
    }
}

/// HAND-03: a body field the schema does not know fails decoding, whichever route.
#[test]
fn decoding_refuses_unknown_fields() {
    for f in [
        "created_by",
        "holder",
        "state",
        "round",
        "project_id",
        "id",
        "incarnation",
    ] {
        assert!(!decodes_create(with(base(), f, json!("x"))), "{f}");
    }
    assert!(
        serde_json::from_value::<ReleaseBody>(json!({ "note": "x" })).is_err(),
        "release has no `note`"
    );
    assert!(serde_json::from_value::<ReleaseBody>(json!({})).is_ok());
    assert!(serde_json::from_value::<CancelBody>(json!({ "reason": "r", "x": 1 })).is_err());
    assert!(serde_json::from_value::<ReviewBody>(json!({ "notes": "n", "x": 1 })).is_err());
    assert!(
        serde_json::from_value::<ReturnBody>(json!({ "summary": "s", "incarnation": 1 })).is_err()
    );
    assert!(
        serde_json::from_value::<ReturnBody>(json!({ "risks": "none" })).is_err(),
        "summary is schema-required"
    );
    assert!(
        serde_json::from_value::<ReturnBody>(ret(
            json!([{ "kind": "output", "inline_b64": "AA==" }])
        ))
        .is_err(),
        "an evidence entry needs its sha256"
    );
    assert!(
        serde_json::from_value::<ReturnBody>(ret(
            json!([{ "kind": "video", "ref": "r", "sha256": "0".repeat(64) }])
        ))
        .is_err(),
        "an unknown evidence kind"
    );
}

#[test]
fn the_digest_type_is_the_workspace_one() {
    let d: Digest = Digest(Sha256::digest(b"x").into());
    assert_eq!(d.to_hex(), hex(b"x"));
}
