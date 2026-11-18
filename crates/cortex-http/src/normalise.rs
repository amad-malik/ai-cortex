//! The last word on error bodies (H-15, H-17). A piece's `Problem` is rendered with the request id.
//! Any other response of 400 or more that is not already
//! problem+json (axum's 404 fallback and 405, an extractor's rejection in a read handler, a
//! timeout's 408) becomes a problem for its status, with a fixed text. The original body is
//! dropped unread, so a rejection that quotes the request never reaches the client (H-18).
use axum::extract::Request;
use axum::http::StatusCode;
use axum::http::header::{ALLOW, CONTENT_TYPE};
use axum::middleware::Next;
use axum::response::Response;
use cortex_module::route::Problem;
use cortex_types::{RequestId, SystemClock};

use crate::problem::{
    PROBLEM_JSON, Refusal, internal_error, invalid_request, not_found, token_invalid, unavailable,
};

/// The middleware: `axum::middleware::from_fn(normalise::layer)`, inside the request-id layer.
pub async fn layer(req: Request, next: Next) -> Response {
    // The request-id layer is outermost and always sets this; the fallback only keeps a body valid
    // if the layers are ever composed wrongly.
    let id = req
        .extensions()
        .get::<RequestId>()
        .copied()
        .unwrap_or_else(|| RequestId::new(&SystemClock));
    let resp = next.run(req).await;
    let status = resp.status();
    // A piece's `Problem` (S0) travels in the response's extensions, without a body.
    let piece = resp.extensions().get::<Problem>().map(Refusal::from);
    if piece.is_none() && (status.as_u16() < 400 || is_problem(&resp)) {
        return resp;
    }
    let allow = resp.headers().get(ALLOW).cloned();
    // The original body is dropped unread.
    let refusal = piece.unwrap_or_else(|| refusal_for(status));
    let mut out = refusal.into_response_with(&id);
    if let Some(a) = allow {
        out.headers_mut().insert(ALLOW, a);
    }
    // The rebuilt response keeps what the guard learned for the log line (route, principal) ...
    crate::log::carry(&resp, &mut out);
    // ... and the token headers the guard stamped: it is still an answer to an AUTHENTICATED request
    // (cox W1-25). A response from outside any guard (the 404 fallback, a 405) has none to keep.
    for name in ["cortex-token-expires", "cortex-token-renew"] {
        if let Some(v) = resp.headers().get(name) {
            out.headers_mut().insert(name, v.clone());
        }
    }
    out
}

fn is_problem(resp: &Response) -> bool {
    resp.headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with(PROBLEM_JSON))
}

fn refusal_for(status: StatusCode) -> Refusal {
    match status {
        StatusCode::NOT_FOUND => not_found(),
        StatusCode::METHOD_NOT_ALLOWED => {
            invalid_request(status, "This address does not take this method.")
        }
        StatusCode::UNSUPPORTED_MEDIA_TYPE => {
            invalid_request(status, "The body must be application/json.")
        }
        StatusCode::PAYLOAD_TOO_LARGE => Refusal {
            status,
            code: cortex_types::ProblemType::BODY_TOO_LARGE.code(),
            title: "Body too large",
            detail: "The body is over this operation's limit.".into(),
            fix: "Send a smaller body.",
            scope: None,
            internal: None,
        },
        StatusCode::UNAUTHORIZED => token_invalid(),
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => unavailable(),
        s if s.is_server_error() => internal_error(),
        s => invalid_request(s, "The request was refused."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::problem::PROBLEM_JSON;
    use crate::request_id::{self, RequestIds};
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{StatusCode, header};
    use axum::routing::get;
    use cortex_types::SystemClock;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn app() -> Router {
        let ids = RequestIds {
            clock: Arc::new(SystemClock),
        };
        Router::new()
            .route("/x", get(|| async { "ok" }))
            .route(
                "/plain422",
                get(|| async { (StatusCode::UNPROCESSABLE_ENTITY, "field canary_51 is bad") }),
            )
            .route("/slow", get(|| async { StatusCode::REQUEST_TIMEOUT }))
            .route(
                "/boom",
                get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "canary_51 trace") }),
            )
            .layer(axum::middleware::from_fn(layer))
            .layer(axum::middleware::from_fn_with_state(ids, request_id::layer))
    }

    async fn call(
        method: &str,
        path: &str,
    ) -> (StatusCode, axum::http::HeaderMap, serde_json::Value, String) {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .expect("a request");
        let r = app().oneshot(req).await.expect("a response");
        let (parts, body) = r.into_parts();
        let bytes = to_bytes(body, 64 * 1024).await.expect("a body");
        let text = String::from_utf8(bytes.to_vec()).expect("UTF-8");
        let v = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        (parts.status, parts.headers, v, text)
    }

    #[tokio::test]
    async fn h17_framework_errors_become_problems() {
        let cases = [
            ("GET", "/nowhere", 404, "not_found"),
            ("POST", "/x", 405, "invalid_request"),
            ("GET", "/plain422", 422, "invalid_request"),
            ("GET", "/slow", 503, "unavailable"),
            ("GET", "/boom", 500, "internal_error"),
        ];
        for (m, p, status, code) in cases {
            let (s, h, v, text) = call(m, p).await;
            assert_eq!(s.as_u16(), status, "{m} {p}: {text}");
            assert_eq!(h[header::CONTENT_TYPE], PROBLEM_JSON, "{m} {p}");
            assert_eq!(h[header::CACHE_CONTROL], "no-store", "{m} {p}");
            assert_eq!(v["type"], code, "{m} {p}: {text}");
            assert_eq!(
                v["request_id"],
                h[request_id::HEADER].to_str().expect("ASCII"),
                "{m} {p}"
            );
            assert!(
                !text.contains("canary_51"),
                "{m} {p} leaked the original body: {text}"
            );
        }
        let (_, h, _, _) = call("POST", "/x").await;
        assert!(h.contains_key(header::ALLOW), "405 keeps Allow");
    }

    #[tokio::test]
    async fn h17_success_passes_untouched() {
        let (s, _, _, text) = call("GET", "/x").await;
        assert_eq!((s, text.as_str()), (StatusCode::OK, "ok"));
    }
}
