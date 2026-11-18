//! The database harness for the stage B cells (needs `supervise.sh`'s throwaway Postgres: `CV2_DB=1`). One fresh
//! database per cell: roles, platform, the S2 identity pack, the tokens migration, and two seeded projects. The
//! `IdentityApi` here is a SQL-backed double (the `worker` query of mod-identity's provider, copied: a piece must
//! not depend on another piece), so the verifier meets the real `identity` tables.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cortex_db::migrate::{Migration, MigrationSet, apply, platform_migrations};
use cortex_db::{DbError, DbPool, ModuleTx, OwnerConn};
use cortex_module::{IdentityApi, Piece, PortError, ProjectView, Role, WorkerKind, WorkerView};
use cortex_testkit::{TestDb, empty_db};
use cortex_types::{
    InstallationId, ModuleId, ProjectId, SystemClock, TeamId, TestClock, Timestamp, WorkerId,
};
use mod_tokens::store::SqlStore;
use mod_tokens::verify::Verifier;
use mod_tokens::{BOOTSTRAP_SQL, ModTokens};
use sqlx::Connection;

pub const NOW: i64 = 1_800_000_000_000_000;
pub const DAY: i64 = 86_400_000_000;

const IDENTITY_BOOTSTRAP: &str = "DO $$ BEGIN\n  BEGIN\n    CREATE ROLE cortex_m_identity NOLOGIN;\n  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;\n  END;\n  BEGIN\n    GRANT cortex_m_identity TO cortex_app;\n  EXCEPTION WHEN unique_violation THEN NULL;\n  END;\n  BEGIN\n    CREATE ROLE cortex_m_audit NOLOGIN;\n  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;\n  END;\nEND $$;";

/// A SQL-backed `IdentityApi`: `worker` only, over the real `identity` tables, in the caller's transaction.
pub struct SqlIdentity;

#[async_trait]
impl IdentityApi for SqlIdentity {
    async fn project(&self, tx: &mut ModuleTx) -> Result<ProjectView, PortError> {
        let id = ModuleId::parse("identity").expect("id");
        let row: Option<(String, String, String, String, String, i32)> = tx
            .delegate(&id, async |t: &mut ModuleTx| {
                Ok(sqlx::query_as(
                    "SELECT p.id::text, n.label, p.state, p.lead_worker_id::text, p.installation_id::text, \
                            p.token_lifetime_days \
                     FROM identity.projects p \
                     JOIN identity.names n ON n.entity_kind = 'project' AND n.entity_id = p.id AND n.current",
                )
                .fetch_optional(t.executor()?)
                .await?)
            })
            .await?;
        let (pid, name, state, lead, inst, days) = row.ok_or(PortError::NotFound)?;
        let bad = |_| PortError::Unavailable;
        Ok(ProjectView {
            id: ProjectId::parse(&pid).map_err(bad)?,
            name,
            state,
            lead: WorkerId::parse(&lead).map_err(bad)?,
            installation: InstallationId::parse(&inst).map_err(bad)?,
            token_lifetime_days: days,
        })
    }
    async fn worker(
        &self,
        tx: &mut ModuleTx,
        w: &WorkerId,
    ) -> Result<Option<WorkerView>, PortError> {
        Ok(workers(tx, Some(*w)).await?.into_iter().next())
    }
    async fn roster(&self, tx: &mut ModuleTx) -> Result<Vec<WorkerView>, PortError> {
        Ok(workers(tx, None).await?)
    }
}

type Row = (
    String,
    String,
    String,
    String,
    Vec<String>,
    Option<String>,
    Option<String>,
    bool,
);

