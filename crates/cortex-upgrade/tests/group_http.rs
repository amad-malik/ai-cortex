//! Piece 14: the real client (`UreqTransport`) against a loopback server, alone and under the `Fetcher`. Plain http
//! only because a loopback server has no certificate; the https path (rustls) is NOT exercised here: that needs a
//! real host (Wave 2) and is listed as a limit.

mod common;

use common::http::{Reply, response, serve};
use cortex_types::Secret;
use cortex_upgrade::fetch::{FetchConfig, FetchError, Fetcher, Request, Transport};
use cortex_upgrade::http::UreqTransport;
use cortex_upgrade::manifest::Digest;
use sha2::Digest as _;
use std::io::Read;
use std::time::{Duration, Instant};

const TOKEN: &str = "SYNTHETIC-TOKEN-8";

fn t(secs: u64) -> UreqTransport {
    UreqTransport::new_allowing_http(Duration::from_secs(secs))
}

fn fetcher(token: Option<&str>) -> Fetcher<UreqTransport> {
    Fetcher::new(
        t(5),
        FetchConfig::default().allow_plain_http(),
        token.map(Secret::new),
    )
}

fn body_of(r: cortex_upgrade::fetch::Response) -> Vec<u8> {
    let mut v = Vec::new();
    r.body.take(10 << 20).read_to_end(&mut v).unwrap();
    v
}

#[test]
fn ht_01_a_get_returns_status_headers_and_body_and_sends_the_bearer_only_when_given() {
    let s = serve(|_| response("200 OK", &[], b"hello"));
    let r = t(5)
        .get(&Request {
            url: &s.url("/a"),
            bearer: Some(TOKEN),
        })
        .unwrap();
    assert_eq!((r.status, r.location.clone()), (200, None));
    assert_eq!(body_of(r), b"hello");
    let r = t(5)
        .get(&Request {
            url: &s.url("/b"),
            bearer: None,
        })
        .unwrap();
    assert_eq!(body_of(r), b"hello");
    let seen = s.requests();
    assert_eq!(
        seen[0].header("authorization"),
        Some(&format!("Bearer {TOKEN}")[..])
    );
    assert_eq!(seen[1].header("authorization"), None);
    assert!(seen[0].request_line.starts_with("GET /a HTTP/1.1"));
}

#[test]
fn ht_02_a_redirect_is_returned_not_followed() {
    let s = serve(|r| {
        if r.path() == "/old" {
            response("302 Found", &[("Location", "/new")], b"")
        } else {
            response("200 OK", &[], b"new")
        }
    });
    let r = t(5)
        .get(&Request {
            url: &s.url("/old"),
            bearer: None,
        })
        .unwrap();
    assert_eq!(r.status, 302);
    assert_eq!(r.location.as_deref(), Some("/new"));
    assert_eq!(s.requests().len(), 1, "the client did not follow it");
}

#[test]
fn ht_03_error_statuses_are_answers_not_errors() {
    for (line, code) in [
        ("404 Not Found", 404u16),
        ("500 Internal Server Error", 500),
        ("401 Unauthorized", 401),
        ("204 No Content", 204),
    ] {
        let s = serve(move |_| response(line, &[], b"SECRET BODY"));
        let r = t(5)
            .get(&Request {
                url: &s.url("/x"),
                bearer: None,
            })
            .unwrap();
        assert_eq!(r.status, code);
    }
}

#[test]
fn ht_04_the_production_constructor_refuses_plain_http_and_never_connects() {
    let s = serve(|_| response("200 OK", &[], b"x"));
    let ok = t(5)
        .get(&Request {
            url: &s.url("/control"),
            bearer: None,
        })
        .expect("control: with the test hook the same server answers");
    assert_eq!(ok.status, 200);
    let before = s.requests().len();
    let e = UreqTransport::new(Duration::from_secs(5))
        .get(&Request {
            url: &s.url("/x"),
            bearer: None,
        })
        .err()
        .unwrap();
    assert_eq!(e, FetchError::InsecureUrl);
    assert_eq!(
        s.requests().len(),
        before,
        "the production constructor made no connection"
    );
}

#[test]
fn ht_05_a_server_that_never_answers_times_out() {
    let s = serve(|_| Reply::Hang);
    let url = s.url("/x");
    let started = Instant::now();
    // on another thread, so that a client WITHOUT a timeout fails this test instead of hanging it
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = UreqTransport::new_allowing_http(Duration::from_millis(500)).get(&Request {
            url: &url,
            bearer: None,
        });
        let _ = tx.send(r.err());
    });
    let got = rx
        .recv_timeout(Duration::from_secs(8))
        .expect("the request did not time out within 8 s");
    assert_eq!(got, Some(FetchError::Timeout));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn ht_06_a_closed_port_is_a_transport_error_without_text() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let live = serve(|_| response("200 OK", &[], b"x"));
    let control = t(5).get(&Request {
        url: &live.url("/x"),
        bearer: Some(TOKEN),
    });
    assert_eq!(control.expect("control: a live server answers").status, 200);
    let e = t(5)
        .get(&Request {
            url: &format!("http://127.0.0.1:{port}/x"),
            bearer: Some(TOKEN),
        })
        .err()
        .unwrap();
    assert_eq!(e, FetchError::Transport);
    assert!(!e.to_string().contains(TOKEN) && !e.to_string().contains("127.0.0.1"));
}

