//! BOOT-51, BOOT-52 (the `Unavailable` half) and BOOT-60 (`mvp/tests/mod-boot.md`). DB-free.
//! BOOT-50 (row counts) and BOOT-52's `Db` half are DB cells.
mod support;

use axum::body::to_bytes;
use axum::extract::FromRequestParts;
use axum::http::Request;
use cortex_module::{PortError, Problem, StrictQuery};
use mod_boot::seam::gather;
use mod_boot::{BootQuery, Format, Inputs, assemble, respond};
use support::*;

const UNAVAILABLE: [(&str, &str, u32); 7] = [
    ("followups", "Follow-ups", 6),
    ("inbox", "Inbox", 7),
    ("related", "Related context", 8),
    ("architecture", "Architecture", 9),
    ("skills", "Skills", 10),
    ("records", "Project records", 11),
    ("team", "Team", 12),
];

#[test]
fn boot_51_sections_6_to_12_say_capability_unavailable() {
    let b = assemble(&inputs_w1(), 12_288).expect("fits");
    let names: Vec<_> = b.boot.sections.iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        [
            "identity",
            "orientation",
            "rules",
            "claimed",
            "pending",
            "followups",
            "inbox",
            "related",
            "architecture",
            "skills",
            "records",
            "team"
        ]
    );
    for (k, (name, title, n)) in UNAVAILABLE.iter().enumerate() {
        assert_eq!(
            section_text(&b.markdown, *n),
            format!("## {n} {title}\ncapability_unavailable\n")
        );
        let s = &b.boot.sections[5 + k];
        assert_eq!(
            (s.name, s.status, s.used, s.omitted),
            (*name, "capability_unavailable", 0, 0)
        );
    }
    assert!(b.markdown.ends_with("## 12 Team\ncapability_unavailable\n"));
}

/// A port that answers `Unavailable` stops the boot: 503 `unavailable`, and nothing to render.
#[tokio::test]
async fn boot_52_an_unavailable_port_is_503_with_no_briefing() {
    let w = world();
    let mut r = Reads::for_project(&w, project_a());
    r.fail = Some(PortError::Unavailable);
    let e = gather(&mut r, &principal_w1())
        .await
        .expect_err("no partial briefing");
    assert_eq!(e, PortError::Unavailable);
    let p = Problem::from(e);
    assert_eq!((p.status, p.code), (503, "unavailable"));
    assert_eq!(r.calls.len(), 1, "the first failing read stops the boot");
}

async fn body_of(r: axum::response::Response) -> (u16, String, Vec<u8>) {
    let status = r.status().as_u16();
    let ct = r
        .headers()
        .get("content-type")
        .map(|v| v.to_str().expect("ascii").to_owned())
        .unwrap_or_default();
    let bytes = to_bytes(r.into_body(), 1 << 20).await.expect("a body");
    (status, ct, bytes.to_vec())
}

#[tokio::test]
async fn boot_60_both_formats_and_the_schema() {
    schema::self_test();
    let cases: Vec<(Inputs, u32)> = vec![
        (inputs_w1(), 12_288),
        (worst_case(), 4_096),
        (worst_case(), 12_288),
        (worst_case(), 262_144),
        (
            Inputs {
                orientation: None,
                rules_project: None,
                rules_worker: None,
                ..inputs_w1()
            },
            4_096,
        ),
    ];
    for (i, budget) in cases {
        let b = assemble(&i, budget).expect("fits");
        let (status, ct, body) = body_of(respond(&b, Format::Json)).await;
        assert_eq!((status, ct.as_str()), (200, "application/json"));
        let v: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
        let errs = schema::validate("Boot", &v);
        assert!(errs.is_empty(), "budget {budget}: {errs:#?}");
        assert_eq!(body, serde_json::to_vec(&b.boot).expect("json"));
        let (status, ct, body) = body_of(respond(&b, Format::Md)).await;
        assert_eq!((status, ct.as_str()), (200, "text/markdown; charset=utf-8"));
        assert_eq!(body, b.markdown.as_bytes());
    }
    let (mut parts, ()) = Request::builder()
        .uri("/v2/boot?format=xml")
        .body(())
        .expect("a request")
        .into_parts();
    let e = StrictQuery::<BootQuery>::from_request_parts(&mut parts, &())
        .await
        .expect_err("xml is not an enum value");
    assert_eq!((e.status, e.code), (422, "invalid_request"));
    let a = mod_boot::args::check(&BootQuery::default()).expect("default");
    assert_eq!(a.format, Format::Md, "Markdown is the default");
}

/// The schema checker itself catches a wrong document (so BOOT-60's pass means something).
#[test]
fn boot_60_the_schema_checker_refuses_a_wrong_boot() {
    let b = assemble(&inputs_w1(), 12_288).expect("fits");
    let mut v = serde_json::to_value(&b.boot).expect("json");
    v["persona"] = serde_json::json!(null);
    v["identity"]["job_roles"] = serde_json::json!(["QA"]);
    v["rules"][0]["sha256"] = serde_json::json!("abc");
    v["sections"][0]["status"] = serde_json::json!("fine");
    v.as_object_mut().expect("object").remove("used");
    let errs = schema::validate("Boot", &v);
    for want in [
        "unknown property persona",
        "does not match",
        "not in",
        "missing required used",
    ] {
        assert!(errs.iter().any(|e| e.contains(want)), "{want}: {errs:#?}");
    }
    // r2.6 (cox W1-20): reading_path is required, 5 to 10 entries, each starting with "/".
    let mut v = serde_json::to_value(&b.boot).expect("json");
    v["orientation"]["reading_path"] = serde_json::json!(["/a", "/b", "relative", "/d"]);
    let errs = schema::validate("Boot", &v);
    for want in ["fewer than 5 items", "\"relative\" does not match"] {
        assert!(errs.iter().any(|e| e.contains(want)), "{want}: {errs:#?}");
    }
    v["orientation"]
        .as_object_mut()
        .expect("object")
        .remove("reading_path");
    let errs = schema::validate("Boot", &v);
    assert!(
        errs.iter()
            .any(|e| e.contains("missing required reading_path")),
        "{errs:#?}"
    );
}
