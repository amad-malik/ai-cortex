//! Test support for mod-search's DB cells (needs `supervise.sh`'s throwaway Postgres: CV2_PG_PORT, CV2_PG_PASSWORD).
//!
//! mod-search owns no table and may not depend on mod-records (cox's fitness ruling), so the records schema comes from a byte copy of the
//! pinned S2 revision 8 pack (`tests/data/records.s2r8-2a2c9440.sql`: `records.sql` of `wave1-pins-20261106-S2r8` from `CREATE SCHEMA` to
//! the end; the pack file's sha256 prefix is 2a2c9440), applied as `cortex_owner` through the migration guard under a fixture module
//! `records`. Rows are seeded under the role `cortex_m_records`, exactly as the records piece would write them.
#![allow(dead_code)]

use std::sync::Arc;

use cortex_db::migrate::{Migration, MigrationSet, apply, platform_migrations};
use cortex_db::{DbPool, ModuleTx, OwnerConn, TxOptions};
use cortex_testkit::{TestDb, ctx, empty_db, module};
use cortex_types::{ProjectId, SystemClock};
use mod_search::{BOOTSTRAP_SQL, ID};

/// The fixture records module's role, created like the other roles (idempotent).
const RECORDS_ROLE_SQL: &str = r#"
DO $$ BEGIN
  BEGIN CREATE ROLE cortex_m_records NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END;
  BEGIN GRANT cortex_m_records TO cortex_app; EXCEPTION WHEN unique_violation THEN NULL; END;
END $$;
"#;

pub const RECORDS_SQL: &str = include_str!("../data/records.s2r8-2a2c9440.sql");

pub struct World {
    pub db: TestDb,
    pub pool: Arc<DbPool>,
    pub a: ProjectId,
    pub b: ProjectId,
}

pub async fn setup(tag: &str) -> World {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("search bootstrap");
    sqlx::raw_sql(RECORDS_ROLE_SQL)
        .execute(&mut admin)
        .await
        .expect("records role");
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    apply(
        &mut owner,
        &MigrationSet {
            module: module("records"),
            schema: "records",
            steps: vec![Migration {
                version: 1,
                name: "records",
                up: RECORDS_SQL,
                down: "DROP SCHEMA records CASCADE;",
            }],
        },
    )
    .await
    .expect("the S2 r8 records pack applies through the guard");
    let pool = Arc::new(
        DbPool::connect(&db.db_config(&[ID, "records"], Arc::new(SystemClock)))
            .await
            .expect("pool"),
    );
    World {
        db,
        pool,
        a: ProjectId::new(&SystemClock),
        b: ProjectId::new(&SystemClock),
    }
}

/// A transaction under the search role for `project`.
pub async fn search_tx(w: &World, project: Option<ProjectId>) -> ModuleTx {
    ModuleTx::begin(&w.pool, ctx(ID, project), TxOptions::default())
        .await
        .expect("begin as search")
}

/// A UUIDv7 text whose embedded timestamp is `ms` milliseconds after the epoch (the table's CHECK ties `created_at` to it); `n` makes it unique.
pub fn uuid7(ms: u64, n: u64) -> String {
    format!(
        "{:08x}-{:04x}-7{:03x}-8{:03x}-{:012x}",
        ms >> 16,
        ms & 0xffff,
        n & 0xfff,
        (n >> 12) & 0xfff,
        n
    )
}

/// 2026-11-06T00:00:00Z in milliseconds: the base of every fixture timestamp.
pub const BASE_MS: u64 = 1_793_923_200_000;

/// One record to seed.
pub struct Rec<'a> {
    pub id: String,
    pub ty: &'a str,
    pub summary: &'a str,
    pub body: Option<&'a str>,
    /// An RFC 3339 text, or None.
    pub valid_until: Option<&'a str>,
}

impl<'a> Rec<'a> {
    /// A note created `secs` seconds after the base.
    pub fn note(n: u64, secs: u64, summary: &'a str, body: Option<&'a str>) -> Rec<'a> {
        Rec {
            id: uuid7(BASE_MS + secs * 1000, n),
            ty: "note",
            summary,
            body,
            valid_until: None,
        }
    }
}

