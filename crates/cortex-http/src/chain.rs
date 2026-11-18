//! Steps 3 to 10 (C8) around S0's routes, and the whole router: the request id outermost, then the
//! normaliser, then each route's guard (token, body cap, scope), then a write's strict decode,
//! idempotency, handler, stored reply and commit in ONE transaction (C1).
use std::collections::{BTreeMap, BTreeSet};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{MethodFilter, MethodRouter, on};
use cortex_db::{Idem, StoredResponse};
use cortex_module::route::{AppState, AuthorizeFn, Ctx, DecodeFn, ExecuteFn, Handler, Routes};
use cortex_module::{ScopePolicy, TargetRule, TokenVerifier};
use cortex_types::{ModuleId, RequestId, ScopeName, SystemClock, Timestamp, WorkerId};
use futures_util::FutureExt;
use http_body_util::{BodyExt, LengthLimitError, Limited};
use tokio::sync::Semaphore;

use crate::bearer::{self, BearerError};
use crate::problem::{self, Refusal};
use crate::request_id::{self, RequestIds};
use crate::{idem, log, normalise, time};

/// Where the chain's log lines go (delta 4: the refusal log line). INTERIM(ponytail): stderr by
/// default, until cortex-server picks its logger (piece 11). A test reads the lines through
/// `LogSink::new`; a process-global sink would race between parallel tests.
#[derive(Clone)]
pub struct LogSink(Arc<dyn Fn(&str) + Send + Sync>);

impl LogSink {
    pub fn stderr() -> LogSink {
        LogSink(Arc::new(|line| eprintln!("{line}")))
    }

    pub fn new(emit: impl Fn(&str) + Send + Sync + 'static) -> LogSink {
        LogSink(Arc::new(emit))
    }

    pub fn emit(&self, line: &str) {
        (self.0)(line);
    }
}

/// The chain's global limits (Wave 1: item D, the in-flight cap, and item E, the request timeout).
///
/// INTERIM(ponytail): a global in-flight cap (tokio semaphore), replaced by M-8's per-worker limiter
/// in the release cox sets. The defaults are mine and are not spec: `cortex-server` (piece 11) sets
/// both from its config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Requests in flight at once, across every route (step 5). A request that cannot get a place
    /// WAITS for one; it is not refused. Must be at least 1.
    pub max_in_flight: usize,
    /// How long one request may take, from the start of its guard (the token check) to its answer,
    /// the wait for a place included (item E). Past it the answer is 503 `unavailable` and the work
    /// is dropped where it stands. Must be non-zero.
    pub request_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_in_flight: 128,
            request_timeout: Duration::from_secs(30),
        }
    }
}

/// The two auth ports the chain calls (S0 section 1.5), where its log lines go, and its limits.
#[derive(Clone)]
pub struct Chain {
    pub verifier: Arc<dyn TokenVerifier>,
    pub policy: Arc<dyn ScopePolicy>,
    pub log: LogSink,
    pub limits: Limits,
}

/// Why the router could not be built. The registry checks these too; the chain refuses rather
/// than panic if it is handed a table the registry did not check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainError {
    BadModule {
        module: &'static str,
    },
    BadScope {
        path: &'static str,
        scope: &'static str,
    },
    BadMethod {
        method: String,
        path: &'static str,
    },
    RouteTwice {
        method: String,
        path: &'static str,
    },
    /// `Limits` that would hang or refuse every request (a cap of 0, a zero timeout).
    BadLimits {
        what: &'static str,
    },
}

/// The header a replayed reply carries (signatures r2 section 4; Q3, pending cox).
pub const REPLAY_HEADER: &str = "cortex-idempotent-replay";

/// A route OUTSIDE the token check (S3 R-9): piece 11's three health routes. It sits inside the
/// request-id and normaliser layers, so its answers carry `Cortex-Request-Id` and its refusals are
/// problem+json; `Authorization` is never read for it (any value, valid, bad or absent, is ignored).
pub struct OpenRoute {
    pub method: Method,
    pub path: &'static str,
    pub handler: MethodRouter<AppState>,
}

/// The whole router with open routes as well (S3 R-9; cox's socket delta 2, row 50ee2c82).
pub fn router_with_open(
    chain: Chain,
    state: AppState,
    ids: RequestIds,
    pieces: Vec<(&'static str, Routes)>,
    open: Vec<OpenRoute>,
) -> Result<Router, ChainError> {
    build(chain, state, ids, pieces, open)
}

/// The whole router, for `cortex-server`: every enabled piece's routes behind the chain. `pieces`
/// is `Registry::take_routes()`: each piece's module id and its routes.
pub fn router(
    chain: Chain,
    state: AppState,
    ids: RequestIds,
    pieces: Vec<(&'static str, Routes)>,
) -> Result<Router, ChainError> {
    build(chain, state, ids, pieces, Vec::new())
}

fn build(
    chain: Chain,
    state: AppState,
    ids: RequestIds,
    pieces: Vec<(&'static str, Routes)>,
    open: Vec<OpenRoute>,
) -> Result<Router, ChainError> {
    let log_sink = chain.log.clone();
    // Limits that would hang (a cap of 0: every request waits for ever) or refuse everything (a zero
    // timeout) do not build.
    if chain.limits.max_in_flight > Semaphore::MAX_PERMITS {
        // `Semaphore::new` PANICS above this; a configured cap must be refused, not crash the build.
        return Err(ChainError::BadLimits {
            what: "max_in_flight is above the semaphore's maximum",
        });
    }
    if chain.limits.max_in_flight == 0 {
        return Err(ChainError::BadLimits {
            what: "max_in_flight is 0: every request would wait for ever",
        });
    }
    if chain.limits.request_timeout.is_zero() {
        return Err(ChainError::BadLimits {
            what: "request_timeout is zero: every request would time out",
        });
    }
    // INTERIM(ponytail): one global semaphore for step 5.
    let limiter = Arc::new(Semaphore::new(chain.limits.max_in_flight));
    let chain = Arc::new(chain);
    let mut seen = BTreeSet::new();
    let mut by_path: BTreeMap<&'static str, MethodRouter<AppState>> = BTreeMap::new();
    for (module, routes) in pieces {
        let module_id = ModuleId::parse(module).map_err(|_| ChainError::BadModule { module })?;
        for e in routes.into_entries() {
            if !seen.insert((e.method.to_string(), e.path)) {
                return Err(ChainError::RouteTwice {
                    method: e.method.to_string(),
                    path: e.path,
                });
            }
            let scope = ScopeName::parse(e.scope).map_err(|_| ChainError::BadScope {
                path: e.path,
                scope: e.scope,
            })?;
            let guard = Arc::new(Guard {
                chain: Arc::clone(&chain),
                scope,
                scope_text: e.scope,
                max_body: e.max_body,
                query: e.query,
                rule: e.target,
                path: e.path,
                limiter: Arc::clone(&limiter),
            });
            let endpoint: MethodRouter<AppState> = match e.handler {
                Handler::Read(mr) => *mr,
                Handler::Write {
                    decode,
                    authorize,
                    execute,
                } => {
                    let filter = MethodFilter::try_from(e.method.clone()).map_err(|_| {
                        ChainError::BadMethod {
                            method: e.method.to_string(),
                            path: e.path,
                        }
                    })?;
                    let w = Arc::new(Write {
                        module: module_id.clone(),
                        decode,
                        authorize,
                        execute,
                    });
                    on(filter, move |State(st): State<AppState>, req: Request| {
                        let w = Arc::clone(&w);
                        async move { write(st, w, req).await }
                    })
                }
            };
            let endpoint = endpoint.route_layer(middleware::from_fn_with_state(guard, guard_step));
            let merged = match by_path.remove(e.path) {
                Some(prev) => prev.merge(endpoint),
                None => endpoint,
            };
            by_path.insert(e.path, merged);
        }
    }
    // Open routes (S3 R-9): the same one-route-per-method-and-path check, no token check, and
    // Authorization removed before the handler, so it is never read there.
    for o in open {
        if !seen.insert((o.method.to_string(), o.path)) {
            return Err(ChainError::RouteTwice {
                method: o.method.to_string(),
                path: o.path,
            });
        }
        let endpoint = o
            .handler
            .route_layer(middleware::from_fn_with_state(o.path, open_step));
        let merged = match by_path.remove(o.path) {
            Some(prev) => prev.merge(endpoint),
            None => endpoint,
        };
        by_path.insert(o.path, merged);
    }
    let mut r: Router<AppState> = Router::new();
    for (path, endpoint) in by_path {
        r = r.route(path, endpoint);
    }
    // Outermost first: the request id, then the log layer (it reads the final response), then the
    // normaliser, then each route's guard.
    Ok(r.with_state(state)
        .layer(middleware::from_fn(normalise::layer))
        .layer(middleware::from_fn_with_state(log_sink, log::step))
        .layer(middleware::from_fn_with_state(ids, request_id::layer)))
}

/// The cause logged for a caught panic. The payload is never logged or sent: it can quote request
/// data, a token or a row. (The default panic hook still prints its message to the process's
/// stderr; `cortex-server`, piece 11, owns the hook.)
struct Panicked;

impl std::fmt::Debug for Panicked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("panic")
    }
}

/// The cause logged for a request that ran past `Limits::request_timeout`.
struct TimedOut;

impl std::fmt::Debug for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("request_timeout")
    }
}

/// An open route's only step: drop any `Authorization` header, so the handler cannot read it.
async fn open_step(State(path): State<&'static str>, mut req: Request, next: Next) -> Response {
    req.headers_mut().remove(AUTHORIZATION);
    let mut resp = next.run(req).await;
    log::facts_mut(&mut resp).route = Some(path);
    resp
}

/// One route's settings for steps 3 to 7.
struct Guard {
    chain: Arc<Chain>,
    scope: ScopeName,
    scope_text: &'static str,
    max_body: u64,
    /// The route declared `.query()` (S3 R-4; finn's default-deny marker).
    query: bool,
    /// The route's DECLARED target rule; the `Ctx` carries it so `ctx.authorize` takes no rule
    /// argument (delta 4, finn's addition 1).
    rule: TargetRule,
    /// The route template, for the log line (never the concrete path).
    path: &'static str,
    /// Step 5: the places a request takes (one semaphore for the whole router).
    limiter: Arc<Semaphore>,
}

