//! F-1b rework 3 (kai's row a87f8569; vera's re-review of 3ff380d). F1-13's contract, a step that
//! creates or alters anything outside its schema is refused with the object named and rolled
//! back, for two gaps she inferred from the PostgreSQL 18.4 source. These tests prove or refute
//! them on the server:
//! - **F1B-RW2-PARENT-ATTRIBUTION:** a relationship row changes the definition of its OTHER end.
//!   An own-schema child attached to an already-partitioned outside parent changes the parent's
//!   partitions (her premise, asserted below: the parent already has a partition, so
//!   `relhassubclass` is already true; the new bound is not a default; and the parent has no
//!   index, trigger, foreign key or default partition). An own-schema function added to, or
//!   dropped from, an extension changes the extension's members (intended `Never` behaviour). An
//!   own-schema statistics object on an outside table changes that table's statistics. Paired
//!   controls: the same changes between own-schema objects, and a mere reference (a view reading
//!   an outside table), are accepted.
//! - **F1B-RW2-INITDB-REMOVAL:** the database owner may drop the empty `public` schema (oid 2200,
//!   made by initdb, owned by `pg_database_owner`). A step of another schema that drops it is
//!   refused and rolled back. Controls: dropping an ordinary outside schema is refused, and a
//!   module creating and dropping its own schema is accepted.
//!
//! Each case has a module id of its own. The guard checks trusted release migrations; it is not
//! confinement (C-32).
use cortex_db::OwnerConn;
use cortex_db::migrate::{
    MigrateError, Migration, MigrationSet, apply, platform_migrations, revert_last,
};
use cortex_testkit::{TestDb, isolated_db, module, test_db, within};
use sqlx::Row;

fn step(
    module_id: &str,
    schema: &'static str,
    name: &'static str,
    up: &'static str,
    down: &'static str,
) -> MigrationSet {
    MigrationSet {
        module: module(module_id),
        schema,
        steps: vec![Migration {
            version: 1,
            name,
            up,
            down,
        }],
    }
}

async fn ready_on(db: &TestDb) -> OwnerConn {
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform migrations");
    owner
}

async fn as_owner(db: &TestDb, sql: &'static str) {
    let mut owner = db.owner().await;
    sqlx::raw_sql(sql)
        .execute(&mut owner)
        .await
        .expect("owner setup");
}

async fn one(db: &TestDb, sql: &'static str) -> String {
    let mut admin = db.admin().await;
    let v: Option<String> = sqlx::query(sql)
        .fetch_one(&mut admin)
        .await
        .expect("probe")
        .get(0);
    v.unwrap_or_else(|| "NULL".into())
}

fn outside(object: &str) -> Result<(), MigrateError> {
    Err(MigrateError::OutsideSchema {
        version: 1,
        object: object.into(),
    })
}

/// vera's premise for the outside parent, asserted rather than assumed.
async fn assert_parent_premise(db: &TestDb, parent: &'static str) {
    let q: &'static str = match parent {
        "test_b.rw3_parent" => {
            "select c.relhassubclass::text || ',' || \
             (select count(*) from pg_inherits i where i.inhparent = c.oid)::text || ',' || \
             (select p.partdefid::text from pg_partitioned_table p where p.partrelid = c.oid) || ',' || \
             (select count(*) from pg_index x where x.indrelid = c.oid)::text || ',' || \
             (select count(*) from pg_trigger t where t.tgrelid = c.oid)::text || ',' || \
             (select count(*) from pg_constraint k where (k.conrelid = c.oid or k.confrelid = c.oid) and k.contype = 'f')::text \
             from pg_class c where c.oid = 'test_b.rw3_parent'::regclass"
        }
        _ => unreachable!(),
    };
    assert_eq!(
        one(db, q).await,
        "true,1,0,0,0,0",
        "premise: relhassubclass true, one partition, no default, no index, trigger or FK"
    );
}

