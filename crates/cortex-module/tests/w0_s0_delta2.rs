//! Socket delta 2, S0 part (cox's row 53fa84bc; freeze note "Socket delta 2"; S3 r2.1 R-4, R-5).
//!
//! - **R-5:** path ids in `Ctx`. An absent or malformed id is 404 `not_found`, the same answer as
//!   another project's id, for a write handler (`Ctx::with_path`, filled by the chain) and for a
//!   read handler (`Ctx` extracted through a real axum router).
//! - **R-4:** `StrictQuery<T>` answers 422 `invalid_request` for an unknown parameter or a bad value,
//!   never axum's 400.
//! - **The query marker is default-deny:** a route takes a query only after `Routes::query()`, and
//!   the registry refuses it on a write route.
//!
//! DB-free.
//!
//! RED: `with_path`, `path_id`, `path_param`, `StrictQuery` and `Routes::query` are `todo!()`,
//! and the registry does not check `QueryOnWrite`.
use std::sync::OnceLock;

use axum::Extension;
use axum::Router;
use axum::body::Body;
use axum::extract::FromRequestParts;
use axum::http::{Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use cortex_db::MigrationSet;
use cortex_module::{
    Assembly, BoxFuture, Ctx, DefaultRole, Health, Manifest, Piece, Ports, Principal, Registry,
    Reply, Role, Routes, ScopeClass, ScopeDecl, StartupError, TargetRule, WorkerKind,
    route::StrictQuery,
};
use cortex_types::{
    ModuleId, ProblemCodeMinter, ProjectId, RequestId, SystemClock, Timestamp, TokenId, WorkerId,
};
use serde::Deserialize;
use tower::ServiceExt;

fn ctx() -> Ctx {
    let c = SystemClock;
    Ctx::new(
        Principal {
            worker: WorkerId::new(&c),
            project: ProjectId::new(&c),
            kind: WorkerKind::Worker,
            role: Role::Member(None),
            token: TokenId::new(&c),
            expires_at: Timestamp::from_unix_micros(0),
            renew_due: false,
        },
        RequestId::new(&c),
    )
}

fn code<T>(r: Result<T, cortex_module::Problem>) -> Result<T, (u16, &'static str)> {
    r.map_err(|p| (p.status, p.code))
}

#[test]
fn w0_d2_a_path_id_parses_or_is_404() {
    let w = WorkerId::new(&SystemClock);
    let c = ctx().with_path([
        ("id".to_owned(), w.to_string()),
        ("v".to_owned(), "plain".to_owned()),
    ]);
    assert_eq!(code(c.path_id::<WorkerId>("id")), Ok(w));
    assert_eq!(code(c.path_param("v")), Ok("plain"));
    // Absent: the same 404.
    assert_eq!(
        code(c.path_id::<WorkerId>("other")),
        Err((404, "not_found"))
    );
    assert_eq!(code(c.path_param("other")), Err((404, "not_found")));
    // Malformed, a UUIDv4, upper case, empty: all the same 404.
    for bad in [
        "not-a-uuid".to_owned(),
        "550e8400-e29b-41d4-a716-446655440000".to_owned(),
        w.to_string().to_uppercase(),
        String::new(),
    ] {
        let c = ctx().with_path([("id".to_owned(), bad.clone())]);
        assert_eq!(
            code(c.path_id::<WorkerId>("id")),
            Err((404, "not_found")),
            "{bad:?}"
        );
    }
}

/// A read route through a real axum router, with the chain's `Ctx` in the request (as the chain
/// puts it): the handler answers 200 with a well-formed id, else the `Problem` from `path_id`.
fn read_router() -> Router {
    Router::new()
        .route(
            "/v2/x/{id}",
            get(|c: Ctx| async move {
                match c.path_id::<WorkerId>("id") {
                    Ok(_) => StatusCode::OK.into_response(),
                    Err(p) => p.into_response(),
                }
            }),
        )
        .layer(Extension(ctx()))
}

async fn status(uri: &str) -> StatusCode {
    read_router()
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("the router answers")
        .status()
}

#[tokio::test]
async fn w0_d2_a_read_handlers_ctx_carries_its_path_ids() {
    let w = WorkerId::new(&SystemClock);
    assert_eq!(status(&format!("/v2/x/{w}")).await, StatusCode::OK);
    assert_eq!(status("/v2/x/not-a-uuid").await, StatusCode::NOT_FOUND);
    // Not UTF-8 once percent-decoded: the path stays empty, so the same 404.
    assert_eq!(status("/v2/x/%FF").await, StatusCode::NOT_FOUND);
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    limit: Option<u32>,
    cursor: Option<String>,
}

async fn query(uri: &str) -> Result<Page, (u16, &'static str)> {
    let (mut parts, ()) = Request::builder()
        .uri(uri)
        .body(())
        .expect("request")
        .into_parts();
    code(
        StrictQuery::<Page>::from_request_parts(&mut parts, &())
            .await
            .map(|q| q.0),
    )
}

#[tokio::test]
async fn w0_d2_strict_query_is_422_for_an_unknown_parameter_or_a_bad_value() {
    assert_eq!(
        query("/x").await,
        Ok(Page {
            limit: None,
            cursor: None
        })
    );
    assert_eq!(
        query("/x?limit=5&cursor=abc").await,
        Ok(Page {
            limit: Some(5),
            cursor: Some("abc".into())
        })
    );
    for bad in ["/x?limit=5&extra=1", "/x?limit=abc", "/x?limit=-1"] {
        assert_eq!(query(bad).await, Err((422, "invalid_request")), "{bad}");
    }
}

async fn ok() -> &'static str {
    "ok"
}