fn request_id_of<B>(req: &axum::http::Request<B>) -> RequestId {
    req.extensions()
        .get::<RequestId>()
        .copied()
        .unwrap_or_else(|| RequestId::new(&SystemClock))
}

/// Steps 3 to 7 for one route: the token, the body under its cap (read before any parsing), the
/// scope. Then the handler (a read) or steps 8 to 10 (a write), with `Ctx` in the request.
async fn guard_step(State(g): State<Arc<Guard>>, req: Request, next: Next) -> Response {
    let id = request_id_of(&req);
    let mut authed: Option<Authed> = None;
    // H-19: a panic anywhere under the guard (the verifier, a handler, `authorize`, `execute`) is a
    // 500 `internal_error`, never the panic text, and the transaction it held is dropped (rolled
    // back) as the stack unwinds. The next request is served.
    // Item E: the request timeout wraps the whole guard, the token check and the wait for a place
    // included, so a stuck verifier or a full queue cannot hold a request for ever. When it passes,
    // the guard's future is DROPPED where it stands (a transaction it held is rolled back as it is
    // dropped) and the answer is 503 `unavailable`.
    let bounded = tokio::time::timeout(
        g.chain.limits.request_timeout,
        guard(&g, req, next, &mut authed),
    );
    let mut resp = match AssertUnwindSafe(bounded).catch_unwind().await {
        Ok(Err(_elapsed)) => {
            logged(problem::unavailable(), "timeout", &TimedOut).into_response_with(&id)
        }
        Ok(Ok(r)) => r,
        Err(_payload) => {
            logged(problem::internal_error(), "handler", &Panicked).into_response_with(&id)
        }
    };
    // HTTP-Q1, ruled (a) by cox (W1-25): EVERY answer to a token-authenticated request carries the
    // token headers. The panic and the timeout answers are made out here, outside the guard's own
    // `stamp`, so they are stamped from what the guard published before it was dropped. A timeout
    // during the token check published nothing, so it carries none (nobody is authenticated yet).
    if let Some(a) = &authed
        && resp.headers().get("cortex-token-expires").is_none()
    {
        token_headers(&mut resp, a.expires, a.renew_due);
    }
    // For the log line (delta 4): the template, and the worker once authentication succeeded.
    let facts = log::facts_mut(&mut resp);
    facts.route = Some(g.path);
    facts.who = authed.map(|a| a.worker);
    resp
}

/// What the guard learns from a successful `verify` and publishes outside its (bounded, unwindable)
/// future: the worker for the log line, and the token's expiry and renewal hint for the headers.
#[derive(Clone, Copy)]
struct Authed {
    worker: WorkerId,
    expires: Timestamp,
    renew_due: bool,
}

async fn guard(g: &Guard, req: Request, next: Next, authed: &mut Option<Authed>) -> Response {
    let id = request_id_of(&req);
    let (mut parts, body) = req.into_parts();
    // Step 3: authentication. The value is a `Secret` from here; the header is then removed, so no
    // handler sees a credential.
    let principal = match bearer::bearer(&parts.headers) {
        Err(BearerError::Missing) => return problem::token_missing().into_response_with(&id),
        Err(BearerError::Invalid) => return problem::token_invalid().into_response_with(&id),
        Ok(secret) => match g.chain.verifier.verify(&secret).await {
            Ok(p) => p,
            Err(e) => {
                return logged(problem::from_auth(&e), "verify", &e).into_response_with(&id);
            }
        },
    };
    parts.headers.remove(AUTHORIZATION);
    let (expires, renew) = (principal.expires_at, principal.renew_due);
    *authed = Some(Authed {
        worker: principal.worker,
        expires,
        renew_due: renew,
    });
    let stamp = move |mut r: Response| {
        token_headers(&mut r, expires, renew);
        r
    };
    // Step 4 (project admission) is reserved and does nothing in v0.2.001.
    // Step 5, the limiter. INTERIM(ponytail): a global in-flight cap (a tokio semaphore of
    // `Limits::max_in_flight` places), replaced by M-8's per-worker limiter in the release cox sets.
    // It comes AFTER verify (an unauthenticated request never waits for a place) and BEFORE the
    // body is read. A request that finds no place WAITS. The place is held until this function
    // returns, so it comes back on an answer, a refusal, a panic (the unwind drops it) and a
    // dropped request.
    let _place = match g.limiter.acquire().await {
        Ok(place) => place,
        Err(closed) => {
            return stamp(
                logged(problem::unavailable(), "limiter", &closed).into_response_with(&id),
            );
        }
    };
    // Step 6: the whole body under the route's cap, before any parsing (F1-26).
    let declared = parts
        .headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > g.max_body) {
        return stamp(problem::body_too_large(g.max_body).into_response_with(&id));
    }
    let cap = usize::try_from(g.max_body).unwrap_or(usize::MAX);
    let bytes = match Limited::new(body, cap).collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) if e.downcast_ref::<LengthLimitError>().is_some() => {
            return stamp(problem::body_too_large(g.max_body).into_response_with(&id));
        }
        Err(_) => {
            let r =
                problem::invalid_request(StatusCode::BAD_REQUEST, "The body could not be read.");
            return stamp(r.into_response_with(&id));
        }
    };
    // Step 7: the scope only (C7); target rules are the handler's.
    if !g.chain.policy.allows(&principal, &g.scope) {
        return stamp(problem::missing_scope(g.scope_text).into_response_with(&id));
    }
    // Step 8 starts here, with the query (S3 R-4, R-16): a route that did not declare `.query()`
    // takes no query string at all; a route that did reads it with S0's StrictQuery (422 itself).
    if !g.query && parts.uri.query().is_some() {
        let r = problem::invalid_request(
            StatusCode::UNPROCESSABLE_ENTITY,
            "This operation takes no query parameters.",
        );
        return stamp(r.into_response_with(&id));
    }
    // The route's declared rule and the policy go into the Ctx once per request (delta 4).
    parts
        .extensions
        .insert(Ctx::new(principal, id).with_route(g.rule, Arc::clone(&g.chain.policy)));
    stamp(
        next.run(Request::from_parts(parts, Body::from(bytes)))
            .await,
    )
}

/// A refusal of 500 or more records its step and the internal cause, for the log line (the log
/// layer writes it with the six fields every refusal has; signatures r2 section 3). Causes are
/// `DbError`, `IdemError`, `AuthError` or a problem code: kinds and codes, never row data or a
/// credential (CONTRACTS section 3). A 4xx records nothing: the line has no cause text for it.
fn logged(r: Refusal, step: &'static str, cause: &dyn std::fmt::Debug) -> Refusal {
    if r.status.as_u16() >= 500 {
        return Refusal {
            internal: Some(Box::new((step, format!("{cause:?}")))),
            ..r
        };
    }
    r
}

fn token_headers(r: &mut Response, expires: Timestamp, renew_due: bool) {
    if let Ok(v) = HeaderValue::from_str(&time::rfc3339(expires)) {
        r.headers_mut().insert("cortex-token-expires", v);
    }
    if renew_due {
        r.headers_mut()
            .insert("cortex-token-renew", HeaderValue::from_static("due"));
    }
}

/// One write route's piece and its erased handler.
struct Write {
    module: ModuleId,
    decode: DecodeFn,
    authorize: AuthorizeFn,
    execute: ExecuteFn,
}

/// A write handler's `Ctx`: the guard's, plus the route's matched path parameters, so the handler
/// can read `ctx.path_id::<T>("id")` (S3 R-5; finn's `Ctx::with_path`). `None` if the guard did not
/// run (the route was mounted around it).
async fn write_ctx(parts: &mut axum::http::request::Parts) -> Option<Ctx> {
    let ctx = parts.extensions.get::<Ctx>().cloned()?;
    // A rejection (bad UTF-8 in a segment) leaves the path empty, so every path_id is 404 (R-5).
    let params: Vec<(String, String)> =
        match axum::extract::RawPathParams::from_request_parts(parts, &()).await {
            Ok(raw) => raw
                .iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
            Err(_) => Vec::new(),
        };
    Some(ctx.with_path(params))
}