#[tokio::test]
async fn rw3_an_own_child_attached_to_an_outside_parent_is_refused_and_rolled_back() {
    within(120, async {
        let db = test_db().await;
        let mut owner = ready_on(db).await;
        as_owner(
            db,
            "CREATE TABLE test_b.rw3_parent (k int NOT NULL, v text) PARTITION BY LIST (k); \
             CREATE TABLE test_b.rw3_parent_1 PARTITION OF test_b.rw3_parent FOR VALUES IN (1); \
             CREATE TABLE test_a.rw3_child (k int NOT NULL, v text);",
        )
        .await;
        assert_parent_premise(db, "test_b.rw3_parent").await;
        let bad = step(
            "rw3_bad_attach",
            "test_a",
            "attach_to_an_outside_parent",
            "ALTER TABLE test_b.rw3_parent ATTACH PARTITION test_a.rw3_child FOR VALUES IN (2);",
            "ALTER TABLE test_b.rw3_parent DETACH PARTITION test_a.rw3_child;",
        );
        assert_eq!(apply(&mut owner, &bad).await, outside("test_b.rw3_parent"));
        assert_eq!(
            one(db, "select relispartition::text || ',' || (select count(*) from pg_inherits where inhparent = 'test_b.rw3_parent'::regclass)::text from pg_class where oid = 'test_a.rw3_child'::regclass").await,
            "false,1",
            "the attach survived the refusal"
        );
    })
    .await
}

#[tokio::test]
async fn rw3_control_an_own_child_attached_to_an_own_parent_is_accepted() {
    within(120, async {
        let db = test_db().await;
        let mut owner = ready_on(db).await;
        as_owner(
            db,
            "CREATE TABLE test_a.rw3_parent_ok (k int NOT NULL, v text) PARTITION BY LIST (k); \
             CREATE TABLE test_a.rw3_parent_ok_1 PARTITION OF test_a.rw3_parent_ok FOR VALUES IN (1); \
             CREATE TABLE test_a.rw3_child_ok (k int NOT NULL, v text);",
        )
        .await;
        let good = step(
            "rw3_ctl_attach",
            "test_a",
            "attach_to_an_own_parent",
            "ALTER TABLE test_a.rw3_parent_ok ATTACH PARTITION test_a.rw3_child_ok FOR VALUES IN (2);",
            "ALTER TABLE test_a.rw3_parent_ok DETACH PARTITION test_a.rw3_child_ok;",
        );
        apply(&mut owner, &good).await.expect("an own attach");
        assert_eq!(
            one(db, "select count(*)::text from pg_inherits where inhparent = 'test_a.rw3_parent_ok'::regclass").await,
            "2"
        );
    })
    .await
}

/// The extension and the two own-schema functions, made once per binary: `rw3_g` is a member of
/// `tcn` from the start, `rw3_f` is not.
static EXTENSION: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn extension_ready(db: &TestDb) {
    EXTENSION
        .get_or_init(|| async {
            as_owner(
                db,
                "CREATE EXTENSION tcn SCHEMA test_b; \
                 CREATE FUNCTION test_a.rw3_f() RETURNS int LANGUAGE sql AS $$ SELECT 1 $$; \
                 CREATE FUNCTION test_a.rw3_g() RETURNS int LANGUAGE sql AS $$ SELECT 2 $$; \
                 ALTER EXTENSION tcn ADD FUNCTION test_a.rw3_g();",
            )
            .await
        })
        .await;
}

/// `true` when `test_a.<name>()` is a member of `tcn`.
async fn member(db: &TestDb, name: &'static str) -> String {
    let q: &'static str = match name {
        "rw3_f" => {
            "select (count(*) = 1)::text from pg_depend d where d.classid = 'pg_proc'::regclass \
                    and d.objid = 'test_a.rw3_f()'::regprocedure and d.deptype = 'e' \
                    and d.refobjid = (select oid from pg_extension where extname = 'tcn')"
        }
        "rw3_g" => {
            "select (count(*) = 1)::text from pg_depend d where d.classid = 'pg_proc'::regclass \
                    and d.objid = 'test_a.rw3_g()'::regprocedure and d.deptype = 'e' \
                    and d.refobjid = (select oid from pg_extension where extname = 'tcn')"
        }
        _ => unreachable!(),
    };
    one(db, q).await
}

