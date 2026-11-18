//! PR-01 to PR-08 and TF-10 (the client side): Problem decoding, answers that are not a Problem, redaction of the token from
//! anything the server says, the response cap. Against the std-only fake. DB-free.
mod support;

use cortex_client::{ClientError, ProblemKind};
use std::time::{Duration, Instant};
use support::fake::{Fake, Reply};
use support::{TOKEN, client, client_with, get};

const RID: &str = "0198d2c0-7000-7000-8000-000000000001";

fn problem_of(e: ClientError) -> cortex_client::Problem {
    match e {
        ClientError::Problem(p) => *p,
        other => panic!("expected a Problem, got {other:?}"),
    }
}

#[test]
fn pr_01_a_problem_decodes_and_displays() {
    let fake = Fake::start();
    fake.push(Reply::problem_with(
        404,
        "not_found",
        "Not found",
        Some("no such thing"),
        "Check the id.",
    ));
    let (c, _) = client(&fake);
    let e = c.send(&get("/v2/records/x")).unwrap_err();
    let text = e.to_string();
    let p = problem_of(e);
    assert_eq!(p.status, 404);
    assert_eq!(p.body_status, None);
    assert_eq!(p.code, "not_found");
    assert_eq!(p.kind(), ProblemKind::NotFound);
    assert_eq!(p.title, "Not found");
    assert_eq!(p.detail.as_deref(), Some("no such thing"));
    assert_eq!(p.fix, "Check the id.");
    assert_eq!(p.request_id, RID);
    assert_eq!(p.scope, None);
    assert_eq!(
        text,
        format!(
            "Not found (not_found, HTTP 404): no such thing Fix: Check the id. [request {RID}]"
        )
    );
}

