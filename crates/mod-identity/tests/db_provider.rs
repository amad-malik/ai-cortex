//! 2a, DB-backed: the migration applies (S2 r6), the `IdentityApi` provider reads the real tables in
//! the caller's transaction, inside the caller's project only.
//!
//! Rows: ID-43 (two-project matrix, RLS), ID-44, ID-45, ID-46, ID-47, ID-48, ID-59 (installation).
//! Needs `supervise.sh`'s throwaway Postgres (CV2_PG_PORT, CV2_PG_PASSWORD). RED: the provider is
//! `todo!()`.
use std::sync::Arc;

use cortex_db::{DbError, DbPool, ModuleTx, TxOptions};
use cortex_module::{IdentityApi, PortError, Role, WorkerKind};
use cortex_testkit::{TestDb, ctx};
use cortex_types::{ModuleId, ProjectId, SystemClock, TeamId, WorkerId};
use mod_identity::Provider;

const CALLER: &str = "test_a";

mod common;

use common::seed::{F, migrate_and_seed};

async fn setup(tag: &str) -> (TestDb, F, DbPool, Provider) {
    let (db, f) = migrate_and_seed(tag).await;
    let pool = DbPool::connect(&db.db_config(&[CALLER, mod_identity::ID], Arc::new(SystemClock)))
        .await
        .expect("pool");
    (db, f, pool, Provider::new())
}

async fn begin(pool: &DbPool, project: Option<ProjectId>) -> ModuleTx {
    ModuleTx::begin(pool, ctx(CALLER, project), TxOptions::default())
        .await
        .expect("begin")
}

fn id_module() -> ModuleId {
    ModuleId::parse(mod_identity::ID).expect("module id")
}

macro_rules! count {
    ($tx:expr, $table:literal) => {{
        let id = id_module();
        $tx.delegate(&id, async |t: &mut ModuleTx| -> Result<i64, DbError> {
            Ok(
                sqlx::query_scalar(concat!("SELECT count(*) FROM identity.", $table))
                    .fetch_one(t.executor()?)
                    .await?,
            )
        })
        .await
    }};
}

#[tokio::test]
async fn id_44_59_project_is_the_callers_with_its_lead_and_the_installation_from_the_row() {
    let (_db, f, pool, p) = setup("mi_project").await;
    let mut tx = begin(&pool, Some(f.a)).await;
    let v = p.project(&mut tx).await.expect("project A");
    assert_eq!((v.id, v.lead), (f.a, f.a_lead));
    assert_eq!(v.name, "Alpha", "the current name, as typed");
    assert_eq!(v.state, "open");
    assert_eq!(
        v.installation, f.inst,
        "ProjectView.installation comes from projects.installation_id"
    );
    drop(tx);
    // Two projects of one install: the same value (BO-11); B sees its own project.
    let mut tx = begin(&pool, Some(f.b)).await;
    let vb = p.project(&mut tx).await.expect("project B");
    assert_eq!((vb.id, vb.lead, vb.installation), (f.b, f.b_lead, f.inst));
    drop(tx);
    // No project in the transaction: nothing to show (the port answers as for an unknown id).
    let mut tx = begin(&pool, None).await;
    assert_eq!(p.project(&mut tx).await, Err(PortError::NotFound));
}

#[tokio::test]
async fn id_45_47_worker_views_roles_kinds_and_the_other_projects_worker_is_none() {
    let (_db, f, pool, p) = setup("mi_worker").await;
    let mut tx = begin(&pool, Some(f.a)).await;

    let lead = p
        .worker(&mut tx, &f.a_lead)
        .await
        .expect("ok")
        .expect("lead");
    assert_eq!(
        (lead.role, lead.kind, lead.active),
        (Role::ProjectLead, WorkerKind::Worker, true)
    );
    assert_eq!(
        (lead.name.as_str(), lead.job_roles.clone()),
        ("Zed", vec!["lead".to_owned()])
    );

    let tl = p
        .worker(&mut tx, &f.a_tl)
        .await
        .expect("ok")
        .expect("team lead");
    assert_eq!(tl.role, Role::TeamLead(f.t1));
    assert_eq!(
        tl.job_roles,
        ["builder", "reviewer"],
        "job roles keep their stored order"
    );

    let m1 = p
        .worker(&mut tx, &f.a_m1)
        .await
        .expect("ok")
        .expect("member");
    assert_eq!(m1.role, Role::Member(Some(f.t1)));
    let m2 = p
        .worker(&mut tx, &f.a_m2)
        .await
        .expect("ok")
        .expect("member in no team");
    assert_eq!(m2.role, Role::Member(None));

    let svc = p
        .worker(&mut tx, &f.a_svc)
        .await
        .expect("ok")
        .expect("service");
    assert_eq!(svc.kind, WorkerKind::Service);

    let ret = p
        .worker(&mut tx, &f.a_ret)
        .await
        .expect("ok")
        .expect("retired is still shown");
    assert!(!ret.active, "retired: active = false");
    assert_eq!(ret.role, Role::Member(Some(f.t1)));

    // B's worker under A's transaction, and a random id: the same answer.
    assert_eq!(p.worker(&mut tx, &f.b_worker).await, Ok(None));
    assert_eq!(
        p.worker(&mut tx, &WorkerId::new(&SystemClock)).await,
        Ok(None)
    );
}