/// One worker (by id, any state) or the active roster, over the real `identity` tables, in the caller's
/// transaction (the project is its row-level security).
async fn workers(tx: &mut ModuleTx, only: Option<WorkerId>) -> Result<Vec<WorkerView>, DbError> {
    let id = ModuleId::parse("identity").expect("id");
    let rows: Vec<Row> = tx
        .delegate(&id, async move |t: &mut ModuleTx| {
            Ok(sqlx::query_as(
                "SELECT w.id::text, n.label, w.kind, w.state, w.job_roles, tm.role, tm.team_id::text, \
                        (p.lead_worker_id = w.id) \
                 FROM identity.workers w \
                 JOIN identity.names n ON n.entity_kind = 'worker' AND n.entity_id = w.id AND n.current \
                 JOIN identity.projects p ON p.id = w.project_id \
                 LEFT JOIN identity.team_members tm ON tm.worker_id = w.id \
                 WHERE ($1::text IS NULL AND w.state = 'active') OR w.id = $1::uuid \
                 ORDER BY w.id",
            )
            .bind(only.map(|w| w.to_string()))
            .fetch_all(t.executor()?)
            .await?)
        })
        .await?;
    rows.into_iter()
        .map(
            |(id, name, kind, state, job_roles, member_role, team, lead)| {
                let bad = || DbError::Other("bad identity row".into());
                let team = team
                    .map(|t| TeamId::parse(&t).map_err(|_| bad()))
                    .transpose()?;
                let role = match (lead, member_role.as_deref(), team) {
                    (true, _, _) => Role::ProjectLead,
                    (false, Some("team_lead"), Some(t)) => Role::TeamLead(t),
                    (false, Some("member"), t) => Role::Member(t),
                    (false, None, _) => Role::Member(None),
                    _ => return Err(bad()),
                };
                Ok(WorkerView {
                    id: WorkerId::parse(&id).map_err(|_| bad())?,
                    name,
                    kind: if kind == "service" {
                        WorkerKind::Service
                    } else {
                        WorkerKind::Worker
                    },
                    role,
                    job_roles,
                    active: state == "active",
                })
            },
        )
        .collect()
}

/// The fixture ids: project A (lead, team lead of T, members W1 and W2 of T, W3 of U, W4 with no team, a retired
/// WR) and project B (lead LB, worker WB1).
#[derive(Clone, Copy)]
pub struct Seed {
    pub inst: InstallationId,
    pub a: ProjectId,
    pub b: ProjectId,
    pub t: TeamId,
    pub u: TeamId,
    pub lead: WorkerId,
    pub tl: WorkerId,
    pub ul: WorkerId,
    pub w1: WorkerId,
    pub w2: WorkerId,
    pub w3: WorkerId,
    pub w4: WorkerId,
    pub wr: WorkerId,
    pub lb: WorkerId,
    pub wb1: WorkerId,
}

pub fn seed_ids() -> Seed {
    let c = SystemClock;
    Seed {
        inst: InstallationId::new(&c),
        a: ProjectId::new(&c),
        b: ProjectId::new(&c),
        t: TeamId::new(&c),
        u: TeamId::new(&c),
        lead: WorkerId::new(&c),
        tl: WorkerId::new(&c),
        ul: WorkerId::new(&c),
        w1: WorkerId::new(&c),
        w2: WorkerId::new(&c),
        w3: WorkerId::new(&c),
        w4: WorkerId::new(&c),
        wr: WorkerId::new(&c),
        lb: WorkerId::new(&c),
        wb1: WorkerId::new(&c),
    }
}

async fn add_name(
    c: &mut sqlx::PgConnection,
    kind: &str,
    entity: String,
    project: ProjectId,
    scope: String,
    label: &str,
) {
    sqlx::query(
        "INSERT INTO identity.names (entity_kind, entity_id, scope_id, project_id, name_key, label, current) \
         VALUES ($1, $2::uuid, $3::uuid, $4::uuid, lower($5), $5, true)",
    )
    .bind(kind)
    .bind(entity)
    .bind(scope)
    .bind(project.to_string())
    .bind(label)
    .execute(&mut *c)
    .await
    .expect("seed name");
}