#[test]
fn pr_02_optional_members_unknown_members_and_a_body_that_is_not_a_problem() {
    let fake = Fake::start();
    let ctype = [("Content-Type", "application/problem+json")];
    // detail absent, an unknown scope value and an unknown extra member are tolerated
    fake.push(Reply::http(
        503,
        &ctype,
        format!(r#"{{"type":"unavailable","title":"T","status":503,"fix":"F","request_id":"{RID}","scope":"weird","extra":1}}"#).as_bytes(),
    ));
    // `fix` missing: not a Problem
    fake.push(Reply::http(
        503,
        &ctype,
        format!(r#"{{"type":"unavailable","title":"T","status":503,"request_id":"{RID}"}}"#)
            .as_bytes(),
    ));
    let (c, _) = client(&fake);
    let p = problem_of(c.send(&get("/v2/whoami")).unwrap_err());
    assert_eq!(p.detail, None);
    assert_eq!(p.scope.as_deref(), Some("weird"));
    assert_eq!(p.kind(), ProblemKind::Unavailable);
    assert!(matches!(
        c.send(&get("/v2/whoami")).unwrap_err(),
        ClientError::Http { status: 503, .. }
    ));
}

#[test]
fn pr_03_the_http_status_wins_over_the_status_member() {
    let fake = Fake::start();
    fake.push(Reply::http(
        403,
        &[("Content-Type", "application/problem+json")],
        format!(
            r#"{{"type":"missing_scope","title":"T","status":404,"fix":"F","request_id":"{RID}"}}"#
        )
        .as_bytes(),
    ));
    let (c, _) = client(&fake);
    let p = problem_of(c.send(&get("/v2/whoami")).unwrap_err());
    assert_eq!((p.status, p.body_status), (403, Some(404)));
    assert!(p.to_string_for_test().contains("HTTP 403"));
}

trait ShowProblem {
    fn to_string_for_test(&self) -> String;
}
impl ShowProblem for cortex_client::Problem {
    fn to_string_for_test(&self) -> String {
        ClientError::Problem(Box::new(self.clone())).to_string()
    }
}

#[test]
fn pr_04_answers_that_are_not_a_problem_become_http_with_a_safe_snippet() {
    let long = "a".repeat(1000);
    let table: [(&str, Option<&str>, &str); 7] = [
        (
            "an HTML page",
            Some("text/html"),
            "<html>bad gateway</html>",
        ),
        ("an empty body", None, ""),
        ("plain text", Some("text/plain"), "oops"),
        ("JSON without type", Some("application/problem+json"), "{}"),
        (
            "truncated JSON",
            Some("application/problem+json"),
            r#"{"type":"x","ti"#,
        ),
        (
            "a Problem under another content type",
            Some("application/json"),
            r#"{"type":"x","title":"t","status":500,"fix":"f","request_id":"r"}"#,
        ),
        (
            "control characters",
            Some("text/plain"),
            "\x1b[31mRED\x07\r\nline2\0end",
        ),
    ];
    for (what, ctype, body) in table {
        let fake = Fake::start();
        let headers: Vec<(&str, &str)> =
            ctype.map(|c| vec![("Content-Type", c)]).unwrap_or_default();
        fake.push(Reply::http(500, &headers, body.as_bytes()));
        let (c, _) = client(&fake);
        let e = c.send(&get("/v2/whoami")).unwrap_err();
        let ClientError::Http {
            status,
            snippet,
            request_id,
        } = &e
        else {
            panic!("{what}: expected Http, got {e:?}")
        };
        assert_eq!(*status, 500, "{what}");
        assert_eq!(*request_id, None, "{what}");
        assert!(snippet.len() <= 200, "{what}: {} bytes", snippet.len());
        assert!(
            snippet.chars().all(|ch| !ch.is_control()),
            "{what}: {snippet:?}"
        );
        assert!(snippet.is_ascii(), "{what}: {snippet:?}");
    }
    let fake = Fake::start();
    fake.push(Reply::http(
        502,
        &[
            ("Content-Type", "text/plain"),
            ("Cortex-Request-Id", "req-7"),
        ],
        long.as_bytes(),
    ));
    let (c, _) = client(&fake);
    let ClientError::Http {
        snippet,
        request_id,
        ..
    } = c.send(&get("/v2/whoami")).unwrap_err()
    else {
        panic!()
    };
    assert_eq!(snippet.len(), 200);
    assert_eq!(request_id.as_deref(), Some("req-7"));
}

fn windows_of(token: &str) -> Vec<&str> {
    (0..=token.len() - 8).map(|i| &token[i..i + 8]).collect()
}

fn assert_token_free(label: &str, text: &str) {
    assert!(!text.contains(TOKEN), "{label} holds the token: {text}");
    for w in windows_of(TOKEN) {
        assert!(
            !text.contains(w),
            "{label} holds the 8-character piece {w:?} of the token: {text}"
        );
    }
}

#[test]
fn pr_05_a_token_echoed_by_the_server_is_redacted_everywhere() {
    let piece = &TOKEN[3..17];
    let fake = Fake::start();
    fake.push(Reply::problem_with(
        400,
        "invalid_request",
        &format!("Bad {piece} request"),
        Some(&format!("you sent Authorization: Bearer {TOKEN}")),
        &format!("Fix {TOKEN}"),
    ));
    fake.push(Reply::http(
        500,
        &[("Content-Type", "text/html")],
        format!("<p>Bearer {TOKEN} rejected</p>").as_bytes(),
    ));
    let (c, _) = client(&fake);
    for _ in 0..2 {
        let e = c.send(&get("/v2/whoami")).unwrap_err();
        for (label, text) in [
            ("Display", e.to_string()),
            ("Debug", format!("{e:?}")),
            ("pretty Debug", format!("{e:#?}")),
        ] {
            assert_token_free(label, &text);
            assert!(
                text.contains("<token>"),
                "{label} shows where the token was: {text}"
            );
        }
    }
}

#[test]
fn pr_06_the_response_cap_content_length_first_then_a_stream() {
    // a Content-Length over the cap is refused before the body is read (the fake sends no body at all for 3 s)
    let fake = Fake::start();
    fake.push(Reply::PromiseOnly {
        status: 200,
        claimed: 5_000_000,
        hold: Duration::from_secs(3),
    });
    let (c, _) = client_with(&fake, |cfg| cfg.max_response_bytes = 1000);
    let t = Instant::now();
    assert_eq!(
        c.send(&get("/v2/whoami")).unwrap_err(),
        ClientError::ResponseTooLarge { cap: 1000 }
    );
    assert!(
        t.elapsed() < Duration::from_millis(1500),
        "refused on the header: {:?}",
        t.elapsed()
    );

    // no Content-Length: the stream would run for 5 s; the client stops at the cap
    let fake = Fake::start();
    fake.push(Reply::Stream {
        status: 200,
        chunk: vec![b'x'; 1024],
        interval: Duration::from_millis(1),
        count: 5000,
    });
    let (c, _) = client_with(&fake, |cfg| cfg.max_response_bytes = 4096);
    let t = Instant::now();
    assert_eq!(
        c.send(&get("/v2/whoami")).unwrap_err(),
        ClientError::ResponseTooLarge { cap: 4096 }
    );
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "stopped at the cap: {:?}",
        t.elapsed()
    );

    // the boundary pair, with a Content-Length and without one: `cap` is fine, `cap + 1` is not
    for with_length in [true, false] {
        let fake = Fake::start();
        let (at, over) = (vec![b'y'; 1000], vec![b'y'; 1001]);
        for body in [&at, &over] {
            if with_length {
                fake.push(Reply::http(200, &[], body));
            } else {
                fake.push(Reply::Stream {
                    status: 200,
                    chunk: body.clone(),
                    interval: Duration::ZERO,
                    count: 1,
                });
            }
        }
        let (c, _) = client_with(&fake, |cfg| cfg.max_response_bytes = 1000);
        assert_eq!(
            c.send(&get("/v2/whoami")).unwrap().body.len(),
            1000,
            "length header: {with_length}"
        );
        assert_eq!(
            c.send(&get("/v2/whoami")).unwrap_err(),
            ClientError::ResponseTooLarge { cap: 1000 },
            "length header: {with_length}"
        );
    }
}

#[test]
fn pr_07_json_decoding_of_a_success_never_panics() {
    let fake = Fake::start();
    fake.push(Reply::http(
        200,
        &[("Cortex-Request-Id", "rid-1")],
        b"not json",
    ));
    fake.push(Reply::http(200, &[("Cortex-Request-Id", "rid-2")], b""));
    fake.push(Reply::json(200, r#"{"a":1}"#));
    fake.push(Reply::http(204, &[], b""));
    let (c, _) = client(&fake);
    let bad = c.send(&get("/v2/whoami")).unwrap().json().unwrap_err();
    assert!(
        matches!(&bad, ClientError::Decode { request_id: Some(r), .. } if r == "rid-1"),
        "{bad:?}"
    );
    assert!(matches!(
        c.send(&get("/v2/whoami")).unwrap().json().unwrap_err(),
        ClientError::Decode { .. }
    ));
    assert_eq!(c.send(&get("/v2/whoami")).unwrap().json().unwrap()["a"], 1);
    let none = c.send(&get("/v2/whoami")).expect("a 204 is a success");
    assert_eq!((none.status, none.body.len()), (204, 0));
}

#[test]
fn pr_08_a_401_is_typed_never_retried_and_has_no_token_headers() {
    for (code, kind) in [
        ("token_missing", ProblemKind::TokenMissing),
        ("token_invalid", ProblemKind::TokenInvalid),
        ("token_expired", ProblemKind::TokenExpired),
        ("token_revoked", ProblemKind::TokenRevoked),
        ("worker_retired", ProblemKind::WorkerRetired),
    ] {
        let fake = Fake::start();
        fake.push(
            Reply::problem_with(401, code, "Unauthenticated", None, "Use a valid token.")
                .header("WWW-Authenticate", "Bearer"),
        );
        let (c, sleeper) = client(&fake);
        let e = c.send(&get("/v2/whoami")).unwrap_err();
        assert!(
            e.to_string().contains("Use a valid token."),
            "the fix is shown: {e}"
        );
        let p = problem_of(e);
        assert_eq!(p.kind(), kind);
        assert_eq!((p.token_expires.clone(), p.token_renew_due), (None, false));
        assert_eq!(fake.count(), 1, "{code}: one attempt");
        assert!(sleeper.waits().is_empty());
    }
}

#[test]
fn tf_10_debug_of_the_client_and_its_config_never_shows_the_token() {
    let fake = Fake::start();
    let (c, _) = client(&fake);
    for shown in [format!("{c:?}"), format!("{c:#?}")] {
        assert_token_free("Client Debug", &shown);
    }
    let cfg = cortex_client::ClientConfig::new(
        fake.endpoint(),
        Some(cortex_client::Token::parse(TOKEN, "t").unwrap()),
    );
    for shown in [format!("{cfg:?}"), format!("{cfg:#?}")] {
        assert_token_free("ClientConfig Debug", &shown);
        assert!(shown.contains("Token(<redacted>)"), "{shown}");
    }
}

// ---- PR-10 (ellis F-1, cox's decisions of 6 Nov 18:37Z): server text on its way to a terminal -------------------------------

/// An independent oracle for the list (the ruling's list, written out), not the library's own predicate.
fn unsafe_oracle(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}' | '\u{2029}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                | '\u{61c}' | '\u{e0000}'..='\u{e007f}'
        )
}

/// One sample of every class, with plain letters between them: ESC CR BEL NUL DEL, a C1 byte (U+009B), U+2028, a bidi mark,
/// a bidi override, a bidi isolate, ALM, a tag character, an LF and a TAB (ellis N-1, N-2).
fn hostile(tag: &str) -> String {
    format!(
        "{tag}:A\x1b[31mB\rC\x07D\0E\x7fF\u{9b}G\u{2028}H\u{200e}I\u{202e}J\u{2066}K\u{61c}L\u{e0041}M\nN\tO"
    )
}

/// The same text as a terminal-safe display shows it: each unsafe sample is a `?`.
fn hostile_shown(tag: &str) -> String {
    format!("{tag}:A?[31mB?C?D?E?F?G?H?I?J?K?L?M?N?O")
}

fn problem_body(code: &str, request_id: &str, title: &str, detail: &str, fix: &str) -> Vec<u8> {
    serde_json::json!({ "type": code, "title": title, "status": 400, "detail": detail, "fix": fix, "request_id": request_id })
        .to_string()
        .into_bytes()
}

const PJ: [(&str, &str); 1] = [("Content-Type", "application/problem+json")];
const GOOD_ID: &str = "0198d2c0-7000-7000-8000-000000000001";

fn error_of(body: Vec<u8>) -> ClientError {
    let fake = Fake::start();
    fake.push(Reply::http(400, &PJ, &body));
    let (c, _) = client(&fake);
    c.send(&get("/v2/whoami")).unwrap_err()
}

#[test]
fn pr_10_a_problem_with_control_characters_in_every_printed_member_displays_without_them() {
    let e = error_of(problem_body(
        "bad_request",
        GOOD_ID,
        &hostile("T"),
        &hostile("D"),
        &hostile("F"),
    ));
    let shown = e.to_string();
    assert!(
        !shown.chars().any(unsafe_oracle),
        "no unsafe character in the Display: {shown:?}"
    );
    assert!(!shown.contains('\n'), "one line");
    assert_eq!(
        shown,
        format!(
            "{} (bad_request, HTTP 400): {} Fix: {} [request {GOOD_ID}]",
            hostile_shown("T"),
            hostile_shown("D"),
            hostile_shown("F")
        )
    );
}

#[test]
fn pr_10_the_structured_fields_keep_what_the_server_sent() {
    let e = error_of(problem_body(
        "bad_request",
        GOOD_ID,
        &hostile("T"),
        &hostile("D"),
        &hostile("F"),
    ));
    let ClientError::Problem(p) = e else {
        panic!("expected a Problem")
    };
    assert_eq!(
        p.title,
        hostile("T"),
        "decision 1: the wire value, after redact"
    );
    assert_eq!(p.detail.as_deref(), Some(hostile("D").as_str()));
    assert_eq!(p.fix, hostile("F"));
    assert_eq!(p.code, "bad_request");
    assert_eq!(p.request_id, GOOD_ID);
}

#[test]
fn pr_10_title_detail_and_fix_are_cut_by_characters_with_the_marker_inside_the_budget() {
    let cut = |n: usize, budget: usize| {
        format!("{}...", "a".repeat(budget - 3))
            .chars()
            .take(n)
            .collect::<String>()
    };
    // 201 / 1001 / 501 characters are one over their budgets (200 / 1000 / 500)
    let e = error_of(problem_body(
        "bad_request",
        GOOD_ID,
        &"a".repeat(201),
        &"b".repeat(1001),
        &"c".repeat(501),
    ));
    let shown = e.to_string();
    assert!(
        shown.starts_with(&format!("{} (bad_request", cut(200, 200))),
        "{shown}"
    );
    assert!(
        shown.contains(&format!(": {}... Fix: ", "b".repeat(997))),
        "detail"
    );
    assert!(
        shown.contains(&format!(" Fix: {}... [request", "c".repeat(497))),
        "fix"
    );
    // exactly the budgets are shown whole
    let e = error_of(problem_body(
        "bad_request",
        GOOD_ID,
        &"a".repeat(200),
        &"b".repeat(1000),
        &"c".repeat(500),
    ));
    let shown = e.to_string();
    assert!(
        shown.contains(&"a".repeat(200)) && !shown.contains("..."),
        "{shown}"
    );
    // 5,000 multibyte characters are cut on a character boundary and the cut is visible
    let e = error_of(problem_body(
        "bad_request",
        GOOD_ID,
        &"\u{e9}".repeat(5000),
        "d",
        "f",
    ));
    let shown = e.to_string();
    assert!(
        shown.starts_with(&format!("{}... (bad_request", "\u{e9}".repeat(197))),
        "{shown}"
    );
    let e = error_of(problem_body(
        "bad_request",
        GOOD_ID,
        "t",
        &"\u{1f600}".repeat(5000),
        "f",
    ));
    assert!(
        e.to_string()
            .contains(&format!(": {}... Fix: ", "\u{1f600}".repeat(997)))
    );
    assert!(
        e.to_string().chars().count() < 1200,
        "bounded whatever the server sent"
    );
}

#[test]
fn pr_10_ordinary_unicode_under_the_budgets_is_shown_unchanged() {
    let title = "\u{3b5}\u{3bb}\u{3bb}\u{3b7}\u{3bd}\u{3b9}\u{3ba}\u{3ac} \u{65e5}\u{672c}\u{8a9e} \u{1f600} \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467} \u{dc}n\u{ef}";
    let e = error_of(problem_body(
        "bad_request",
        GOOD_ID,
        title,
        "d\u{e9}tail",
        "fix",
    ));
    assert_eq!(
        e.to_string(),
        format!("{title} (bad_request, HTTP 400): d\u{e9}tail Fix: fix [request {GOOD_ID}]")
    );
}

#[test]
fn pr_10_type_and_request_id_are_validated_not_cut_a_bad_one_is_not_a_problem() {
    let ok64 = "a".repeat(64);
    let e = error_of(problem_body(&ok64, &"b".repeat(64), "t", "d", "f"));
    let ClientError::Problem(p) = e else {
        panic!("64 printable ASCII characters are fine")
    };
    assert_eq!((p.code.len(), p.request_id.len()), (64, 64));
    let bad: [(&str, String, String); 8] = [
        ("an escape in type", "bad\x1b[31m".into(), GOOD_ID.into()),
        ("a space in type", "bad request".into(), GOOD_ID.into()),
        ("a non-ASCII type", "b\u{e9}d".into(), GOOD_ID.into()),
        ("an empty type", String::new(), GOOD_ID.into()),
        ("65 characters of type", "a".repeat(65), GOOD_ID.into()),
        (
            "a control in request_id",
            "bad_request".into(),
            "id\x07".into(),
        ),
        (
            "a space in request_id",
            "bad_request".into(),
            "0198 d2c0".into(),
        ),
        (
            "65 characters of request_id",
            "bad_request".into(),
            "r".repeat(65),
        ),
    ];
    for (what, code, rid) in bad {
        let e = error_of(problem_body(&code, &rid, "t", "d", "f"));
        let ClientError::Http {
            status, snippet, ..
        } = &e
        else {
            panic!("{what}: expected the Http path, got {e:?}")
        };
        assert_eq!(*status, 400, "{what}");
        assert!(
            snippet.is_ascii() && !snippet.chars().any(unsafe_oracle),
            "{what}: {snippet:?}"
        );
        assert!(!e.to_string().chars().any(unsafe_oracle), "{what}");
    }
}

#[test]
fn pr_10_the_token_is_still_redacted_next_to_control_characters() {
    let piece = &TOKEN[2..16];
    let title = format!("Bearer {TOKEN}\x1b[0m and {piece}\u{202e}");
    let e = error_of(problem_body(
        "bad_request",
        GOOD_ID,
        &title,
        &format!("\x07{TOKEN}"),
        "fix",
    ));
    for (label, text) in [("Display", e.to_string()), ("Debug", format!("{e:?}"))] {
        assert_token_free(label, &text);
    }
    let shown = e.to_string();
    assert!(
        shown.contains("<token>") && shown.contains('?') && !shown.chars().any(unsafe_oracle),
        "{shown}"
    );
}
