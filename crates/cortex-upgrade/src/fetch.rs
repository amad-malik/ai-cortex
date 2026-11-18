//! Fetching the manifest and the assets (design §3, §4 steps 2 and 4, UP-02, UP-03, UP-15). The rules live here
//! and are tested with a fake [`Transport`]: the URL rules (https only, no userinfo, no query, no dot segments),
//! the redirect rules (at most three hops, never https to http, the token never leaves the manifest's origin), the
//! size caps, and the digest. The real HTTP client implements [`Transport`] and follows NO redirects itself.

use crate::manifest::{Digest, MANIFEST_MAX_BYTES};
use cortex_types::{Secret, Sha256};
use sha2::Digest as _;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Redirect hops followed at most.
pub const MAX_REDIRECTS: usize = 3;
/// The largest asset accepted unless the configuration says otherwise (4 GiB).
pub const DEFAULT_MAX_ASSET_BYTES: u64 = 4 << 30;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    #[error("fetch refused: the URL is not acceptable")]
    BadUrl,
    #[error("fetch refused: plain http is not allowed")]
    InsecureUrl,
    #[error("fetch failed: the host answered status {0}")]
    Status(u16),
    #[error("fetch failed: too many redirects")]
    TooManyRedirects,
    #[error("fetch failed: a redirect led somewhere that is not acceptable")]
    BadRedirect,
    #[error("fetch failed: the answer is larger than the limit")]
    TooLarge,
    #[error("fetch failed: the file does not match its digest")]
    DigestMismatch,
    #[error("fetch failed: could not reach the host")]
    Transport,
    #[error("fetch failed: timed out")]
    Timeout,
    #[error("fetch failed: local file error: {0}")]
    Io(std::io::ErrorKind),
}

/// One GET. `bearer` is the token to send as `Authorization: Bearer ...`, or `None` (the fetcher decides).
#[derive(Clone, PartialEq, Eq)]
pub struct Request<'a> {
    pub url: &'a str,
    pub bearer: Option<&'a str>,
}

/// The token is never printed: a log line that holds a `Request` shows `Some("<redacted>")` for it.
impl std::fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("url", &self.url)
            .field("bearer", &self.bearer.map(|_| "<redacted>"))
            .finish()
    }
}

/// An answer. A transport follows NO redirects and turns NO status into an error; it applies the timeout.
pub struct Response {
    pub status: u16,
    pub location: Option<String>,
    pub body: Box<dyn Read + Send>,
}

pub trait Transport: Send + Sync {
    fn get(&self, request: &Request<'_>) -> Result<Response, FetchError>;
}

#[derive(Debug, Clone)]
pub struct FetchConfig {
    pub max_asset_bytes: u64,
    allow_http: bool,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            max_asset_bytes: DEFAULT_MAX_ASSET_BYTES,
            allow_http: false,
        }
    }
}

impl FetchConfig {
    /// Test builds only: allow `http://` (a fake or a loopback server).
    #[cfg(feature = "test-hooks")]
    pub fn allow_plain_http(mut self) -> Self {
        self.allow_http = true;
        self
    }
    /// The largest asset accepted.
    pub fn with_max_asset_bytes(mut self, bytes: u64) -> Self {
        self.max_asset_bytes = bytes;
        self
    }
    pub fn allows_http(&self) -> bool {
        self.allow_http
    }
}

// ---------------------------------------------------------------- URLs
//
// Deliberately small and strict: `scheme://host[:port]/path` and nothing else. No userinfo, no query, no fragment,
// no percent-escapes, no dot segments, no empty segments, no IPv6 literal, printable ASCII only, at most 2048 bytes.

const URL_MAX_BYTES: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Parsed {
    https: bool,
    host: String,
    port: Option<u16>,
    path: String,
}

impl Parsed {
    fn same_origin(&self, other: &Parsed) -> bool {
        self.https == other.https
            && self.host == other.host
            && self.effective_port() == other.effective_port()
    }
    fn effective_port(&self) -> u16 {
        self.port.unwrap_or(if self.https { 443 } else { 80 })
    }
    fn origin(&self) -> String {
        let port = self.port.map(|p| format!(":{p}")).unwrap_or_default();
        format!(
            "{}://{}{}",
            if self.https { "https" } else { "http" },
            self.host,
            port
        )
    }
    fn render(&self) -> String {
        format!("{}{}", self.origin(), self.path)
    }
    /// The path up to and including its last `/`.
    fn directory(&self) -> &str {
        &self.path[..=self.path.rfind('/').unwrap_or(0)]
    }
}

