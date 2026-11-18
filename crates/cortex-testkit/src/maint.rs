//! F1-31's fixture (TESTS r7 F1-31; A7 r4 item 2; CONTRACTS r7 §3 "Runtime DDL").
//!
//! - **One database per case** (`isolated_db`): every case changes the catalogue.
//! - **Two modules,** `test_a` and `test_b`, each with a partitioned table `<schema>.p`
//!   (list-partitioned by project, each project's partition range-partitioned by month), a
//!   default partition, and the template's functions. Their windows differ, and so do their
//!   `drop_eligible` predicates (category 9).
//! - **A stand-in `identity_pub.projects`** (I-2's `identity_pub` does not exist yet): `id` and
//!   `state` (`open` or `closed`), read by the template's existence check.
//! - **A fixed clock:** 2026-11-15T12:00:00Z, so "this month" is 2026-11.
//! - **Variants (F5, mike):** derived from the reviewed template's own output by one edit at a
//!   named marker. A missing marker is an error (`VariantError`), so a control can never run
//!   against an unchanged template and pass by accident.
use std::collections::BTreeSet;
use std::sync::Arc;

use cortex_db::maintenance::templates::{
    self, AuditBinding, GeneratedFn, Grant, GrantRole, MigrationSql, PartitionSpec,
};
use cortex_db::maintenance::{
    DefinerError, MaintError, MaintKind, MaintenanceFn, Window, check_definer_functions,
};
use cortex_db::{AdvisoryClass, DbError, DbPool, ModuleTx, OwnerConn, TxOptions};
use cortex_types::{Clock, ProjectId, TestClock, Timestamp};
use sha2::Digest;
use sqlx::{PgConnection, Row};

use crate::pg::{MODULES, TestDb, ctx, isolated_db, module};

/// 2026-11-15T12:00:00Z.
pub const NOW_MICROS: i64 = 1_794_744_000_000_000;
/// The month of `NOW_MICROS`.
pub const NOW_YEAR: i32 = 2026;
pub const NOW_MONTH: i32 = 11;

/// The first day of the month `offset` months from 2026-11, as `YYYY-MM-01`.
pub fn month(offset: i32) -> String {
    let (y, m) = year_month(offset);
    format!("{y:04}-{m:02}-01")
}

/// The year and month `offset` months from 2026-11.
pub fn year_month(offset: i32) -> (i32, u32) {
    let total = NOW_YEAR * 12 + (NOW_MONTH - 1) + offset;
    (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32)
}

/// The retention role of `test_a` (r2, E3): it alone gets `DELETE` on a child, as records'
/// retention role does (A7 r4 item 2 step 6; S2 §3.8). A registered module, so a `ModuleTx` can
/// act as it.
pub const RETENTION_ROLE: &str = "cortex_m_test_ret";

/// `test_a`, records' shape (r2; nemo's M-1 confirmation r1): create M to M+2; drop every month
/// strictly before M, **unbounded into the past**. Predicate: a flag per project and month in
/// `test_a.eligible_flags` (category 8; the real records predicate is M-1's: the project's own
/// retention, whole-leaf cutoff, empty after completed bounded expiry). r2 (E3): records' grant
/// map, INSERT and SELECT to the module, DELETE to the retention role only.
pub const SPEC_A: PartitionSpec = PartitionSpec {
    module: "test_a",
    schema: "test_a",
    root_table: "p",
    list_column: "project_id",
    range_column: "month",
    grants: &[
        Grant {
            role: GrantRole::Module,
            privileges: &["INSERT", "SELECT"],
        },
        Grant {
            role: GrantRole::Named(RETENTION_ROLE),
            privileges: &["DELETE"],
        },
    ],
    create_window: Window {
        earliest: Some(0),
        latest: 2,
    },
    drop_window: Window {
        earliest: None,
        latest: -1,
    },
    audit: AuditBinding {
        relation: "test_audit.r",
        run_id_column: "run_id",
    },
};

/// `test_b`: create this month to one ahead; drop from 12 to 6 months back: a FINITE lower bound,
/// so a bounded window stays covered too. Predicate: at least
/// six months old by `platform.now()`, with no flag table (category 9).
pub const SPEC_B: PartitionSpec = PartitionSpec {
    module: "test_b",
    schema: "test_b",
    root_table: "p",
    list_column: "project_id",
    range_column: "month",
    grants: &[Grant {
        role: GrantRole::Module,
        privileges: &["INSERT", "SELECT"],
    }],
    create_window: Window {
        earliest: Some(0),
        latest: 1,
    },
    drop_window: Window {
        earliest: Some(-12),
        latest: -6,
    },
    audit: AuditBinding {
        relation: "test_audit.r",
        run_id_column: "run_id",
    },
};

/// `test_a`'s module-supplied predicate (inventoried; a helper of `drop_partition`).
pub const ELIGIBLE_A: &str = r#"CREATE FUNCTION test_a.drop_eligible(project_id uuid, month date) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, test_a, pg_temp AS $body$
BEGIN
  RETURN coalesce((SELECT f.ok FROM test_a.eligible_flags f
                   WHERE f.project_id = drop_eligible.project_id AND f.month = drop_eligible.month), false);
