//! Piece 14, test definitions r4: the transport rules (UP-02 fetch side, UP-03 library half, UP-15 token parts), with a
//! FAKE transport: no socket, no real host, no real token. The real HTTP client plugs in behind `Transport` later and
//! must follow no redirects itself.

mod common;

use common::*;
use cortex_types::Secret;
use cortex_upgrade::fetch::{
    FetchConfig, FetchError, Fetcher, MAX_REDIRECTS, Request, Response, Transport,
    resolve_asset_url, resolve_redirect, same_origin,
};
use cortex_upgrade::manifest::{Digest, MANIFEST_MAX_BYTES};
use sha2::Digest as _;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

const BASE: &str = "https://rel.example.invalid/releases/manifest.json";
const TOKEN: &str = "SYNTHETIC-TOKEN-7";

fn digest(bytes: &[u8]) -> Digest {
    Digest(sha2::Sha256::digest(bytes).into())
}

enum Route {
    Body(Vec<u8>),
    Status(u16, Vec<u8>),
    Redirect(u16, Option<String>),
    Fail(FetchError),
    /// Delivers `ok` bytes and then fails with an I/O error.
    BreakAfter(usize),
    /// A large body made of `n` bytes.
    Big(u64),
}

#[derive(Default)]
struct Fake {
    routes: Mutex<HashMap<String, Route>>,
    log: Mutex<Vec<(String, Option<String>)>>,
}

impl Fake {
    fn new() -> Arc<Fake> {
        Arc::new(Fake::default())
    }
    fn route(self: &Arc<Self>, url: &str, r: Route) -> Arc<Self> {
        self.routes.lock().unwrap().insert(url.to_string(), r);
        self.clone()
    }
    fn calls(&self) -> Vec<(String, Option<String>)> {
        self.log.lock().unwrap().clone()
    }
}

struct Broken(usize);
impl Read for Broken {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0 == 0 {
            return Err(std::io::Error::other("connection reset"));
        }
        let n = self.0.min(buf.len());
        buf[..n].fill(b'z');
        self.0 -= n;
        Ok(n)
    }
}

/// The orphan rule: a local wrapper carries the trait impl.
struct Handle(Arc<Fake>);

impl Transport for Handle {
    fn get(&self, r: &Request<'_>) -> Result<Response, FetchError> {
        let fake = &self.0;
        fake.log
            .lock()
            .unwrap()
            .push((r.url.to_string(), r.bearer.map(str::to_string)));
        let routes = fake.routes.lock().unwrap();
        let resp = |status: u16, location: Option<String>, body: Box<dyn Read + Send>| {
            Ok(Response {
                status,
                location,
                body,
            })
        };
        match routes.get(r.url) {
            None => resp(404, None, Box::new(Cursor::new(b"not found".to_vec()))),
            Some(Route::Body(b)) => resp(200, None, Box::new(Cursor::new(b.clone()))),
            Some(Route::Status(s, b)) => resp(*s, None, Box::new(Cursor::new(b.clone()))),
            Some(Route::Redirect(s, l)) => resp(*s, l.clone(), Box::new(Cursor::new(vec![]))),
            Some(Route::Fail(e)) => Err(e.clone()),
            Some(Route::BreakAfter(n)) => resp(200, None, Box::new(Broken(*n))),
            Some(Route::Big(n)) => resp(200, None, Box::new(std::io::repeat(b'x').take(*n))),
        }
    }
}

fn fetcher(f: &Arc<Fake>, token: Option<&str>) -> Fetcher<Handle> {
    Fetcher::new(
        Handle(f.clone()),
        FetchConfig::default(),
        token.map(Secret::new),
    )
}

fn safe(e: &FetchError) {
    let m = e.to_string();
    assert!(
        !m.is_empty() && m.len() < 200 && !m.chars().any(char::is_control),
        "{m:?}"
    );
    assert!(!m.contains(TOKEN) && !m.contains("SECRET BODY"), "{m:?}");
}