fn path_byte_ok(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'-' | b'/')
}

/// `a/b/c`: non-empty segments, none of them `.` or `..`, only path bytes.
pub(crate) fn segments_ok(rel: &str) -> bool {
    !rel.is_empty()
        && rel.len() <= URL_MAX_BYTES
        && rel.bytes().all(path_byte_ok)
        && rel
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// A joined URL is held to the same length limit as a parsed one.
fn bounded(url: String, err: FetchError) -> Result<String, FetchError> {
    if url.len() > URL_MAX_BYTES {
        Err(err)
    } else {
        Ok(url)
    }
}

fn host_ok(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn parse_absolute(url: &str, allow_http: bool) -> Result<Parsed, FetchError> {
    if url.len() > URL_MAX_BYTES || !url.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(FetchError::BadUrl);
    }
    let (scheme, rest) = url.split_once("://").ok_or(FetchError::BadUrl)?;
    let https = match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" if allow_http => false,
        "http" => return Err(FetchError::InsecureUrl),
        _ => return Err(FetchError::BadUrl),
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => return Err(FetchError::BadUrl),
    };
    if authority.contains('@') {
        return Err(FetchError::BadUrl);
    }
    let (host, port) = match authority.split_once(':') {
        None => (authority, None),
        Some((h, p)) => {
            let ok = !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit());
            let port: u16 = if ok {
                p.parse().map_err(|_| FetchError::BadUrl)?
            } else {
                0
            };
            if port == 0 {
                return Err(FetchError::BadUrl);
            }
            (h, Some(port))
        }
    };
    let host = host.to_ascii_lowercase();
    if !host_ok(&host) || !segments_ok(&path[1..]) {
        return Err(FetchError::BadUrl);
    }
    Ok(Parsed {
        https,
        host,
        port,
        path: path.to_string(),
    })
}

/// Resolve an asset reference from the manifest: a plain relative path (against the directory of the manifest's
/// URL) or an absolute `https` URL. Anything else is refused. `http` is accepted only when `allow_http`.
pub fn resolve_asset_url(
    manifest_url: &str,
    reference: &str,
    allow_http: bool,
) -> Result<String, FetchError> {
    let base = parse_absolute(manifest_url, allow_http)?;
    if reference.contains("://") {
        let target = parse_absolute(reference, allow_http)?;
        // everything comes from the release host: another host, port or scheme is refused
        if !base.same_origin(&target) {
            return Err(FetchError::BadUrl);
        }
        return Ok(target.render());
    }
    if !segments_ok(reference) {
        return Err(FetchError::BadUrl);
    }
    bounded(
        format!("{}{}{}", base.origin(), base.directory(), reference),
        FetchError::BadUrl,
    )
}

/// Resolve a `Location` header: absolute, absolute-path (`/x`) or relative. Never to userinfo, never `//host`, and
/// never from https to http, even when plain http is allowed. Every refusal is `BadRedirect`.
pub fn resolve_redirect(
    current_url: &str,
    location: &str,
    allow_http: bool,
) -> Result<String, FetchError> {
    let bad = |_| FetchError::BadRedirect;
    let cur = parse_absolute(current_url, allow_http).map_err(bad)?;
    if location.is_empty()
        || !location.bytes().all(|b| (0x21..=0x7e).contains(&b))
        || location.starts_with("//")
    {
        return Err(FetchError::BadRedirect);
    }
    if location.contains("://") {
        let target = parse_absolute(location, allow_http).map_err(bad)?;
        // never another host, port or scheme (which includes https to http)
        if !cur.same_origin(&target) {
            return Err(FetchError::BadRedirect);
        }
        return Ok(target.render());
    }
    if let Some(abs) = location.strip_prefix('/') {
        if !segments_ok(abs) {
            return Err(FetchError::BadRedirect);
        }
        return bounded(
            format!("{}{}", cur.origin(), location),
            FetchError::BadRedirect,
        );
    }
    if !segments_ok(location) {
        return Err(FetchError::BadRedirect);
    }
    bounded(
        format!("{}{}{}", cur.origin(), cur.directory(), location),
        FetchError::BadRedirect,
    )
}