END
$body$;
REVOKE ALL ON FUNCTION test_a.drop_eligible(uuid, date) FROM PUBLIC;
"#;

/// `test_b`'s predicate: a different rule (category 9).
pub const ELIGIBLE_B: &str = r#"CREATE FUNCTION test_b.drop_eligible(project_id uuid, month date) RETURNS boolean
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, test_b, pg_temp AS $body$
BEGIN
  RETURN drop_eligible.month <= (date_trunc('month', platform.now()) - interval '6 months')::date;
END
$body$;
REVOKE ALL ON FUNCTION test_b.drop_eligible(uuid, date) FROM PUBLIC;
"#;

/// The body between `$body$` markers: what `prosrc` holds.
pub fn body_of(create_function_sql: &str) -> String {
    let start = create_function_sql.find("$body$").expect("a $body$ start") + "$body$".len();
    let end = create_function_sql[start..]
        .find("$body$")
        .expect("a $body$ end")
        + start;
    create_function_sql[start..end].to_owned()
}

/// The fixture tables F1-31 adds to the base schema, as `cortex_owner`.
const MAINT_SCHEMA: &str = r#"
SET ROLE cortex_owner;
CREATE SCHEMA identity_pub;
CREATE TABLE identity_pub.projects (id uuid PRIMARY KEY, state text NOT NULL CHECK (state IN ('open', 'closed')));
CREATE TABLE test_a.p (project_id uuid NOT NULL, month date NOT NULL, id bigint NOT NULL, v text NOT NULL) PARTITION BY LIST (project_id);
CREATE TABLE test_a.p_default PARTITION OF test_a.p DEFAULT;
CREATE TABLE test_b.p (project_id uuid NOT NULL, month date NOT NULL, id bigint NOT NULL, v text NOT NULL) PARTITION BY LIST (project_id);
CREATE TABLE test_b.p_default PARTITION OF test_b.p DEFAULT;
ALTER TABLE test_a.p ENABLE ROW LEVEL SECURITY; ALTER TABLE test_a.p FORCE ROW LEVEL SECURITY;
ALTER TABLE test_b.p ENABLE ROW LEVEL SECURITY; ALTER TABLE test_b.p FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON test_a.p
  USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
  WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON test_b.p
  USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
  WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
GRANT SELECT, INSERT ON test_a.p TO cortex_m_test_a;
GRANT USAGE ON SCHEMA test_a TO cortex_m_test_ret;
GRANT SELECT, INSERT ON test_b.p TO cortex_m_test_b;
CREATE TABLE test_a.eligible_flags (project_id uuid NOT NULL, month date NOT NULL, ok boolean NOT NULL, PRIMARY KEY (project_id, month));
ALTER TABLE test_audit.r ADD COLUMN run_id uuid;
RESET ROLE;
"#;

/// One F1-31 case's world.
pub struct Maint {
    pub db: TestDb,
    pub pool: DbPool,
    pub clock: Arc<TestClock>,
    /// Two open projects.
    pub a: ProjectId,
    pub b: ProjectId,
    /// What was installed for each module (the reviewed template, or a variant for `test_a`).
    pub sql_a: MigrationSql,
    pub sql_b: MigrationSql,
}

/// The fixture with the reviewed template for both modules.
pub async fn maint(tag: &str) -> Maint {
    let sql_a = templates::partition_maintenance(&SPEC_A);
    maint_with(tag, sql_a).await
}

/// The fixture with `sql_a` installed for `test_a` (a variant, for a control), and the reviewed
/// template for `test_b`.
pub async fn maint_with(tag: &str, sql_a: MigrationSql) -> Maint {
    let sql_b = templates::partition_maintenance(&SPEC_B);
    let db = isolated_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(MAINT_SCHEMA)
        .execute(&mut admin)
        .await
        .expect("F1-31 fixture tables");
    // The platform schema (platform.now()), as a release migration installs it.
    let mut owner = OwnerConn::connect(&db.owner_config())
        .await
        .expect("owner connection");
    cortex_db::migrate::apply(&mut owner, &cortex_db::migrate::platform_migrations())
        .await
        .expect("platform migrations");
    let mut raw_owner = db.owner().await;
    for sql in [sql_a.up.as_str(), sql_b.up.as_str(), ELIGIBLE_A, ELIGIBLE_B] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(&mut raw_owner)
            .await
            .expect("install a maintenance migration as cortex_owner");
    }
    let clock = Arc::new(TestClock::at(Timestamp::from_unix_micros(NOW_MICROS)));
    let (a, b) = (ProjectId::new(&*clock), ProjectId::new(&*clock));
    for p in [a, b] {
        add_project(&mut raw_owner, p, "open").await;
    }
    let dyn_clock: Arc<dyn Clock> = clock.clone();
    let pool = DbPool::connect(&db.db_config(MODULES, dyn_clock))
        .await
        .expect("connect");
    Maint {
        db,
        pool,
        clock,
        a,
        b,
        sql_a,
        sql_b,
    }
}

