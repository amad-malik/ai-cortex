//! The client: one `send` that adds the token, the key, the limits and the retry, and decodes the answer.
//!
//! What it never does: follow a redirect, read a proxy variable, retry anything but a 429, send the token to an open route,
//! print a token, or put one in an error. See `mvp/tests/cortex-client-and-cli.md`.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::endpoint::Endpoint;
use crate::error::{ClientError, Problem, TransportKind};
use crate::idempotency::IdempotencyKey;
use crate::request::{Auth, Method, Mutates, Request, encode_component};
use crate::retry::{RetryPolicy, Sleeper, ThreadSleeper};
use crate::token::{Token, TokenError};

/// The largest `x-max-body` of the twin (R-1): the default cap for a request body.
pub const DEFAULT_MAX_REQUEST_BYTES: usize = 9_437_184;
/// The cap on a response body (W1-43 Q-10): 16 MiB.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
/// The total timeout of one call (W1-43 Q-10): 30 s.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How much of an answer that is not a Problem is looked at, and how much of it is shown.
const SNIPPET_LOOKED_AT: usize = 4096;
const SNIPPET_SHOWN: usize = 200;
/// Longest header value kept from `Cortex-Request-Id` and `Cortex-Token-Expires`.
const HEADER_KEPT_MAX: usize = 64;

#[derive(Debug)]
pub struct ClientConfig {
    pub endpoint: Endpoint,
    pub token: Option<Token>,
    pub timeout: Duration,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub retry: RetryPolicy,
}

impl ClientConfig {
    pub fn new(endpoint: Endpoint, token: Option<Token>) -> ClientConfig {
        ClientConfig {
            endpoint,
            token,
            timeout: DEFAULT_TIMEOUT,
            max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            retry: RetryPolicy::default(),
        }
    }
}

/// A decoded success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub request_id: Option<String>,
    /// `Cortex-Idempotent-Replay: true`.
    pub replayed: bool,
    /// `Cortex-Token-Expires` as sent.
    pub token_expires: Option<String>,
    /// `Cortex-Token-Renew: due`.
    pub token_renew_due: bool,
}

impl Response {
    /// The body as JSON; anything else is a `Decode` error that names the request id (PR-07).
    pub fn json(&self) -> Result<serde_json::Value, ClientError> {
        serde_json::from_slice(&self.body).map_err(|_| ClientError::Decode {
            what: "the body is not JSON".to_owned(),
            request_id: self.request_id.clone(),
        })
    }
}

pub struct Client {
    agent: ureq::Agent,
    config: ClientConfig,
    sleeper: Arc<dyn Sleeper>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// What one attempt came to.
enum Attempt {
    Done(Result<Response, ClientError>),
    /// 429: the `Retry-After` value as sent and the decoded Problem, if the body was one.
    Busy {
        retry_after: Option<String>,
        last: Option<Box<Problem>>,
    },
}

/// The headers the client reads from every answer.
struct Seen {
    request_id: Option<String>,
    replayed: bool,
    token_expires: Option<String>,
    token_renew_due: bool,
    retry_after: Option<String>,
    content_type: Option<String>,
}

impl Client {
    pub fn new(config: ClientConfig) -> Client {
        Client::with_sleeper(config, Arc::new(ThreadSleeper))
    }

    pub fn with_sleeper(config: ClientConfig, sleeper: Arc<dyn Sleeper>) -> Client {
        // No redirects, no proxy (ureq 3 reads the proxy variables by default: config.rs `Proxy::try_from_env`), no
        // status-as-error, one total timeout, no encodings advertised.
        let agent_config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .timeout_global(Some(config.timeout))
            .user_agent(format!("cortex-client/{}", env!("CARGO_PKG_VERSION")))
            .accept("application/json")
            .accept_encoding("")
            .build();
        Client {
            agent: ureq::Agent::new_with_config(agent_config),
            config,
            sleeper,
        }
    }

    pub fn send(&self, request: &Request) -> Result<Response, ClientError> {
        if let Some(body) = &request.body
            && body.len() > self.config.max_request_bytes
        {
            return Err(ClientError::RequestTooLarge {
                size: body.len(),
                cap: self.config.max_request_bytes,
            });
        }
        let token = match request.auth {
            Auth::Token => Some(
                self.config
                    .token
                    .as_ref()
                    .ok_or(ClientError::Token(TokenError::NoSource))?,
            ),
            Auth::None => None,
        };
        // One key per logical operation, kept across every retry of it (R-11).
        let key = (request.mutates == Mutates::Yes)
            .then(|| request.key.clone().unwrap_or_else(IdempotencyKey::generate));
        let url = self.url(request);

        let policy = self.config.retry;
        let mut attempts = 0u32;
        let mut waited = Duration::ZERO;
        loop {
            attempts += 1;
            match self.attempt(request, &url, token, key.as_ref()) {
                Attempt::Done(result) => return result,
                Attempt::Busy { retry_after, last } => {
                    let wait = policy.wait_for(retry_after.as_deref());
                    if attempts >= policy.max_attempts || waited + wait > policy.max_total_wait {
                        return Err(ClientError::Busy {
                            attempts,
                            retry_after_secs: wait.as_secs(),
                            last,
                        });
                    }
                    self.sleeper.sleep(wait);
                    waited += wait;
                }
            }
        }
    }