/// Same scheme, host and port (the default port counts as written or not). Either URL invalid: `false`.
pub fn same_origin(a: &str, b: &str) -> bool {
    match (parse_absolute(a, true), parse_absolute(b, true)) {
        (Ok(a), Ok(b)) => a.same_origin(&b),
        _ => false,
    }
}

// ---------------------------------------------------------------- the fetcher

pub struct Fetcher<T: Transport> {
    transport: T,
    config: FetchConfig,
    token: Option<Secret>,
}

impl<T: Transport> std::fmt::Debug for Fetcher<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fetcher")
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

impl<T: Transport> Fetcher<T> {
    pub fn new(transport: T, config: FetchConfig, token: Option<Secret>) -> Self {
        Self {
            transport,
            config,
            token,
        }
    }

    /// GET `start`, following at most [`MAX_REDIRECTS`] redirects by our own rules, every one of them on the origin of
    /// `origin_url` (the release host); the token goes on each request. Status 200 only.
    fn get(&self, start: &str, origin_url: &str) -> Result<Response, FetchError> {
        let allow = self.config.allow_http;
        let mut url = parse_absolute(start, allow)?.render();
        for hop in 0..=MAX_REDIRECTS {
            // Every request stays on the release host: the starting URL and each redirect were checked when they were
            // resolved, and this is the last line of defence before anything is sent (the token goes with it).
            if !same_origin(&url, origin_url) {
                return Err(FetchError::BadRedirect);
            }
            let bearer = self.token.as_ref().map(Secret::expose);
            let response = self.transport.get(&Request { url: &url, bearer })?;
            match response.status {
                200 => return Ok(response),
                s if is_redirect(s) => {
                    if hop == MAX_REDIRECTS {
                        return Err(FetchError::TooManyRedirects);
                    }
                    let location = response.location.ok_or(FetchError::BadRedirect)?;
                    url = resolve_redirect(&url, &location, allow)?;
                }
                s => return Err(FetchError::Status(s)),
            }
        }
        Err(FetchError::TooManyRedirects)
    }

    /// The manifest, at most [`MANIFEST_MAX_BYTES`] bytes (the parser checks it again).
    pub fn fetch_manifest(&self, manifest_url: &str) -> Result<Vec<u8>, FetchError> {
        let response = self.get(manifest_url, manifest_url)?;
        let mut body = Vec::new();
        response
            .body
            .take(MANIFEST_MAX_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|_| FetchError::Transport)?;
        if body.len() > MANIFEST_MAX_BYTES {
            return Err(FetchError::TooLarge);
        }
        Ok(body)
    }

    /// Downloads `asset_url` (resolved against `manifest_url`) into the NEW file `dest` (0600), checking the size cap
    /// and the digest as it streams; on any failure `dest` does not exist afterwards. An existing `dest` is never
    /// touched (and no request is made). Returns the bytes written.
    pub fn download(
        &self,
        manifest_url: &str,
        asset_url: &str,
        want: &Digest,
        dest: &Path,
    ) -> Result<u64, FetchError> {
        let url = resolve_asset_url(manifest_url, asset_url, self.config.allow_http)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dest)
            .map_err(|e| FetchError::Io(e.kind()))?;
        let result = self.stream_into(&url, manifest_url, want, &mut file);
        drop(file);
        if result.is_err() {
            let _ = std::fs::remove_file(dest);
        }
        result
    }

    fn stream_into(
        &self,
        url: &str,
        manifest_url: &str,
        want: &Digest,
        file: &mut std::fs::File,
    ) -> Result<u64, FetchError> {
        let max = self.config.max_asset_bytes;
        let mut body = self
            .get(url, manifest_url)?
            .body
            .take(max.saturating_add(1));
        let (mut hasher, mut total, mut buf) = (sha2::Sha256::new(), 0u64, vec![0u8; 64 * 1024]);
        loop {
            let n = body.read(&mut buf).map_err(|_| FetchError::Transport)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > max {
                return Err(FetchError::TooLarge);
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .map_err(|e| FetchError::Io(e.kind()))?;
        }
        file.sync_all().map_err(|e| FetchError::Io(e.kind()))?;
        let got = Sha256(hasher.finalize().into());
        if !got.ct_eq(&Sha256(want.0)) {
            return Err(FetchError::DigestMismatch);
        }
        Ok(total)
    }
}
