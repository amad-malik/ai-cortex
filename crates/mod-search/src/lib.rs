//! `mod-search` (piece 7): keyword search over the token's project, `POST /v2/search` (S3 r2.7 `search.query`; S2 revision 8 views
//! `records.pub_sources` and `records.pub_state`; card `mvp/ponytail/mod-search.md`; tests `mvp/pieces/mod-search-TEST-DEFINITIONS-r0.md`).
//!
//! O-1 (hale, piece 6): `RecordsApi::changed_since` orders by CREATION, not by commit, so a row that commits late can be missed by a reader
//! that has paged past its key. Search therefore never reads that feed and keeps no index of its own: it queries the two S2 revision 8 views
//! (`records.pub_sources`, `records.pub_state`) at request time, under the role `cortex_m_search`, and no future index feed may trust
//! `changed_since`.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, header};
use axum::response::{IntoResponse, Response};
use cortex_db::{MigrationSet, TxContext};
use cortex_module::route::AppState;
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, Ctx, Health, Manifest, Piece, Ports, Problem, ProblemSpec, Routes,
    TargetRule,
};
use cortex_types::ModuleId;

pub mod cap;
pub mod query;
pub mod request;
pub mod response;
pub mod validate;

pub use cap::Cap;
pub use query::{Limits, answer};
pub use request::{Mode, RecordType, SearchRequest};
pub use response::{Hit, SearchResult};
pub use validate::Validated;

/// The piece's id and (empty) schema.
pub const ID: &str = "search";
/// The one route.
pub const PATH: &str = "/v2/search";
/// Its scope, declared by the records piece (S3 `x-scope`).
pub const SCOPE: &str = "records:read";
/// The route's whole-body cap in bytes (S3 `x-max-body`).
pub const MAX_BODY: u64 = 16_384;

/// 422 `query_too_long` (S3 r2.7: the property `query` has `x-code: query_too_long`).
pub const QUERY_TOO_LONG: ProblemSpec = ProblemSpec {
    code: "query_too_long",
    status: 422,
    title: "Query too long",
    fix: "Send a query of 1 to 1000 characters.",
};

pub static MANIFEST: Manifest = Manifest {
    id: ID,
    version: env!("CARGO_PKG_VERSION"),
    contract: CONTRACT_VERSION,
    core: false,
    schema: ID,
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[QUERY_TOO_LONG],
};

/// Search's own concurrency cap (S3 R-9 case b): two searches at once.
pub const CAP: usize = 2;

/// Run by a superuser before the migrations: the piece's role `cortex_m_search` (its `ModuleTx` runs under it) and `cortex_app`'s
/// membership. The role reads only the two S2 views; the records piece grants that. Idempotent and race-safe, in the same form as the
/// other pieces' (an existing role, 42710, or the loser of a race, 23505, is fine), because the records piece's bootstrap creates the
/// same role.
pub const BOOTSTRAP_SQL: &str = r#"
DO $$ BEGIN
  BEGIN
    CREATE ROLE cortex_m_search NOLOGIN;
  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
  END;
  BEGIN
    GRANT cortex_m_search TO cortex_app;
  EXCEPTION WHEN unique_violation THEN NULL;
  END;
END $$;
"#;

/// The piece.
pub struct Search {
    id: ModuleId,
}

impl Search {
    pub fn new() -> Search {
        Search {
            id: ModuleId::parse(ID).expect("a valid module id"),
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Self::new()
    }
}

impl Piece for Search {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    /// Search owns no table in 0.2.001: its schema has no step.
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: self.id.clone(),
            schema: ID,
            steps: Vec::new(),
        }
    }

    fn routes(&self, _ports: &Ports) -> Routes {
        let cap = Cap::new(CAP);
        let limits = Limits::default();
        let id = self.id.clone();
        Routes::new()
            .read(
                Method::POST,
                PATH,
                SCOPE,
                TargetRule::None,
                move |State(st): State<AppState>, ctx: Ctx, headers: HeaderMap, body: Bytes| {
                    let cap = cap.clone();
                    let id = id.clone();
                    let limits = limits;
                    async move { handle(&st, &ctx, &id, &cap, &limits, &headers, &body).await }
                },
            )
            .max_body(MAX_BODY)
    }

    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

/// The handler, in the order S3 R-16 gives search: the request's bounds (422, 415, 400), THEN the cap (429), both before the first
/// database statement, so an invalid request takes no place and a refused one touches no database; then one read-only transaction.
pub async fn handle(
    st: &AppState,
    ctx: &Ctx,
    id: &ModuleId,
    cap: &Cap,
    limits: &Limits,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Response, Problem> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    let v = validate::check(content_type, body)?;
    let tctx = TxContext {
        module: id.clone(),
        project: Some(ctx.project()),
        actor: ctx.actor(),
        request_id: ctx.request_id(),
    };
    let result = answer(&st.pool, tctx, cap, limits, &v).await?;
    Ok(axum::Json(result).into_response())
}
