//! Step 1 (F1-25): every response carries `Cortex-Request-Id`, a fresh UUIDv7. A client-sent one
//! is removed before anything downstream sees it and is never echoed (tower-http's `SetRequestId`
//! keeps a client's id, which is why this step is ours). The id is put in the request's
//! extensions for the problem bodies.
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;
use cortex_types::{Clock, RequestId};

/// The response header.
pub const HEADER: &str = "cortex-request-id";

/// The clock the ids come from.
#[derive(Clone)]
pub struct RequestIds {
    pub clock: Arc<dyn Clock>,
}

/// The middleware: `axum::middleware::from_fn_with_state(ids, request_id::layer)`, outermost.
pub async fn layer(State(ids): State<RequestIds>, mut req: Request, next: Next) -> Response {
    let id = RequestId::new(ids.clock.as_ref());
    // Every value of a client-sent header is removed; nothing downstream can read it.
    req.headers_mut().remove(HEADER);
    req.extensions_mut().insert(id);
    let mut resp = next.run(req).await;
    let value = HeaderValue::from_str(&id.to_string()).expect("a hyphenated UUID is visible ASCII");
    resp.headers_mut().insert(HEADER, value);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;
    use cortex_types::{RequestId, SystemClock};
    use tower::ServiceExt;

    fn app() -> Router {
        let ids = RequestIds {
            clock: Arc::new(SystemClock),
        };
        Router::new()
            .route(
                "/seen",
                get(|h: HeaderMap| async move {
                    // The handler must never see a client-sent id.
                    if h.contains_key(HEADER) {
                        StatusCode::INTERNAL_SERVER_ERROR
                    } else {
                        StatusCode::OK
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(ids, layer))
    }

    async fn call(path: &str, spoof: Option<&str>) -> (StatusCode, String) {
        let mut b = Request::builder().uri(path);
        if let Some(s) = spoof {
            b = b.header(HEADER, s);
        }
        let r = app()
            .oneshot(b.body(Body::empty()).expect("a request"))
            .await
            .expect("a response");
        let id = r
            .headers()
            .get(HEADER)
            .expect("Cortex-Request-Id")
            .to_str()
            .expect("ASCII")
            .to_owned();
        (r.status(), id)
    }

    #[tokio::test]
    async fn h04_every_response_has_a_fresh_v7_id_and_a_client_id_is_dropped() {
        let (s, a) = call("/seen", Some("0193e6a0-0000-7000-8000-0000000000ff")).await;
        assert_eq!(s, StatusCode::OK, "the handler saw the client's id");
        assert_ne!(
            a, "0193e6a0-0000-7000-8000-0000000000ff",
            "the client's id was echoed"
        );
        RequestId::parse(&a).expect("a UUIDv7");
        let (_, b) = call("/seen", None).await;
        assert_ne!(a, b, "one id per request");
        let (s404, c) = call("/nowhere", None).await;
        assert_eq!(s404, StatusCode::NOT_FOUND);
        RequestId::parse(&c).expect("a 404 has an id too");
    }
}