/// A project row and its list partition under both modules' roots, with row security, as the
/// module's own project creation would leave them.
pub async fn add_project(owner: &mut PgConnection, p: ProjectId, state: &str) {
    let u = uuid(p);
    let mut sql =
        format!("INSERT INTO identity_pub.projects (id, state) VALUES ('{u}', '{state}');\n");
    for schema in ["test_a", "test_b"] {
        let part = templates::project_partition_name("p", &u);
        sql.push_str(&format!(
            "CREATE TABLE {schema}.{part} PARTITION OF {schema}.p FOR VALUES IN ('{u}') PARTITION BY RANGE (month);\n\
             ALTER TABLE {schema}.{part} ENABLE ROW LEVEL SECURITY; ALTER TABLE {schema}.{part} FORCE ROW LEVEL SECURITY;\n\
             CREATE POLICY by_project ON {schema}.{part} USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid) \
               WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);\n"
        ));
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(owner)
        .await
        .expect("a project and its partitions");
}

pub fn uuid(p: ProjectId) -> uuid::Uuid {
    uuid::Uuid::parse_str(&p.to_string()).expect("uuid")
}

/// The leaf's name for `project` and the month `offset` from 2026-11.
pub fn leaf(project: &uuid::Uuid, offset: i32) -> String {
    let (y, m) = year_month(offset);
    templates::month_partition_name("p", project, y, m)
}

/// A refused call: the template's own error, another SQLSTATE, or a `DbError`.
///
/// r2 (vera's F1B-B2-01): a template refusal keeps its public shape, the message, DETAIL and
/// HINT as the server sent them, so a test compares the whole of it (`refused`), not just the
/// SQLSTATE. A wrong message or one carrying row data fails the comparison.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Maint {
        code: MaintError,
        message: String,
        detail: Option<String>,
        hint: Option<String>,
    },
    Sql(String),
    Db(DbError),
}

/// The exact refusal the template must give: the code's SQLSTATE, the message exactly the code,
/// no DETAIL and no HINT.
pub fn refused(code: MaintError) -> Refusal {
    Refusal::Maint {
        code,
        message: code.code().to_owned(),
        detail: None,
        hint: None,
    }
}

/// The sanitised commit-time refusal (E12): `DbError::Other(<code>)`, exactly.
pub fn refused_at_commit(code: MaintError) -> DbError {
    DbError::Other(code.code().to_owned())
}

fn refusal(e: sqlx::Error) -> Refusal {
    let Some(d) = e.as_database_error() else {
        return Refusal::Db(DbError::Other(e.to_string()));
    };
    let Some(code) = d.code() else {
        return Refusal::Db(DbError::Other(e.to_string()));
    };
    match MaintError::from_sqlstate(&code) {
        Some(m) => {
            let pg = d.try_downcast_ref::<sqlx::postgres::PgDatabaseError>();
            Refusal::Maint {
                code: m,
                message: d.message().to_owned(),
                detail: pg.and_then(|p| p.detail()).map(str::to_owned),
                hint: pg.and_then(|p| p.hint()).map(str::to_owned),
            }
        }
        None => Refusal::Sql(code.into_owned()),
    }
}

/// `select <schema>.create_partition($1::uuid, $2::date)` through the transaction's own executor.
pub async fn create(
    tx: &mut ModuleTx,
    schema: &str,
    project: Option<uuid::Uuid>,
    month: Option<&str>,
) -> Result<String, Refusal> {
    let conn = tx.executor().map_err(Refusal::Db)?;
    sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(format!(
        "select {schema}.create_partition($1::uuid, $2::date)"
    )))
    .bind(project)
    .bind(month)
    .fetch_one(conn)
    .await
    .map_err(refusal)
}

/// `select <schema>.drop_partition($1::uuid, $2::date, $3::uuid)`.
pub async fn drop(
    tx: &mut ModuleTx,
    schema: &str,
    project: Option<uuid::Uuid>,
    month: Option<&str>,
    run_id: uuid::Uuid,
) -> Result<String, Refusal> {
    let conn = tx.executor().map_err(Refusal::Db)?;
    sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(format!(
        "select {schema}.drop_partition($1::uuid, $2::date, $3::uuid)"
    )))
    .bind(project)
    .bind(month)
    .bind(run_id)
    .fetch_one(conn)
    .await
    .map_err(refusal)
}

/// The caller's audit record for a run (S1 `AuditSink`'s double): through
/// `delegate(test_audit, …)`, taking `AuditChain` as I-1 does, then the row with the `run_id`.
pub async fn audit(
    tx: &mut ModuleTx,
    project: uuid::Uuid,
    run_id: uuid::Uuid,
    v: &str,
) -> Result<(), DbError> {
    let v = v.to_owned();
    tx.delegate(&module("test_audit"), async move |t| {
        t.advisory_lock(AdvisoryClass::AuditChain, 1).await?;
        sqlx::query("insert into test_audit.r (project_id, v, run_id) values ($1, $2, $3)")
            .bind(project)
            .bind(&v)
            .bind(run_id)
            .execute(t.executor()?)
            .await
            .map(|_| ())
            .map_err(|e| DbError::Other(e.to_string()))
    })
    .await
}

