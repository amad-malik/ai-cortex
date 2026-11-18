//! `mod-handoffs` (Cortex v0.2.001 piece 8, owner bob): a project's work items and the loop around
//! them. `pending -> claimed -> returned -> accepted`, with rework, release and cancel.
//!
//! **Wave 1 (row 2a86598d):** the piece registers through S0 and declares what S3 and S2 say: the manifest
//! (ports, six scopes, thirteen codes), the migration (S2's `handoffs.sql`, byte for byte), the bootstrap
//! role, and the nine routes with their scope, declared target rule and body limit. All nine handlers are
//! real: two-phase (authorize takes no lock; execute locks the row, checks the state, writes, and leaves a
//! transition row and an audit row in the same transaction). The test definitions are
//! `mvp/tests/mod-handoffs.md`.
//!
//! Contracts: S2 revision 6, S3 r2.7, the socket contract, and cox's rulings W1-9, W1-10, W1-33, W1-34 and
//! W1-39 (`WAVE1-RULINGS.md`).
//!
//! - **Manifest:** id and schema `handoffs`, provides `HandoffsApi`, uses `IdentityApi` and `AuditSink`.
//! - **Role:** [`BOOTSTRAP_SQL`] creates `cortex_m_handoffs` and grants it to `cortex_app`; a superuser
//!   runs it before the migrations (the plug's lesson: roles from the bootstrap, not from migrations).

pub mod api;
pub mod body;
mod problem;
pub mod rules;
pub mod store;
pub mod validate;

use std::sync::Arc;

use axum::extract::State;
use axum::http::Method;
use cortex_db::{Migration, MigrationSet};
use cortex_module::route::AppState;
use cortex_module::{
    Authorized, BoxFuture, CONTRACT_VERSION, Ctx, DefaultRole, HandoffsApi, Health, Manifest,
    Piece, Port, Ports, PortsBuilder, ProblemSpec, Routes, ScopeClass, ScopeDecl, StrictQuery,
    TargetRule,
};
use cortex_types::{HandoffId, ModuleId};

use crate::api::{CreatePlan, Deps, ListQuery, NoBody, Provider};
use crate::body::{CancelBody, CreateBody, ReleaseBody, ReturnBody, ReviewBody};

/// The piece's id, schema and role suffix.
pub const ID: &str = "handoffs";

/// The six scopes of S3 r2.5 (`x-scopes`). Every role holds each; the target rule narrows it.
pub const SCOPE_CREATE: &str = "handoff:create";
pub const SCOPE_READ: &str = "handoff:read";
pub const SCOPE_CLAIM: &str = "handoff:claim";
pub const SCOPE_HOLD: &str = "handoff:hold";
pub const SCOPE_REVIEW: &str = "handoff:review";
pub const SCOPE_MANAGE: &str = "handoff:manage";

const ALL_ROLES: &[DefaultRole] = &[
    DefaultRole::ProjectLead,
    DefaultRole::TeamLead,
    DefaultRole::Member,
];

const fn scope(name: &'static str) -> ScopeDecl {
    ScopeDecl {
        name,
        class: ScopeClass::Work,
        default_roles: ALL_ROLES,
    }
}

/// The thirteen codes this piece mints (S3 r2.5 `x-error-codes.pieces`). The `fix` texts are fixed
/// text: they never carry a request value or a row.
pub static PROBLEMS: [ProblemSpec; 13] = [
    ProblemSpec {
        code: "already_claimed",
        status: 409,
        title: "Handoff already claimed",
        fix: "Another worker holds this handoff; read it with show and take other work.",
    },
    ProblemSpec {
        code: "evidence_hash_mismatch",
        status: 422,
        title: "Evidence hash mismatch",
        fix: "Send the SHA-256 of the exact bytes you attach.",
    },
    ProblemSpec {
        code: "evidence_required",
        status: 422,
        title: "Evidence required",
        fix: "A return needs at least one evidence entry.",
    },
    ProblemSpec {
        code: "evidence_source_invalid",
        status: 422,
        title: "Evidence source invalid",
        fix: "Give each evidence entry exactly one of ref and inline_b64.",
    },
    ProblemSpec {
        code: "invalid_transition",
        status: 409,
        title: "Invalid transition",
        fix: "The handoff's state does not allow this move; read it with show.",
    },
    ProblemSpec {
        code: "not_holder",
        status: 409,
        title: "Not the holder",
        fix: "Only the worker holding the handoff can do this; the project lead may release it with a reason.",
    },
    ProblemSpec {
        code: "notes_required",
        status: 422,
        title: "Notes required",
        fix: "A rework needs notes saying what to change.",
    },
    ProblemSpec {
        code: "packet_incomplete",
        status: 422,
        title: "Packet incomplete",
        fix: "Send a summary, a next action and acceptance criteria.",
    },
    ProblemSpec {
        code: "reason_required",
        status: 422,
        title: "Reason required",
        fix: "Send the reason this operation requires, in the form its documentation gives.",
    },
    ProblemSpec {
        code: "risks_required",
        status: 422,
        title: "Risks required",
        fix: "State the risks, or the word none.",
    },
    ProblemSpec {
        code: "self_review_forbidden",
        status: 403,
        title: "Self review forbidden",
        fix: "The creator or the project lead, not the author of the return, must review it.",
    },
    ProblemSpec {
        code: "target_retired",
        status: 409,
        title: "Worker retired",
        fix: "Use an active worker.",
    },
    ProblemSpec {
        code: "unknown_addressee",
        status: 422,
        title: "Unknown addressee",
        fix: "Address a job role that some active worker holds.",
    },
];