async fn add_worker(
    c: &mut sqlx::PgConnection,
    id: WorkerId,
    project: ProjectId,
    label: &str,
    retired: bool,
) {
    sqlx::query(
        "INSERT INTO identity.workers (id, project_id, kind, state, retired_at, job_roles) \
         VALUES ($1::uuid, $2::uuid, 'worker', CASE WHEN $3 THEN 'retired' ELSE 'active' END, \
                 CASE WHEN $3 THEN now() END, '{}'::text[])",
    )
    .bind(id.to_string())
    .bind(project.to_string())
    .bind(retired)
    .execute(&mut *c)
    .await
    .expect("seed worker");
    add_name(
        c,
        "worker",
        id.to_string(),
        project,
        project.to_string(),
        label,
    )
    .await;
}

async fn add_team(c: &mut sqlx::PgConnection, t: TeamId, p: ProjectId, label: &str) {
    sqlx::query("INSERT INTO identity.teams (id, project_id) VALUES ($1::uuid, $2::uuid)")
        .bind(t.to_string())
        .bind(p.to_string())
        .execute(&mut *c)
        .await
        .expect("seed team");
    add_name(c, "team", t.to_string(), p, p.to_string(), label).await;
}

async fn add_member(c: &mut sqlx::PgConnection, w: WorkerId, t: TeamId, p: ProjectId, role: &str) {
    sqlx::query("INSERT INTO identity.team_members (worker_id, team_id, project_id, role) VALUES ($1::uuid, $2::uuid, $3::uuid, $4)")
        .bind(w.to_string())
        .bind(t.to_string())
        .bind(p.to_string())
        .bind(role)
        .execute(&mut *c)
        .await
        .expect("seed member");
}

/// Everything a stage B cell needs.
pub struct Env {
    pub db: TestDb,
    pub seed: Seed,
    pub pool: Arc<DbPool>,
    pub clock: Arc<TestClock>,
}

impl Env {
    /// A verifier over the SQL store at the harness clock.
    pub fn verifier(&self) -> Verifier {
        self.verifier_with(SqlStore::new(
            self.pool.clone(),
            Arc::new(SqlIdentity),
            self.clock.clone(),
        ))
    }

    pub fn verifier_with(&self, store: SqlStore) -> Verifier {
        Verifier::new(Arc::new(store), self.clock.clone())
    }

    pub fn set_now(&self, micros: i64) {
        self.clock.set(Timestamp::from_unix_micros(micros));
    }
}

pub async fn setup(tag: &str) -> Env {
    let (db, s) = bootstrap(tag).await;
    let clock = Arc::new(TestClock::at(Timestamp::from_unix_micros(NOW)));
    let pool = Arc::new(
        DbPool::connect(&db.db_config(&["tokens", "identity", "platform"], clock.clone()))
            .await
            .expect("pool"),
    );
    Env {
        db,
        seed: s,
        pool,
        clock,
    }
}

