//! Piece 14, cox's ruling on the engine ports (13:30Z): the transport for the LOCAL server accepts only the literal
//! `http://127.0.0.1:<port>`, follows no redirects, caps the answer at 64 KiB, has a timeout, and reads NO proxy from the
//! environment; and the release transport reads none either. Loopback servers only; no real token.

mod common;

use common::http::{Reply, response, serve};
use cortex_upgrade::fetch::{FetchError, Request, Transport};
use cortex_upgrade::http::{LOCAL_BODY_MAX_BYTES, LocalTransport, UreqTransport};
use std::io::Read;
use std::time::{Duration, Instant};

const TOKEN: &str = "SYNTHETIC-OPERATOR-TOKEN-1";

fn lt() -> LocalTransport {
    LocalTransport::new(Duration::from_secs(5))
}

fn read_all(mut r: cortex_upgrade::fetch::Response) -> std::io::Result<Vec<u8>> {
    let mut v = Vec::new();
    r.body.read_to_end(&mut v).map(|_| v)
}

#[test]
fn lt_01_a_get_to_the_literal_loopback_address_works_and_carries_the_bearer() {
    let s = serve(|_| response("200 OK", &[], b"{\"ok\":true}"));
    let r = lt()
        .get(&Request {
            url: &s.url("/v2/whoami"),
            bearer: Some(TOKEN),
        })
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(read_all(r).unwrap(), b"{\"ok\":true}");
    let seen = s.requests();
    assert_eq!(
        seen[0].header("authorization"),
        Some(&format!("Bearer {TOKEN}")[..])
    );
    assert!(seen[0].request_line.starts_with("GET /v2/whoami HTTP/1.1"));
    let r = lt()
        .get(&Request {
            url: &s.url("/v2/health/ready"),
            bearer: None,
        })
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(s.requests()[1].header("authorization"), None);
}

#[test]
fn lt_02_anything_but_the_literal_loopback_address_is_refused_before_any_connection() {
    let s = serve(|_| response("200 OK", &[], b"x"));
    let port = s.addr.port();
    assert!(
        lt().get(&Request {
            url: &s.url("/control"),
            bearer: None
        })
        .is_ok(),
        "control: the literal address works"
    );
    let before = s.requests().len();
    let bad = [
        format!("http://localhost:{port}/x"),
        format!("http://[::1]:{port}/x"),
        format!("http://127.0.0.2:{port}/x"),
        format!("http://0.0.0.0:{port}/x"),
        format!("http://127.1:{port}/x"),
        format!("http://2130706433:{port}/x"),
        format!("https://127.0.0.1:{port}/x"),
        "http://127.0.0.1/x".to_string(),
        "http://127.0.0.1:0/x".to_string(),
        format!("http://127.0.0.1:{}/x", 70000),
        format!("http://u@127.0.0.1:{port}/x"),
        format!("http://127.0.0.1:{port}@evil.example/x"),
        format!("http://127.0.0.1:{port}"),
        format!("http://127.0.0.1:{port}/x?y=1"),
        format!("http://127.0.0.1:{port}/x#f"),
        format!("http://127.0.0.1:{port}/../x"),
        format!("http://127.0.0.1:{port}//x"),
        format!("http://127.0.0.1:{port}/x y"),
        format!("HTTP://127.0.0.1:{port}/x"),
        format!("ftp://127.0.0.1:{port}/x"),
        String::new(),
    ];
    for url in bad {
        let e = lt()
            .get(&Request {
                url: &url,
                bearer: Some(TOKEN),
            })
            .err()
            .unwrap_or_else(|| panic!("must be refused: {url:?}"));
        assert!(
            matches!(e, FetchError::BadUrl | FetchError::InsecureUrl),
            "{url:?}: {e:?}"
        );
        assert!(!e.to_string().contains(TOKEN));
    }
    assert_eq!(
        s.requests().len(),
        before,
        "no connection was made for any of them"
    );
}

#[test]
fn lt_03_a_redirect_is_returned_not_followed() {
    let s = serve(|r| {
        if r.path() == "/old" {
            response("302 Found", &[("Location", "/new")], b"")
        } else {
            response("200 OK", &[], b"new")
        }
    });
    let r = lt()
        .get(&Request {
            url: &s.url("/old"),
            bearer: Some(TOKEN),
        })
        .unwrap();
    assert_eq!((r.status, r.location.as_deref()), (302, Some("/new")));
    assert_eq!(s.requests().len(), 1);
}

