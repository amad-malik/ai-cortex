//! The shared two-project fixture: roles, platform, the S2 r6 migration, and A, B, C seeded as the
//! superuser in one transaction (the lead foreign key is deferred). Project C has a lead and nothing else.
#![allow(dead_code)]

use cortex_db::OwnerConn;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_module::Piece;
use cortex_testkit::{TestDb, empty_db};
use cortex_types::{InstallationId, ProjectId, SystemClock, TeamId, WorkerId};
use mod_identity::{BOOTSTRAP_SQL, ModIdentity};
use sqlx::Connection;

#[derive(Clone, Copy)]
pub struct F {
    pub inst: InstallationId,
    pub a: ProjectId,
    pub b: ProjectId,
    pub c: ProjectId,
    pub c_lead: WorkerId,
    pub t1: TeamId,
    pub tb: TeamId,
    pub a_lead: WorkerId,
    pub a_tl: WorkerId,
    pub a_m1: WorkerId,
    pub a_m2: WorkerId,
    pub a_svc: WorkerId,
    pub a_ret: WorkerId,
    pub b_lead: WorkerId,
    pub b_worker: WorkerId,
}

pub fn ids() -> F {
    let c = SystemClock;
    F {
        inst: InstallationId::new(&c),
        a: ProjectId::new(&c),
        b: ProjectId::new(&c),
        c: ProjectId::new(&c),
        c_lead: WorkerId::new(&c),
        t1: TeamId::new(&c),
        tb: TeamId::new(&c),
        a_lead: WorkerId::new(&c),
        a_tl: WorkerId::new(&c),
        a_m1: WorkerId::new(&c),
        a_m2: WorkerId::new(&c),
        a_svc: WorkerId::new(&c),
        a_ret: WorkerId::new(&c),
        b_lead: WorkerId::new(&c),
        b_worker: WorkerId::new(&c),
    }
}

/// One worker row and its current name, as the superuser seeds it.
pub async fn add_worker(
    tx: &mut sqlx::PgConnection,
    id: WorkerId,
    project: ProjectId,
    label: &str,
    kind: &str,
    retired: bool,
    job_roles: &[&str],
) {
    sqlx::query(
        "INSERT INTO identity.workers (id, project_id, kind, state, retired_at, job_roles) \
         VALUES ($1::uuid, $2::uuid, $3, CASE WHEN $4 THEN 'retired' ELSE 'active' END, \
                 CASE WHEN $4 THEN now() END, $5::text[])",
    )
    .bind(id.to_string())
    .bind(project.to_string())
    .bind(kind)
    .bind(retired)
    .bind(
        job_roles
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>(),
    )
    .execute(&mut *tx)
    .await
    .expect("seed worker");
    add_name(
        tx,
        "worker",
        id.to_string(),
        project,
        project.to_string(),
        label,
    )
    .await;
}

pub async fn add_name(
    tx: &mut sqlx::PgConnection,
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
    .execute(&mut *tx)
    .await
    .expect("seed name");
}

pub async fn add_member(
    tx: &mut sqlx::PgConnection,
    w: WorkerId,
    t: TeamId,
    p: ProjectId,
    role: &str,
) {
    sqlx::query(
        "INSERT INTO identity.team_members (worker_id, team_id, project_id, role) VALUES ($1::uuid, $2::uuid, $3::uuid, $4)",
    )
    .bind(w.to_string())
    .bind(t.to_string())
    .bind(p.to_string())
    .bind(role)
    .execute(&mut *tx)
    .await
    .expect("seed member");
}

pub async fn add_team(tx: &mut sqlx::PgConnection, t: TeamId, p: ProjectId, label: &str) {
    sqlx::query("INSERT INTO identity.teams (id, project_id) VALUES ($1::uuid, $2::uuid)")
        .bind(t.to_string())
        .bind(p.to_string())
        .execute(&mut *tx)
        .await
        .expect("seed team");
    add_name(tx, "team", t.to_string(), p, p.to_string(), label).await;
}

/// Roles, platform, then the piece's migrations; two projects in ONE installation, seeded as the
/// superuser in one transaction (the lead foreign key is deferred).
pub async fn migrate_only(tag: &str) -> TestDb {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("bootstrap");
    // The socket's provisioning creates cortex_install_admin; the testkit does not yet (finn's wave1/ports item d).
    sqlx::raw_sql(
        "DO $$ BEGIN CREATE ROLE cortex_install_admin NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END $$;",
    )
    .execute(&mut admin)
    .await
    .expect("install admin role");
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    apply(&mut owner, &ModIdentity::new().migrations())
        .await
        .expect("identity migrations (S2 r6) apply");
    db
}

/// Roles, platform, the S2 r6 migration, and projects A, B and C seeded as the superuser.
pub async fn migrate_and_seed(tag: &str) -> (TestDb, F) {
    let db = migrate_only(tag).await;
    let mut admin = db.admin().await;
    let f = ids();
    let mut tx = admin.begin().await.expect("seed tx");
    sqlx::query("INSERT INTO identity.installations (id, label) VALUES ($1::uuid, 'inst')")
        .bind(f.inst.to_string())
        .execute(&mut *tx)
        .await
        .expect("installation");
    for (p, lead) in [(f.a, f.a_lead), (f.b, f.b_lead), (f.c, f.c_lead)] {
        sqlx::query(
            "INSERT INTO identity.projects (id, installation_id, lead_worker_id) VALUES ($1::uuid, $2::uuid, $3::uuid)",
        )
        .bind(p.to_string())
        .bind(f.inst.to_string())
        .bind(lead.to_string())
        .execute(&mut *tx)
        .await
        .expect("project");
    }
    add_name(
        &mut tx,
        "project",
        f.a.to_string(),
        f.a,
        f.inst.to_string(),
        "Alpha",
    )
    .await;
    add_name(
        &mut tx,
        "project",
        f.b.to_string(),
        f.b,
        f.inst.to_string(),
        "Beta",
    )
    .await;
    add_name(
        &mut tx,
        "project",
        f.c.to_string(),
        f.c,
        f.inst.to_string(),
        "Gamma",
    )
    .await;
    add_team(&mut tx, f.t1, f.a, "Platform").await;
    add_team(&mut tx, f.tb, f.b, "Core").await;
    add_worker(&mut tx, f.a_lead, f.a, "Zed", "worker", false, &["lead"]).await;
    add_worker(
        &mut tx,
        f.a_tl,
        f.a,
        "alice",
        "worker",
        false,
        &["builder", "reviewer"],
    )
    .await;
    add_worker(&mut tx, f.a_m1, f.a, "Bob", "worker", false, &[]).await;
    add_worker(&mut tx, f.a_m2, f.a, "carol", "worker", false, &["qa"]).await;
    add_worker(&mut tx, f.a_svc, f.a, "svc-1", "service", false, &[]).await;
    add_worker(&mut tx, f.a_ret, f.a, "dave", "worker", true, &[]).await;
    add_worker(&mut tx, f.b_lead, f.b, "bl", "worker", false, &[]).await;
    add_worker(&mut tx, f.b_worker, f.b, "bw", "worker", false, &["b-role"]).await;
    add_worker(&mut tx, f.c_lead, f.c, "cl", "worker", false, &[]).await;
    add_member(&mut tx, f.a_tl, f.t1, f.a, "team_lead").await;
    add_member(&mut tx, f.a_m1, f.t1, f.a, "member").await;
    add_member(&mut tx, f.a_ret, f.t1, f.a, "member").await;
    tx.commit().await.expect("seed commit");
    (db, f)
}