/// Steps 8 to 10 (C1, R-16, R-19): strict decoding, the key, then in ONE transaction: authorize
/// (step-0 422, 404, 403), the lock ledger must be empty, the idempotency lookup, the handler's
/// execute, the stored reply, commit. The lookup comes AFTER authorize, so a replay is never
/// returned to a caller whose target check does not pass now; it takes the erased proof, so no code
/// path in this function can skip authorize. An authorize refusal has reserved nothing.
async fn write(st: AppState, w: Arc<Write>, req: Request) -> Response {
    let id = request_id_of(&req);
    let (mut parts, body) = req.into_parts();
    let Some(ctx) = write_ctx(&mut parts).await else {
        // Mounted around the guard: refuse closed.
        let r = logged(
            problem::internal_error(),
            "ctx",
            &"no Ctx: the route bypassed the guard",
        );
        return r.into_response_with(&id);
    };
    // The guard read the body under its cap already; this only takes the buffered bytes.
    let bytes = match body.collect().await {
        Ok(c) => c.to_bytes(),
        Err(e) => {
            return logged(problem::internal_error(), "body", &e).into_response_with(&id);
        }
    };
    // Step 8: strict decoding (400 syntax, 422 shape), and the canonical hash for the lookup.
    let hash = match idem::body_hash(&bytes) {
        Ok((_, h)) => h,
        Err(r) => return r.into_response_with(&id),
    };
    let decoded = match (w.decode)(&bytes) {
        Ok(d) => d,
        Err(p) => return Refusal::from(&p).into_response_with(&id),
    };
    // The key (428 / 400) needs no database, so it is checked before any transaction (H-21). The
    // key is bound to the principal, the project, METHOD + concrete path and the hash.
    let key = match idem::key(&parts.headers) {
        Ok(k) => k,
        Err(r) => return r.into_response_with(&id),
    };
    let route = idem::route(&parts.method, parts.uri.path());
    let mut tx = match ctx.begin(&st.pool, &w.module).await {
        Ok(tx) => tx,
        Err(e) => {
            let cause = format!("{e:?}");
            return logged(problem::from_db(e), "begin", &cause).into_response_with(&id);
        }
    };
    // AUTHORIZE: the piece's reads and the route's declared target rule. Nothing is reserved yet.
    let authorization = match (w.authorize)(&mut tx, &ctx, &decoded).await {
        Ok(a) => a,
        Err(p) => {
            let _ = tx.rollback().await;
            return logged(Refusal::from(&p), "authorize", &p.code).into_response_with(&id);
        }
    };
    // `authorize` takes no advisory lock (finn's addition 2): a gate lock taken here would make the
    // lookup's Idempotency lock fail on the lock order. A breach is a bug in the piece: 500.
    if !tx.lock_ledger().steps().is_empty() {
        let cause = format!("authorize took a lock: {:?}", tx.lock_ledger().steps());
        let _ = tx.rollback().await;
        return logged(problem::internal_error(), "authorize", &cause).into_response_with(&id);
    }
    // LOOKUP: replay, 422 idempotency_key_reused, 409 already_issued, or a reservation. Only
    // `idem::lookup` reaches the stored reply, and it takes the proof.
    let token = match idem::lookup(&mut tx, &ctx, &key, &route, hash, &authorization).await {
        Ok(Idem::Proceed(t)) => t,
        Ok(Idem::Replay { status, response }) => {
            let _ = tx.rollback().await;
            return replay(status, response, &id);
        }
        Err(e) => {
            let _ = tx.rollback().await;
            let cause = format!("{e:?}");
            return logged(problem::from_idem(e), "idempotency.begin", &cause)
                .into_response_with(&id);
        }
    };
    // EXECUTE: the 409 checks and the write, in this transaction. It moves the proof.
    let reply = match (w.execute)(&mut tx, &ctx, authorization, decoded).await {
        Ok(r) => r,
        Err(p) => {
            let _ = tx.rollback().await;
            return logged(Refusal::from(&p), "handler", &p.code).into_response_with(&id);
        }
    };
    let stored = match reply.issued {
        Some(token_id) => StoredResponse::SecretNotStored { token_id },
        None => StoredResponse::Body(reply.body.clone()),
    };
    if let Err(e) = idem::finish(&mut tx, token, reply.status.code(), stored).await {
        let _ = tx.rollback().await;
        let cause = format!("{e:?}");
        return logged(problem::from_idem(e), "idempotency.finish", &cause).into_response_with(&id);
    }
    if let Err(e) = tx.commit().await {
        // Nothing was committed, so a retry with the same key runs again.
        return logged(problem::unavailable(), "commit", &e).into_response_with(&id);
    }
    json(reply.status.code(), &reply.body)
}

/// The stored reply, with `Cortex-Idempotent-Replay: true`. A secret marker never comes back as a
/// body: `begin` refuses it as 409 `already_issued` first, and this is the closed fallback.
fn replay(status: u16, response: StoredResponse, id: &RequestId) -> Response {
    match response {
        StoredResponse::Body(v) => {
            let mut r = json(status, &v);
            r.headers_mut()
                .insert(REPLAY_HEADER, HeaderValue::from_static("true"));
            r
        }
        StoredResponse::SecretNotStored { token_id } => {
            problem::from_idem(cortex_db::IdemError::AlreadyIssued { token_id })
                .into_response_with(id)
        }
    }
}