#[test]
fn lt_04_the_answer_is_capped_at_64_kib() {
    assert_eq!(LOCAL_BODY_MAX_BYTES, 64 * 1024);
    let exact = vec![b'a'; LOCAL_BODY_MAX_BYTES as usize];
    let s = serve(move |_| response("200 OK", &[], &exact));
    assert_eq!(
        read_all(
            lt().get(&Request {
                url: &s.url("/x"),
                bearer: None
            })
            .unwrap()
        )
        .unwrap()
        .len(),
        LOCAL_BODY_MAX_BYTES as usize,
        "exactly the cap is read"
    );
    let over = vec![b'a'; LOCAL_BODY_MAX_BYTES as usize + 1];
    let s = serve(move |_| response("200 OK", &[], &over));
    let err = read_all(
        lt().get(&Request {
            url: &s.url("/x"),
            bearer: None,
        })
        .unwrap(),
    )
    .unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::InvalidData,
        "one byte more is an error, not a truncation"
    );
    let huge = vec![b'a'; 4 << 20];
    let s = serve(move |_| response("200 OK", &[], &huge));
    assert!(
        read_all(
            lt().get(&Request {
                url: &s.url("/x"),
                bearer: None
            })
            .unwrap()
        )
        .is_err()
    );
}

#[test]
fn lt_05_a_server_that_never_answers_times_out_and_a_closed_port_is_a_transport_error() {
    let s = serve(|_| Reply::Hang);
    let url = s.url("/x");
    let (tx, rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    std::thread::spawn(move || {
        let r = LocalTransport::new(Duration::from_millis(500)).get(&Request {
            url: &url,
            bearer: None,
        });
        let _ = tx.send(r.err());
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(8))
            .expect("did not time out within 8 s"),
        Some(FetchError::Timeout)
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let live = serve(|_| response("200 OK", &[], b"x"));
    assert_eq!(
        lt().get(&Request {
            url: &live.url("/x"),
            bearer: None
        })
        .expect("control")
        .status,
        200
    );
    assert_eq!(
        lt().get(&Request {
            url: &format!("http://127.0.0.1:{port}/x"),
            bearer: None
        })
        .err(),
        Some(FetchError::Transport)
    );
}

// ---------------------------------------------------------------- no proxy from the environment, on either transport

const PROXY_VARS: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

/// The child half: does the GET and prints the status; does nothing unless the marker names a transport and a URL.
fn child(which: &str) {
    let Ok(spec) = std::env::var("CORTEX_TEST_PROXY_CHILD") else {
        return;
    };
    let (kind, url) = spec.split_once(' ').unwrap();
    assert_eq!(kind, which);
    let r = if which == "local" {
        LocalTransport::new(Duration::from_secs(5)).get(&Request { url, bearer: None })
    } else {
        UreqTransport::new_allowing_http(Duration::from_secs(5)).get(&Request { url, bearer: None })
    };
    println!(
        "CHILD-STATUS {}",
        r.map(|r| r.status.to_string())
            .unwrap_or_else(|e| format!("error {e:?}"))
    );
}

fn proxy_cell(which: &str, test_name: &str) {
    let target = serve(|_| response("200 OK", &[], b"target"));
    let sniffer = serve(|_| response("502 Bad Gateway", &[], b"a proxy answered"));
    let proxy_url = format!("http://127.0.0.1:{}", sniffer.addr.port());
    let envs: Vec<(&str, &str)> = PROXY_VARS
        .iter()
        .map(|k| (*k, proxy_url.as_str()))
        .collect();
    let out = common::rerun_self(
        test_name,
        (
            "CORTEX_TEST_PROXY_CHILD",
            &format!("{which} {}", target.url("/x")),
        ),
        &envs,
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("CHILD-STATUS 200"),
        "the child did not reach the target directly: {text}"
    );
    assert_eq!(
        target.requests().len(),
        1,
        "the target was asked once, directly"
    );
    assert!(
        sniffer.requests().is_empty(),
        "the proxy named in the environment was contacted: {:?}",
        sniffer.requests()
    );
}

#[test]
fn lt_06_the_local_transport_reads_no_proxy_from_the_environment() {
    if std::env::var("CORTEX_TEST_PROXY_CHILD").is_ok() {
        return child("local");
    }
    proxy_cell(
        "local",
        "lt_06_the_local_transport_reads_no_proxy_from_the_environment",
    );
}

#[test]
fn lt_07_the_release_transport_reads_no_proxy_from_the_environment() {
    if std::env::var("CORTEX_TEST_PROXY_CHILD").is_ok() {
        return child("release");
    }
    proxy_cell(
        "release",
        "lt_07_the_release_transport_reads_no_proxy_from_the_environment",
    );
}