/// Roles, platform, the identity pack, the tokens migration and the seed (two projects). No pool.
pub async fn bootstrap(tag: &str) -> (TestDb, Seed) {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("tokens bootstrap");
    sqlx::raw_sql(IDENTITY_BOOTSTRAP)
        .execute(&mut admin)
        .await
        .expect("identity bootstrap");
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    let identity_sql: &'static str = Box::leak(super::s2_identity_sql().into_boxed_str());
    apply(
        &mut owner,
        &MigrationSet {
            module: ModuleId::parse("identity").expect("id"),
            schema: "identity",
            steps: vec![Migration {
                version: 1,
                name: "identity",
                up: identity_sql,
                down: "DROP SCHEMA identity CASCADE;",
            }],
        },
    )
    .await
    .expect("identity pack");
    apply(&mut owner, &ModTokens::new().migrations())
        .await
        .expect("tokens pack");

    let s = seed_ids();
    let mut tx = admin.begin().await.expect("seed tx");
    sqlx::query("INSERT INTO identity.installations (id, label) VALUES ($1::uuid, 'inst')")
        .bind(s.inst.to_string())
        .execute(&mut *tx)
        .await
        .expect("installation");
    for (p, lead, label) in [(s.a, s.lead, "Alpha"), (s.b, s.lb, "Beta")] {
        sqlx::query("INSERT INTO identity.projects (id, installation_id, lead_worker_id) VALUES ($1::uuid, $2::uuid, $3::uuid)")
            .bind(p.to_string())
            .bind(s.inst.to_string())
            .bind(lead.to_string())
            .execute(&mut *tx)
            .await
            .expect("project");
        add_name(
            &mut tx,
            "project",
            p.to_string(),
            p,
            s.inst.to_string(),
            label,
        )
        .await;
    }
    add_team(&mut tx, s.t, s.a, "Tee").await;
    add_team(&mut tx, s.u, s.a, "You").await;
    for (w, p, l, r) in [
        (s.lead, s.a, "lead", false),
        (s.tl, s.a, "tl", false),
        (s.ul, s.a, "ul", false),
        (s.w1, s.a, "w1", false),
        (s.w2, s.a, "w2", false),
        (s.w3, s.a, "w3", false),
        (s.w4, s.a, "w4", false),
        (s.wr, s.a, "wr", true),
        (s.lb, s.b, "lb", false),
        (s.wb1, s.b, "wb1", false),
    ] {
        add_worker(&mut tx, w, p, l, r).await;
    }
    add_member(&mut tx, s.tl, s.t, s.a, "team_lead").await;
    add_member(&mut tx, s.w1, s.t, s.a, "member").await;
    add_member(&mut tx, s.w2, s.t, s.a, "member").await;
    add_member(&mut tx, s.ul, s.u, s.a, "team_lead").await;
    add_member(&mut tx, s.w3, s.u, s.a, "member").await;
    tx.commit().await.expect("seed commit");
    (db, s)
}

/// Short statement timeout for the cell that makes the lookup wait on a lock.
pub const SHORT: Duration = Duration::from_millis(300);

/// Inserts a token row as the module role, in the project, exactly as an issue would (no handler needed): used to
/// give the fixture workers tokens. Returns the bearer and the token id.
pub async fn insert_token(
    pool: &DbPool,
    clock: &dyn cortex_types::Clock,
    worker: WorkerId,
    project: ProjectId,
    install_gen: i64,
    project_gen: i64,
) -> (String, cortex_types::TokenId) {
    let g = mod_tokens::token::generate(clock).expect("generated");
    let bearer = g.token().expose().to_owned();
    let issued = clock.now().unix_micros();
    let mut tx = ModuleTx::begin(
        pool,
        cortex_testkit::ctx("tokens", Some(project)),
        cortex_db::TxOptions::default(),
    )
    .await
    .expect("begin");
    sqlx::query(
        "INSERT INTO tokens.tokens (id, worker_id, project_id, digest, install_gen, project_gen, issued_at, \
                                    expires_at, renew_after, issued_by) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, \
                 timestamptz 'epoch' + $7::bigint * interval '1 microsecond', \
                 timestamptz 'epoch' + $8::bigint * interval '1 microsecond', \
                 timestamptz 'epoch' + $9::bigint * interval '1 microsecond', \
                 jsonb_build_object('kind', 'host'))",
    )
    .bind(g.id().to_string())
    .bind(worker.to_string())
    .bind(project.to_string())
    .bind(g.digest().to_vec())
    .bind(install_gen)
    .bind(project_gen)
    .bind(issued)
    .bind(issued + 90 * DAY)
    .bind(issued + 60 * DAY)
    .execute(tx.executor().expect("executor"))
    .await
    .expect("insert");
    tx.commit().await.expect("commit");
    (bearer, g.id())
}