pub static MANIFEST: Manifest = Manifest {
    id: ID,
    version: env!("CARGO_PKG_VERSION"),
    contract: CONTRACT_VERSION,
    core: false,
    schema: ID,
    provides: &[Port::Handoffs],
    uses: &[Port::Identity, Port::Audit],
    scopes: &[
        scope(SCOPE_CLAIM),
        scope(SCOPE_CREATE),
        scope(SCOPE_HOLD),
        scope(SCOPE_MANAGE),
        scope(SCOPE_READ),
        scope(SCOPE_REVIEW),
    ],
    problems: &PROBLEMS,
};

/// Run by a superuser before the migrations: the piece's role, and `cortex_app`'s membership.
///
/// Idempotent, and safe when two run at once: roles are cluster-wide, so two bootstraps in two
/// databases can race, and an advisory lock (per database) cannot order them. An existing role
/// (42710) or membership, and the loser of a race (23505), are both fine (the plug's DB run 3).
pub const BOOTSTRAP_SQL: &str = r#"
DO $$ BEGIN
  BEGIN
    CREATE ROLE cortex_m_handoffs NOLOGIN;
  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
  END;
  BEGIN
    GRANT cortex_m_handoffs TO cortex_app;
  EXCEPTION WHEN unique_violation THEN NULL;
  END;
END $$;
"#;

/// The piece.
pub struct Handoffs {
    id: ModuleId,
    provider: Arc<Provider>,
}

impl Handoffs {
    pub fn new() -> Handoffs {
        Handoffs {
            id: ModuleId::parse(ID).expect("a valid module id"),
            provider: Arc::new(Provider::new()),
        }
    }
}

impl Default for Handoffs {
    fn default() -> Self {
        Self::new()
    }
}