    fn url(&self, request: &Request) -> String {
        let mut url = format!("{}{}", self.config.endpoint, request.path);
        for (i, (name, value)) in request.query.iter().enumerate() {
            url.push(if i == 0 { '?' } else { '&' });
            url.push_str(&encode_component(name));
            url.push('=');
            url.push_str(&encode_component(value));
        }
        url
    }

    fn addr(&self) -> String {
        std::net::SocketAddr::new(self.config.endpoint.ip(), self.config.endpoint.port())
            .to_string()
    }

    fn transport(&self, error: &ureq::Error, key: Option<&IdempotencyKey>) -> ClientError {
        use std::io::ErrorKind as K;
        let kind = match error {
            ureq::Error::Timeout(_) | ureq::Error::BodyStalled => TransportKind::Timeout,
            ureq::Error::ConnectionFailed => TransportKind::Refused,
            ureq::Error::Protocol(_) => TransportKind::Closed,
            ureq::Error::Io(e) => match e.kind() {
                K::ConnectionRefused => TransportKind::Refused,
                K::TimedOut | K::WouldBlock => TransportKind::Timeout,
                K::ConnectionReset | K::ConnectionAborted | K::UnexpectedEof | K::BrokenPipe => {
                    TransportKind::Closed
                }
                _ => TransportKind::Other,
            },
            _ => TransportKind::Other,
        };
        ClientError::Transport {
            kind,
            addr: self.addr(),
            key: key.map(|k| k.to_string()),
        }
    }

