//! Wave 0 plug, DB-backed: the identity fixture's migrations apply to a fresh database after
//! `platform`, and its port runs in the caller's transaction, under its own role, inside the
//! caller's project only (sam's checklist P-03, P-04 and P-06 in part; the chain's cases are
//! mira's).
//!
//! Needs `supervise.sh`'s throwaway Postgres (CV2_PG_PORT, CV2_PG_PASSWORD). Default features.
use std::sync::Arc;

use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, ModuleTx, OwnerConn, TxOptions};
use cortex_module::{IdentityApi, Piece, PortError, Role, WorkerKind};
use cortex_testkit::{ctx, empty_db};
use cortex_types::{InstallationId, ProjectId, SystemClock, TeamId, WorkerId};
use plug_identity::{BOOTSTRAP_SQL, FIXTURE_INSTALLATION, PlugIdentity, Provider};

/// A caller module from the testkit's roles (`cortex_m_test_a`).
const CALLER: &str = "test_a";

struct Fixture {
    a: ProjectId,
    b: ProjectId,
    a_lead: WorkerId,
    a_member: WorkerId,
    a_team: TeamId,
    b_worker: WorkerId,
    /// Project B is seeded WITH an installation (A takes the column's default), so the port is shown
    /// to read it from the row and not to return a constant.
    b_installation: InstallationId,
}

fn ids() -> Fixture {
    let c = SystemClock;
    Fixture {
        a: ProjectId::new(&c),
        b: ProjectId::new(&c),
        a_lead: WorkerId::new(&c),
        a_member: WorkerId::new(&c),
        a_team: TeamId::new(&c),
        b_worker: WorkerId::new(&c),
        b_installation: InstallationId::new(&c),
    }
}

/// Bootstrap the role, apply platform then the fixture's migrations, seed two projects as the
/// superuser, and connect a `cortex_app` pool that knows the caller and the provider.
async fn setup(tag: &str) -> (cortex_testkit::TestDb, Fixture, DbPool, Provider) {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("bootstrap");
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    apply(&mut owner, &PlugIdentity::new().migrations())
        .await
        .expect("plug_identity migrations");
    let f = ids();
    // A names no installation (the fixture default); B names one.
    sqlx::query(
        "INSERT INTO plug_identity.projects (id, name, lead) VALUES ($1::uuid, 'A', $2::uuid)",
    )
    .bind(f.a.to_string())
    .bind(f.a_lead.to_string())
    .execute(&mut admin)
    .await
    .expect("seed project A");
    sqlx::query(
        "INSERT INTO plug_identity.projects (id, name, lead, installation_id) VALUES ($1::uuid, 'B', $2::uuid, $3::uuid)",
    )
    .bind(f.b.to_string())
    .bind(f.b_worker.to_string())
    .bind(f.b_installation.to_string())
    .execute(&mut admin)
    .await
    .expect("seed project B");
    sqlx::query(
        "INSERT INTO plug_identity.workers (id, project_id, name, kind, role, team_id, job_roles) VALUES \
         ($1::uuid, $2::uuid, 'a-lead', 'worker', 'project_lead', NULL, '{lead}'), \
         ($3::uuid, $2::uuid, 'a-member', 'service', 'member', $4::uuid, '{builder}'), \
         ($5::uuid, $6::uuid, 'b-worker', 'worker', 'project_lead', NULL, '{}')",
    )
    .bind(f.a_lead.to_string())
    .bind(f.a.to_string())
    .bind(f.a_member.to_string())
    .bind(f.a_team.to_string())
    .bind(f.b_worker.to_string())
    .bind(f.b.to_string())
    .execute(&mut admin)
    .await
    .expect("seed workers");
    let pool = DbPool::connect(&db.db_config(&[CALLER, plug_identity::ID], Arc::new(SystemClock)))
        .await
        .expect("pool");
    let provider = provider();
    (db, f, pool, provider)
}

fn provider() -> Provider {
    Provider::new()
}

async fn begin(pool: &DbPool, project: ProjectId) -> ModuleTx {
    ModuleTx::begin(pool, ctx(CALLER, Some(project)), TxOptions::default())
        .await
        .expect("begin")
}

async fn current_user(tx: &mut ModuleTx) -> String {
    sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("current_user")
}