impl Piece for Handoffs {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: self.id.clone(),
            schema: ID,
            steps: vec![Migration {
                version: 1,
                name: "handoffs",
                up: include_str!("../migrations/0001_handoffs.up.sql"),
                down: include_str!("../migrations/0001_handoffs.down.sql"),
            }],
        }
    }

    fn provide(&self, ports: &mut PortsBuilder) {
        ports.handoffs(Arc::clone(&self.provider) as Arc<dyn HandoffsApi>);
    }

    fn routes(&self, ports: &Ports) -> Routes {
        let identity = ports.identity().expect("the registry checked `uses`");
        let audit = ports.audit().expect("the registry checked `uses`");
        // The provider needs the identity port to know a caller's job roles; it only exists now.
        self.provider.attach(Arc::clone(&identity));
        let d = Deps { identity, audit };
        let (list_d, create_a, create_e, claim_a, claim_e) =
            (d.clone(), d.clone(), d.clone(), d.clone(), d.clone());
        // The order is the S3 table's. P3 replaces the remaining placeholders.
        let r = Routes::new()
            .read(
                Method::GET,
                "/v2/handoffs",
                SCOPE_READ,
                TargetRule::None,
                move |State(st): State<AppState>,
                      ctx: Ctx,
                      StrictQuery(q): StrictQuery<ListQuery>| {
                    let d = list_d.clone();
                    async move { api::list(&d, &st, &ctx, q).await }
                },
            )
            .query()
            .read(
                Method::GET,
                "/v2/handoffs/{id}",
                SCOPE_READ,
                TargetRule::None,
                |State(st): State<AppState>, ctx: Ctx| async move { api::show(&st, &ctx).await },
            );
        let r = r
            .write(
                Method::POST,
                "/v2/handoffs",
                SCOPE_CREATE,
                TargetRule::TaskAddressee,
                move |tx, ctx, b: &CreateBody| {
                    let d = create_a.clone();
                    Box::pin(async move { api::create_authorize(&d, tx, ctx, b).await })
                },
                move |tx, ctx, auth: Authorized<CreatePlan>, _b: CreateBody| {
                    let d = create_e.clone();
                    Box::pin(async move { api::create_execute(&d, tx, ctx, auth).await })
                },
            )
            .max_body(262_144);
        let r = r
            .write(
                Method::POST,
                "/v2/handoffs/{id}/claim",
                SCOPE_CLAIM,
                TargetRule::SelfWorker,
                move |tx, ctx, _b: &Option<NoBody>| {
                    let d = claim_a.clone();
                    Box::pin(async move { api::claim_authorize(&d, tx, ctx).await })
                },
                move |tx, ctx, auth: Authorized<HandoffId>, _b: Option<NoBody>| {
                    let d = claim_e.clone();
                    Box::pin(async move { api::claim_execute(&d, tx, ctx, auth).await })
                },
            )
            .max_body(16_384);
        let rel = (d.clone(), d.clone());
        let ret = (d.clone(), d.clone());
        let acc = (d.clone(), d.clone());
        let rew = (d.clone(), d.clone());
        let can = (d.clone(), d);
        r.write(
            Method::POST,
            "/v2/handoffs/{id}/release",
            SCOPE_HOLD,
            TargetRule::None,
            move |tx, ctx, b: &ReleaseBody| {
                let d = rel.0.clone();
                Box::pin(async move { api::release_authorize(&d, tx, ctx, b).await })
            },
            move |tx, ctx, auth: Authorized<api::ReleasePlan>, _b: ReleaseBody| {
                let d = rel.1.clone();
                Box::pin(async move { api::release_execute(&d, tx, ctx, auth).await })
            },
        )
        .max_body(16_384)
        .write(
            Method::POST,
            "/v2/handoffs/{id}/return",
            SCOPE_HOLD,
            TargetRule::None,
            move |tx, ctx, b: &ReturnBody| {
                let d = ret.0.clone();
                Box::pin(async move { api::return_authorize(&d, tx, ctx, b).await })
            },
            move |tx, ctx, auth: Authorized<api::ReturnPlan>, _b: ReturnBody| {
                let d = ret.1.clone();
                Box::pin(async move { api::return_execute(&d, tx, ctx, auth).await })
            },
        )
        .max_body(2_097_152)
        .write(
            Method::POST,
            "/v2/handoffs/{id}/accept",
            SCOPE_REVIEW,
            TargetRule::SelfOrProjectLead,
            move |tx, ctx, b: &ReviewBody| {
                let d = acc.0.clone();
                Box::pin(async move { api::accept_authorize(&d, tx, ctx, b).await })
            },
            move |tx, ctx, auth: Authorized<api::ReviewPlan>, _b: ReviewBody| {
                let d = acc.1.clone();
                Box::pin(async move { api::accept_execute(&d, tx, ctx, auth).await })
            },
        )
        .max_body(16_384)
        .write(
            Method::POST,
            "/v2/handoffs/{id}/rework",
            SCOPE_REVIEW,
            TargetRule::SelfOrProjectLead,
            move |tx, ctx, b: &ReviewBody| {
                let d = rew.0.clone();
                Box::pin(async move { api::rework_authorize(&d, tx, ctx, b).await })
            },
            move |tx, ctx, auth: Authorized<api::ReviewPlan>, _b: ReviewBody| {
                let d = rew.1.clone();
                Box::pin(async move { api::rework_execute(&d, tx, ctx, auth).await })
            },
        )
        .max_body(16_384)
        .write(
            Method::POST,
            "/v2/handoffs/{id}/cancel",
            SCOPE_MANAGE,
            TargetRule::Lineage,
            move |tx, ctx, b: &CancelBody| {
                let d = can.0.clone();
                Box::pin(async move { api::cancel_authorize(&d, tx, ctx, b).await })
            },
            move |tx, ctx, auth: Authorized<api::CancelPlan>, _b: CancelBody| {
                let d = can.1.clone();
                Box::pin(async move { api::cancel_execute(&d, tx, ctx, auth).await })
            },
        )
        .max_body(16_384)
    }

    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}