#[tokio::test]
async fn id_46_roster_is_the_projects_active_workers_ordered_by_name_key_then_id() {
    let (_db, f, pool, p) = setup("mi_roster").await;
    let mut tx = begin(&pool, Some(f.a)).await;
    let names: Vec<String> = p
        .roster(&mut tx)
        .await
        .expect("roster A")
        .into_iter()
        .map(|w| w.name)
        .collect();
    // name_key order (case-insensitive), the retired worker `dave` and project B's workers absent.
    assert_eq!(names, ["alice", "Bob", "carol", "svc-1", "Zed"]);
    drop(tx);
    let mut tx = begin(&pool, Some(f.b)).await;
    let names: Vec<String> = p
        .roster(&mut tx)
        .await
        .expect("roster B")
        .into_iter()
        .map(|w| w.name)
        .collect();
    assert_eq!(names, ["bl", "bw"]);
    drop(tx);
    // The order is the BYTE order of the key (COLLATE "C"), not the database locale's: a hyphen sorts
    // before a digit or a letter, so `k-b` comes before `ka` whatever the locale does with punctuation.
    let id = id_module();
    let mut tx = begin(&pool, Some(f.a)).await;
    for (label, key) in [("ka", "ka"), ("k-b", "k-b")] {
        let new = WorkerId::new(&SystemClock);
        tx.delegate(&id, async move |t: &mut ModuleTx| -> Result<(), DbError> {
            sqlx::query("INSERT INTO identity.workers (id, project_id, kind) VALUES ($1::uuid, $2::uuid, 'worker')")
                .bind(new.to_string())
                .bind(f.a.to_string())
                .execute(t.executor()?)
                .await?;
            sqlx::query(
                "INSERT INTO identity.names (entity_kind, entity_id, scope_id, project_id, name_key, label, current) \
                 VALUES ('worker', $1::uuid, $2::uuid, $2::uuid, $3, $4, true)",
            )
            .bind(new.to_string())
            .bind(f.a.to_string())
            .bind(key)
            .bind(label)
            .execute(t.executor()?)
            .await?;
            Ok(())
        })
        .await
        .expect("insert in the caller's transaction");
    }
    let names: Vec<String> = p
        .roster(&mut tx)
        .await
        .expect("roster")
        .into_iter()
        .map(|w| w.name)
        .collect();
    let pos = |n: &str| names.iter().position(|x| x == n).expect("present");
    assert!(pos("k-b") < pos("ka"), "byte order: {names:?}");
}

#[tokio::test]
async fn id_48_the_port_runs_in_the_callers_transaction() {
    let (_db, f, pool, p) = setup("mi_calltx").await;
    let new = WorkerId::new(&SystemClock);
    let id = id_module();
    let mut tx = begin(&pool, Some(f.a)).await;
    // A worker written earlier in the SAME, uncommitted transaction (as the identity role).
    tx.delegate(&id, async |t: &mut ModuleTx| -> Result<(), DbError> {
        sqlx::query("INSERT INTO identity.workers (id, project_id, kind) VALUES ($1::uuid, $2::uuid, 'worker')")
            .bind(new.to_string())
            .bind(f.a.to_string())
            .execute(t.executor()?)
            .await?;
        sqlx::query(
            "INSERT INTO identity.names (entity_kind, entity_id, scope_id, project_id, name_key, label, current) \
             VALUES ('worker', $1::uuid, $2::uuid, $2::uuid, 'eve', 'Eve', true)",
        )
        .bind(new.to_string())
        .bind(f.a.to_string())
        .execute(t.executor()?)
        .await?;
        Ok(())
    })
    .await
    .expect("insert in the caller's transaction");
    let seen = p
        .worker(&mut tx, &new)
        .await
        .expect("ok")
        .expect("visible in the same transaction");
    assert_eq!(seen.name, "Eve");
    tx.rollback().await.expect("rollback");
    // Rolled back with the caller's work: a new transaction does not see it.
    let mut tx = begin(&pool, Some(f.a)).await;
    assert_eq!(p.worker(&mut tx, &new).await, Ok(None));
}

