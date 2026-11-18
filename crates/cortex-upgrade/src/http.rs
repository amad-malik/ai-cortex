//! The real HTTP client behind [`Transport`] (`ureq` 3, rustls with the ring provider only). It follows NO redirects
//! and turns NO status into an error: the redirect, origin, size and digest rules are ours, in `fetch`.

use crate::fetch::{FetchError, Request, Response, Transport};
use std::io::Read;
use std::time::Duration;

pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    /// HTTPS only. `timeout` bounds one whole request.
    pub fn new(timeout: Duration) -> Self {
        Self {
            agent: build(timeout, true),
        }
    }

    /// Test builds only: plain http allowed (loopback servers).
    #[cfg(feature = "test-hooks")]
    pub fn new_allowing_http(timeout: Duration) -> Self {
        Self {
            agent: build(timeout, false),
        }
    }
}

/// No redirects (ours to follow), no status turned into an error, one global timeout, https only unless told.
fn build(timeout: Duration, https_only: bool) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .https_only(https_only)
        .max_redirects(0)
        .http_status_as_error(false)
        .proxy(None) // the environment's HTTP_PROXY, HTTPS_PROXY and ALL_PROXY are NOT read (cox, 13:30Z)
        .timeout_global(Some(timeout))
        .user_agent(concat!("cortex-upgrade/", env!("CARGO_PKG_VERSION")))
        .build();
    ureq::Agent::new_with_config(config)
}

/// A refusal carries no text from the library or the server (a URL or a header value could be in it).
fn map(e: ureq::Error) -> FetchError {
    match e {
        ureq::Error::Timeout(_) => FetchError::Timeout,
        ureq::Error::RequireHttpsOnly(_) => FetchError::InsecureUrl,
        _ => FetchError::Transport,
    }
}

impl Transport for UreqTransport {
    fn get(&self, request: &Request<'_>) -> Result<Response, FetchError> {
        let mut call = self.agent.get(request.url);
        if let Some(token) = request.bearer {
            call = call.header("Authorization", format!("Bearer {token}"));
        }
        let response = call.call().map_err(map)?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        Ok(Response {
            status,
            location,
            body: Box::new(response.into_body().into_reader()),
        })
    }
}

/// The transport for the LOCAL server (health and `whoami`): the literal `http://127.0.0.1:<port>/<path>` only, no
/// redirects, no proxy from the environment, a timeout, and an answer of at most [`LOCAL_BODY_MAX_BYTES`] (a longer body
/// is an `InvalidData` error when read). The operator's token goes to this transport and to no other.
pub struct LocalTransport {
    agent: ureq::Agent,
}

/// The largest answer read from the local server.
pub const LOCAL_BODY_MAX_BYTES: u64 = 64 * 1024;

impl LocalTransport {
    pub fn new(timeout: Duration) -> Self {
        Self {
            agent: build(timeout, false),
        }
    }
}

/// `http://127.0.0.1:<1..65535>/<path>`: nothing else. No name, no other address, no IPv6, no userinfo, no query.
fn local_url_ok(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://127.0.0.1:") else {
        return false;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let port_ok = (1..=5).contains(&digits)
        && rest[..digits]
            .parse::<u32>()
            .is_ok_and(|p| (1..=65535).contains(&p));
    let path = &rest[digits..];
    port_ok && path.starts_with('/') && path.len() <= 256 && crate::fetch::segments_ok(&path[1..])
}

/// A body that errors, instead of truncating, when more than `left` bytes would be read.
struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            let mut probe = [0u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "the answer is larger than the limit",
                )),
            };
        }
        let room = usize::try_from(self.left)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let n = self.inner.read(&mut buf[..room])?;
        self.left -= n as u64;
        Ok(n)
    }
}

impl Transport for LocalTransport {
    fn get(&self, request: &Request<'_>) -> Result<Response, FetchError> {
        if !local_url_ok(request.url) {
            return Err(FetchError::BadUrl);
        }
        let mut call = self.agent.get(request.url);
        if let Some(token) = request.bearer {
            call = call.header("Authorization", format!("Bearer {token}"));
        }
        let response = call.call().map_err(map)?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = Capped {
            inner: response.into_body().into_reader(),
            left: LOCAL_BODY_MAX_BYTES,
        };
        Ok(Response {
            status,
            location,
            body: Box::new(body),
        })
    }
}
