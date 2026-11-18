//! Helpers shared by F-1b's tests. Run every test under crates/cortex-testkit/supervise.sh (L-9).
#![allow(dead_code)]

use std::sync::Arc;

use cortex_db::{DbConfig, DbPool, ModuleTx, TxOptions};
use cortex_testkit::{TestDb, ctx, test_db};
use cortex_types::{Clock, ProjectId, SystemClock};
use sqlx::{PgConnection, Row};

pub use cortex_testkit::pg::MODULES;

pub fn clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock)
}

/// A `cortex_app` pool of `size` connections over the fixture's modules.
pub async fn pool(size: u32) -> (&'static TestDb, DbPool) {
    let db = test_db().await;
    let mut cfg: DbConfig = db.db_config(MODULES, clock());
    cfg.max_connections = size;
    let pool = DbPool::connect(&cfg).await.expect("connect");
    (db, pool)
}

pub fn project() -> ProjectId {
    ProjectId::new(&SystemClock)
}

pub async fn begin(pool: &DbPool, module: &str, project: Option<ProjectId>) -> ModuleTx {
    ModuleTx::begin(pool, ctx(module, project), TxOptions::default())
        .await
        .expect("begin")
}

/// `current_user` and the project setting, as the session sees them.
pub async fn who(conn: &mut PgConnection) -> (String, Option<String>) {
    let r = sqlx::query("select current_user::text, current_setting('cortex.project_id', true)")
        .fetch_one(conn)
        .await
        .expect("who");
    (r.get(0), r.get(1))
}

/// Rows of `table` for `project`, read past RLS by the superuser.
pub async fn count_as_admin(db: &TestDb, table: &str, project: ProjectId) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "select count(*) from {table} where project_id = $1"
    )))
    .bind(uuid::Uuid::parse_str(&project.to_string()).expect("uuid"))
    .fetch_one(&mut admin)
    .await
    .expect("count")
    .get(0)
}

/// The project id as a `uuid::Uuid`, for binds.
pub fn uuid_of(p: ProjectId) -> uuid::Uuid {
    uuid::Uuid::parse_str(&p.to_string()).expect("uuid")
}