    fn attempt(
        &self,
        request: &Request,
        url: &str,
        token: Option<&Token>,
        key: Option<&IdempotencyKey>,
    ) -> Attempt {
        let build_failed = || {
            Attempt::Done(Err(ClientError::BadRequest(
                "the request cannot be built".to_owned(),
            )))
        };
        let mut builder = ureq::http::Request::builder()
            .method(request.method.as_str())
            .uri(url);
        if let Some(t) = token {
            builder = builder.header("Authorization", format!("Bearer {}", t.expose()));
        }
        if let Some(k) = key {
            builder = builder.header("Idempotency-Key", k.as_str());
        }
        let sent = match &request.body {
            Some(body) => match builder
                .header("Content-Type", "application/json")
                .body(body.clone())
            {
                Ok(r) => self.agent.run(r),
                Err(_) => return build_failed(),
            },
            None if matches!(request.method, Method::Post | Method::Put) => {
                match builder.body(Vec::<u8>::new()) {
                    Ok(r) => self.agent.run(r),
                    Err(_) => return build_failed(),
                }
            }
            None => match builder.body(()) {
                Ok(r) => self.agent.run(r),
                Err(_) => return build_failed(),
            },
        };
        let mut response = match sent {
            Ok(r) => r,
            Err(e) => return Attempt::Done(Err(self.transport(&e, key))),
        };

        let status = response.status().as_u16();
        let seen = read_headers(response.headers());
        if (300..400).contains(&status) {
            return Attempt::Done(Err(ClientError::UnexpectedRedirect { status }));
        }
        let cap = self.config.max_response_bytes;
        if response
            .body()
            .content_length()
            .is_some_and(|n| n > cap as u64)
        {
            return Attempt::Done(Err(ClientError::ResponseTooLarge { cap }));
        }
        // ureq's limit reader fails on the read AFTER `limit` bytes, so a body of exactly `cap` bytes would be refused at
        // `limit(cap)`: allow one byte more; the byte over the cap is what fails.
        let body = match response
            .body_mut()
            .with_config()
            .limit((cap as u64).saturating_add(1))
            .read_to_vec()
        {
            Ok(b) => b,
            Err(ureq::Error::BodyExceedsLimit(_)) => {
                return Attempt::Done(Err(ClientError::ResponseTooLarge { cap }));
            }
            Err(e) => return Attempt::Done(Err(self.transport(&e, key))),
        };

        if (200..300).contains(&status) {
            return Attempt::Done(Ok(Response {
                status,
                body,
                request_id: seen.request_id,
                replayed: seen.replayed,
                token_expires: seen.token_expires,
                token_renew_due: seen.token_renew_due,
            }));
        }
        let decoded = decode_problem(status, &seen, &body, token);
        if status == 429 {
            return Attempt::Busy {
                retry_after: seen.retry_after,
                last: decoded.ok().map(Box::new),
            };
        }
        Attempt::Done(Err(match decoded {
            Ok(problem) => ClientError::Problem(Box::new(problem)),
            Err(()) => ClientError::Http {
                status,
                snippet: snippet(&body, token),
                request_id: seen.request_id,
            },
        }))
    }
}

fn header_text(headers: &ureq::http::HeaderMap, name: &str, max: usize) -> Option<String> {
    let value = headers.get(name)?.as_bytes();
    if value.is_empty() || value.len() > max || !value.iter().all(|b| (0x20..=0x7e).contains(b)) {
        return None;
    }
    Some(String::from_utf8_lossy(value).into_owned())
}

fn read_headers(headers: &ureq::http::HeaderMap) -> Seen {
    Seen {
        request_id: header_text(headers, "cortex-request-id", HEADER_KEPT_MAX),
        replayed: headers
            .get("cortex-idempotent-replay")
            .is_some_and(|v| v.as_bytes() == b"true"),
        token_expires: header_text(headers, "cortex-token-expires", HEADER_KEPT_MAX),
        token_renew_due: headers
            .get("cortex-token-renew")
            .is_some_and(|v| v.as_bytes() == b"due"),
        retry_after: headers
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        content_type: headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
    }
}

/// `application/problem+json` with `type`, `title`, `status`, `fix`, `request_id` (R-13); `detail` and `scope` optional;
/// anything else is not a Problem. Every string is redacted against the token.
fn decode_problem(
    status: u16,
    seen: &Seen,
    body: &[u8],
    token: Option<&Token>,
) -> Result<Problem, ()> {
    let is_problem_json = seen.content_type.as_deref().is_some_and(|c| {
        c.split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/problem+json")
    });
    if !is_problem_json {
        return Err(());
    }
    let value: serde_json::Value = serde_json::from_slice(body).map_err(|_| ())?;
    let object = value.as_object().ok_or(())?;
    let text = |name: &str| {
        object
            .get(name)
            .and_then(|v| v.as_str())
            .map(|s| redact(s, token))
    };
    // `type` and `request_id` are machine-semantic: printable ASCII, 1 to 64 bytes, or the answer is not a Problem (cox, ellis F-1).
    let identifier = |name: &str| {
        let s = object.get(name)?.as_str()?;
        let ok = !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| (0x21..=0x7e).contains(&b));
        ok.then(|| redact(s, token))
    };
    let code = identifier("type").ok_or(())?;
    let title = text("title").ok_or(())?;
    let fix = text("fix").ok_or(())?;
    let request_id = identifier("request_id").ok_or(())?;
    let body_status = object.get("status").and_then(|v| v.as_u64()).ok_or(())?;
    let body_status = u16::try_from(body_status).map_err(|_| ())?;
    Ok(Problem {
        status,
        body_status: (body_status != status).then_some(body_status),
        code,
        title,
        detail: text("detail"),
        fix,
        request_id,
        scope: text("scope"),
        token_expires: seen.token_expires.clone(),
        token_renew_due: seen.token_renew_due,
    })
}

/// The first bytes of an answer that is not a Problem: redacted, printable ASCII only, at most 200 bytes.
fn snippet(body: &[u8], token: Option<&Token>) -> String {
    let looked_at = &body[..body.len().min(SNIPPET_LOOKED_AT)];
    let text = redact(&String::from_utf8_lossy(looked_at), token);
    let mut out = String::with_capacity(SNIPPET_SHOWN);
    for b in text.bytes().take(SNIPPET_SHOWN) {
        out.push(match b {
            0x20..=0x7e => b as char,
            b'\n' | b'\r' | b'\t' => ' ',
            _ => '?',
        });
    }
    out
}

/// Replace the token, and every run of 8 or more characters it shares with `text`, by `<token>`.
fn redact(text: &str, token: Option<&Token>) -> String {
    let Some(token) = token else {
        return text.to_owned();
    };
    let t = token.expose();
    let text = text.replace(t, "<token>");
    const WINDOW: usize = 8;
    if t.len() < WINDOW || text.len() < WINDOW {
        return text;
    }
    let windows: HashSet<&[u8]> = t.as_bytes().windows(WINDOW).collect();
    let bytes = text.as_bytes();
    let mut covered = vec![false; bytes.len()];
    for i in 0..=bytes.len() - WINDOW {
        if windows.contains(&bytes[i..i + WINDOW]) {
            covered[i..i + WINDOW].iter_mut().for_each(|c| *c = true);
        }
    }
    if !covered.iter().any(|c| *c) {
        return text;
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if covered[i] {
            while i < bytes.len() && covered[i] {
                i += 1;
            }
            out.push_str("<token>");
        } else {
            let start = i;
            while i < bytes.len() && !covered[i] {
                i += 1;
            }
            // A covered byte is ASCII, so every cut is on a character boundary.
            out.push_str(&text[start..i]);
        }
    }
    out
}