/// Runs `sql` through the transaction's own executor.
pub async fn exec(tx: &mut ModuleTx, sql: &str) -> Result<(), Refusal> {
    let conn = tx.executor().map_err(Refusal::Db)?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(conn)
        .await
        .map(|_| ())
        .map_err(refusal)
}

/// `select count(*) …` through the transaction's own executor.
pub async fn count(tx: &mut ModuleTx, sql: &str) -> Result<i64, Refusal> {
    let conn = tx.executor().map_err(Refusal::Db)?;
    sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(conn)
        .await
        .map_err(refusal)
}

/// r2 (mike's F1B-R0-07): the catalogue fields the refusals claim to leave unchanged, and the
/// DATA itself, not only row counts: each table's rows, ordered, are digested (md5 of their text
/// forms), so a count-preserving change is seen. Relations carry their owner; policies their
/// roles, permissiveness and `WITH CHECK`; functions their return type and language.
pub async fn snapshot_of(db: &TestDb) -> Snapshot {
    let mut admin = db.admin().await;
    let relations = list(&mut admin, format!(
        "select n.nspname || '.' || c.relname || ' ' || c.relkind::text || ' ' || c.relispartition || ' ' || \
         c.relowner::regrole::text || ' ' || coalesce(c.relacl::text, '-') || ' ' || c.relrowsecurity || ' ' || \
         c.relforcerowsecurity || ' ' || coalesce(pg_get_expr(c.relpartbound, c.oid), '-') \
         from pg_class c join pg_namespace n on n.oid = c.relnamespace \
         where n.nspname in {SNAP_SCHEMAS} order by 1"
    ))
    .await;
    let inherits = list(&mut admin, format!(
        "select i.inhrelid::regclass::text || ' < ' || i.inhparent::regclass::text from pg_inherits i \
         join pg_class c on c.oid = i.inhrelid join pg_namespace n on n.oid = c.relnamespace \
         where n.nspname in {SNAP_SCHEMAS} order by 1"
    ))
    .await;
    let functions = list(&mut admin, format!(
        "select p.oid::regprocedure::text || ' ' || p.prorettype::regtype::text || ' ' || l.lanname || ' ' || \
         p.prosecdef || ' ' || encode(sha256(convert_to(p.prosrc, 'UTF8')), 'hex') || ' ' || \
         coalesce(p.proconfig::text, '-') || ' ' || coalesce(p.proacl::text, '-') || ' ' || p.proowner::regrole::text \
         from pg_proc p join pg_namespace n on n.oid = p.pronamespace join pg_language l on l.oid = p.prolang \
         where n.nspname in {SNAP_SCHEMAS} order by 1"
    ))
    .await;
    let policies = list(&mut admin, format!(
        "select schemaname || '.' || tablename || ' ' || policyname || ' ' || permissive || ' ' || roles::text || ' ' || \
         cmd || ' ' || coalesce(qual, '-') || ' ' || coalesce(with_check, '-') \
         from pg_policies where schemaname in {SNAP_SCHEMAS} order by 1"
    ))
    .await;
    let tables = list(
        &mut admin,
        format!(
            "select quote_ident(n.nspname) || '.' || quote_ident(c.relname) from pg_class c \
             join pg_namespace n on n.oid = c.relnamespace \
             where n.nspname in {SNAP_SCHEMAS} and c.relkind = 'r' order by 1"
        ),
    )
    .await;
    let mut rows = Vec::new();
    for t in tables {
        let (n, digest): (i64, Option<String>) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select count(*), md5(string_agg(x::text, E'\\n' order by x::text)) from {t} x"
        )))
        .fetch_one(&mut admin)
        .await
        .expect("row digest");
        rows.push(format!("{t} {n} {}", digest.unwrap_or_else(|| "-".into())));
    }
    Snapshot {
        relations,
        inherits,
        functions,
        policies,
        rows,
    }
}

/// The catalogue and data an F1-31 refusal must leave unchanged: the fixture schemas' relations
/// (with kind, partition flag, ACL and row-security flags), `pg_inherits`, the functions (with
/// `prosrc` hash, `proconfig` and ACL), the policies, and every table's row count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub relations: Vec<String>,
    pub inherits: Vec<String>,
    pub functions: Vec<String>,
    pub policies: Vec<String>,
    pub rows: Vec<String>,
}

const SNAP_SCHEMAS: &str = "('test_a', 'test_b', 'test_audit', 'identity_pub', 'public')";

impl Maint {
    /// A `ModuleTx` for `module` with `project` set.
    pub async fn begin(&self, m: &str, project: Option<ProjectId>) -> ModuleTx {
        ModuleTx::begin(&self.pool, ctx(m, project), TxOptions::default())
            .await
            .expect("begin")
    }