#[tokio::test]
async fn w0_plug_identity_port_sees_only_the_callers_project() {
    let (_db, f, pool, p) = setup("pi_project").await;

    let mut tx = begin(&pool, f.a).await;
    let roster = p.roster(&mut tx).await.expect("roster in A");
    let names: Vec<&str> = roster.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["a-lead", "a-member"], "A's workers only");
    let member = roster.iter().find(|w| w.id == f.a_member).expect("member");
    assert_eq!(
        (member.kind, member.role),
        (WorkerKind::Service, Role::Member(Some(f.a_team)))
    );
    let project = p.project(&mut tx).await.expect("project A");
    // Wave 1 ports3: the plug has no lifetime column; its fixed answer is the default.
    assert_eq!(
        project.token_lifetime_days, 90,
        "the plug's default lifetime"
    );
    assert_eq!((project.id, project.lead), (f.a, f.a_lead));
    // Delta 5: the installation comes from the row, under the project's RLS. A names none, so it
    // has the fixture default.
    assert_eq!(
        project.installation,
        InstallationId::parse(FIXTURE_INSTALLATION).expect("a UUIDv7")
    );
    // B's worker, asked for from A: absent, not an error.
    assert_eq!(p.worker(&mut tx, &f.b_worker).await, Ok(None));
    assert!(p.worker(&mut tx, &f.a_lead).await.expect("lead").is_some());
    // The caller's role is back after each delegated call.
    assert_eq!(current_user(&mut tx).await, "cortex_m_test_a");
    tx.rollback().await.expect("rollback");

    // Control: from B, B's worker only.
    let mut tx = begin(&pool, f.b).await;
    let roster = p.roster(&mut tx).await.expect("roster in B");
    assert_eq!(
        roster.iter().map(|w| w.id).collect::<Vec<_>>(),
        [f.b_worker]
    );
    // B's own installation, not A's and not the default.
    let project = p.project(&mut tx).await.expect("project B");
    assert_eq!(project.installation, f.b_installation);
    assert_ne!(
        project.installation,
        InstallationId::parse(FIXTURE_INSTALLATION).expect("a UUIDv7")
    );
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn w0_plug_identity_without_a_project_sees_nothing() {
    let (_db, _f, pool, p) = setup("pi_noproject").await;
    let mut tx = ModuleTx::begin(&pool, ctx(CALLER, None), TxOptions::default())
        .await
        .expect("begin");
    assert_eq!(p.roster(&mut tx).await, Ok(vec![]));
    assert_eq!(p.project(&mut tx).await, Err(PortError::NotFound));
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn w0_plug_identity_role_reads_but_never_writes() {
    let (db, _f, _pool, _p) = setup("pi_readonly").await;
    let mut admin = db.admin().await;
    let grants: Vec<String> = sqlx::query_scalar(
        "SELECT privilege_type FROM information_schema.role_table_grants \
         WHERE grantee = 'cortex_m_plug_identity' AND table_schema = 'plug_identity' \
         ORDER BY table_name, privilege_type",
    )
    .fetch_all(&mut admin)
    .await
    .expect("grants");
    assert_eq!(
        grants,
        ["SELECT", "SELECT"],
        "SELECT on the two tables, nothing else"
    );
    let forced: Vec<bool> = sqlx::query_scalar(
        "SELECT relforcerowsecurity FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'plug_identity' AND c.relkind = 'r' ORDER BY c.relname",
    )
    .fetch_all(&mut admin)
    .await
    .expect("rls");
    assert_eq!(
        forced,
        [true, true],
        "FORCE ROW LEVEL SECURITY on both tables"
    );
}

#[tokio::test]
async fn w0_plug_identity_migrations_apply_once() {
    let (db, _f, _pool, _p) = setup("pi_once").await;
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    // A second startup applies nothing and keeps the rows.
    apply(&mut owner, &PlugIdentity::new().migrations())
        .await
        .expect("again");
    let mut admin = db.admin().await;
    let (steps, rows): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM platform.migrations WHERE module = 'plug_identity'), \
                (SELECT count(*) FROM plug_identity.workers)",
    )
    .fetch_one(&mut admin)
    .await
    .expect("counts");
    assert_eq!((steps, rows), (1, 3));
}
