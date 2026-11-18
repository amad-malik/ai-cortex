//! RQ-01 to RQ-06 and IK-02, IK-03, IK-05, IK-06, IK-08: what the client sends and how it keeps the Idempotency-Key.
//! Against the std-only fake server (tests/support/fake.rs). DB-free.
mod support;

use cortex_client::{ClientError, Method, Mutates, PathId, ProblemKind, Request, TransportKind};
use support::fake::{Fake, Reply};
use support::{TOKEN, UUID_A, client, client_with, get, post};

#[test]
fn rq_01_the_open_health_route_is_sent_without_authorization() {
    let fake = Fake::start();
    let (c, _) = client(&fake);
    c.send(&get("/v2/health").open()).expect("health");
    let r = &fake.requests()[0];
    assert_eq!(r.method, "GET");
    assert_eq!(r.target, "/v2/health");
    assert!(
        r.header("authorization").is_none(),
        "an open route carries no Authorization: {}",
        r.head
    );
    assert!(!r.head.contains(TOKEN));
}

#[test]
fn rq_02_a_token_route_carries_exactly_one_bearer_and_the_token_nowhere_else() {
    let fake = Fake::start();
    let (c, _) = client(&fake);
    c.send(&get("/v2/whoami").query("a", "b")).expect("whoami");
    let r = &fake.requests()[0];
    assert_eq!(
        r.headers_named("authorization"),
        vec![format!("Bearer {TOKEN}")]
    );
    assert_eq!(
        r.head.matches(TOKEN).count(),
        1,
        "the token occurs once in the whole head: {}",
        r.head
    );
    assert!(!r.target.contains(TOKEN));
    assert!(!String::from_utf8_lossy(&r.body).contains(TOKEN));
}

#[test]
fn rq_02_without_a_token_a_token_route_is_refused_before_any_request() {
    let fake = Fake::start();
    let (c, _) = client_with(&fake, |cfg| cfg.token = None);
    let e = c.send(&get("/v2/whoami")).unwrap_err();
    assert!(matches!(e, ClientError::Token(_)), "{e:?}");
    assert_eq!(fake.count(), 0);
}

#[test]
fn rq_03_path_ids_are_lower_case_hyphenated_uuids_and_nothing_else() {
    PathId::parse(UUID_A).expect("a good id");
    let long = "a".repeat(36);
    let refused = [
        "../x",
        "%2f",
        "",
        "0198D2C0-7000-7000-8000-00000000000A",
        "{0198d2c0-7000-7000-8000-00000000000a}",
        "0198d2c0700070008000-00000000000a",
        "0198d2c0-7000-7000-8000-00000000000",
        "0198d2c0-7000-7000-8000-00000000000aa",
        "0198d2c0-7000-7000-8000-00000000000a/",
        " 0198d2c0-7000-7000-8000-00000000000a",
        "urn:uuid:0198d2c0-7000-7000-8000-00000000000a",
        &long,
    ];
    for s in refused {
        assert!(PathId::parse(s).is_err(), "{s:?} must be refused");
    }
    let id = PathId::parse(UUID_A).unwrap();
    let ok = Request::new(
        Method::Post,
        "/v2/handoffs/{id}/claim",
        std::slice::from_ref(&id),
        Mutates::Yes,
    );
    assert!(ok.is_ok());
    // placeholder count and id count must agree
    assert!(Request::new(Method::Get, "/v2/handoffs/{id}", &[], Mutates::No).is_err());
    assert!(
        Request::new(
            Method::Get,
            "/v2/handoffs",
            std::slice::from_ref(&id),
            Mutates::No
        )
        .is_err()
    );
    assert!(
        Request::new(
            Method::Get,
            "/v2/a/{x}/b/{y}",
            std::slice::from_ref(&id),
            Mutates::No
        )
        .is_err()
    );

    let fake = Fake::start();
    let (c, _) = client(&fake);
    let r = Request::new(Method::Post, "/v2/handoffs/{id}/claim", &[id], Mutates::Yes).unwrap();
    c.send(&r).unwrap();
    assert_eq!(
        fake.requests()[0].target,
        format!("/v2/handoffs/{UUID_A}/claim")
    );
}

#[test]
fn rq_04_query_values_are_percent_encoded_and_keep_their_order() {
    let fake = Fake::start();
    let (c, _) = client(&fake);
    let r = get("/v2/records")
        .query("q", "a&b=c#d+e f")
        .query("q", "\u{e9}")
        .query("n", "\0")
        .query("safe", "AZaz09-._~");
    c.send(&r).unwrap();
    assert_eq!(
        fake.requests()[0].target,
        "/v2/records?q=a%26b%3Dc%23d%2Be%20f&q=%C3%A9&n=%00&safe=AZaz09-._~"
    );
}

