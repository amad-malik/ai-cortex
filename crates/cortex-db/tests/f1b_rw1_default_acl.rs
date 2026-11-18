//! F-1b rework 1 (kai's row 4598a4ed; vera's F1B-GREEN-DEFAULT-ACL on a1d1171). Before any
//! provisioning, `cortex_owner`'s default privileges give `cortex_m_test_a` INSERT, UPDATE and
//! DELETE on every new table (an ordinary default-privilege configuration). After the platform
//! migration and the maintenance template, the module role still has exactly its allowed grants:
//! SELECT on `maintenance_runs`; SELECT and INSERT on `platform.objects` and `platform.refs`; nothing
//! on `platform.migrations` or `platform.idempotency`. Its own database: default privileges are
//! per database, and no other case may inherit them.
mod common;

use cortex_db::OwnerConn;
use cortex_db::maintenance::templates;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, ModuleTx};
use cortex_testkit::maint::SPEC_A;
use cortex_testkit::{isolated_db, sqlstate, within};
use cortex_types::ProjectId;

/// The SQLSTATE of `sql` run as `cortex_m_test_a` in a transaction of its own (rolled back), or
/// `None` when it ran.
async fn as_module(pool: &DbPool, p: ProjectId, sql: &'static str) -> Option<String> {
    let mut tx: ModuleTx = common::begin(pool, "test_a", Some(p)).await;
    let r = sqlx::query(sql)
        .execute(tx.executor().expect("executor"))
        .await;
    tx.rollback().await.ok();
    r.err().and_then(|e| sqlstate(&e))
}

const DENIED: &[&str] = &[
    "delete from test_a.maintenance_runs",
    "update test_a.maintenance_runs set action = action",
    "insert into test_a.maintenance_runs (run_id, project_id, month, relation, action) \
     values (gen_random_uuid(), gen_random_uuid(), date '2026-11-01', 'rw1', 'dropped')",
    "update platform.objects set kind = kind",
    "delete from platform.objects",
    "update platform.refs set kind = kind",
    "delete from platform.refs",
    "delete from platform.migrations",
    "delete from platform.idempotency",
];

const ALLOWED: &[&str] = &[
    "select count(*) from test_a.maintenance_runs",
    "select count(*) from platform.objects",
    "insert into platform.objects (id, project_id, kind, module) \
     values (gen_random_uuid(), nullif(current_setting('cortex.project_id', true), '')::uuid, 'rw1', 'test_a')",
    "select count(*) from platform.refs",
];

#[tokio::test]
async fn rw1_named_default_privileges_do_not_survive_provisioning() {
    within(180, async {
        let db = isolated_db("rw1_default_acl").await;
        let mut admin = db.admin().await;
        sqlx::raw_sql(
            "ALTER DEFAULT PRIVILEGES FOR ROLE cortex_owner \
             GRANT INSERT, UPDATE, DELETE ON TABLES TO cortex_m_test_a",
        )
        .execute(&mut admin)
        .await
        .expect("seed the owner's default privileges");
        let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
        apply(&mut owner, &platform_migrations())
            .await
            .expect("platform migrations under the seeded defaults");
        let mut raw_owner = db.owner().await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(
            templates::partition_maintenance(&SPEC_A).up,
        ))
        .execute(&mut raw_owner)
        .await
        .expect("test_a's maintenance migration under the seeded defaults");

        let pool = DbPool::connect(&db.db_config(common::MODULES, common::clock()))
            .await
            .expect("connect");
        let p = common::project();
        let mut wrong = Vec::new();
        for sql in DENIED {
            let got = as_module(&pool, p, sql).await;
            if got.as_deref() != Some("42501") {
                wrong.push(format!("DENIED but got {got:?}: {sql}"));
            }
        }
        for sql in ALLOWED {
            let got = as_module(&pool, p, sql).await;
            if got.is_some() {
                wrong.push(format!("ALLOWED but got {got:?}: {sql}"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    })
    .await
}