    /// A leaf created directly by the owner: what an earlier, legitimate run left behind (drop
    /// cases need months the create window does not reach).
    pub async fn make_leaf(&self, schema: &str, project: ProjectId, offset: i32) {
        let u = uuid(project);
        let parent = templates::project_partition_name("p", &u);
        let name = leaf(&u, offset);
        let sql = format!(
            "CREATE TABLE {schema}.{name} PARTITION OF {schema}.{parent} FOR VALUES FROM ('{}') TO ('{}')",
            month(offset),
            month(offset + 1)
        );
        let mut owner = self.db.owner().await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut owner)
            .await
            .expect("a leaf, made by the owner");
    }

    /// Whether `schema.name` exists, and its `relkind`.
    pub async fn relkind(&self, schema: &str, name: &str) -> Option<String> {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar::<_, String>(
            "select c.relkind::text from pg_class c join pg_namespace n on n.oid = c.relnamespace \
             where n.nspname = $1 and c.relname = $2",
        )
        .bind(schema)
        .bind(name)
        .fetch_optional(&mut admin)
        .await
        .expect("pg_class")
    }

    /// The parent of `schema.name` in `pg_inherits`, if any.
    pub async fn parent_of(&self, schema: &str, name: &str) -> Option<String> {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar::<_, String>(
            "select i.inhparent::regclass::text from pg_inherits i \
             where i.inhrelid = (quote_ident($1) || '.' || quote_ident($2))::regclass",
        )
        .bind(schema)
        .bind(name)
        .fetch_optional(&mut admin)
        .await
        .expect("pg_inherits")
    }

    pub async fn snapshot(&self) -> Snapshot {
        snapshot_of(&self.db).await
    }

    /// The test inventory: the installed template's functions for both modules, and both
    /// predicates (helpers of `drop_partition`), each pinned by the sha256 of its body.
    pub fn inventory(&self) -> Vec<MaintenanceFn> {
        let mut inv = Vec::new();
        for (spec, sql, eligible) in [
            (&SPEC_A, &self.sql_a, ELIGIBLE_A),
            (&SPEC_B, &self.sql_b, ELIGIBLE_B),
        ] {
            let eligible_name: &'static str =
                leak(format!("{}.{}", spec.schema, templates::ELIGIBLE_FN));
            for f in &sql.functions {
                inv.push(entry(spec, f, eligible_name));
            }
            inv.push(MaintenanceFn {
                schema: spec.schema,
                name: templates::ELIGIBLE_FN,
                arg_types: &["uuid", "date"],
                returns: "boolean",
                kind: MaintKind::Eligible,
                module: None,
                body_sha256: sha256(&body_of(eligible)),
                helpers: &[],
                window: None,
            });
        }
        inv
    }

    /// `check_definer_functions` over both module schemas with this world's inventory.
    pub async fn check(&self) -> Result<(), Vec<DefinerError>> {
        self.check_with(&self.inventory()).await
    }

    pub async fn check_with(&self, inv: &[MaintenanceFn]) -> Result<(), Vec<DefinerError>> {
        let mut owner = OwnerConn::connect(&self.db.owner_config())
            .await
            .expect("owner connection");
        check_definer_functions(&mut owner, inv, &["test_a", "test_b"]).await
    }

    /// Runs `sql` as the superuser (to plant a fixture).
    pub async fn as_admin(&self, sql: &str) {
        let mut admin = self.db.admin().await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(&mut admin)
            .await
            .expect("admin SQL");
    }

    /// Runs `sql` as `cortex_owner` (what an owner-level change, or a migration, does).
    pub async fn as_owner(&self, sql: &str) {
        let mut owner = self.db.owner().await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(&mut owner)
            .await
            .expect("owner SQL");
    }

    /// `test_a`'s eligibility flag for `project` and the month `offset` from 2026-11.
    pub async fn flag(&self, project: ProjectId, offset: i32, ok: bool) {
        self.as_owner(&format!(
            "INSERT INTO test_a.eligible_flags (project_id, month, ok) VALUES ('{}', '{}', {ok}) \
             ON CONFLICT (project_id, month) DO UPDATE SET ok = excluded.ok",
            uuid(project),
            month(offset)
        ))
        .await;
    }

    /// A relation's effective ACL, `grantee privilege grantable`, PUBLIC as `PUBLIC`, sorted.
    pub async fn rel_acl(&self, schema: &str, name: &str) -> Vec<String> {
        let mut admin = self.db.admin().await;
        list(
            &mut admin,
            format!(
                "select case when a.grantee = 0 then 'PUBLIC' else a.grantee::regrole::text end || ' ' || \
                 a.privilege_type || ' ' || a.is_grantable from pg_class c, aclexplode(c.relacl) a \
                 where c.oid = '{schema}.{name}'::regclass order by 1"
            ),
        )
        .await
    }

    /// A function's effective ACL, the same way, as one line sorted by entry; `NULL` if `proacl`
    /// is NULL. (r2: the aggregate orders by the entry text; `order by 1` there was a constant.)
    pub async fn fn_acl(&self, regprocedure: &str) -> Vec<String> {
        let mut admin = self.db.admin().await;
        list(
            &mut admin,
            format!(
                "select coalesce((select string_agg(e, ', ' order by e) from ( \
                   select case when a.grantee = 0 then 'PUBLIC' else a.grantee::regrole::text end \
                   || ' ' || a.privilege_type || ' ' || a.is_grantable as e \
                   from pg_proc p, aclexplode(p.proacl) a where p.oid = '{regprocedure}'::regprocedure) s), \
                 case when (select proacl from pg_proc where oid = '{regprocedure}'::regprocedure) is null then 'NULL' else '' end)"
            ),
        )
        .await
    }

    /// `has_function_privilege(role, fn, 'EXECUTE')`.
    pub async fn can_execute(&self, role: &str, regprocedure: &str) -> bool {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar("select has_function_privilege($1, $2::regprocedure, 'EXECUTE')")
            .bind(role)
            .bind(regprocedure)
            .fetch_one(&mut admin)
            .await
            .expect("has_function_privilege")
    }

    /// Whether `regprocedure` exists (`to_regprocedure` is not NULL).
    pub async fn function_exists(&self, regprocedure: &str) -> bool {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar("select to_regprocedure($1) is not null")
            .bind(regprocedure)
            .fetch_one(&mut admin)
            .await
            .expect("to_regprocedure")
    }

    /// Rows of `<schema>.maintenance_runs` for `run_id`, read by the superuser.
    pub async fn runs(&self, schema: &str, run_id: uuid::Uuid) -> i64 {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "select count(*) from {schema}.{} where run_id = $1",
            templates::RUNS_TABLE
        )))
        .bind(run_id)
        .fetch_one(&mut admin)
        .await
        .expect("maintenance_runs")
    }

    /// Audit rows for `run_id`, read by the superuser.
    pub async fn audits(&self, run_id: uuid::Uuid) -> i64 {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar("select count(*) from test_audit.r where run_id = $1")
            .bind(run_id)
            .fetch_one(&mut admin)
            .await
            .expect("audit rows")
    }
}

