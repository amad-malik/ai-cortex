//! RT-01, RT-02 (the loop part), RT-03, RT-04, RT-06, RT-07: busy retry, no hidden retries, no redirects, timeouts, refusal.
//! RT-05 (the proxy variables) needs a child process and lives in the cortex-cli tests. DB-free.
mod support;

use cortex_client::{ClientError, Endpoint, Mutates, TransportKind};
use std::net::TcpListener;
use std::time::{Duration, Instant};
use support::fake::{Fake, Reply};
use support::{TOKEN, client, client_with, get, post};

#[test]
fn rt_01_four_attempts_then_busy() {
    let fake = Fake::start();
    fake.set_default(Reply::problem(429, "busy").header("Retry-After", "1"));
    let (c, sleeper) = client(&fake);
    let e = c.send(&get("/v2/whoami")).unwrap_err();
    let ClientError::Busy {
        attempts,
        retry_after_secs,
        last,
    } = &e
    else {
        panic!("expected Busy, got {e:?}")
    };
    assert_eq!((*attempts, *retry_after_secs), (4, 1));
    assert_eq!(last.as_ref().map(|p| p.status), Some(429));
    assert_eq!(fake.count(), 4);
    assert_eq!(sleeper.waits(), vec![Duration::from_secs(1); 3]);
}

#[test]
fn rt_02_the_total_wait_never_exceeds_the_cap() {
    let fake = Fake::start();
    fake.set_default(Reply::problem(429, "busy").header("Retry-After", "1"));
    let (c, sleeper) = client_with(&fake, |cfg| {
        cfg.retry.max_attempts = 10;
        cfg.retry.max_total_wait = Duration::from_secs(2);
    });
    let e = c.send(&get("/v2/whoami")).unwrap_err();
    assert!(matches!(e, ClientError::Busy { attempts: 3, .. }), "{e:?}");
    assert_eq!(
        sleeper.waits(),
        vec![Duration::from_secs(1); 2],
        "2 s in all, never 3"
    );

    // a long Retry-After is capped per wait at 5 s; the total cap of 10 s stops the loop
    let fake = Fake::start();
    fake.set_default(Reply::problem(429, "busy").header("Retry-After", "120"));
    let (c, sleeper) = client_with(&fake, |cfg| cfg.retry.max_attempts = 10);
    let _ = c.send(&get("/v2/whoami")).unwrap_err();
    let waits = sleeper.waits();
    assert_eq!(
        waits,
        vec![Duration::from_secs(5); 2],
        "5 + 5 = 10, the third would be 15"
    );
    assert!(waits.iter().sum::<Duration>() <= Duration::from_secs(10));
}

#[test]
fn rt_03_nothing_but_429_is_retried() {
    let cases: [(u16, &str); 10] = [
        (500, "internal_error"),
        (400, "invalid_request"),
        (403, "missing_scope"),
        (404, "not_found"),
        (409, "conflict"),
        (422, "invalid_request"),
        (503, "unavailable"),
        (503, "maintenance"),
        (503, "restore_in_progress"),
        (503, "capability_unavailable"),
    ];
    for (status, code) in cases {
        let fake = Fake::start();
        fake.set_default(Reply::problem(status, code));
        let (c, sleeper) = client(&fake);
        let e = c
            .send(&post("/v2/records", Mutates::Yes).json_body(b"{}".to_vec()))
            .unwrap_err();
        assert!(
            matches!(e, ClientError::Problem(_)),
            "{status} {code}: {e:?}"
        );
        assert_eq!(fake.count(), 1, "{status} {code}: exactly one attempt");
        assert!(sleeper.waits().is_empty(), "{status} {code}");
    }
}

#[test]
fn rt_04_a_redirect_is_never_followed_and_the_token_goes_nowhere_else() {
    for status in [301u16, 302, 303, 307, 308] {
        let other = Fake::start();
        let fake = Fake::start();
        fake.push(Reply::http(
            status,
            &[("Location", &format!("{}/steal", other.url()))],
            b"",
        ));
        fake.push(Reply::http(status, &[("Location", "/elsewhere")], b""));
        let (c, _) = client(&fake);
        for _ in 0..2 {
            let e = c.send(&get("/v2/whoami")).unwrap_err();
            assert_eq!(e, ClientError::UnexpectedRedirect { status });
        }
        assert_eq!(
            other.count(),
            0,
            "{status}: the second origin saw NO request"
        );
        assert_eq!(other.connections(), 0, "{status}: and no connection");
        assert_eq!(
            fake.count(),
            2,
            "{status}: one request per send, none followed"
        );
    }
}

#[test]
fn rt_06_the_total_timeout_covers_a_slow_answer_and_a_trickled_body() {
    let fake = Fake::start();
    fake.push(Reply::ok().delayed(Duration::from_secs(2)));
    let (c, _) = client_with(&fake, |cfg| cfg.timeout = Duration::from_millis(300));
    let t = Instant::now();
    let e = c.send(&get("/v2/whoami")).unwrap_err();
    assert!(
        matches!(
            e,
            ClientError::Transport {
                kind: TransportKind::Timeout,
                ..
            }
        ),
        "{e:?}"
    );
    let took = t.elapsed();
    assert!(
        took >= Duration::from_millis(250) && took < Duration::from_millis(1500),
        "{took:?}"
    );

    // headers at once, then one byte every 200 ms: a per-read timeout would never fire
    let fake = Fake::start();
    fake.push(Reply::Trickle {
        status: 200,
        body: vec![b'z'; 20],
        per_byte: Duration::from_millis(200),
    });
    let (c, _) = client_with(&fake, |cfg| cfg.timeout = Duration::from_millis(500));
    let t = Instant::now();
    let e = c.send(&get("/v2/whoami")).unwrap_err();
    assert!(
        matches!(
            e,
            ClientError::Transport {
                kind: TransportKind::Timeout,
                ..
            }
        ),
        "{e:?}"
    );
    assert!(
        t.elapsed() < Duration::from_millis(1500),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn rt_07_nothing_listens_on_the_port() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let fake = Fake::start();
    let (c, _) = {
        let ep = Endpoint::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let mut cfg = cortex_client::ClientConfig::new(
            ep,
            Some(cortex_client::Token::parse(TOKEN, "t").unwrap()),
        );
        cfg.timeout = Duration::from_secs(5);
        (cortex_client::Client::new(cfg), &fake)
    };
    let e = c.send(&get("/v2/whoami")).unwrap_err();
    let ClientError::Transport { kind, addr, key } = &e else {
        panic!("expected Transport, got {e:?}")
    };
    assert_eq!(*kind, TransportKind::Refused);
    assert_eq!(addr, &format!("127.0.0.1:{port}"));
    assert_eq!(*key, None);
    assert!(!e.to_string().contains(TOKEN));
    assert!(e.to_string().contains(&format!("127.0.0.1:{port}")));
}