#[test]
fn ht_07_a_bearer_that_cannot_be_a_header_value_is_refused_and_nothing_is_sent() {
    let s = serve(|_| response("200 OK", &[], b"x"));
    let control = t(5).get(&Request {
        url: &s.url("/control"),
        bearer: Some(TOKEN),
    });
    assert_eq!(control.expect("control: a good bearer works").status, 200);
    for bad in ["abc\r\nX-Evil: 1", "abc\ndef", "ab\0c"] {
        let e = t(5)
            .get(&Request {
                url: &s.url("/x"),
                bearer: Some(bad),
            })
            .err()
            .unwrap();
        assert_eq!(e, FetchError::Transport, "{bad:?}");
    }
    assert_eq!(
        s.requests().len(),
        1,
        "only the control request reached the server: none carried an injected header"
    );
}

#[test]
fn ht_08_chunked_bodies_are_decoded() {
    let s = serve(|_| {
        let mut v =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
        v.extend_from_slice(b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n");
        Reply::Raw(v)
    });
    let r = t(5)
        .get(&Request {
            url: &s.url("/x"),
            bearer: None,
        })
        .unwrap();
    assert_eq!(body_of(r), b"hello world");
}

// ---------------------------------------------------------------- under the Fetcher

#[test]
fn ht_09_the_fetcher_over_the_real_client_fetches_follows_a_redirect_inside_the_host_and_checks_the_digest()
 {
    let tar = b"the release tarball".to_vec();
    let d = Digest(sha2::Sha256::digest(&tar).into());
    let tar2 = tar.clone();
    let s = serve(move |r| match r.path() {
        "/releases/manifest.json" => response("200 OK", &[], b"{\"schema\":1}"),
        "/releases/a.tar" => response("302 Found", &[("Location", "/mirror/a.tar")], b""),
        "/mirror/a.tar" => response("200 OK", &[], &tar2),
        _ => response("404 Not Found", &[], b"no"),
    });
    let dir = common::Tmp::new("http");
    let f = fetcher(Some(TOKEN));
    let m = s.url("/releases/manifest.json");
    assert_eq!(f.fetch_manifest(&m).unwrap(), b"{\"schema\":1}");
    assert_eq!(
        f.download(&m, "a.tar", &d, &dir.path().join("a.tar"))
            .unwrap(),
        tar.len() as u64
    );
    assert_eq!(std::fs::read(dir.path().join("a.tar")).unwrap(), tar);
    let seen = s.requests();
    assert_eq!(
        seen.len(),
        3,
        "manifest, the asset path (302), the mirror (200)"
    );
    assert!(
        seen.iter()
            .all(|r| r.header("authorization") == Some(&format!("Bearer {TOKEN}")[..])),
        "every request to the release host carried the token"
    );
}

#[test]
fn ht_10_a_redirect_to_another_port_is_refused_and_that_server_is_never_contacted() {
    let other = serve(|_| response("200 OK", &[], b"never"));
    let target = other.url("/stolen");
    let s = serve(move |_| response("302 Found", &[("Location", &target)], b""));
    let e = fetcher(Some(TOKEN))
        .fetch_manifest(&s.url("/m.json"))
        .unwrap_err();
    assert_eq!(e, FetchError::BadRedirect);
    assert!(other.requests().is_empty(), "the other origin saw nothing");
}

#[test]
fn ht_11_a_huge_manifest_is_cut_at_the_cap_and_a_wrong_digest_leaves_no_file() {
    let big = vec![b' '; 2 << 20];
    let s = serve(move |_| response("200 OK", &[], &big));
    assert_eq!(
        fetcher(None).fetch_manifest(&s.url("/m.json")).unwrap_err(),
        FetchError::TooLarge
    );
    let s = serve(|_| response("200 OK", &[], b"bytes"));
    let dir = common::Tmp::new("http");
    let dest = dir.path().join("x");
    let wrong = Digest(sha2::Sha256::digest(b"other").into());
    assert_eq!(
        fetcher(None)
            .download(&s.url("/m.json"), "x", &wrong, &dest)
            .unwrap_err(),
        FetchError::DigestMismatch
    );
    assert!(!dest.exists());
}