// ---------------------------------------------------------------- URLs

#[test]
fn fe_01_relative_assets_resolve_against_the_directory_of_the_manifest() {
    let r = |rel: &str| resolve_asset_url(BASE, rel, false).unwrap();
    assert_eq!(
        r("cortex-0.2.1-rc.2-x86_64-linux.tar.gz"),
        "https://rel.example.invalid/releases/cortex-0.2.1-rc.2-x86_64-linux.tar.gz"
    );
    assert_eq!(
        r("sub/dir/file.tar"),
        "https://rel.example.invalid/releases/sub/dir/file.tar"
    );
    assert_eq!(
        resolve_asset_url("https://h.example/manifest.json", "a.tar", false).unwrap(),
        "https://h.example/a.tar"
    );
    assert_eq!(
        resolve_asset_url("https://h.example:8443/a/m.json", "b.tar", false).unwrap(),
        "https://h.example:8443/a/b.tar"
    );
    // an absolute URL is allowed only on the release host itself (kai's rule), and is normalised
    assert_eq!(
        resolve_asset_url(BASE, "https://rel.example.invalid/x/y.tar", false).unwrap(),
        "https://rel.example.invalid/x/y.tar"
    );
    assert_eq!(
        resolve_asset_url(BASE, "HTTPS://REL.Example.Invalid/x.tar", false).unwrap(),
        "https://rel.example.invalid/x.tar"
    );
    // another host, another port, another scheme: refused
    for other in [
        "https://cdn.example.invalid/x.tar",
        "https://rel.example.invalid:8443/x.tar",
        "https://rel.example.invalid.evil.example/x.tar",
        "https://xrel.example.invalid/x.tar",
    ] {
        assert_eq!(
            resolve_asset_url(BASE, other, false),
            Err(FetchError::BadUrl),
            "{other}"
        );
    }
    assert_eq!(
        resolve_asset_url(
            "http://rel.example.invalid/m.json",
            "http://rel.example.invalid:81/x",
            true
        ),
        Err(FetchError::BadUrl)
    );
}