#[tokio::test]
async fn id_43_two_project_matrix_and_the_guc_edge_cases() {
    let (db, f, pool, _p) = setup("mi_rls").await;
    // A sees A's rows on every table; the installation row is install-level (no module policy).
    let mut tx = begin(&pool, Some(f.a)).await;
    assert_eq!(
        count!(tx, "installations"),
        Ok(0),
        "installations: host paths only"
    );
    assert_eq!(count!(tx, "projects"), Ok(1));
    assert_eq!(count!(tx, "teams"), Ok(1));
    assert_eq!(count!(tx, "workers"), Ok(6));
    assert_eq!(count!(tx, "team_members"), Ok(3));
    assert_eq!(count!(tx, "names"), Ok(8));
    drop(tx);
    let mut tx = begin(&pool, Some(f.b)).await;
    assert_eq!(count!(tx, "projects"), Ok(1));
    assert_eq!(count!(tx, "teams"), Ok(1));
    assert_eq!(count!(tx, "workers"), Ok(2));
    assert_eq!(count!(tx, "team_members"), Ok(0));
    assert_eq!(count!(tx, "names"), Ok(4));
    drop(tx);
    // Unset: zero rows.
    let mut tx = begin(&pool, None).await;
    assert_eq!(count!(tx, "workers"), Ok(0));
    assert_eq!(count!(tx, "projects"), Ok(0));
    drop(tx);
    // Empty string: zero rows.
    let id = id_module();
    let mut tx = begin(&pool, Some(f.a)).await;
    tx.delegate(&id, async |t: &mut ModuleTx| -> Result<(), DbError> {
        sqlx::query("SELECT set_config('cortex.project_id', '', true)")
            .execute(t.executor()?)
            .await?;
        Ok(())
    })
    .await
    .expect("set empty");
    assert_eq!(count!(tx, "workers"), Ok(0));
    drop(tx);
    // A value that is not a UUID errors and returns nothing.
    let mut tx = begin(&pool, Some(f.a)).await;
    tx.delegate(&id, async |t: &mut ModuleTx| -> Result<(), DbError> {
        sqlx::query("SELECT set_config('cortex.project_id', 'not-a-uuid', true)")
            .execute(t.executor()?)
            .await?;
        Ok(())
    })
    .await
    .expect("set junk");
    assert!(
        count!(tx, "workers").is_err(),
        "a non-UUID project id errors"
    );
    drop(tx);
    // Inserting B's project id under A fails with 42501; updating B's row under A changes 0 rows.
    let mut tx = begin(&pool, Some(f.a)).await;
    let r = tx
        .delegate(&id, async |t: &mut ModuleTx| -> Result<(), DbError> {
            sqlx::query("INSERT INTO identity.teams (id, project_id) VALUES ($1::uuid, $2::uuid)")
                .bind(TeamId::new(&SystemClock).to_string())
                .bind(f.b.to_string())
                .execute(t.executor()?)
                .await?;
            Ok(())
        })
        .await;
    assert_eq!(
        r,
        Err(DbError::PermissionDenied),
        "WITH CHECK refuses another project's row"
    );
    drop(tx);
    let mut tx = begin(&pool, Some(f.a)).await;
    let n = tx
        .delegate(&id, async |t: &mut ModuleTx| -> Result<u64, DbError> {
            Ok(
                sqlx::query("UPDATE identity.workers SET job_roles = '{x}' WHERE id = $1::uuid")
                    .bind(f.b_worker.to_string())
                    .execute(t.executor()?)
                    .await?
                    .rows_affected(),
            )
        })
        .await;
    assert_eq!(n, Ok(0), "B's row is invisible to an update under A");
    drop(tx);
    // The host path: cortex_install_admin sees every project's rows.
    let mut admin = db.admin().await;
    sqlx::raw_sql("SET ROLE cortex_install_admin")
        .execute(&mut admin)
        .await
        .expect("set role");
    let all: i64 = sqlx::query_scalar("SELECT count(*) FROM identity.workers")
        .fetch_one(&mut admin)
        .await
        .expect("install admin reads");
    assert_eq!(all, 9, "6 of A, 2 of B and 1 of C");
}

/// Wave 1 ports3 (the tokens piece's gap): the project's lifetime comes from the SAME row and the same SELECT as the
/// installation id: 90 by default, and the stored value (7 to 365) after it is set.
#[tokio::test]
async fn w1_p3_project_carries_the_token_lifetime_from_the_same_row() {
    let (db, f, pool, p) = setup("mi_lifetime").await;
    let mut tx = begin(&pool, Some(f.a)).await;
    assert_eq!(
        p.project(&mut tx)
            .await
            .expect("project A")
            .token_lifetime_days,
        90,
        "the default"
    );
    drop(tx);
    let mut admin = db.admin().await;
    for (project, days) in [(f.a, 7_i32), (f.b, 365)] {
        sqlx::query("UPDATE identity.projects SET token_lifetime_days = $1 WHERE id = $2::uuid")
            .bind(days)
            .bind(project.to_string())
            .execute(&mut admin)
            .await
            .expect("set the lifetime");
    }
    for (project, days) in [(f.a, 7_i32), (f.b, 365)] {
        let mut tx = begin(&pool, Some(project)).await;
        let v = p.project(&mut tx).await.expect("project");
        assert_eq!(v.token_lifetime_days, days);
        assert_eq!(
            v.installation, f.inst,
            "the installation is still read from the same row"
        );
    }
}