/// Extension membership is intended `Never` behaviour: adding an own function to an extension
/// changes the extension.
#[tokio::test]
async fn rw3_adding_an_own_function_to_an_extension_is_refused_and_rolled_back() {
    within(180, async {
        let db = test_db().await;
        let mut owner = ready_on(db).await;
        extension_ready(db).await;
        assert_eq!(
            member(db, "rw3_f").await,
            "false",
            "premise: rw3_f is not a member"
        );
        let add = step(
            "rw3_bad_ext_add",
            "test_a",
            "add_an_own_function_to_an_extension",
            "ALTER EXTENSION tcn ADD FUNCTION test_a.rw3_f();",
            "ALTER EXTENSION tcn DROP FUNCTION test_a.rw3_f();",
        );
        assert_eq!(apply(&mut owner, &add).await, outside("extension tcn"));
        assert_eq!(
            member(db, "rw3_f").await,
            "false",
            "the added member survived"
        );
    })
    .await
}

/// The same for dropping an own function from an extension.
#[tokio::test]
async fn rw3_dropping_an_own_function_from_an_extension_is_refused_and_rolled_back() {
    within(180, async {
        let db = test_db().await;
        let mut owner = ready_on(db).await;
        extension_ready(db).await;
        assert_eq!(
            member(db, "rw3_g").await,
            "true",
            "premise: rw3_g is a member"
        );
        let drop = step(
            "rw3_bad_ext_drop",
            "test_a",
            "drop_an_own_function_from_an_extension",
            "ALTER EXTENSION tcn DROP FUNCTION test_a.rw3_g();",
            "ALTER EXTENSION tcn ADD FUNCTION test_a.rw3_g();",
        );
        assert_eq!(apply(&mut owner, &drop).await, outside("extension tcn"));
        assert_eq!(
            member(db, "rw3_g").await,
            "true",
            "the dropped member stayed out"
        );
    })
    .await
}

/// A reference is not a change: a view of the step's schema reading an outside table.
#[tokio::test]
async fn rw3_control_a_reference_to_an_outside_object_is_accepted() {
    within(120, async {
        let db = test_db().await;
        let mut owner = ready_on(db).await;
        as_owner(db, "CREATE TABLE test_b.rw3_ref_t (k int);").await;
        let good = step(
            "rw3_ctl_reference",
            "test_a",
            "a_view_reading_an_outside_table",
            "CREATE VIEW test_a.rw3_v AS SELECT k FROM test_b.rw3_ref_t;",
            "DROP VIEW test_a.rw3_v;",
        );
        apply(&mut owner, &good).await.expect("a reference");
        assert_eq!(
            one(
                db,
                "select count(*)::text from pg_class where oid = to_regclass('test_a.rw3_v')"
            )
            .await,
            "1"
        );
    })
    .await
}

#[tokio::test]
async fn rw3_a_statistics_object_on_an_outside_table_is_refused_and_rolled_back() {
    within(120, async {
        let db = test_db().await;
        let mut owner = ready_on(db).await;
        as_owner(db, "CREATE TABLE test_b.rw3_stat_t (a int, b int);").await;
        let bad = step(
            "rw3_bad_stats",
            "test_a",
            "statistics_on_an_outside_table",
            "CREATE STATISTICS test_a.rw3_s (dependencies) ON a, b FROM test_b.rw3_stat_t;",
            "DROP STATISTICS test_a.rw3_s;",
        );
        assert_eq!(apply(&mut owner, &bad).await, outside("test_b.rw3_stat_t"));
        assert_eq!(
            one(
                db,
                "select count(*)::text from pg_statistic_ext where stxname = 'rw3_s'"
            )
            .await,
            "0"
        );
    })
    .await
}