#[test]
fn fe_02_urls_that_are_not_plain_https_paths_are_refused() {
    let host = "https://rel.example.invalid";
    assert!(
        resolve_asset_url(BASE, "a.tar", false).is_ok(),
        "control: a plain relative name is accepted"
    );
    assert!(
        resolve_asset_url(BASE, &format!("{host}/a/b.tar"), false).is_ok(),
        "control: a plain absolute https URL on the release host is accepted"
    );
    // references: relative forms and absolute forms ON the release host, each broken in one way
    let long = format!("a{}.tar", "b".repeat(3000));
    let long_abs = format!("{host}/{}", "c".repeat(3000));
    let long_total = format!("{host}/{}", "c".repeat(2040));
    let mut bad: Vec<String> = [
        "",
        "/abs",
        "//evil.example/x",
        "../x",
        "a/../x",
        "./x",
        "a/./x",
        "a//b",
        "a\\b",
        "a b",
        "x?y=1",
        "x#f",
        "x%2e%2e/y",
        "x\ny",
        "x\ry",
        "x\0y",
        "x\x1by",
        "é.tar",
        "ftp://rel.example.invalid/x",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/plain,x",
        "https:/rel.example.invalid/x",
        "https//rel.example.invalid/x",
        "mailto:a@b",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for tail in [
        "/a/../x", "/a?b", "/a#b", "/a b", "/a//b", "/a/./b", "/a%2fb", "", "/",
    ] {
        bad.push(format!("{host}{tail}"));
    }
    bad.extend([
        "https://u:p@rel.example.invalid/x".to_string(),
        "https://u@rel.example.invalid/x".to_string(),
        "https:///x".to_string(),
    ]);
    bad.extend([long, long_abs, long_total]);
    for b in bad {
        let e = resolve_asset_url(BASE, &b, false).expect_err(&format!("must be refused: {b:?}"));
        assert!(
            matches!(e, FetchError::BadUrl | FetchError::InsecureUrl),
            "{b:?}: {e:?}"
        );
        safe(&e);
    }
    // the host and port syntax: a manifest URL that is wrong in one way is refused as a base
    for base in [
        "https://h .example/m.json",
        "https://-bad.example/m.json",
        "https://bad-.example/m.json",
        "https://h..example/m.json",
        "https://.example/m.json",
        "https://h.example:0/m.json",
        "https://h.example:70000/m.json",
        "https://h.example:abc/m.json",
        "https://h.example:/m.json",
        "https://u@h.example/m.json",
        "https://h_x.example/m.json",
        "https://h.example/m.json?x=1",
        "https://h.example/m.json#f",
        "https://[::1]/m.json",
        "https://h.example",
        "https://h.example/",
    ] {
        let e = resolve_asset_url(base, "a.tar", false)
            .expect_err(&format!("base must be refused: {base:?}"));
        assert!(
            matches!(e, FetchError::BadUrl | FetchError::InsecureUrl),
            "{base:?}: {e:?}"
        );
    }
    assert!(
        resolve_asset_url("https://h.example:65535/m.json", "a.tar", false).is_ok(),
        "control: the largest port"
    );
    assert!(
        resolve_asset_url("https://h-1.ex-ample.example/m.json", "a.tar", false).is_ok(),
        "control: dashes inside labels"
    );
}

#[test]
fn fe_03_plain_http_is_refused_unless_the_test_hook_allows_it_and_a_bad_base_is_refused() {
    assert_eq!(
        resolve_asset_url("http://h.example/m.json", "a.tar", false),
        Err(FetchError::InsecureUrl)
    );
    assert_eq!(
        resolve_asset_url(BASE, "http://h.example/a.tar", false),
        Err(FetchError::InsecureUrl)
    );
    assert_eq!(
        resolve_asset_url("http://h.example/m.json", "a.tar", true).unwrap(),
        "http://h.example/a.tar"
    );
    assert_eq!(
        resolve_asset_url("ftp://h.example/m.json", "a.tar", true),
        Err(FetchError::BadUrl)
    );
    for base in [
        "",
        "manifest.json",
        "https://",
        "https://u@h.example/m.json",
    ] {
        assert!(resolve_asset_url(base, "a.tar", false).is_err(), "{base:?}");
    }
}

#[test]
fn fe_04_redirect_targets_follow_the_same_rules_and_never_downgrade() {
    let cur = "https://rel.example.invalid/releases/a.tar";
    assert_eq!(
        resolve_redirect(cur, "/other/b.tar", false).unwrap(),
        "https://rel.example.invalid/other/b.tar"
    );
    assert_eq!(
        resolve_redirect(cur, "b.tar", false).unwrap(),
        "https://rel.example.invalid/releases/b.tar"
    );
    assert_eq!(
        resolve_redirect(cur, "https://rel.example.invalid/c.tar", false).unwrap(),
        "https://rel.example.invalid/c.tar"
    );
    // a redirect to another host (or port) is refused, not followed without the token
    for other in [
        "https://cdn.example.invalid/c.tar",
        "https://rel.example.invalid:8443/c.tar",
    ] {
        assert_eq!(
            resolve_redirect(cur, other, false),
            Err(FetchError::BadRedirect),
            "{other}"
        );
    }
    for bad in [
        "",
        "//evil.example/x",
        "/a/../b",
        "/a?b",
        "https://u@h.example/x",
        "ftp://h.example/x",
        "/x y",
        "\n",
    ] {
        assert_eq!(
            resolve_redirect(cur, bad, false),
            Err(FetchError::BadRedirect),
            "{bad:?}"
        );
    }
    // https to http is never followed, even when plain http is allowed; http to http is, when allowed
    assert_eq!(
        resolve_redirect(cur, "http://h.example/x", true),
        Err(FetchError::BadRedirect)
    );
    // the same host and port under another scheme is a downgrade, and is refused
    assert_eq!(
        resolve_redirect("https://h.example:8080/a", "http://h.example:8080/x", true),
        Err(FetchError::BadRedirect)
    );
    assert_eq!(
        resolve_redirect("http://h.example/a", "/b", true).unwrap(),
        "http://h.example/b"
    );
    assert_eq!(
        resolve_redirect("http://h.example/a", "/b", false),
        Err(FetchError::BadRedirect)
    );
}

#[test]
fn fe_05_same_origin_means_scheme_host_and_port() {
    assert!(same_origin("https://h.example/a", "https://h.example/b/c"));
    assert!(same_origin(
        "https://H.Example/a",
        "https://h.example:443/b"
    ));
    assert!(same_origin("http://h.example/a", "http://h.example:80/b"));
    assert!(!same_origin(
        "https://h.example/a",
        "https://h.example:8443/a"
    ));
    assert!(!same_origin("https://h.example/a", "https://i.example/a"));
    assert!(!same_origin("https://h.example/a", "http://h.example/a"));
    // the same port number under another scheme is another origin
    assert!(!same_origin(
        "https://h.example:8080/a",
        "http://h.example:8080/a"
    ));
    assert!(!same_origin("https://h.example/a", "not a url"));
}

// ---------------------------------------------------------------- the manifest

#[test]
fn fe_06_the_manifest_is_fetched_with_the_token_and_capped_at_one_mib() {
    let f = Fake::new().route(BASE, Route::Body(b"{\"schema\":1}".to_vec()));
    assert_eq!(
        fetcher(&f, Some(TOKEN)).fetch_manifest(BASE).unwrap(),
        b"{\"schema\":1}"
    );
    assert_eq!(f.calls(), vec![(BASE.to_string(), Some(TOKEN.to_string()))]);

    let f = Fake::new().route(BASE, Route::Body(b"{}".to_vec()));
    assert_eq!(fetcher(&f, None).fetch_manifest(BASE).unwrap(), b"{}");
    assert_eq!(
        f.calls(),
        vec![(BASE.to_string(), None)],
        "no token configured: no Authorization"
    );

    let at = vec![b' '; MANIFEST_MAX_BYTES];
    let f = Fake::new().route(BASE, Route::Body(at.clone()));
    assert_eq!(
        fetcher(&f, None).fetch_manifest(BASE).unwrap().len(),
        MANIFEST_MAX_BYTES,
        "exactly the cap is accepted"
    );
    let f = Fake::new().route(BASE, Route::Body(vec![b' '; MANIFEST_MAX_BYTES + 1]));
    assert_eq!(
        fetcher(&f, None).fetch_manifest(BASE),
        Err(FetchError::TooLarge)
    );
    let f = Fake::new().route(BASE, Route::Big(50 * MANIFEST_MAX_BYTES as u64));
    assert_eq!(
        fetcher(&f, None).fetch_manifest(BASE),
        Err(FetchError::TooLarge)
    );
}

#[test]
fn fe_07_only_status_200_is_an_answer_and_no_body_text_is_ever_in_an_error() {
    for status in [
        201u16, 204, 206, 300, 304, 400, 401, 403, 404, 429, 500, 503,
    ] {
        let f = Fake::new().route(BASE, Route::Status(status, b"SECRET BODY".to_vec()));
        let e = fetcher(&f, Some(TOKEN))
            .fetch_manifest(BASE)
            .expect_err(&status.to_string());
        assert_eq!(e, FetchError::Status(status));
        safe(&e);
    }
    let f = Fake::new().route(BASE, Route::Fail(FetchError::Timeout));
    assert_eq!(
        fetcher(&f, None).fetch_manifest(BASE),
        Err(FetchError::Timeout)
    );
    let f = Fake::new().route(BASE, Route::Fail(FetchError::Transport));
    assert_eq!(
        fetcher(&f, None).fetch_manifest(BASE),
        Err(FetchError::Transport)
    );
}

#[test]
fn fe_08_plain_http_is_refused_before_any_request() {
    let f = Fake::new().route(
        "http://rel.example.invalid/m.json",
        Route::Body(b"{}".to_vec()),
    );
    assert_eq!(
        fetcher(&f, Some(TOKEN)).fetch_manifest("http://rel.example.invalid/m.json"),
        Err(FetchError::InsecureUrl)
    );
    assert!(f.calls().is_empty(), "no connection was attempted");
    // with the test hook it goes through
    let cfg = FetchConfig::default().allow_plain_http();
    assert!(
        Fetcher::new(Handle(f.clone()), cfg, None)
            .fetch_manifest("http://rel.example.invalid/m.json")
            .is_ok()
    );
}

// ---------------------------------------------------------------- redirects and the token

#[test]
fn fe_09_up_to_three_redirects_are_followed_a_fourth_is_not() {
    let a = "https://rel.example.invalid/releases/a";
    let f = Fake::new()
        .route(BASE, Route::Redirect(301, Some("/r1".into())))
        .route(
            "https://rel.example.invalid/r1",
            Route::Redirect(302, Some("/r2".into())),
        )
        .route(
            "https://rel.example.invalid/r2",
            Route::Redirect(307, Some("/r3".into())),
        )
        .route(
            "https://rel.example.invalid/r3",
            Route::Body(b"ok".to_vec()),
        );
    assert_eq!(MAX_REDIRECTS, 3);
    assert_eq!(fetcher(&f, None).fetch_manifest(BASE).unwrap(), b"ok");
    assert_eq!(f.calls().len(), 4);

    let f = Fake::new()
        .route(BASE, Route::Redirect(301, Some("/r1".into())))
        .route(
            "https://rel.example.invalid/r1",
            Route::Redirect(302, Some("/r2".into())),
        )
        .route(
            "https://rel.example.invalid/r2",
            Route::Redirect(303, Some("/r3".into())),
        )
        .route(
            "https://rel.example.invalid/r3",
            Route::Redirect(308, Some("/r4".into())),
        )
        .route(
            "https://rel.example.invalid/r4",
            Route::Body(b"never".to_vec()),
        );
    assert_eq!(
        fetcher(&f, None).fetch_manifest(BASE),
        Err(FetchError::TooManyRedirects)
    );
    assert_eq!(f.calls().len(), 4, "the fifth request is not made");

    // a loop is the same thing
    let f = Fake::new().route(a, Route::Redirect(302, Some(a.into())));
    assert_eq!(
        fetcher(&f, None).fetch_manifest(a),
        Err(FetchError::TooManyRedirects)
    );
}

#[test]
fn fe_10_a_bad_redirect_is_refused() {
    let f = Fake::new().route(BASE, Route::Redirect(302, None));
    assert_eq!(
        fetcher(&f, None).fetch_manifest(BASE),
        Err(FetchError::BadRedirect),
        "no Location"
    );
    let f = Fake::new().route(
        BASE,
        Route::Redirect(302, Some("http://rel.example.invalid/x".into())),
    );
    assert_eq!(
        fetcher(&f, None).fetch_manifest(BASE),
        Err(FetchError::BadRedirect),
        "downgrade to http"
    );
    let f = Fake::new().route(BASE, Route::Redirect(302, Some("//evil.example/x".into())));
    assert_eq!(
        fetcher(&f, None).fetch_manifest(BASE),
        Err(FetchError::BadRedirect)
    );
    assert_eq!(f.calls().len(), 1, "the bad target was not requested");
}

#[test]
fn fe_11_a_redirect_inside_the_release_host_keeps_the_token_and_one_leaving_it_is_refused_before_any_request()
 {
    let other = "https://cdn.example.invalid/r1";
    let f = Fake::new()
        .route(BASE, Route::Redirect(302, Some("/same".into())))
        .route(
            "https://rel.example.invalid/same",
            Route::Redirect(302, Some("https://rel.example.invalid/ok".into())),
        )
        .route(
            "https://rel.example.invalid/ok",
            Route::Body(b"ok".to_vec()),
        );
    assert_eq!(
        fetcher(&f, Some(TOKEN)).fetch_manifest(BASE).unwrap(),
        b"ok"
    );
    let c = f.calls();
    assert_eq!(c.len(), 3);
    assert!(
        c.iter().all(|(_, b)| b.as_deref() == Some(TOKEN)),
        "every request inside the release host carries the token"
    );

    // a redirect to another host is refused; the other host is never contacted
    let f = Fake::new()
        .route(BASE, Route::Redirect(302, Some("/same".into())))
        .route(
            "https://rel.example.invalid/same",
            Route::Redirect(302, Some(other.into())),
        )
        .route(other, Route::Body(b"never".to_vec()));
    let e = fetcher(&f, Some(TOKEN)).fetch_manifest(BASE).unwrap_err();
    assert_eq!(e, FetchError::BadRedirect);
    safe(&e);
    assert!(
        f.calls().iter().all(|(u, _)| !u.contains("cdn.")),
        "no request reached the other host"
    );
}

#[test]
fn fe_12_an_absolute_asset_url_on_another_host_is_refused_before_any_request_and_the_token_is_never_printed()
 {
    let tar = b"tarball".to_vec();
    let f = Fake::new()
        .route(
            "https://rel.example.invalid/releases/a.tar",
            Route::Body(tar.clone()),
        )
        .route(
            "https://cdn.example.invalid/b.tar",
            Route::Body(tar.clone()),
        );
    let dir = Tmp::new("fetch");
    let fe = fetcher(&f, Some(TOKEN));
    fe.download(BASE, "a.tar", &digest(&tar), &dir.path().join("a"))
        .unwrap();
    let b = dir.path().join("b");
    let e = fe
        .download(BASE, "https://cdn.example.invalid/b.tar", &digest(&tar), &b)
        .unwrap_err();
    assert_eq!(e, FetchError::BadUrl);
    safe(&e);
    assert!(!b.exists(), "no file was created for a refused URL");
    let c = f.calls();
    assert_eq!(c.len(), 1, "only the release host was contacted");
    assert_eq!(c[0].1.as_deref(), Some(TOKEN));
    assert!(
        !format!("{fe:?}").contains(TOKEN),
        "Debug does not show the token"
    );
}

// ---------------------------------------------------------------- the download

#[test]
fn fe_13_a_download_is_written_0600_and_checked_against_its_digest() {
    let body = b"cortex tarball bytes".to_vec();
    let f = Fake::new().route(
        "https://rel.example.invalid/releases/a.tar",
        Route::Body(body.clone()),
    );
    let dir = Tmp::new("fetch");
    let dest = dir.path().join("a.tar");
    let n = fetcher(&f, None)
        .download(BASE, "a.tar", &digest(&body), &dest)
        .unwrap();
    assert_eq!(n, body.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), body);
    assert_eq!(
        std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // the empty file has a digest too
    let f = Fake::new().route(
        "https://rel.example.invalid/releases/e",
        Route::Body(vec![]),
    );
    assert_eq!(
        fetcher(&f, None)
            .download(BASE, "e", &digest(b""), &dir.path().join("e"))
            .unwrap(),
        0
    );
}

#[test]
fn fe_14_a_changed_byte_a_wrong_digest_or_a_too_large_file_leaves_no_file() {
    let body = b"cortex tarball bytes".to_vec();
    let mut changed = body.clone();
    changed[3] ^= 1;
    let dir = Tmp::new("fetch");
    let f = Fake::new().route(
        "https://rel.example.invalid/releases/a.tar",
        Route::Body(changed),
    );
    let dest = dir.path().join("a.tar");
    assert_eq!(
        fetcher(&f, None).download(BASE, "a.tar", &digest(&body), &dest),
        Err(FetchError::DigestMismatch)
    );
    assert!(!dest.exists(), "a mismatching download is deleted");

    // the cap: more than max_asset_bytes
    let f = Fake::new().route("https://rel.example.invalid/releases/big", Route::Big(1000));
    let cfg = FetchConfig::default().with_max_asset_bytes(100);
    let dest = dir.path().join("big");
    assert_eq!(
        Fetcher::new(Handle(f.clone()), cfg.clone(), None).download(
            BASE,
            "big",
            &digest(b"x"),
            &dest
        ),
        Err(FetchError::TooLarge)
    );
    assert!(!dest.exists());
    // exactly the cap is fine
    let f = Fake::new().route(
        "https://rel.example.invalid/releases/ok",
        Route::Body(vec![b'x'; 100]),
    );
    assert_eq!(
        Fetcher::new(Handle(f), cfg, None)
            .download(BASE, "ok", &digest(&[b'x'; 100]), &dir.path().join("ok"))
            .unwrap(),
        100
    );
}

#[test]
fn fe_15_status_errors_broken_streams_and_an_existing_destination_leave_things_as_they_were() {
    let dir = Tmp::new("fetch");
    let f = Fake::new().route(
        "https://rel.example.invalid/releases/a.tar",
        Route::Status(500, b"SECRET BODY".to_vec()),
    );
    let dest = dir.path().join("a.tar");
    let e = fetcher(&f, Some(TOKEN))
        .download(BASE, "a.tar", &digest(b"x"), &dest)
        .unwrap_err();
    assert_eq!(e, FetchError::Status(500));
    safe(&e);
    assert!(!dest.exists());

    let f = Fake::new().route(
        "https://rel.example.invalid/releases/b.tar",
        Route::BreakAfter(10),
    );
    let dest = dir.path().join("b.tar");
    let e = fetcher(&f, None)
        .download(BASE, "b.tar", &digest(b"x"), &dest)
        .unwrap_err();
    assert!(
        matches!(e, FetchError::Transport | FetchError::Io(_)),
        "{e:?}"
    );
    assert!(!dest.exists(), "a broken stream leaves no partial file");

    // an existing destination is never overwritten
    let dest = dir.path().join("c.tar");
    std::fs::write(&dest, "KEEP").unwrap();
    let f = Fake::new().route(
        "https://rel.example.invalid/releases/c.tar",
        Route::Body(b"new".to_vec()),
    );
    let e = fetcher(&f, None)
        .download(BASE, "c.tar", &digest(b"new"), &dest)
        .unwrap_err();
    assert_eq!(e, FetchError::Io(std::io::ErrorKind::AlreadyExists));
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "KEEP");
    assert!(
        f.calls().is_empty(),
        "the destination was checked before the request"
    );
}

// ---------------------------------------------------------------- mike, packet 3: a Request never prints its bearer

#[test]
fn fe_16_the_debug_form_of_a_request_does_not_show_the_bearer() {
    let with = Request {
        url: BASE,
        bearer: Some(TOKEN),
    };
    for shown in [format!("{with:?}"), format!("{with:#?}")] {
        assert!(!shown.contains(TOKEN), "the token is shown: {shown}");
        assert!(shown.contains(BASE), "the url is shown: {shown}");
        assert!(
            shown.contains("<redacted>"),
            "there is a bearer and it says so: {shown}"
        );
    }
    let without = Request {
        url: BASE,
        bearer: None,
    };
    let shown = format!("{without:?}");
    assert!(
        shown.contains("None") && !shown.contains("<redacted>"),
        "{shown}"
    );
    // equality still compares the bearer (a derived trait is not lost)
    assert_ne!(with.clone(), without.clone());
    assert_eq!(with.clone(), with);
}
