//! `mod-boot` (piece 9): the signed-in worker's budgeted, byte-deterministic briefing,
//! `GET /v2/boot` (S3 r2.6 `boot.get`; S2 §7; card `mvp/ponytail/mod-boot.md`; tests
//! `mvp/tests/mod-boot.md`).
//!
//! - **Who:** only the token's principal (`Ctx::principal`); the route takes no worker, project or
//!   header. The project is the transaction's (row-level security, through the ports).
//! - **What:** sections 1 identity, 2 orientation, 3 rules (pointers, never text), 4 claimed and
//!   5 pending handoffs; sections 6 to 12 say `capability_unavailable`.
//! - **How:** [`seam::gather`] makes the port calls in ONE `ModuleTx` (through [`seam::BootReads`],
//!   so the tests can run it against fakes without a database); [`render::assemble`] is pure: the
//!   S2 §7.2 allocator and the Markdown, with the JSON as the same selection.
//! - **Writes:** none. No table, no migration step, no audit row.

use std::sync::Arc;

use axum::extract::State;
use axum::http::Method;
use axum::response::Response;
use cortex_db::MigrationSet;
use cortex_module::route::AppState;
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, Ctx, HandoffsApi, Health, IdentityApi, Manifest, Piece, Port,
    PortError, Ports, Problem, ProblemSpec, RecordsApi, Routes, StrictQuery, TargetRule,
};
use cortex_types::ModuleId;

pub mod args;
pub mod inputs;
pub mod json;
pub mod render;
pub mod seam;
mod text;
mod time;

pub use args::{Args, BootQuery, Format};
pub use inputs::{HState, Handoff, Identity, Inputs, Kind};
pub use render::{Briefing, Needed, assemble, respond};

/// The piece's id and (empty) schema.
pub const ID: &str = "boot";
/// The one route.
pub const PATH: &str = "/v2/boot";
/// Its scope: declared by the identity piece, not by boot (R-12: held by kind worker only).
pub const SCOPE: &str = "self:read";

/// 422 `boot_budget_too_small` (S3 r2.6 `x-error-codes.pieces`); `needed` and `budget` are in
/// `detail` (R-13: no extension members).
pub const BUDGET_TOO_SMALL: ProblemSpec = ProblemSpec {
    code: "boot_budget_too_small",
    status: 422,
    title: "Boot budget too small",
    fix: "Ask for a larger budget; the detail names the bytes the briefing needs.",
};

pub static MANIFEST: Manifest = Manifest {
    id: ID,
    version: env!("CARGO_PKG_VERSION"),
    contract: CONTRACT_VERSION,
    core: false,
    schema: ID,
    provides: &[],
    uses: &[Port::Identity, Port::Records, Port::Handoffs],
    scopes: &[],
    problems: &[BUDGET_TOO_SMALL],
};

/// Run by a superuser before the migrations: the piece's role `cortex_m_boot` (its `ModuleTx`
/// runs under it) and `cortex_app`'s membership. Boot's own role reads nothing itself: every
/// read is a port call, delegated to the provider's role. Idempotent and race-safe, in the same
/// form as plug-identity's (an existing role, 42710, or the loser of a race, 23505, is fine).
pub const BOOTSTRAP_SQL: &str = r#"
DO $$ BEGIN
  BEGIN
    CREATE ROLE cortex_m_boot NOLOGIN;
  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
  END;
  BEGIN
    GRANT cortex_m_boot TO cortex_app;
  EXCEPTION WHEN unique_violation THEN NULL;
  END;
END $$;
"#;

/// The piece.
pub struct Boot {
    id: ModuleId,
}

impl Boot {
    pub fn new() -> Boot {
        Boot {
            id: ModuleId::parse(ID).expect("a valid module id"),
        }
    }
}

impl Default for Boot {
    fn default() -> Self {
        Self::new()
    }
}

/// The three ports boot reads, as the registry handed them (each present when the manifest's
/// `uses` held; a missing one is 503, never a panic).
#[derive(Clone)]
struct Deps {
    identity: Option<Arc<dyn IdentityApi>>,
    records: Option<Arc<dyn RecordsApi>>,
    handoffs: Option<Arc<dyn HandoffsApi>>,
}

impl Piece for Boot {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    /// Boot owns no table in 0.2.001 (card, rung 1): its schema has no step.
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: self.id.clone(),
            schema: ID,
            steps: Vec::new(),
        }
    }

    fn routes(&self, ports: &Ports) -> Routes {
        let deps = Deps {
            identity: ports.identity(),
            records: ports.records(),
            handoffs: ports.handoffs(),
        };
        let id = self.id.clone();
        Routes::new()
            .read(
                Method::GET,
                PATH,
                SCOPE,
                TargetRule::None,
                move |State(st): State<AppState>, ctx: Ctx, q: StrictQuery<BootQuery>| {
                    let deps = deps.clone();
                    let id = id.clone();
                    async move { handle(&st, &ctx, &id, &deps, q.0).await }
                },
            )
            .query()
    }

    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

/// The handler: the argument checks (no transaction), the reads in one transaction, then the
/// pure assembly. A refusal renders nothing.
async fn handle(
    st: &AppState,
    ctx: &Ctx,
    id: &ModuleId,
    deps: &Deps,
    q: BootQuery,
) -> Result<Response, Problem> {
    let a = args::check(&q)?;
    let (Some(identity), Some(records), Some(handoffs)) = (
        deps.identity.as_deref(),
        deps.records.as_deref(),
        deps.handoffs.as_deref(),
    ) else {
        return Err(Problem::from(PortError::Unavailable));
    };
    let mut tx = ctx.begin(&st.pool, id).await?;
    let inputs = {
        let mut reads = seam::PortReads {
            tx: &mut tx,
            identity,
            records,
            handoffs,
        };
        seam::gather(&mut reads, ctx.principal()).await?
    };
    tx.commit().await?;
    let b = assemble(&inputs, a.budget).map_err(|n| n.problem())?;
    Ok(respond(&b, a.format))
}
