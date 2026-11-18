//! HD-01 to HD-04: the response headers of R-10 and W1-25 are surfaced, never trusted and never fatal.
mod support;

use cortex_client::ClientError;
use support::fake::{Fake, Reply};
use support::{client, get};

#[test]
fn hd_01_the_request_id_is_captured_on_success_and_on_a_plain_http_error() {
    let fake = Fake::start();
    fake.push(Reply::ok().header("Cortex-Request-Id", "req-ok"));
    fake.push(Reply::http(502, &[("Cortex-Request-Id", "req-bad")], b"x"));
    let (c, _) = client(&fake);
    assert_eq!(
        c.send(&get("/v2/whoami")).unwrap().request_id.as_deref(),
        Some("req-ok")
    );
    assert!(matches!(
        c.send(&get("/v2/whoami")).unwrap_err(),
        ClientError::Http { request_id: Some(r), .. } if r == "req-bad"
    ));
}

#[test]
fn hd_02_token_expires_is_kept_raw_when_printable_ascii_of_1_to_64_bytes() {
    let ok64 = "x".repeat(64);
    let ok65 = "x".repeat(65);
    let cases: [(&str, Option<&str>); 5] = [
        ("2026-12-01T00:00:00Z", Some("2026-12-01T00:00:00Z")),
        (ok64.as_str(), Some(ok64.as_str())),
        (ok65.as_str(), None),
        ("", None),
        ("exp\u{e9}", None),
    ];
    for (sent, want) in cases {
        let fake = Fake::start();
        fake.push(Reply::ok().header("Cortex-Token-Expires", sent));
        let (c, _) = client(&fake);
        let r = c
            .send(&get("/v2/whoami"))
            .expect("a malformed header never fails a successful call");
        assert_eq!(r.token_expires.as_deref(), want, "sent {sent:?}");
    }
}

#[test]
fn hd_03_renew_due_only_for_exactly_due() {
    for (sent, want) in [
        ("due", true),
        ("DUE", false),
        ("yes", false),
        ("", false),
        ("due,due", false),
        ("1", false),
    ] {
        let fake = Fake::start();
        fake.push(Reply::ok().header("Cortex-Token-Renew", sent));
        let (c, _) = client(&fake);
        assert_eq!(
            c.send(&get("/v2/whoami")).unwrap().token_renew_due,
            want,
            "sent {sent:?}"
        );
    }
    let fake = Fake::start();
    let (c, _) = client(&fake);
    assert!(
        !c.send(&get("/v2/whoami")).unwrap().token_renew_due,
        "absent"
    );
}

#[test]
fn hd_04_the_headers_ride_inside_the_typed_problem_on_refusals_after_authentication() {
    for (status, code) in [
        (403, "missing_scope"),
        (409, "conflict"),
        (422, "invalid_request"),
    ] {
        let fake = Fake::start();
        fake.push(
            Reply::problem(status, code)
                .header("Cortex-Token-Expires", "2026-12-01T00:00:00Z")
                .header("Cortex-Token-Renew", "due"),
        );
        let (c, _) = client(&fake);
        let ClientError::Problem(p) = c.send(&get("/v2/whoami")).unwrap_err() else {
            panic!()
        };
        assert_eq!(
            p.token_expires.as_deref(),
            Some("2026-12-01T00:00:00Z"),
            "{status}"
        );
        assert!(p.token_renew_due, "{status}");
    }
    // the final 429 keeps them too
    let fake = Fake::start();
    fake.set_default(
        Reply::problem(429, "busy")
            .header("Retry-After", "1")
            .header("Cortex-Token-Expires", "2026-12-01T00:00:00Z")
            .header("Cortex-Token-Renew", "due"),
    );
    let (c, _) = client(&fake);
    let ClientError::Busy { last: Some(p), .. } = c.send(&get("/v2/whoami")).unwrap_err() else {
        panic!()
    };
    assert_eq!(p.token_expires.as_deref(), Some("2026-12-01T00:00:00Z"));
    assert!(p.token_renew_due);
}