#[tokio::test]
async fn rw3_control_a_statistics_object_on_an_own_table_is_accepted() {
    within(120, async {
        let db = test_db().await;
        let mut owner = ready_on(db).await;
        as_owner(db, "CREATE TABLE test_a.rw3_stat_ok (a int, b int);").await;
        let good = step(
            "rw3_ctl_stats",
            "test_a",
            "statistics_on_an_own_table",
            "CREATE STATISTICS test_a.rw3_s_ok (dependencies) ON a, b FROM test_a.rw3_stat_ok;",
            "DROP STATISTICS test_a.rw3_s_ok;",
        );
        apply(&mut owner, &good).await.expect("own statistics");
        assert_eq!(
            one(
                db,
                "select count(*)::text from pg_statistic_ext where stxname = 'rw3_s_ok'"
            )
            .await,
            "1"
        );
    })
    .await
}

/// The empty `public` schema, made by initdb and owned by `pg_database_owner`, of which the
/// database owner `cortex_owner` is a member. A database of its own: the step must not be able to
/// drop the shared database's `public`.
#[tokio::test]
async fn rw3_dropping_the_public_schema_is_refused_and_rolled_back() {
    within(180, async {
        let db = isolated_db("rw3_public").await;
        let mut owner = ready_on(&db).await;
        assert_eq!(
            one(
                &db,
                "select n.oid::text || ',' || n.nspowner::regrole::text || ',' || \
                      pg_has_role('cortex_owner', n.nspowner, 'MEMBER')::text || ',' || \
                      (select count(*) from pg_class where relnamespace = n.oid)::text \
                      from pg_namespace n where n.nspname = 'public'"
            )
            .await,
            "2200,pg_database_owner,true,0",
            "premise: public is initdb's, owned through pg_database_owner, and empty"
        );
        let bad = step(
            "rw3_bad_public",
            "test_a",
            "drop_public",
            "DROP SCHEMA public;",
            "CREATE SCHEMA public;",
        );
        assert_eq!(apply(&mut owner, &bad).await, outside("public"));
        assert_eq!(
            one(
                &db,
                "select count(*)::text from pg_namespace where nspname = 'public' and oid = 2200"
            )
            .await,
            "1",
            "public did not survive the refusal"
        );
    })
    .await
}

#[tokio::test]
async fn rw3_control_dropping_an_ordinary_outside_schema_is_refused() {
    within(120, async {
        let db = test_db().await;
        let mut owner = ready_on(db).await;
        as_owner(db, "CREATE SCHEMA rw3_other;").await;
        let bad = step(
            "rw3_bad_schema",
            "test_a",
            "drop_an_outside_schema",
            "DROP SCHEMA rw3_other;",
            "CREATE SCHEMA rw3_other;",
        );
        assert_eq!(apply(&mut owner, &bad).await, outside("rw3_other"));
        assert_eq!(
            one(
                db,
                "select count(*)::text from pg_namespace where nspname = 'rw3_other'"
            )
            .await,
            "1"
        );
    })
    .await
}

#[tokio::test]
async fn rw3_control_a_module_creating_and_dropping_its_own_schema_is_accepted() {
    within(120, async {
        let db = test_db().await;
        let mut owner = ready_on(db).await;
        let own = step(
            "rw3_ctl_own",
            "rw3_own",
            "own_schema",
            "CREATE SCHEMA rw3_own; CREATE TABLE rw3_own.t (k int);",
            "DROP TABLE rw3_own.t; DROP SCHEMA rw3_own;",
        );
        apply(&mut owner, &own)
            .await
            .expect("create the own schema");
        revert_last(&mut owner, &own)
            .await
            .expect("drop the own schema");
        assert_eq!(
            one(
                db,
                "select count(*)::text from pg_namespace where nspname = 'rw3_own'"
            )
            .await,
            "0"
        );
    })
    .await
}
