//! The three OPEN health routes (contract r0.4 section 2; S3 r2.7 `GET /v2/health`, `/live`, `/ready`; piece 11 `serve`, mira).
//!
//! They sit OUTSIDE the token check (`chain::router_with_open`): no `Authorization` is read for them. Every other route needs a
//! token. The wrong method is a 405 `invalid_request` with `Allow` (the chain's normaliser), and an unknown path 404.
//!
//! - `GET /v2/health/live`: `{"ok":true}` while the process runs, whatever the database says.
//! - `GET /v2/health`: the document of [`crate::health::health_doc`] (state, version, ready, pieces, last_good_dump_at, backup).
//! - `GET /v2/health/ready`: `{"ok":true}` only when the document is ready AND the test fault is not on; else 503
//!   `capability_unavailable` with a FIXED body (no reason, no name: the route is open).
//!
//! The database side is a trait ([`Probe`]) so this is testable without a database, as `migrate.rs` does.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::Extension;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::Response;
use axum::routing::get;
use cortex_http::chain::OpenRoute;
use cortex_http::problem::Refusal;
use cortex_types::{Clock, ProblemType, RequestId};

use crate::fault::Fault;
use crate::health::{HealthDoc, HealthInput, LastGood, PieceHealth, health_doc, read_last_good};

/// ONE observation of everything readiness depends on, taken together: the three facts a health document is built from always come from
/// the same moment (vera SERVE-V1: three separate questions could be answered from two different cached observations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeSnapshot {
    /// Can the pool reach the database?
    pub reachable: bool,
    /// Is every compiled and enabled migration step applied with an equal checksum?
    pub current: bool,
    /// Every compiled piece, with whether it is enabled and (for an enabled one) ready.
    pub pieces: Vec<PieceHealth>,
}

/// What the readiness decision asks the rest of the server. It is asked ONCE per document or per readiness check.
#[async_trait]
pub trait Probe: Send + Sync + 'static {
    async fn snapshot(&self) -> ProbeSnapshot;
}

/// Everything the three handlers read.
pub struct HealthState {
    /// SemVer machine version (`0.2.1`), as `--version` prints it.
    pub version: String,
    pub probe: Arc<dyn Probe>,
    /// Where nix's `LAST_GOOD` file is mounted, if anywhere (OPEN for hue and piece 12). `None` = `backup: none`.
    pub last_good: Option<PathBuf>,
    /// The test-only fault of this build (`Fault::None` in a release build).
    pub fault: Fault,
    pub clock: Arc<dyn Clock>,
}

impl HealthState {
    /// The open document, read now.
    pub async fn document(&self) -> HealthDoc {
        let observed = self.probe.snapshot().await;
        let last_good = self
            .last_good
            .as_deref()
            .map_or(LastGood::Missing, read_last_good);
        health_doc(&HealthInput {
            version: &self.version,
            database_reachable: observed.reachable,
            migrations_current: observed.current,
            pieces: &observed.pieces,
            last_good: &last_good,
            now: self.clock.now(),
        })
    }

    /// Ready for traffic: the document says so AND the test fault is not on.
    pub async fn ready(&self) -> bool {
        self.fault != Fault::Ready503 && self.document().await.ready
    }
}

fn json(status: StatusCode, body: Vec<u8>) -> Response {
    let mut r = Response::new(axum::body::Body::from(body));
    *r.status_mut() = status;
    r.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    r
}

fn ok() -> Response {
    json(StatusCode::OK, br#"{"ok":true}"#.to_vec())
}

/// 503 `capability_unavailable`: the same bytes whatever the reason (the route is open, so it says nothing about why).
fn not_ready(id: &RequestId) -> Response {
    Refusal {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: ProblemType::CAPABILITY_UNAVAILABLE.code(),
        title: "Capability unavailable",
        detail: Cow::Borrowed("The server is not ready to serve requests."),
        fix: "Retry shortly; GET /v2/health says more.",
        scope: None,
        internal: None,
    }
    .into_response_with(id)
}

/// The three open routes, for `chain::router_with_open`.
pub fn routes(state: Arc<HealthState>) -> Vec<OpenRoute> {
    let doc = state.clone();
    let ready = state;
    vec![
        OpenRoute {
            method: Method::GET,
            path: "/v2/health/live",
            handler: get(|| async { ok() }),
        },
        OpenRoute {
            method: Method::GET,
            path: "/v2/health",
            handler: get(move || {
                let s = doc.clone();
                async move {
                    let body = serde_json::to_vec(&s.document().await)
                        .expect("a struct of strings, bools and an enum serialises");
                    json(StatusCode::OK, body)
                }
            }),
        },
        OpenRoute {
            method: Method::GET,
            path: "/v2/health/ready",
            handler: get(move |Extension(id): Extension<RequestId>| {
                let s = ready.clone();
                async move {
                    if s.ready().await {
                        ok()
                    } else {
                        not_ready(&id)
                    }
                }
            }),
        },
    ]
}