#[test]
fn rq_05_headers_and_body_of_a_post_and_of_a_get() {
    let fake = Fake::start();
    let (c, _) = client(&fake);
    c.send(&post("/v2/records", Mutates::Yes).json_body(br#"{"a":1}"#.to_vec()))
        .unwrap();
    c.send(&get("/v2/whoami")).unwrap();
    let reqs = fake.requests();
    let (p, g) = (&reqs[0], &reqs[1]);
    assert_eq!(p.header("content-type"), Some("application/json"));
    assert_eq!(p.header("accept"), Some("application/json"));
    assert!(
        p.header("user-agent")
            .unwrap_or("")
            .starts_with("cortex-client/"),
        "{}",
        p.head
    );
    assert_eq!(p.body, br#"{"a":1}"#);
    assert!(
        p.header("accept-encoding").is_none(),
        "no encodings are advertised: {}",
        p.head
    );
    assert!(g.header("content-type").is_none(), "{}", g.head);
    assert!(g.body.is_empty());
    assert!(
        g.header("content-length").is_none_or(|v| v == "0"),
        "{}",
        g.head
    );
}

#[test]
fn rq_06_the_request_body_cap_pair() {
    let fake = Fake::start();
    let (c, _) = client_with(&fake, |cfg| cfg.max_request_bytes = 100);
    let at = post("/v2/records", Mutates::Yes).json_body(vec![b'x'; 100]);
    c.send(&at).expect("the cap itself is allowed");
    assert_eq!(fake.count(), 1);
    let over = post("/v2/records", Mutates::Yes).json_body(vec![b'x'; 101]);
    assert_eq!(
        c.send(&over).unwrap_err(),
        ClientError::RequestTooLarge {
            size: 101,
            cap: 100
        }
    );
    assert_eq!(fake.count(), 1, "nothing was sent for the oversized body");
}

fn is_uuid_v7(s: &str) -> bool {
    s.len() == 36
        && s.as_bytes()[14] == b'7'
        && matches!(s.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
        && [8, 13, 18, 23].iter().all(|&i| s.as_bytes()[i] == b'-')
}

#[test]
fn ik_02_a_retried_call_keeps_its_key_path_and_body() {
    let fake = Fake::start();
    fake.push_n(2, Reply::problem(429, "busy").header("Retry-After", "1"));
    fake.push(Reply::json(201, r#"{"id":"x"}"#));
    let (c, sleeper) = client(&fake);
    let r = post("/v2/records", Mutates::Yes).json_body(br#"{"k":"v"}"#.to_vec());
    let resp = c.send(&r).expect("the third attempt succeeds");
    assert_eq!(resp.status, 201);
    let reqs = fake.requests();
    assert_eq!(reqs.len(), 3);
    let keys: Vec<_> = reqs
        .iter()
        .map(|q| q.header("idempotency-key").unwrap().to_owned())
        .collect();
    assert!(is_uuid_v7(&keys[0]), "{}", keys[0]);
    assert!(
        keys.iter().all(|k| *k == keys[0]),
        "the SAME key on every attempt: {keys:?}"
    );
    assert!(
        reqs.iter()
            .all(|q| q.target == reqs[0].target && q.body == reqs[0].body)
    );
    assert_eq!(sleeper.waits().len(), 2);
}

#[test]
fn ik_03_the_caller_says_whether_a_call_mutates_the_method_does_not() {
    let fake = Fake::start();
    let (c, _) = client(&fake);
    c.send(&post("/v2/search", Mutates::No).json_body(b"{}".to_vec()))
        .unwrap();
    c.send(&get("/v2/whoami")).unwrap();
    c.send(&post("/v2/records", Mutates::Yes).json_body(b"{}".to_vec()))
        .unwrap();
    let r = fake.requests();
    assert!(
        r[0].header("idempotency-key").is_none(),
        "a search POST carries no key"
    );
    assert!(r[1].header("idempotency-key").is_none());
    assert_eq!(r[2].headers_named("idempotency-key").len(), 1);
}

#[test]
fn ik_05_the_replay_header_sets_replayed_only_for_exactly_true() {
    let fake = Fake::start();
    let (c, _) = client(&fake);
    for (value, want) in [
        (Some("true"), true),
        (Some("false"), false),
        (Some("TRUE"), false),
        (Some("1"), false),
        (None, false),
    ] {
        let reply = match value {
            Some(v) => Reply::ok().header("Cortex-Idempotent-Replay", v),
            None => Reply::ok(),
        };
        fake.push(reply);
        assert_eq!(
            c.send(&get("/v2/whoami")).unwrap().replayed,
            want,
            "{value:?}"
        );
    }
}

#[test]
fn ik_06_428_and_422_key_errors_are_typed_and_never_retried() {
    for (status, code, kind) in [
        (
            428,
            "idempotency_key_required",
            ProblemKind::IdempotencyKeyRequired,
        ),
        (
            422,
            "idempotency_key_reused",
            ProblemKind::IdempotencyKeyReused,
        ),
    ] {
        let fake = Fake::start();
        fake.push(Reply::problem(status, code));
        let (c, sleeper) = client(&fake);
        let e = c
            .send(&post("/v2/records", Mutates::Yes).json_body(b"{}".to_vec()))
            .unwrap_err();
        let ClientError::Problem(p) = e else {
            panic!("expected a Problem, got {e:?}")
        };
        assert_eq!(p.kind(), kind);
        assert_eq!(p.status, status);
        assert_eq!(fake.count(), 1, "exactly one attempt");
        assert!(sleeper.waits().is_empty());
    }
}

#[test]
fn ik_08_a_connection_closed_after_a_mutating_request_is_one_attempt_and_names_the_key() {
    let fake = Fake::start();
    fake.push(Reply::CloseAfterRequest);
    let (c, _) = client(&fake);
    let e = c
        .send(&post("/v2/records", Mutates::Yes).json_body(b"{}".to_vec()))
        .unwrap_err();
    assert_eq!(fake.count(), 1, "ONE attempt, no hidden replay");
    let sent = fake.requests()[0]
        .header("idempotency-key")
        .unwrap()
        .to_owned();
    let ClientError::Transport { kind, key, .. } = &e else {
        panic!("expected Transport, got {e:?}")
    };
    assert!(
        matches!(kind, TransportKind::Closed | TransportKind::Other),
        "{kind:?}"
    );
    assert_eq!(key.as_deref(), Some(sent.as_str()));
    let text = e.to_string();
    assert!(
        text.contains(&sent) && text.contains("may have been applied"),
        "{text}"
    );
    assert!(!text.contains(TOKEN));

    // a read has no key to repeat
    let fake = Fake::start();
    fake.push(Reply::CloseAfterRequest);
    let (c, _) = client(&fake);
    let ClientError::Transport { key, .. } = c.send(&get("/v2/whoami")).unwrap_err() else {
        panic!()
    };
    assert_eq!(key, None);
}

#[test]
fn ik_07_two_calls_with_the_same_body_and_token_get_different_keys() {
    let fake = Fake::start();
    let (c, _) = client(&fake);
    let r = post("/v2/records", Mutates::Yes).json_body(b"{}".to_vec());
    c.send(&r).unwrap();
    c.send(&r).unwrap();
    let reqs = fake.requests();
    let (a, b) = (
        reqs[0].header("idempotency-key").unwrap(),
        reqs[1].header("idempotency-key").unwrap(),
    );
    assert_ne!(
        a, b,
        "a key is per logical operation: a second send() is a second operation"
    );
    assert!(!a.contains(&TOKEN[..8]) && !b.contains(&TOKEN[..8]));
}

// ---- F-2 (ellis, cox's batch of 7 Nov): the advice to repeat with the same key is for a request that MAY have been applied -------

#[test]
fn ik_09_a_refused_connection_on_a_mutating_call_does_not_say_the_request_may_have_been_applied() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let ep = cortex_client::Endpoint::parse(&format!("http://127.0.0.1:{port}")).unwrap();
    let cfg = cortex_client::ClientConfig::new(
        ep,
        Some(cortex_client::Token::parse(TOKEN, "t").unwrap()),
    );
    let c = cortex_client::Client::new(cfg);
    let e = c
        .send(&post("/v2/records", Mutates::Yes).json_body(b"{}".to_vec()))
        .unwrap_err();
    let ClientError::Transport { kind, key, .. } = &e else {
        panic!("expected Transport, got {e:?}")
    };
    assert_eq!(*kind, TransportKind::Refused, "nothing listens on the port");
    let text = e.to_string();
    assert!(
        !text.contains("may have been applied"),
        "nothing was sent: {text}"
    );
    assert!(
        !text.contains("Idempotency-Key"),
        "no key to repeat: {text}"
    );
    if let Some(k) = key {
        assert!(!text.contains(k.as_str()), "{text}");
    }
    assert!(
        text.contains(&format!("127.0.0.1:{port}")) && text.contains("refused"),
        "{text}"
    );
}

#[test]
fn ik_09_a_timeout_on_a_mutating_call_keeps_the_advice_and_a_read_never_had_it() {
    let fake = Fake::start();
    fake.push(Reply::ok().delayed(std::time::Duration::from_secs(2)));
    fake.push(Reply::ok().delayed(std::time::Duration::from_secs(2)));
    let (c, _) = client_with(&fake, |cfg| {
        cfg.timeout = std::time::Duration::from_millis(300)
    });
    let e = c
        .send(&post("/v2/records", Mutates::Yes).json_body(b"{}".to_vec()))
        .unwrap_err();
    let ClientError::Transport { kind, key, .. } = &e else {
        panic!("expected Transport, got {e:?}")
    };
    assert_eq!(*kind, TransportKind::Timeout);
    let sent = fake.requests()[0]
        .header("idempotency-key")
        .unwrap()
        .to_owned();
    assert_eq!(key.as_deref(), Some(sent.as_str()));
    let text = e.to_string();
    assert!(
        text.contains("may have been applied") && text.contains(&sent),
        "{text}"
    );
    // a read carries no key and no advice
    let e = c.send(&get("/v2/whoami")).unwrap_err();
    assert!(!e.to_string().contains("may have been applied"), "{e}");
}