#[test]
fn w0_d2_a_route_takes_a_query_only_when_marked() {
    let r = Routes::new()
        .read(Method::GET, "/v2/a", "probe:read", TargetRule::None, ok)
        .read(Method::GET, "/v2/b", "probe:read", TargetRule::None, ok)
        .query()
        .write_open(
            Method::POST,
            "/v2/c",
            "probe:write",
            |_tx, _ctx, _b: serde_json::Value| Box::pin(async move { Ok(Reply::ok(())) }),
        );
    let q: Vec<(&str, bool)> = r.entries().map(|e| (e.path, e.query)).collect();
    assert_eq!(q, [("/v2/a", false), ("/v2/b", true), ("/v2/c", false)]);
}

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

static PROBE: Manifest = Manifest {
    id: "probe",
    version: "0.0.1",
    contract: 1,
    core: false,
    schema: "probe",
    provides: &[],
    uses: &[],
    scopes: &[
        ScopeDecl {
            name: "probe:read",
            class: ScopeClass::Work,
            default_roles: &[DefaultRole::Member],
        },
        ScopeDecl {
            name: "probe:write",
            class: ScopeClass::Work,
            default_roles: &[DefaultRole::Member],
        },
    ],
    problems: &[],
};

struct Probe(bool);

impl Piece for Probe {
    fn manifest(&self) -> &'static Manifest {
        &PROBE
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("probe").expect("id"),
            schema: "probe",
            steps: vec![],
        }
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        let r = Routes::new()
            .read(Method::GET, "/v2/probe", "probe:read", TargetRule::None, ok)
            .query()
            .write_open(
                Method::POST,
                "/v2/probe",
                "probe:write",
                |_tx, _ctx, _b: serde_json::Value| Box::pin(async move { Ok(Reply::ok(())) }),
            );
        if self.0 { r.query() } else { r }
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

fn assemble(query_on_write: bool) -> Result<Registry, Vec<StartupError>> {
    let enabled = [ModuleId::parse("probe").expect("id")];
    Registry::assemble(Assembly {
        compiled: vec![Box::new(Probe(query_on_write))],
        enabled: &enabled,
        minter: minter(),
        chain_codes: &[],
    })
}

#[test]
fn w0_d2_the_registry_refuses_a_query_on_a_write_route() {
    match assemble(true) {
        Err(e) => assert_eq!(
            e,
            [StartupError::QueryOnWrite {
                method: "POST".into(),
                path: "/v2/probe".into()
            }]
        ),
        Ok(_) => panic!("a write route marked to take a query must refuse startup"),
    }
    // Control: the same piece with the query on its read route only assembles.
    assemble(false).expect("a read route may take a query");
}