/// Insert one record into `project` as the records role. Returns its id.
pub async fn seed(w: &World, project: ProjectId, r: &Rec<'_>) -> String {
    let mut tx = ModuleTx::begin(&w.pool, ctx("records", Some(project)), TxOptions::default())
        .await
        .expect("begin as records");
    // knowledge needs a source; the other types must have none.
    let (kind, rf) = if r.ty == "knowledge" {
        (Some("file"), Some("/fixture.md"))
    } else {
        (None, None)
    };
    sqlx::query(
        "INSERT INTO records.records (id, project_id, created_at, type, provenance, author_kind, author_id, written_by, summary, body, \
         summary_sha256, body_sha256, fields_sha256, valid_until, source_kind, source_ref) \
         VALUES ($1::uuid, $2::uuid, uuid_extract_timestamp($1::uuid), $3, 'worker_written', 'worker', 'fixture', $2::uuid, $4, $5, \
         sha256(convert_to($4, 'UTF8')), sha256(convert_to(coalesce($5, ''), 'UTF8')), sha256('{}'::bytea), $6::timestamptz, $7, $8)",
    )
    .bind(&r.id)
    .bind(project.to_string())
    .bind(r.ty)
    .bind(r.summary)
    .bind(r.body)
    .bind(r.valid_until)
    .bind(kind)
    .bind(rf)
    .execute(tx.executor().expect("executor"))
    .await
    .unwrap_or_else(|e| panic!("seed {}: {e}", r.id));
    tx.commit().await.expect("commit");
    r.id.clone()
}

/// Mark a record superseded or invalidated, as the records role would.
pub async fn set_state(w: &World, project: ProjectId, id: &str, state: &str, by: Option<&str>) {
    let mut tx = ModuleTx::begin(&w.pool, ctx("records", Some(project)), TxOptions::default())
        .await
        .expect("begin as records");
    sqlx::query(
        "INSERT INTO records.state (project_id, record_id, state, superseded_by, reason, by_kind, by_id) \
         VALUES ($1::uuid, $2::uuid, $3, $4::uuid, 'fixture', 'worker', 'fixture')",
    )
    .bind(project.to_string())
    .bind(id)
    .bind(state)
    .bind(by)
    .execute(tx.executor().expect("executor"))
    .await
    .expect("state row");
    tx.commit().await.expect("commit");
}

/// A context for the search module acting for `project` (what `Ctx::begin` builds in the handler).
pub fn tctx(project: ProjectId) -> cortex_db::TxContext {
    ctx(ID, Some(project))
}

/// A superuser connection that holds ACCESS EXCLUSIVE on `records.records` until it is released: every search blocks behind it, which makes a
/// slow query that needs no hook in the product code.
pub async fn lock_records(w: &World) -> sqlx::PgConnection {
    use sqlx::Connection;
    let mut c = w.db.admin().await;
    sqlx::query("BEGIN").execute(&mut c).await.expect("begin");
    sqlx::query("LOCK TABLE records.records IN ACCESS EXCLUSIVE MODE")
        .execute(&mut c)
        .await
        .expect("lock");
    let _ = c.ping().await;
    c
}

/// Let the blocked searches go.
pub async fn release(mut c: sqlx::PgConnection) {
    sqlx::query("COMMIT").execute(&mut c).await.expect("commit");
}

/// How many backends are running a search statement (the query text names the view) right now, seen from a separate superuser connection.
pub async fn searching(w: &World) -> i64 {
    let mut c = w.db.admin().await;
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid() \
         AND state = 'active' AND query LIKE '%records.pub_sources%'",
    )
    .fetch_one(&mut c)
    .await
    .expect("activity")
}

/// Wait until `searching` equals `n` (up to `secs` seconds); the last value is returned.
pub async fn wait_searching(w: &World, n: i64, secs: u64) -> i64 {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let now = searching(w).await;
        if now == n || std::time::Instant::now() > until {
            return now;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