async fn list(admin: &mut PgConnection, sql: String) -> Vec<String> {
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_all(admin)
        .await
        .expect("snapshot query")
        .iter()
        .map(|r| r.get::<String, _>(0))
        .collect()
}

fn entry(spec: &PartitionSpec, f: &GeneratedFn, eligible: &'static str) -> MaintenanceFn {
    let (module, window, helpers): (
        Option<&'static str>,
        Option<Window>,
        &'static [&'static str],
    ) = match f.kind {
        MaintKind::Create => (Some(spec.module), Some(spec.create_window), &[]),
        MaintKind::Drop => (
            Some(spec.module),
            Some(spec.drop_window),
            Box::leak(vec![eligible].into_boxed_slice()),
        ),
        MaintKind::AuditTrigger | MaintKind::Eligible => (None, None, &[]),
    };
    MaintenanceFn {
        schema: spec.schema,
        name: leak(f.name.clone()),
        arg_types: Box::leak(
            f.arg_types
                .iter()
                .map(|a| leak(a.clone()))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        ),
        returns: leak(f.returns.clone()),
        kind: f.kind,
        module,
        body_sha256: sha256(&f.prosrc),
        helpers,
        window,
    }
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

pub fn sha256(s: &str) -> [u8; 32] {
    sha2::Sha256::digest(s.as_bytes()).into()
}

/// The identities a `DefinerError` list names, for set comparisons in tests.
pub fn named(errors: &[DefinerError]) -> BTreeSet<String> {
    errors.iter().map(|e| format!("{e:?}")).collect()
}

/// A testkit-only variant of the reviewed template (F5): exactly one named change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutation {
    /// Category 1's control: the binding to the transaction's project removed (`project`).
    ProjectBindingRemoved,
    /// Category 2's control: the window check removed (`window`).
    WindowCheckRemoved,
    /// Category 6's control: on a name collision, ATTACH the existing table (`conflict`).
    AttachOnCollision,
    /// Category 7's control: the drop's ancestry check removed (`ancestry`).
    AncestryCheckRemoved,
    /// Category 8's control: the `drop_eligible` call removed (`eligible`).
    PredicateCallRemoved,
    /// Category 10: `GRANT SELECT … TO PUBLIC` planted after the repair and before the verify;
    /// `verify: false` also removes the verify (`verify-access`).
    PlantPublicSelect { verify: bool },
    /// Category 11: a permissive policy `USING (true)` planted at the same point.
    PlantPermissivePolicy { verify: bool },
    /// Category 5's control: `pg_temp` first in `search_path`, and the project table read
    /// unqualified.
    SearchPathTempFirst,
    /// r2 (vera's F1B-B2-04): the exact-bounds comparison removed (`bounds`).
    BoundsCheckRemoved,
    /// r2: the owner comparison removed (`owner`).
    OwnerCheckRemoved,
    /// r2: the prevalidation locks removed (`lock`); the DDL keeps its own locks.
    LockSkipped,
    /// r2: a test gate after validation and before any DDL (`after-validation`): the function
    /// waits there on the transaction advisory lock (`PAUSE_CLASS`, `PAUSE_KEY`), which the test
    /// holds until it has seen the competitor wait.
    PauseAfterValidation,
    /// r2 (vera's F1B-B2-01): every message gets a wrong prefix; the SQLSTATE is kept.
    MessageWrongPrefix,
    /// r2: every message carries row data (the transaction's project id); the SQLSTATE is kept.
    MessageLeaksRow,
    /// r2 (E3): the retention role's GRANT line removed from the repair; the verify is kept.
    RetentionGrantOmitted,
    /// r2 (E3, nemo's "extra-grant rollback control"): an extra `DELETE` for the module role,
    /// planted after the repair; the verify is kept.
    ExtraModuleDelete,
}