fn json(status: u16, body: &serde_json::Value) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    if status == StatusCode::NO_CONTENT {
        let mut r = Response::new(Body::empty());
        *r.status_mut() = status;
        return r;
    }
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"null".to_vec());
    let mut r = Response::new(Body::from(bytes));
    *r.status_mut() = status;
    r.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::rfc3339;
    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::{HeaderMap, Method, StatusCode};
    use bytes::Bytes;
    use cortex_db::{DbConfig, DbEndpoint, DbPool, ModuleRegistry};
    use cortex_module::route::{Ctx, Problem, ProblemSpec, Reply, StrictQuery};
    use cortex_module::{AuthError, Principal, Role, Target, TargetRule, WorkerKind};
    use cortex_types::{
        HandoffId, ProjectId, ScopeName, Secret, SystemClock, Timestamp, TokenId, WorkerId,
    };
    use serde::Deserialize;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Semaphore;
    use tower::ServiceExt;

    // Synthetic credentials only; none is a real token.
    const A_TOKEN: &str = "ctx2_test.valid-a";
    const RENEW_TOKEN: &str = "ctx2_test.renew-a";
    const NOSCOPE_TOKEN: &str = "ctx2_test.noscope-a";
    /// Wave 1 item E: a token whose verification never answers (a stuck database lookup).
    const HANG_TOKEN: &str = "ctx2_test.hang";
    const ALL_TOKENS: [&str; 3] = [A_TOKEN, RENEW_TOKEN, NOSCOPE_TOKEN];

    fn a() -> ProjectId {
        ProjectId::parse("0193e6a0-0000-7000-8000-00000000000a").expect("v7")
    }
    fn b() -> ProjectId {
        ProjectId::parse("0193e6a0-0000-7000-8000-00000000000b").expect("v7")
    }
    fn w1() -> WorkerId {
        WorkerId::parse("0193e6a0-0000-7000-8000-000000000001").expect("v7")
    }
    fn w2() -> WorkerId {
        WorkerId::parse("0193e6a0-0000-7000-8000-000000000002").expect("v7")
    }
    const EXPIRES: i64 = 1_700_000_000_000_000;

    fn principal(worker: WorkerId, renew_due: bool) -> Principal {
        Principal {
            worker,
            project: a(),
            kind: WorkerKind::Worker,
            role: Role::Member(None),
            token: TokenId::parse("0193e6a0-0000-7000-8000-0000000000aa").expect("v7"),
            expires_at: Timestamp::from_unix_micros(EXPIRES),
            renew_due,
        }
    }

    struct Verifier;
    #[async_trait]
    impl TokenVerifier for Verifier {
        async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
            match bearer.expose() {
                A_TOKEN => Ok(principal(w1(), false)),
                RENEW_TOKEN => Ok(principal(w1(), true)),
                NOSCOPE_TOKEN => Ok(principal(w2(), false)),
                HANG_TOKEN => std::future::pending().await,
                "ctx2_malformed" => Err(AuthError::Malformed),
                "ctx2_test.expired" => Err(AuthError::Expired),
                "ctx2_test.revoked" => Err(AuthError::Revoked),
                "ctx2_test.retired" => Err(AuthError::Retired),
                "ctx2_test.down" => Err(AuthError::Unavailable),
                _ => Err(AuthError::Unknown),
            }
        }
    }

    /// w1 holds both probe scopes; w2 holds none.
    struct Policy;
    impl ScopePolicy for Policy {
        fn allows(&self, p: &Principal, scope: &ScopeName) -> bool {
            p.worker == w1() && matches!(scope.as_str(), "probe:read" | "probe:write")
        }
        // The real policy answers true for rule None; a rule-gated route is refused (403).
        fn target(&self, _p: &Principal, rule: TargetRule, _t: &Target) -> bool {
            matches!(rule, TargetRule::None)
        }
    }

    /// Wave 1 item C (H-19): a handler that panics. The canary is what must never reach the client or
    /// the log.
    async fn boom() -> &'static str {
        panic!("canary_h19_do_not_leak")
    }

    const CONFLICT: ProblemSpec = ProblemSpec {
        code: "probe_conflict",
        status: 409,
        title: "Probe conflict",
        fix: "Change the request.",
    };

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Touch {
        n: i64,
    }

    fn probe() -> Routes {
        Routes::new()
            .read(
                Method::GET,
                "/v2/probe/{id}",
                "probe:read",
                TargetRule::None,
                // What a handler can see: its Ctx's project, and whether a credential reached it.
                |ctx: Ctx, h: HeaderMap| async move {
                    format!("{}|{}", ctx.project(), h.contains_key("authorization"))
                },
            )
            .write_open(
                Method::POST,
                "/v2/probe/{id}/touch",
                "probe:write",
                |_tx, _ctx, b: Touch| Box::pin(async move { Ok(Reply::created(b.n)) }),
            )
            .max_body(64)
            // R-4: the one route that declares a query reads it strictly.
            .read(
                Method::GET,
                "/v2/probes",
                "probe:read",
                TargetRule::None,
                |StrictQuery(q): StrictQuery<ListQ>| async move { q.limit.to_string() },
            )
            .query()
            // R-5: a read handler's path id; malformed or unknown is 404.
            .read(
                Method::GET,
                "/v2/probe/item/{id}",
                "probe:read",
                TargetRule::None,
                |ctx: Ctx| async move { ctx.path_id::<HandoffId>("id").map(|i| i.to_string()) },
            )
            // Delta 4 (the refusal log line): a rule-gated read the policy refuses (403
            // target_not_allowed), and a read whose handler answers a 409.
            .read(
                Method::GET,
                "/v2/targets/{id}",
                "probe:read",
                TargetRule::SelfWorker,
                |ctx: Ctx| async move { ctx.authorize(&Target::default(), ()).map(|_| "ok") },
            )
            .read(
                Method::GET,
                "/v2/boom",
                "probe:read",
                TargetRule::None,
                boom,
            )
            .read(
                Method::GET,
                "/v2/conflicts",
                "probe:read",
                TargetRule::None,
                || async { Err::<&'static str, Problem>(Problem::new(&CONFLICT, "It conflicts.")) },
            )
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ListQ {
        limit: u32,
    }

    const MODULE: &str = "probe";

    fn module() -> cortex_types::ModuleId {
        cortex_types::ModuleId::parse(MODULE).expect("a module id")
    }

    /// A pool that never connects: nothing listens on port 9. A test that reaches the database
    /// gets the chain's 503, which is what it asserts.
    fn state() -> AppState {
        let cfg = DbConfig {
            endpoint: DbEndpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 9,
            },
            database: "none".into(),
            app_user: "none".into(),
            app_password: Secret::new("none"),
            max_connections: 1,
            statement_cache: false,
            registry: ModuleRegistry::new([module()]),
            clock: Arc::new(SystemClock),
        };
        AppState {
            pool: Arc::new(DbPool::connect_lazy(&cfg)),
        }
    }

    fn chain_with(log: LogSink) -> Chain {
        Chain {
            verifier: Arc::new(Verifier),
            policy: Arc::new(Policy),
            log,
            limits: Limits::default(),
        }
    }

    fn chain() -> Chain {
        chain_with(LogSink::stderr())
    }

    fn ids() -> RequestIds {
        RequestIds {
            clock: Arc::new(SystemClock),
        }
    }

    fn app() -> Router {
        router(chain(), state(), ids(), vec![(MODULE, probe())]).expect("a router")
    }

    struct Answer {
        status: StatusCode,
        headers: HeaderMap,
        text: String,
        json: serde_json::Value,
    }

    async fn send(
        method: Method,
        path: &str,
        auth: &[&str],
        extra: &[(&str, &str)],
        body: Body,
    ) -> Answer {
        send_to(app(), method, path, auth, extra, body).await
    }

    async fn send_to(
        app: Router,
        method: Method,
        path: &str,
        auth: &[&str],
        extra: &[(&str, &str)],
        body: Body,
    ) -> Answer {
        let mut req = Request::builder().method(method).uri(path);
        for v in auth {
            req = req.header("authorization", *v);
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let resp = app
            .oneshot(req.body(body).expect("a request"))
            .await
            .expect("a response");
        let (parts, body) = resp.into_parts();
        let text = String::from_utf8(to_bytes(body, 1 << 20).await.expect("a body").to_vec())
            .expect("UTF-8");
        let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        // H-10 on every answer: no synthetic credential in the body or in any header.
        for t in ALL_TOKENS {
            assert!(!text.contains(t), "a token reached the body: {text}");
            for v in parts.headers.values() {
                assert!(
                    !v.to_str().unwrap_or("").contains(t),
                    "a token reached a header"
                );
            }
        }
        // H-04 on every answer.
        let rid = parts
            .headers
            .get("cortex-request-id")
            .expect("Cortex-Request-Id")
            .to_str()
            .expect("ASCII")
            .to_owned();
        if parts.status.as_u16() >= 400 {
            assert_eq!(
                parts.headers["content-type"], "application/problem+json",
                "{text}"
            );
            assert_eq!(json["request_id"], rid, "{text}");
        }
        Answer {
            status: parts.status,
            headers: parts.headers,
            text,
            json,
        }
    }

    fn bearer(t: &str) -> String {
        format!("Bearer {t}")
    }

    async fn get(auth: &[&str], extra: &[(&str, &str)]) -> Answer {
        send(
            Method::GET,
            "/v2/probe/0193e6a0-0000-7000-8000-0000000000c1",
            auth,
            extra,
            Body::empty(),
        )
        .await
    }

    async fn touch(auth: &[&str], extra: &[(&str, &str)], body: Body) -> Answer {
        send(
            Method::POST,
            "/v2/probe/0193e6a0-0000-7000-8000-0000000000c1/touch",
            auth,
            extra,
            body,
        )
        .await
    }

    fn without_id(a: &Answer) -> String {
        a.text
            .replace(a.json["request_id"].as_str().unwrap_or(""), "ID")
    }

    #[tokio::test]
    async fn h07_no_token_is_401_token_missing() {
        let r = get(&[], &[]).await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (401, Some("token_missing"))
        );
        assert_eq!(r.headers["www-authenticate"], "Bearer");
        assert_eq!(r.headers["cache-control"], "no-store");
    }

    #[tokio::test]
    async fn h08_every_bad_token_gets_one_identical_body() {
        let mut bodies = Vec::new();
        for auth in [
            vec!["Basic x".to_owned()],
            vec!["Bearer a b".to_owned()],
            vec![bearer(A_TOKEN), bearer(A_TOKEN)],
            vec![bearer("ctx2_malformed")],
            vec![bearer("ctx2_test.unknown")],
        ] {
            let auth: Vec<&str> = auth.iter().map(String::as_str).collect();
            let r = get(&auth, &[]).await;
            assert_eq!(
                (r.status.as_u16(), r.json["type"].as_str()),
                (401, Some("token_invalid")),
                "{auth:?}"
            );
            bodies.push(without_id(&r));
        }
        assert!(bodies.windows(2).all(|w| w[0] == w[1]), "{bodies:#?}");
    }

    #[tokio::test]
    async fn h09_a_token_that_cannot_be_used_says_why_and_a_down_verifier_is_503() {
        for (t, status, code) in [
            ("ctx2_test.expired", 401, "token_expired"),
            ("ctx2_test.revoked", 401, "token_revoked"),
            ("ctx2_test.retired", 401, "worker_retired"),
            ("ctx2_test.down", 503, "unavailable"),
        ] {
            let r = get(&[&bearer(t)], &[]).await;
            assert_eq!(
                (r.status.as_u16(), r.json["type"].as_str()),
                (status, Some(code)),
                "{t}"
            );
        }
    }

    #[tokio::test]
    async fn h05_token_headers_on_an_authenticated_answer() {
        let r = get(&[&bearer(A_TOKEN)], &[]).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text);
        assert_eq!(
            r.headers["cortex-token-expires"],
            rfc3339(Timestamp::from_unix_micros(EXPIRES)).as_str()
        );
        assert!(r.headers.get("cortex-token-renew").is_none());
        let due = get(&[&bearer(RENEW_TOKEN)], &[]).await;
        assert_eq!(due.headers["cortex-token-renew"], "due");
    }

    #[tokio::test]
    async fn h11_identity_comes_from_the_token_and_no_credential_reaches_a_handler() {
        let r = get(
            &[&bearer(A_TOKEN)],
            &[
                ("x-project", &b().to_string()),
                ("x-agent-name", "someone-in-b"),
            ],
        )
        .await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.text, format!("{}|false", a()));
    }

    #[tokio::test]
    async fn h12_the_earliest_refusal_wins() {
        let big = Body::from(vec![b' '; 100]);
        assert_eq!(
            touch(&[], &[], big).await.status.as_u16(),
            401,
            "token before size"
        );
        let big = Body::from(vec![b' '; 100]);
        assert_eq!(
            touch(&[&bearer(NOSCOPE_TOKEN)], &[], big)
                .await
                .status
                .as_u16(),
            413,
            "size before scope"
        );
        let bad = touch(&[&bearer(NOSCOPE_TOKEN)], &[], Body::from("{")).await;
        assert_eq!(bad.status.as_u16(), 403, "scope before decoding");
        assert_eq!(bad.json["type"], "missing_scope");
        let unknown = touch(&[&bearer(A_TOKEN)], &[], Body::from(r#"{"n":1,"x":2}"#)).await;
        assert_eq!(
            (unknown.status.as_u16(), unknown.json["type"].as_str()),
            (422, Some("invalid_request")),
            "decoding before the key"
        );
        let nokey = touch(&[&bearer(A_TOKEN)], &[], Body::from(r#"{"n":1}"#)).await;
        assert_eq!(
            (nokey.status.as_u16(), nokey.json["type"].as_str()),
            (428, Some("idempotency_key_required"))
        );
    }

    fn padded(n: usize, json: &str) -> Vec<u8> {
        let mut v = json.as_bytes().to_vec();
        v.resize(n, b' ');
        v
    }

    /// A body with no length: ten-byte chunks from a stream, so only reading can find the size.
    fn chunked(bytes: Vec<u8>) -> Body {
        let chunks: Vec<Result<bytes::Bytes, std::io::Error>> = bytes
            .chunks(10)
            .map(|c| Ok(bytes::Bytes::copy_from_slice(c)))
            .collect();
        Body::from_stream(futures_util::stream::iter(chunks))
    }

    #[tokio::test]
    async fn h13_the_cap_holds_with_and_without_a_length() {
        let at = |n| padded(n, r#"{"n":1}"#);
        let t = [bearer(A_TOKEN)];
        let t: Vec<&str> = t.iter().map(String::as_str).collect();
        assert_eq!(
            touch(&t, &[], Body::from(at(64))).await.status.as_u16(),
            428,
            "exactly the cap passes it"
        );
        assert_eq!(
            touch(&t, &[], Body::from(at(65))).await.status.as_u16(),
            413
        );
        assert_eq!(
            touch(&t, &[], chunked(at(64))).await.status.as_u16(),
            428,
            "chunked, at the cap"
        );
        assert_eq!(
            touch(&t, &[], chunked(at(65))).await.status.as_u16(),
            413,
            "chunked, over the cap"
        );
        let garbage = Body::from(vec![b'{'; 65]);
        assert_eq!(
            touch(&t, &[], garbage).await.status.as_u16(),
            413,
            "over the cap before any parsing"
        );
    }

    #[tokio::test]
    async fn h21_a_bad_key_is_400_before_any_transaction() {
        let r = touch(
            &[&bearer(A_TOKEN)],
            &[("idempotency-key", "short")],
            Body::from(r#"{"n":1}"#),
        )
        .await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (400, Some("invalid_request"))
        );
    }

    #[tokio::test]
    async fn h17_a_database_that_cannot_be_reached_is_503_unavailable() {
        let key = [("idempotency-key", "0193e6a0-key-0001")];
        let r = touch(&[&bearer(A_TOKEN)], &key, Body::from(r#"{"n":1}"#)).await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (503, Some("unavailable")),
            "{}",
            r.text
        );
        assert!(r.json.get("scope").is_none());
    }

    #[tokio::test]
    async fn h17_unknown_path_and_wrong_method() {
        let r = send(Method::GET, "/v2/nowhere", &[], &[], Body::empty()).await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (404, Some("not_found"))
        );
        let m = send(
            Method::GET,
            "/v2/probe/0193e6a0-0000-7000-8000-0000000000c1/touch",
            &[],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(
            (m.status.as_u16(), m.json["type"].as_str()),
            (405, Some("invalid_request"))
        );
        assert!(m.headers.contains_key("allow"));
    }

    #[test]
    fn h02_two_pieces_on_one_route_or_a_bad_scope_refuse_to_build() {
        let twice = router(
            chain(),
            state_blocking(),
            ids(),
            vec![(MODULE, probe()), (MODULE, probe())],
        );
        assert!(
            matches!(twice, Err(ChainError::RouteTwice { .. })),
            "{:?}",
            twice.err()
        );
        let bad = Routes::new().read(
            Method::GET,
            "/v2/x",
            "Not A Scope",
            TargetRule::None,
            || async { "x" },
        );
        let r = router(chain(), state_blocking(), ids(), vec![(MODULE, bad)]);
        assert!(
            matches!(r, Err(ChainError::BadScope { .. })),
            "{:?}",
            r.err()
        );
    }

    /// `connect_lazy` may spawn pool tasks, so it is built inside a runtime even for a sync test.
    fn state_blocking() -> AppState {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(async { state() })
    }

    /// Piece 11's health routes as the chain would get them: live echoes whether a credential
    /// reached it; ready refuses with a plain 503, which the normaliser must turn into a problem.
    fn health() -> Vec<OpenRoute> {
        vec![
            OpenRoute {
                method: Method::GET,
                path: "/v2/health/live",
                handler: axum::routing::get(|h: HeaderMap| async move {
                    format!("live|{}", h.contains_key("authorization"))
                }),
            },
            OpenRoute {
                method: Method::GET,
                path: "/v2/health/ready",
                handler: axum::routing::get(|| async {
                    (StatusCode::SERVICE_UNAVAILABLE, "db down")
                }),
            },
        ]
    }

    fn open_app(open: Vec<OpenRoute>) -> Result<Router, ChainError> {
        router_with_open(chain(), state(), ids(), vec![(MODULE, probe())], open)
    }

    #[tokio::test]
    async fn r9_open_routes_answer_without_a_token_and_never_see_authorization() {
        let app = open_app(health()).expect("a router");
        for auth in [
            vec![],
            vec!["Bearer ctx2_test.unknown".to_owned()],
            vec!["Basic x".to_owned()],
            vec![bearer(A_TOKEN)],
        ] {
            let auth: Vec<&str> = auth.iter().map(String::as_str).collect();
            let r = send_to(
                app.clone(),
                Method::GET,
                "/v2/health/live",
                &auth,
                &[],
                Body::empty(),
            )
            .await;
            assert_eq!(
                (r.status, r.text.as_str()),
                (StatusCode::OK, "live|false"),
                "{auth:?}"
            );
            assert!(
                r.headers.get("cortex-token-expires").is_none(),
                "no token was checked"
            );
        }
        // The guarded routes are unchanged beside them.
        let guarded = send_to(
            app,
            Method::GET,
            "/v2/probe/0193e6a0-0000-7000-8000-0000000000c1",
            &[],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(guarded.status.as_u16(), 401);
    }

    #[tokio::test]
    async fn r9_an_open_routes_refusal_is_a_problem_with_a_request_id() {
        let app = open_app(health()).expect("a router");
        let r = send_to(
            app,
            Method::GET,
            "/v2/health/ready",
            &["Bearer ctx2_test.unknown"],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (503, Some("unavailable")),
            "{}",
            r.text
        );
        assert!(
            !r.text.contains("db down"),
            "the handler's text is not sent"
        );
    }

    #[test]
    fn r9_an_open_route_on_a_piece_routes_method_and_path_refuses_to_build() {
        let clash = vec![OpenRoute {
            method: Method::GET,
            path: "/v2/probe/{id}",
            handler: axum::routing::get(|| async { "x" }),
        }];
        let built = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(async { open_app(clash) });
        assert!(
            matches!(built, Err(ChainError::RouteTwice { .. })),
            "{:?}",
            built.err()
        );
    }

    #[tokio::test]
    async fn r9_an_open_route_and_a_piece_route_share_a_path_by_method() {
        let open = vec![OpenRoute {
            method: Method::GET,
            path: "/v2/probe/{id}/touch",
            handler: axum::routing::get(|| async { "open" }),
        }];
        let app = open_app(open).expect("one path, two methods");
        let path = "/v2/probe/0193e6a0-0000-7000-8000-0000000000c1/touch";
        let get = send_to(app.clone(), Method::GET, path, &[], &[], Body::empty()).await;
        assert_eq!((get.status, get.text.as_str()), (StatusCode::OK, "open"));
        let post = send_to(app, Method::POST, path, &[], &[], Body::from(r#"{"n":1}"#)).await;
        assert_eq!(
            post.status.as_u16(),
            401,
            "the piece's write is still guarded"
        );
    }

    #[tokio::test]
    async fn r4_a_query_string_on_a_route_without_query_is_422_after_the_scope_check() {
        let obj = "/v2/probe/0193e6a0-0000-7000-8000-0000000000c1";
        let r = send(
            Method::GET,
            &format!("{obj}?x=1"),
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (422, Some("invalid_request")),
            "{}",
            r.text
        );
        let scope_first = send(
            Method::GET,
            &format!("{obj}?x=1"),
            &[&bearer(NOSCOPE_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(
            scope_first.status.as_u16(),
            403,
            "the scope check comes first"
        );
        let before_key = send(
            Method::POST,
            &format!("{obj}/touch?x=1"),
            &[&bearer(A_TOKEN)],
            &[],
            Body::from(r#"{"n":1}"#),
        )
        .await;
        assert_eq!(
            (before_key.status.as_u16(), before_key.json["type"].as_str()),
            (422, Some("invalid_request")),
            "before the 428"
        );
        let plain = send(Method::GET, obj, &[&bearer(A_TOKEN)], &[], Body::empty()).await;
        assert_eq!(plain.status, StatusCode::OK);
    }

    #[tokio::test]
    async fn r4_a_route_that_declares_a_query_reads_it_strictly() {
        let ok = send(
            Method::GET,
            "/v2/probes?limit=5",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!((ok.status, ok.text.as_str()), (StatusCode::OK, "5"));
        for q in ["bogus=1", "limit=x", "limit=5&bogus=1"] {
            let r = send(
                Method::GET,
                &format!("/v2/probes?{q}"),
                &[&bearer(A_TOKEN)],
                &[],
                Body::empty(),
            )
            .await;
            assert_eq!(
                (r.status.as_u16(), r.json["type"].as_str()),
                (422, Some("invalid_request")),
                "{q}: {}",
                r.text
            );
        }
    }

    #[tokio::test]
    async fn r5_a_malformed_path_id_is_the_same_404_as_an_unknown_one() {
        let good = "0193e6a0-0000-7000-8000-0000000000d1";
        let ok = send(
            Method::GET,
            &format!("/v2/probe/item/{good}"),
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!((ok.status, ok.text.as_str()), (StatusCode::OK, good));
        let mut bodies = Vec::new();
        for bad in [
            "abc",
            "0193e6a0-0000-4000-8000-0000000000d1",
            "0193E6A0-0000-7000-8000-0000000000D1",
        ] {
            let r = send(
                Method::GET,
                &format!("/v2/probe/item/{bad}"),
                &[&bearer(A_TOKEN)],
                &[],
                Body::empty(),
            )
            .await;
            assert_eq!(
                (r.status.as_u16(), r.json["type"].as_str()),
                (404, Some("not_found")),
                "{bad}"
            );
            bodies.push(without_id(&r));
        }
        assert!(
            bodies.windows(2).all(|w| w[0] == w[1]),
            "one body for every malformed id"
        );
    }

    /// R-5 for writes, without a database: `write_ctx` gives the handler its matched path id.
    #[tokio::test]
    async fn r5_a_write_handlers_ctx_carries_its_path_ids() {
        let ctx = Ctx::new(
            principal(w1(), false),
            cortex_types::RequestId::new(&SystemClock),
        );
        let app: Router = Router::new()
            .route(
                "/v2/x/{id}/mark",
                axum::routing::post(|req: Request| async move {
                    let (mut parts, _) = req.into_parts();
                    match write_ctx(&mut parts).await {
                        Some(c) => match c.path_id::<HandoffId>("id") {
                            Ok(i) => i.to_string(),
                            Err(p) => format!("{} {}", p.status, p.code),
                        },
                        None => "no ctx".to_owned(),
                    }
                }),
            )
            .layer(axum::Extension(ctx));
        let call = |path: &'static str| {
            let app = app.clone();
            async move {
                let resp = app
                    .oneshot(
                        Request::builder()
                            .method(Method::POST)
                            .uri(path)
                            .body(Body::empty())
                            .expect("a request"),
                    )
                    .await
                    .expect("a response");
                String::from_utf8(
                    to_bytes(resp.into_body(), 1 << 16)
                        .await
                        .expect("a body")
                        .to_vec(),
                )
                .expect("UTF-8")
            }
        };
        assert_eq!(
            call("/v2/x/0193e6a0-0000-7000-8000-0000000000d2/mark").await,
            "0193e6a0-0000-7000-8000-0000000000d2"
        );
        assert_eq!(
            call("/v2/x/abc/mark").await,
            "404 not_found",
            "malformed is the same 404"
        );
    }

    // ---- Delta 4: the refusal log line (kai's condition for unaudited refusals; DB-free) --------
    //
    // One line per refusal of 400 or more, whatever step made it: request id, principal ("-" until
    // authentication succeeded), method, the matched ROUTE TEMPLATE ("(unmatched)" for the 404
    // fallback, never the raw path), status and code. Never a header value, a token, a body, a
    // query string or a cause for a 4xx. A 5xx adds ` step=<step> cause=<cause>` after the six.

    type Lines = Arc<std::sync::Mutex<Vec<String>>>;

    fn logged_app() -> (Router, Lines) {
        let lines: Lines = Arc::default();
        let into = Arc::clone(&lines);
        let sink = LogSink::new(move |l| into.lock().expect("lock").push(l.to_owned()));
        let app =
            router(chain_with(sink), state(), ids(), vec![(MODULE, probe())]).expect("a router");
        (app, lines)
    }

    const LOG_KEYS: [&str; 6] = [
        "request_id",
        "principal",
        "method",
        "path",
        "status",
        "code",
    ];

    /// The first six `key=value` fields of a line, in order.
    fn six_fields(line: &str) -> Vec<(String, String)> {
        line.strip_prefix("cortex-http: ")
            .unwrap_or(line)
            .split(' ')
            .take(6)
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    fn val<'a>(f: &'a [(String, String)], k: &str) -> &'a str {
        f.iter()
            .find(|(key, _)| key == k)
            .map_or("<absent>", |(_, v)| v.as_str())
    }

    /// Sends one request through a logging router; asserts exactly one well-formed line, that it
    /// names this answer's request id, status and code, and returns the answer and the fields.
    async fn refused(
        method: Method,
        path: &str,
        auth: &[&str],
        extra: &[(&str, &str)],
        body: Body,
    ) -> (Answer, Vec<(String, String)>) {
        let (app, lines) = logged_app();
        let r = send_to(app, method, path, auth, extra, body).await;
        let g = lines.lock().expect("lock").clone();
        assert_eq!(
            g.len(),
            1,
            "exactly one log line per refusal: {g:?} for {}",
            r.text
        );
        let f = six_fields(&g[0]);
        let keys: Vec<&str> = f.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, LOG_KEYS, "the six fields, in this order: {}", g[0]);
        assert_eq!(
            val(&f, "request_id"),
            r.json["request_id"].as_str().unwrap_or("")
        );
        assert_eq!(val(&f, "status"), r.status.as_u16().to_string());
        assert_eq!(val(&f, "code"), r.json["type"].as_str().unwrap_or(""));
        (r, f)
    }

    const OBJ: &str = "/v2/probe/0193e6a0-0000-7000-8000-0000000000c1";
    const W1: &str = "0193e6a0-0000-7000-8000-000000000001";
    const W2: &str = "0193e6a0-0000-7000-8000-000000000002";

    #[tokio::test]
    async fn d4_log_401_a_missing_and_a_bad_token_are_logged_without_a_principal() {
        let (_, f) = refused(Method::GET, OBJ, &[], &[], Body::empty()).await;
        assert_eq!(
            (
                val(&f, "principal"),
                val(&f, "method"),
                val(&f, "path"),
                val(&f, "status"),
                val(&f, "code")
            ),
            ("-", "GET", "/v2/probe/{id}", "401", "token_missing")
        );
        let (_, f) = refused(
            Method::GET,
            OBJ,
            &[&bearer("ctx2_nobody")],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!((val(&f, "principal"), val(&f, "status")), ("-", "401"));
    }

    #[tokio::test]
    async fn d4_log_403_a_missing_scope_and_a_refused_target_name_the_worker() {
        let (_, f) = refused(
            Method::GET,
            OBJ,
            &[&bearer(NOSCOPE_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(
            (
                val(&f, "principal"),
                val(&f, "path"),
                val(&f, "status"),
                val(&f, "code")
            ),
            (W2, "/v2/probe/{id}", "403", "missing_scope")
        );
        let target = "/v2/targets/0193e6a0-0000-7000-8000-0000000000c2";
        let (_, f) = refused(Method::GET, target, &[&bearer(A_TOKEN)], &[], Body::empty()).await;
        assert_eq!(
            (
                val(&f, "principal"),
                val(&f, "path"),
                val(&f, "status"),
                val(&f, "code")
            ),
            (W1, "/v2/targets/{id}", "403", "target_not_allowed")
        );
    }

    #[tokio::test]
    async fn d4_log_404_a_malformed_id_logs_the_template_and_an_unmatched_path_logs_none() {
        let (_, f) = refused(
            Method::GET,
            "/v2/probe/item/abc",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(
            (
                val(&f, "principal"),
                val(&f, "path"),
                val(&f, "status"),
                val(&f, "code")
            ),
            (W1, "/v2/probe/item/{id}", "404", "not_found"),
            "the template, not the concrete path"
        );
        let (_, f) = refused(Method::GET, "/v2/nowhere/at/all", &[], &[], Body::empty()).await;
        assert_eq!(
            (val(&f, "principal"), val(&f, "path"), val(&f, "status")),
            ("-", "(unmatched)", "404")
        );
    }

    /// Added at GREEN. A 405 comes from the method router, outside every route's guard, so the
    /// guard has not authenticated anyone or named a route: the line says `-` and `(unmatched)`.
    /// This documents that limit (log.rs); it is not a claim that a 405 is "unmatched".
    #[tokio::test]
    async fn d4_log_405_a_wrong_method_is_logged_without_a_route_or_a_principal() {
        let (r, f) = refused(
            Method::POST,
            "/v2/probe/item/0193e6a0-0000-7000-8000-0000000000c3",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(r.status.as_u16(), 405, "{}", r.text);
        assert_eq!(
            (val(&f, "principal"), val(&f, "method"), val(&f, "path")),
            ("-", "POST", "(unmatched)")
        );
    }

    #[tokio::test]
    async fn d4_log_409_a_handlers_conflict_is_logged() {
        let (_, f) = refused(
            Method::GET,
            "/v2/conflicts",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(
            (
                val(&f, "principal"),
                val(&f, "path"),
                val(&f, "status"),
                val(&f, "code")
            ),
            (W1, "/v2/conflicts", "409", "probe_conflict")
        );
    }

    #[tokio::test]
    async fn d4_log_422_a_query_on_a_route_without_one_and_a_bad_body_are_logged() {
        let (_, f) = refused(
            Method::GET,
            &format!("{OBJ}?x=1"),
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(
            (
                val(&f, "principal"),
                val(&f, "path"),
                val(&f, "status"),
                val(&f, "code")
            ),
            (W1, "/v2/probe/{id}", "422", "invalid_request"),
            "no query string in the line"
        );
        let (_, f) = refused(
            Method::POST,
            &format!("{OBJ}/touch"),
            &[&bearer(A_TOKEN)],
            &[],
            Body::from(r#"{"n":1,"extra":true}"#),
        )
        .await;
        assert_eq!(
            (val(&f, "path"), val(&f, "status")),
            ("/v2/probe/{id}/touch", "422")
        );
    }

    #[tokio::test]
    async fn d4_log_413_a_body_over_the_cap_is_logged() {
        let (_, f) = refused(
            Method::POST,
            &format!("{OBJ}/touch"),
            &[&bearer(A_TOKEN)],
            &[],
            Body::from(padded(200, r#"{"n":1}"#)),
        )
        .await;
        assert_eq!((val(&f, "principal"), val(&f, "status")), (W1, "413"));
    }

    #[tokio::test]
    async fn d4_log_a_5xx_adds_its_step_and_cause_and_a_4xx_never_does() {
        let (app, lines) = logged_app();
        let key = [("idempotency-key", "0193e6a0-key-0002")];
        let r = send_to(
            app,
            Method::POST,
            &format!("{OBJ}/touch"),
            &[&bearer(A_TOKEN)],
            &key,
            Body::from(r#"{"n":1}"#),
        )
        .await;
        assert_eq!(r.status.as_u16(), 503, "{}", r.text);
        let g = lines.lock().expect("lock").clone();
        assert_eq!(g.len(), 1, "{g:?}");
        let f = six_fields(&g[0]);
        assert_eq!(
            (
                val(&f, "principal"),
                val(&f, "path"),
                val(&f, "status"),
                val(&f, "code")
            ),
            (W1, "/v2/probe/{id}/touch", "503", "unavailable")
        );
        assert!(
            g[0].contains(" step=begin cause="),
            "a 5xx names its step and cause: {}",
            g[0]
        );
        let (_, f4) = refused(
            Method::GET,
            OBJ,
            &[&bearer(NOSCOPE_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(f4.len(), 6, "a 4xx line is the six fields and nothing else");
    }

    #[tokio::test]
    async fn d4_log_a_success_writes_no_line() {
        let (app, lines) = logged_app();
        let r = send_to(
            app,
            Method::GET,
            OBJ,
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(r.status, StatusCode::OK);
        assert!(
            lines.lock().expect("lock").is_empty(),
            "{:?}",
            lines.lock().expect("lock")
        );
    }

    #[tokio::test]
    async fn d4_log_leak_scan_no_credential_query_body_header_or_raw_path_reaches_the_log() {
        const LEAK: &str = "ctx2_leak.Zq9Xv3Lm7Pw2Rt5Yn8Kd4Hs6";
        let key = [("idempotency-key", "0193e6a0-key-0003")];
        let leak_header = [("x-api-key", LEAK), ("cookie", LEAK)];
        // One attempt: what carries the caller-chosen text, and the request that carries it.
        struct Attempt {
            what: &'static str,
            method: Method,
            path: String,
            auth: Vec<String>,
            extra: Vec<(&'static str, &'static str)>,
            body: Body,
        }
        let attempt =
            |what, method, path: String, auth: Vec<String>, extra: Vec<_>, body| Attempt {
                what,
                method,
                path,
                auth,
                extra,
                body,
            };
        let cases = vec![
            attempt(
                "authorization header",
                Method::GET,
                OBJ.into(),
                vec![bearer(LEAK)],
                leak_header.to_vec(),
                Body::empty(),
            ),
            attempt(
                "query string",
                Method::GET,
                format!("{OBJ}?token={LEAK}"),
                vec![bearer(A_TOKEN)],
                vec![],
                Body::empty(),
            ),
            attempt(
                "body",
                Method::POST,
                format!("{OBJ}/touch"),
                vec![bearer(A_TOKEN)],
                key.to_vec(),
                Body::from(format!(r#"{{"n":1,"x":"{LEAK}"}}"#)),
            ),
            attempt(
                "unmatched path",
                Method::GET,
                format!("/v2/{LEAK}"),
                vec![],
                vec![],
                Body::empty(),
            ),
            attempt(
                "matched path id",
                Method::GET,
                format!("/v2/probe/item/{LEAK}"),
                vec![bearer(A_TOKEN)],
                vec![],
                Body::empty(),
            ),
            // A method token is caller-chosen text too (added at GREEN, found while designing the
            // layer): only the standard methods are named in the line.
            attempt(
                "method token",
                Method::from_bytes(LEAK.as_bytes()).expect("a valid method token"),
                OBJ.into(),
                vec![bearer(A_TOKEN)],
                vec![],
                Body::empty(),
            ),
        ];
        for Attempt {
            what,
            method,
            path,
            auth,
            extra,
            body,
        } in cases
        {
            let (app, lines) = logged_app();
            let auth: Vec<&str> = auth.iter().map(String::as_str).collect();
            let r = send_to(app, method, &path, &auth, &extra, body).await;
            assert!(r.status.as_u16() >= 400, "{what}: {}", r.text);
            let g = lines.lock().expect("lock").clone();
            assert_eq!(g.len(), 1, "{what}: one line per refusal: {g:?}");
            assert!(
                !g[0].contains("ctx2_leak") && !g[0].contains("Zq9Xv3"),
                "{what}: the log line carries caller-chosen text: {}",
                g[0]
            );
        }
    }

    // ---- Wave 1 item C: H-19, a handler panic (DB-free) --------------------------------------
    //
    // A panic in a handler is a 500 `internal_error` in problem+json, never the panic text, and the
    // next request on the same router is served. The log line is the six fields plus
    // `step=handler cause=panic`, and carries no panic text either.

    #[tokio::test]
    async fn h19_a_handler_panic_is_a_500_problem_and_the_next_request_is_served() {
        let (app, lines) = logged_app();
        let r = send_to(
            app.clone(),
            Method::GET,
            "/v2/boom",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (500, Some("internal_error")),
            "{}",
            r.text
        );
        assert!(
            !r.text.contains("canary_h19"),
            "the panic text reached the client: {}",
            r.text
        );
        // The same router serves the next request.
        let ok = send_to(
            app,
            Method::GET,
            OBJ,
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
        // One line for the one refusal: the six fields, then the step and the cause, no panic text.
        let g = lines.lock().expect("lock").clone();
        assert_eq!(g.len(), 1, "{g:?}");
        let f = six_fields(&g[0]);
        assert_eq!(
            (
                val(&f, "principal"),
                val(&f, "path"),
                val(&f, "status"),
                val(&f, "code")
            ),
            (W1, "/v2/boom", "500", "internal_error"),
            "{}",
            g[0]
        );
        assert!(g[0].contains(" step=handler cause=panic"), "{}", g[0]);
        assert!(
            !g[0].contains("canary_h19"),
            "the panic text reached the log: {}",
            g[0]
        );
    }

    // ---- Wave 1 item D: the INTERIM in-flight cap, step 5 (H-13's limiter-order half; DB-free) -----
    //
    // After `verify` and before the body is read, a request takes one of `max_in_flight` places and
    // WAITS if none is free. A request without a valid token never waits (401 comes first). A place
    // comes back whenever the request ends: answered, refused, panicked or dropped.

    /// A route whose handler counts how many requests are inside it and waits for `go` before it
    /// answers; the test decides when each one finishes.
    struct Slow {
        entered: AtomicUsize,
        running: AtomicUsize,
        peak: AtomicUsize,
        go: Semaphore,
    }

    fn new_slow() -> Arc<Slow> {
        Arc::new(Slow {
            entered: AtomicUsize::new(0),
            running: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            go: Semaphore::new(0),
        })
    }

    fn slow_routes(s: Arc<Slow>) -> Routes {
        Routes::new()
            .read(
                Method::GET,
                "/v2/slow",
                "probe:read",
                TargetRule::None,
                move || {
                    let s = Arc::clone(&s);
                    async move {
                        s.entered.fetch_add(1, Ordering::SeqCst);
                        let now = s.running.fetch_add(1, Ordering::SeqCst) + 1;
                        s.peak.fetch_max(now, Ordering::SeqCst);
                        s.go.acquire().await.expect("open").forget();
                        s.running.fetch_sub(1, Ordering::SeqCst);
                        "done"
                    }
                },
            )
            .read(
                Method::GET,
                "/v2/fast",
                "probe:read",
                TargetRule::None,
                || async { "fast" },
            )
            .read(
                Method::GET,
                "/v2/boom",
                "probe:read",
                TargetRule::None,
                boom,
            )
    }

    fn capped(max_in_flight: usize, routes: Routes) -> Router {
        let limits = Limits {
            max_in_flight,
            ..Limits::default()
        };
        let chain = Chain { limits, ..chain() };
        router(chain, state(), ids(), vec![(MODULE, routes)]).expect("a router")
    }

    fn spawn_get(app: &Router, path: &'static str, token: &str) -> tokio::task::JoinHandle<Answer> {
        let (app, auth) = (app.clone(), bearer(token));
        tokio::spawn(
            async move { send_to(app, Method::GET, path, &[&auth], &[], Body::empty()).await },
        )
    }

    /// Joins a spawned request, bounded: a place that never came back would otherwise hang the test
    /// for ever instead of failing it.
    async fn done(t: tokio::task::JoinHandle<Answer>) -> Answer {
        tokio::time::timeout(Duration::from_secs(5), t)
            .await
            .expect("the request finished (a lost place would hang it for ever)")
            .expect("joined")
    }

    /// Waits, bounded, until `cond` holds.
    async fn until(what: &str, cond: impl Fn() -> bool) {
        for _ in 0..400 {
            if cond() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test]
    async fn h13_the_limiter_comes_after_verify_and_before_the_body_is_read() {
        let slow = new_slow();
        let app = capped(1, slow_routes(Arc::clone(&slow)));
        let first = spawn_get(&app, "/v2/slow", A_TOKEN);
        until("the first request is inside its handler", || {
            slow.entered.load(Ordering::SeqCst) == 1
        })
        .await;

        // A second authenticated request WAITS for a place: its handler is not entered and its
        // body has not even been polled (step 5 comes before step 6).
        let polled = Arc::new(AtomicBool::new(false));
        let body = {
            let polled = Arc::clone(&polled);
            Body::from_stream(futures_util::stream::once(async move {
                polled.store(true, Ordering::SeqCst);
                Ok::<_, std::convert::Infallible>(Bytes::new())
            }))
        };
        let second = {
            let (app, auth) = (app.clone(), bearer(A_TOKEN));
            tokio::spawn(
                async move { send_to(app, Method::GET, "/v2/slow", &[&auth], &[], body).await },
            )
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            slow.entered.load(Ordering::SeqCst),
            1,
            "the second request must wait for a place"
        );
        assert!(
            !polled.load(Ordering::SeqCst),
            "the body was read before the limiter let the request in (step 5 is before step 6)"
        );
        assert!(!second.is_finished());

        // Verify comes BEFORE the limiter: the only place is taken, and a request with no token or
        // a bad token is still 401 at once.
        for auth in [vec![], vec![bearer("ctx2_nobody")]] {
            let refs: Vec<&str> = auth.iter().map(String::as_str).collect();
            let r = tokio::time::timeout(
                Duration::from_secs(1),
                send_to(
                    app.clone(),
                    Method::GET,
                    "/v2/fast",
                    &refs,
                    &[],
                    Body::empty(),
                ),
            )
            .await
            .expect("a request that is not authenticated never waits for a place");
            assert_eq!(r.status.as_u16(), 401, "{}", r.text);
        }

        slow.go.add_permits(2);
        assert_eq!(done(first).await.status, StatusCode::OK);
        assert_eq!(done(second).await.status, StatusCode::OK);
        assert!(
            polled.load(Ordering::SeqCst),
            "the body is read once the request is in"
        );
        assert_eq!(
            slow.peak.load(Ordering::SeqCst),
            1,
            "never two inside at once"
        );
    }

    #[tokio::test]
    async fn h13_no_more_than_the_cap_are_inside_at_once() {
        let slow = new_slow();
        let app = capped(2, slow_routes(Arc::clone(&slow)));
        let tasks: Vec<_> = (0..5)
            .map(|_| spawn_get(&app, "/v2/slow", A_TOKEN))
            .collect();
        until("two requests are inside", || {
            slow.entered.load(Ordering::SeqCst) >= 2
        })
        .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            slow.entered.load(Ordering::SeqCst),
            2,
            "the other three wait"
        );
        slow.go.add_permits(5);
        for t in tasks {
            assert_eq!(done(t).await.status, StatusCode::OK);
        }
        assert_eq!(
            slow.entered.load(Ordering::SeqCst),
            5,
            "every one was served in the end"
        );
        assert_eq!(
            slow.peak.load(Ordering::SeqCst),
            2,
            "never more than the cap inside"
        );
    }

    #[tokio::test]
    async fn h13_a_place_comes_back_after_a_panic_a_refusal_and_a_dropped_request() {
        let slow = new_slow();
        let app = capped(1, slow_routes(Arc::clone(&slow)));
        let fast = || async {
            tokio::time::timeout(
                Duration::from_secs(2),
                send_to(
                    app.clone(),
                    Method::GET,
                    "/v2/fast",
                    &[&bearer(A_TOKEN)],
                    &[],
                    Body::empty(),
                ),
            )
            .await
        };
        // After a handler panic.
        let r = send_to(
            app.clone(),
            Method::GET,
            "/v2/boom",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(r.status.as_u16(), 500, "{}", r.text);
        let r = fast().await.expect("the place came back after a panic");
        assert_eq!(r.status, StatusCode::OK);
        // After a refusal made once the request held a place (the scope check is step 7).
        let r = send_to(
            app.clone(),
            Method::GET,
            "/v2/fast",
            &[&bearer(NOSCOPE_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(r.status.as_u16(), 403, "{}", r.text);
        let r = fast().await.expect("the place came back after a refusal");
        assert_eq!(r.status, StatusCode::OK);
        // After a request is dropped while it is INSIDE its handler.
        let inside = spawn_get(&app, "/v2/slow", A_TOKEN);
        until("the request is inside", || {
            slow.entered.load(Ordering::SeqCst) == 1
        })
        .await;
        inside.abort();
        let _ = inside.await;
        let r = fast()
            .await
            .expect("the place came back after a dropped request");
        assert_eq!(r.status, StatusCode::OK);
        // After a request is dropped while it WAITS for a place.
        let holder = spawn_get(&app, "/v2/slow", A_TOKEN);
        until("the holder is inside", || {
            slow.entered.load(Ordering::SeqCst) == 2
        })
        .await;
        let waiting = spawn_get(&app, "/v2/slow", A_TOKEN);
        tokio::time::sleep(Duration::from_millis(50)).await;
        waiting.abort();
        let _ = waiting.await;
        slow.go.add_permits(1);
        assert_eq!(done(holder).await.status, StatusCode::OK);
        let r = fast()
            .await
            .expect("the place came back after a waiting request was dropped");
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(
            slow.entered.load(Ordering::SeqCst),
            2,
            "the dropped waiter never entered"
        );
    }

    #[tokio::test]
    async fn h13_limits_that_would_hang_or_refuse_everything_do_not_build() {
        for limits in [
            Limits {
                max_in_flight: 0,
                ..Limits::default()
            },
            Limits {
                request_timeout: Duration::ZERO,
                ..Limits::default()
            },
        ] {
            let chain = Chain { limits, ..chain() };
            let built = router(chain, state(), ids(), vec![(MODULE, probe())]);
            assert!(
                matches!(built, Err(ChainError::BadLimits { .. })),
                "{limits:?} must not build"
            );
        }
    }

    // ---- Wave 1 item E: the request timeout (DB-free) ----------------------------------------------
    //
    // `Limits::request_timeout` bounds one request from the start of its guard (the token check)
    // to its answer, the wait for a place included. Past it the answer is 503 `unavailable`
    // (signatures section 3), the work is dropped where it stands, and the place comes back.

    /// Sets a flag when it is dropped: the proof that a handler's future was cancelled.
    struct Dropped(Arc<AtomicBool>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn stuck_routes(dropped: Arc<AtomicBool>) -> Routes {
        Routes::new()
            .read(
                Method::GET,
                "/v2/stuck",
                "probe:read",
                TargetRule::None,
                move || {
                    let flag = Arc::clone(&dropped);
                    async move {
                        let _witness = Dropped(flag);
                        std::future::pending::<()>().await;
                        "never"
                    }
                },
            )
            .read(
                Method::GET,
                "/v2/fast",
                "probe:read",
                TargetRule::None,
                || async { "fast" },
            )
    }

    fn timed_logged(
        request_timeout: Duration,
        max_in_flight: usize,
        routes: Routes,
    ) -> (Router, Lines) {
        let lines: Lines = Arc::default();
        let into = Arc::clone(&lines);
        let chain = Chain {
            limits: Limits {
                max_in_flight,
                request_timeout,
            },
            ..chain_with(LogSink::new(move |l| {
                into.lock().expect("lock").push(l.to_owned())
            }))
        };
        let app = router(chain, state(), ids(), vec![(MODULE, routes)]).expect("a router");
        (app, lines)
    }

    /// Bounded, so a missing timeout FAILS a cell instead of hanging it.
    async fn bounded<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), f)
            .await
            .expect("the request must have been answered within 5 s")
    }

    #[tokio::test]
    async fn e_a_handler_past_the_timeout_is_a_503_unavailable_and_is_dropped() {
        let dropped = Arc::new(AtomicBool::new(false));
        let (app, lines) = timed_logged(
            Duration::from_millis(150),
            8,
            stuck_routes(Arc::clone(&dropped)),
        );
        let r = bounded(send_to(
            app.clone(),
            Method::GET,
            "/v2/stuck",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        ))
        .await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (503, Some("unavailable")),
            "{}",
            r.text
        );
        assert!(
            r.json.get("scope").is_none(),
            "a timeout names no scope: {}",
            r.text
        );
        assert!(
            dropped.load(Ordering::SeqCst),
            "the handler's future was dropped where it stood"
        );
        let g = lines.lock().expect("lock").clone();
        assert_eq!(g.len(), 1, "{g:?}");
        let f = six_fields(&g[0]);
        assert_eq!(
            (
                val(&f, "principal"),
                val(&f, "path"),
                val(&f, "status"),
                val(&f, "code")
            ),
            (W1, "/v2/stuck", "503", "unavailable"),
            "{}",
            g[0]
        );
        assert!(
            g[0].contains(" step=timeout cause=request_timeout"),
            "{}",
            g[0]
        );
        // The next request on the same router is served.
        let ok = bounded(send_to(
            app,
            Method::GET,
            "/v2/fast",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        ))
        .await;
        assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    }

    #[tokio::test]
    async fn e_a_token_check_that_never_answers_times_out_too() {
        let (app, lines) =
            timed_logged(Duration::from_millis(150), 8, stuck_routes(Arc::default()));
        let r = bounded(send_to(
            app,
            Method::GET,
            "/v2/fast",
            &[&bearer(HANG_TOKEN)],
            &[],
            Body::empty(),
        ))
        .await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (503, Some("unavailable")),
            "{}",
            r.text
        );
        let g = lines.lock().expect("lock").clone();
        assert_eq!(g.len(), 1, "{g:?}");
        let f = six_fields(&g[0]);
        assert_eq!(
            (val(&f, "principal"), val(&f, "status")),
            ("-", "503"),
            "no principal: authentication never finished: {}",
            g[0]
        );
        assert!(g[0].contains(" step=timeout"), "{}", g[0]);
    }

    #[tokio::test]
    async fn e_the_wait_for_a_place_is_inside_the_timeout() {
        // One place, two stuck requests: the second WAITS for the place, and its wait counts. Both
        // are answered 503 well inside 5 s: the queue is bounded, nothing hangs.
        let (app, _lines) =
            timed_logged(Duration::from_millis(200), 1, stuck_routes(Arc::default()));
        let a = spawn_get(&app, "/v2/stuck", A_TOKEN);
        let b = spawn_get(&app, "/v2/stuck", A_TOKEN);
        for t in [a, b] {
            let r = done(t).await;
            assert_eq!(
                (r.status.as_u16(), r.json["type"].as_str()),
                (503, Some("unavailable")),
                "{}",
                r.text
            );
        }
    }

    #[tokio::test]
    async fn e_the_place_comes_back_after_a_timeout() {
        let (app, _lines) =
            timed_logged(Duration::from_millis(100), 1, stuck_routes(Arc::default()));
        let r = bounded(send_to(
            app.clone(),
            Method::GET,
            "/v2/stuck",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        ))
        .await;
        assert_eq!(r.status.as_u16(), 503, "{}", r.text);
        let ok = bounded(send_to(
            app,
            Method::GET,
            "/v2/fast",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        ))
        .await;
        assert_eq!(
            ok.status,
            StatusCode::OK,
            "the only place came back: {}",
            ok.text
        );
    }

    // ---- Wave 1: vera's review of items A, C, D, E (REVIEW.md d4b2f3dd) ----------------------------
    //
    // HTTP-Q1: every answer to an AUTHENTICATED request carries the token headers, the 500 for a
    // handler panic and the 503 for a post-authentication timeout included (S1 244/701, H-05, and
    // the refusals that already do). Before authentication there is no principal, so there are none.

    fn expires_header() -> String {
        crate::time::rfc3339(Timestamp::from_unix_micros(EXPIRES))
    }

    #[tokio::test]
    async fn q1_a_handler_panic_after_authentication_carries_the_token_headers() {
        let (app, _lines) = logged_app();
        let not_due = send_to(
            app.clone(),
            Method::GET,
            "/v2/boom",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(not_due.status.as_u16(), 500, "{}", not_due.text);
        assert_eq!(
            not_due.headers["cortex-token-expires"],
            expires_header().as_str()
        );
        assert!(
            not_due.headers.get("cortex-token-renew").is_none(),
            "renewal is not due"
        );
        let due = send_to(
            app,
            Method::GET,
            "/v2/boom",
            &[&bearer(RENEW_TOKEN)],
            &[],
            Body::empty(),
        )
        .await;
        assert_eq!(due.status.as_u16(), 500, "{}", due.text);
        assert_eq!(
            due.headers["cortex-token-expires"],
            expires_header().as_str()
        );
        assert_eq!(due.headers["cortex-token-renew"], "due");
    }

    #[tokio::test]
    async fn q1_a_timeout_after_authentication_carries_the_token_headers() {
        let (app, _lines) =
            timed_logged(Duration::from_millis(100), 8, stuck_routes(Arc::default()));
        let not_due = bounded(send_to(
            app.clone(),
            Method::GET,
            "/v2/stuck",
            &[&bearer(A_TOKEN)],
            &[],
            Body::empty(),
        ))
        .await;
        assert_eq!(not_due.status.as_u16(), 503, "{}", not_due.text);
        assert_eq!(
            not_due.headers["cortex-token-expires"],
            expires_header().as_str()
        );
        assert!(
            not_due.headers.get("cortex-token-renew").is_none(),
            "renewal is not due"
        );
        let due = bounded(send_to(
            app,
            Method::GET,
            "/v2/stuck",
            &[&bearer(RENEW_TOKEN)],
            &[],
            Body::empty(),
        ))
        .await;
        assert_eq!(due.status.as_u16(), 503, "{}", due.text);
        assert_eq!(
            due.headers["cortex-token-expires"],
            expires_header().as_str()
        );
        assert_eq!(due.headers["cortex-token-renew"], "due");
    }

    #[tokio::test]
    async fn q1_a_timeout_during_verification_carries_no_token_headers() {
        // The control: nobody is authenticated yet, so there is nothing to say about a token.
        let (app, _lines) =
            timed_logged(Duration::from_millis(100), 8, stuck_routes(Arc::default()));
        let r = bounded(send_to(
            app,
            Method::GET,
            "/v2/fast",
            &[&bearer(HANG_TOKEN)],
            &[],
            Body::empty(),
        ))
        .await;
        assert_eq!(r.status.as_u16(), 503, "{}", r.text);
        assert!(r.headers.get("cortex-token-expires").is_none());
        assert!(r.headers.get("cortex-token-renew").is_none());
    }

    #[tokio::test]
    async fn h13_a_cap_above_the_semaphore_maximum_does_not_build() {
        // Low note: Semaphore::new PANICS above MAX_PERMITS; the chain must answer BadLimits instead.
        for max_in_flight in [Semaphore::MAX_PERMITS + 1, usize::MAX] {
            let limits = Limits {
                max_in_flight,
                ..Limits::default()
            };
            let built = router(
                Chain { limits, ..chain() },
                state(),
                ids(),
                vec![(MODULE, probe())],
            );
            assert!(
                matches!(built, Err(ChainError::BadLimits { .. })),
                "a cap of {max_in_flight} must not build"
            );
        }
        // The maximum itself is a valid cap.
        let limits = Limits {
            max_in_flight: Semaphore::MAX_PERMITS,
            ..Limits::default()
        };
        assert!(
            router(
                Chain { limits, ..chain() },
                state(),
                ids(),
                vec![(MODULE, probe())]
            )
            .is_ok()
        );
    }

    #[tokio::test]
    async fn q1_a_problem_a_read_handler_returns_carries_the_token_headers() {
        // The normaliser REBUILDS a read handler's Problem (here the 404 for a malformed path id);
        // the rebuilt answer is still an answer to an authenticated request (cox W1-25).
        for (token, due) in [(A_TOKEN, false), (RENEW_TOKEN, true)] {
            let r = send(
                Method::GET,
                "/v2/probe/item/abc",
                &[&bearer(token)],
                &[],
                Body::empty(),
            )
            .await;
            assert_eq!(r.status.as_u16(), 404, "{}", r.text);
            assert_eq!(
                r.headers["cortex-token-expires"],
                expires_header().as_str(),
                "{token}"
            );
            assert_eq!(
                r.headers.get("cortex-token-renew").is_some(),
                due,
                "{token}"
            );
        }
        // The control: nobody is authenticated, so a 401 carries neither.
        let r = send(Method::GET, "/v2/probe/item/abc", &[], &[], Body::empty()).await;
        assert_eq!(r.status.as_u16(), 401, "{}", r.text);
        assert!(
            r.headers.get("cortex-token-expires").is_none()
                && r.headers.get("cortex-token-renew").is_none()
        );
    }
}
