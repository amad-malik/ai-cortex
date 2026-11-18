//! The refusal log line (delta 4; kai's condition for refused calls not being audited in 0.2.001):
//! EVERY refusal of status 400 or more leaves one structured line, whatever step made it.
//!
//! ```text
//! cortex-http: request_id=<id> principal=<worker|-> method=<M> path=<template> status=<n> code=<code>
//! cortex-http: request_id=<id> ... code=<code> step=<step> cause=<cause>     (a 5xx adds these two)
//! ```
//!
//! What each field is, and what it is never:
//! - `principal` is the worker id once authentication succeeded, `-` before.
//! - `method` is one of the standard methods, else `(other)`: a method token is caller-chosen text.
//! - `path` is the matched ROUTE TEMPLATE (`/v2/tokens/{id}/revoke`), `(unmatched)` for the 404
//!   fallback and a 405. The raw path is caller-chosen, so a secret could ride in it; the template
//!   cannot. No query string, ever.
//! - `code` is the problem code. `step` and `cause` exist only on a 5xx, as kinds and codes.
//! - NEVER an `Authorization` or any header value, a token, a body, or a cause text on a 4xx.
//!
//! The facts travel on the response as one extension (`Facts`), which is never sent to the client.
//! The guard adds the route and the principal; `Refusal::into_response_with` adds the code and a
//! 5xx's cause; the normaliser carries the first two across when it rebuilds a response. This layer
//! reads them last, outside the normaliser.
use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use cortex_module::route::Problem;
use cortex_types::{RequestId, SystemClock, WorkerId};

use crate::chain::LogSink;

/// What the log line needs, carried on the response. Never rendered.
#[derive(Clone, Debug, Default)]
pub(crate) struct Facts {
    /// The route template the guard ran for; `None` for the 404 fallback and a 405.
    pub route: Option<&'static str>,
    /// The authenticated worker; `None` until authentication succeeded.
    pub who: Option<WorkerId>,
    /// The problem code of the refusal.
    pub code: Option<&'static str>,
    /// A 5xx's step and internal cause.
    pub internal: Option<(&'static str, String)>,
}

/// The response's facts, created empty on first use.
pub(crate) fn facts_mut(r: &mut Response) -> &mut Facts {
    if r.extensions().get::<Facts>().is_none() {
        r.extensions_mut().insert(Facts::default());
    }
    r.extensions_mut()
        .get_mut::<Facts>()
        .expect("inserted just above")
}

/// A rebuilt response keeps the route and the principal of the one it replaces.
pub(crate) fn carry(from: &Response, to: &mut Response) {
    if let Some(f) = from.extensions().get::<Facts>() {
        let t = facts_mut(to);
        t.route = f.route;
        t.who = f.who;
    }
}

/// The layer: just inside the request-id layer, outside the normaliser and every guard, so it sees
/// the final response. A success writes nothing.
pub(crate) async fn step(State(sink): State<LogSink>, req: Request, next: Next) -> Response {
    let id = req
        .extensions()
        .get::<RequestId>()
        .copied()
        .unwrap_or_else(|| RequestId::new(&SystemClock));
    let method = standard(req.method());
    let resp = next.run(req).await;
    if resp.status().as_u16() >= 400 {
        sink.emit(&line(&id, method, &resp));
    }
    resp
}

/// A standard method's name; anything else is caller-chosen text and is not logged.
fn standard(m: &Method) -> &'static str {
    match *m {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        _ => "(other)",
    }
}

fn line(id: &RequestId, method: &str, resp: &Response) -> String {
    let facts = resp.extensions().get::<Facts>();
    let who = facts
        .and_then(|f| f.who)
        .map_or_else(|| "-".to_owned(), |w| w.to_string());
    let route = facts.and_then(|f| f.route).unwrap_or("(unmatched)");
    let code = facts
        .and_then(|f| f.code)
        .or_else(|| resp.extensions().get::<Problem>().map(|p| p.code))
        .unwrap_or("-");
    let mut s = format!(
        "cortex-http: request_id={id} principal={who} method={method} path={route} status={} code={code}",
        resp.status().as_u16()
    );
    if let Some((step, cause)) = facts.and_then(|f| f.internal.as_ref()) {
        s.push_str(&format!(" step={step} cause={cause}"));
    }
    s
}