/// The pause gate's advisory lock (two-int form; class 99 is test-only, outside `AdvisoryClass`).
pub const PAUSE_CLASS: i32 = 99;
pub const PAUSE_KEY: i32 = 1;

/// Several mutations, applied in order; each must find its marker.
pub fn variants(
    base: &MigrationSql,
    spec: &PartitionSpec,
    ms: &[Mutation],
) -> Result<MigrationSql, VariantError> {
    let mut sql = base.clone();
    for m in ms {
        sql = variant(&sql, spec, *m)?;
    }
    Ok(sql)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantError {
    /// The named marker is not in the template's output: the control would measure nothing.
    MarkerMissing(String),
}

/// Derives the variant from `base` (the reviewed template's output for `spec`).
pub fn variant(
    base: &MigrationSql,
    spec: &PartitionSpec,
    m: Mutation,
) -> Result<MigrationSql, VariantError> {
    let schema = spec.schema;
    let edit = |text: &str| -> Result<String, VariantError> {
        match m {
            Mutation::ProjectBindingRemoved => remove_block(text, "project"),
            Mutation::WindowCheckRemoved => remove_block(text, "window"),
            Mutation::AncestryCheckRemoved => remove_block(text, "ancestry"),
            Mutation::PredicateCallRemoved => remove_block(text, "eligible"),
            Mutation::AttachOnCollision => replace_block(
                text,
                "conflict",
                &format!(
                    "EXECUTE format('ALTER TABLE %s ATTACH PARTITION %I.%I FOR VALUES FROM (%L) TO (%L)', \
                     v_parent, '{schema}', v_name, v_from, v_to);\n  RETURN 'exists';\n"
                ),
            ),
            Mutation::PlantPublicSelect { verify } => {
                let t = insert_at_hook(
                    text,
                    "after-repair",
                    &format!(
                        "EXECUTE format('GRANT SELECT ON %I.%I TO PUBLIC', '{schema}', v_name);\n"
                    ),
                )?;
                if verify {
                    Ok(t)
                } else {
                    remove_block(&t, "verify-access")
                }
            }
            Mutation::PlantPermissivePolicy { verify } => {
                let t = insert_at_hook(
                    text,
                    "after-repair",
                    &format!(
                        "EXECUTE format('CREATE POLICY planted ON %I.%I USING (true)', '{schema}', v_name);\n"
                    ),
                )?;
                if verify {
                    Ok(t)
                } else {
                    remove_block(&t, "verify-access")
                }
            }
            Mutation::BoundsCheckRemoved => remove_block(text, "bounds"),
            Mutation::OwnerCheckRemoved => remove_block(text, "owner"),
            Mutation::LockSkipped => remove_block(text, "lock"),
            Mutation::PauseAfterValidation => insert_at_hook(
                text,
                "after-validation",
                &format!("PERFORM pg_advisory_xact_lock({PAUSE_CLASS}, {PAUSE_KEY});\n"),
            ),
            Mutation::MessageWrongPrefix => edit_messages(text, |code| format!("x_{code}")),
            Mutation::MessageLeaksRow => edit_messages(text, |code| {
                format!(
                    "{code}' || ' ' || coalesce(current_setting('cortex.project_id', true), '-') || '"
                )
            }),
            Mutation::ExtraModuleDelete => insert_at_hook(
                text,
                "after-repair",
                &format!(
                    "EXECUTE format('GRANT DELETE ON %I.%I TO cortex_m_{}', '{schema}', v_name);\n",
                    spec.module
                ),
            ),
            Mutation::RetentionGrantOmitted => {
                let marker = format!("-- cortex:grant {RETENTION_ROLE}");
                if !text.contains(&marker) {
                    return Err(VariantError::MarkerMissing(marker));
                }
                Ok(text
                    .lines()
                    .filter(|l| !l.contains(&marker))
                    .map(|l| format!("{l}\n"))
                    .collect())
            }
            Mutation::SearchPathTempFirst => {
                let from = format!("SET search_path = pg_catalog, {schema}, pg_temp");
                let to = format!("SET search_path = pg_temp, pg_catalog, {schema}");
                let qref = "/*cortex:qref*/identity_pub.projects";
                if !text.contains(qref) {
                    return Err(VariantError::MarkerMissing(qref.into()));
                }
                Ok(text.replace(&from, &to).replace(qref, "projects"))
            }
        }
    };
    let up = edit(&base.up)?;
    if up == base.up {
        return Err(VariantError::MarkerMissing(format!(
            "{m:?}: nothing changed"
        )));
    }
    let functions = base
        .functions
        .iter()
        .map(|f| {
            // A function whose body has no such marker keeps its body.
            let prosrc = edit(&f.prosrc).unwrap_or_else(|_| f.prosrc.clone());
            GeneratedFn {
                prosrc,
                ..f.clone()
            }
        })
        .collect();
    Ok(MigrationSql { up, functions })
}

/// Rewrites every `/*cortex:msg*/'<code>'` literal's text with `f(code)`.
fn edit_messages(text: &str, f: impl Fn(&str) -> String) -> Result<String, VariantError> {
    let marker = "/*cortex:msg*/'";
    if !text.contains(marker) {
        return Err(VariantError::MarkerMissing(marker.into()));
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(marker) {
        out.push_str(&rest[..i + marker.len()]);
        let after = &rest[i + marker.len()..];
        let close = after
            .find('\'')
            .ok_or_else(|| VariantError::MarkerMissing("the message's closing quote".into()))?;
        out.push_str(&f(&after[..close]));
        rest = &after[close..];
    }
    out.push_str(rest);
    Ok(out)
}

fn remove_block(text: &str, name: &str) -> Result<String, VariantError> {
    replace_block(text, name, "")
}

/// Replaces every `-- cortex:check <name>` … `-- cortex:end <name>` block (both marker lines
/// included) with `with`. At least one block must exist.
fn replace_block(text: &str, name: &str, with: &str) -> Result<String, VariantError> {
    let open = format!("-- cortex:check {name}\n");
    let close = format!("-- cortex:end {name}\n");
    if !text.contains(&open) {
        return Err(VariantError::MarkerMissing(open));
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(&open) {
        let line_start = rest[..i].rfind('\n').map_or(0, |n| n + 1);
        out.push_str(&rest[..line_start]);
        let after = &rest[i..];
        let j = after
            .find(&close)
            .ok_or_else(|| VariantError::MarkerMissing(close.clone()))?;
        out.push_str(with);
        rest = &after[j + close.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

fn insert_at_hook(text: &str, hook: &str, line: &str) -> Result<String, VariantError> {
    let marker = format!("-- cortex:hook {hook}\n");
    if !text.contains(&marker) {
        return Err(VariantError::MarkerMissing(marker));
    }
    Ok(text.replace(&marker, &format!("{marker}{line}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_count_from_2026_11() {
        assert_eq!(month(0), "2026-11-01");
        assert_eq!(month(2), "2027-01-01");
        assert_eq!(month(-24), "2024-11-01");
        assert_eq!(month(-11), "2025-12-01");
    }

    #[test]
    fn a_variant_never_passes_unchanged_text() {
        let base = MigrationSql {
            up: "no markers here\n".into(),
            functions: vec![],
        };
        for m in [
            Mutation::ProjectBindingRemoved,
            Mutation::WindowCheckRemoved,
            Mutation::AttachOnCollision,
            Mutation::AncestryCheckRemoved,
            Mutation::PredicateCallRemoved,
            Mutation::PlantPublicSelect { verify: true },
            Mutation::PlantPermissivePolicy { verify: false },
            Mutation::SearchPathTempFirst,
            Mutation::BoundsCheckRemoved,
            Mutation::OwnerCheckRemoved,
            Mutation::LockSkipped,
            Mutation::PauseAfterValidation,
            Mutation::MessageWrongPrefix,
            Mutation::MessageLeaksRow,
            Mutation::RetentionGrantOmitted,
            Mutation::ExtraModuleDelete,
        ] {
            assert!(
                matches!(
                    variant(&base, &SPEC_A, m),
                    Err(VariantError::MarkerMissing(_))
                ),
                "{m:?} passed without its marker"
            );
        }
    }

    #[test]
    fn a_block_is_removed_with_its_markers() {
        let t =
            "a\n  -- cortex:check window\n  IF x THEN RAISE; END IF;\n  -- cortex:end window\nb\n";
        assert_eq!(remove_block(t, "window").expect("removed"), "a\nb\n");
    }

    #[test]
    fn message_edits_keep_the_sqlstate_and_change_only_the_text() {
        let t = "RAISE EXCEPTION USING ERRCODE = 'P0C01', MESSAGE = /*cortex:msg*/'partition_conflict';\n";
        assert_eq!(
            edit_messages(t, |c| format!("x_{c}")).expect("edited"),
            "RAISE EXCEPTION USING ERRCODE = 'P0C01', MESSAGE = /*cortex:msg*/'x_partition_conflict';\n"
        );
        let leak = edit_messages(t, |c| format!("{c}' || ' ' || 'row")).expect("edited");
        assert!(
            leak.contains("ERRCODE = 'P0C01'")
                && leak.contains("'partition_conflict' || ' ' || 'row'")
        );
    }
}
