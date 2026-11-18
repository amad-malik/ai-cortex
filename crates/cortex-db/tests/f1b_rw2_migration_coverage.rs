//! F-1b rework 2 (kai's row 23fb47be; vera's F1B-RW1-MIGRATION-CATALOG-COVERAGE on dfc192d): F1-13's
//! contract (a step that creates or alters anything outside its schema is refused, the object
//! named, and the step rolled back) for catalogues the eight-catalogue guard did not read. Each
//! case has a module id of its own, so an accepted step (on dfc192d) records nothing another case
//! reads. Paired controls: the same change inside the step's own schema is accepted.
//! - `pg_enum`: `ALTER TYPE … ADD VALUE` (vera's input, exactly);
//! - `pg_rewrite`: `ALTER TABLE … DISABLE RULE` (vera's input, exactly);
//! - `pg_description`: `COMMENT ON`, an object-reference catalogue;
//! - `pg_default_acl`: `ALTER DEFAULT PRIVILEGES IN SCHEMA`;
//! - a catalogue no release migration may change at all (`pg_largeobject_metadata`, by
//!   `lo_create`): refused whatever the schema, so it has no paired control;
//! - the guard fails closed when the server does not count catalogue writes (`track_counts` off).
//!
//! The guard catches mistakes in trusted release migrations; it is not confinement, and no
//! hostile-migration containment is claimed.

use cortex_db::migrate::{MigrateError, Migration, MigrationSet, apply, platform_migrations};
use cortex_db::{DbError, OwnerConn};
use cortex_testkit::{TestDb, isolated_db, module, test_db, within};
use sqlx::Row;

fn one_step(
    module_id: &str,
    name: &'static str,
    up: &'static str,
    down: &'static str,
) -> MigrationSet {
    MigrationSet {
        module: module(module_id),
        schema: "test_a",
        steps: vec![Migration {
            version: 1,
            name,
            up,
            down,
        }],
    }
}

async fn ready() -> (&'static TestDb, OwnerConn) {
    let db = test_db().await;
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform migrations");
    (db, owner)
}

/// Setup made and committed by the owner, outside any migration.
async fn as_owner(db: &TestDb, sql: &'static str) {
    let mut owner = db.owner().await;
    sqlx::raw_sql(sql)
        .execute(&mut owner)
        .await
        .expect("owner setup");
}

async fn texts(db: &TestDb, sql: &'static str) -> Vec<String> {
    let mut admin = db.admin().await;
    sqlx::query(sql)
        .fetch_all(&mut admin)
        .await
        .expect("probe")
        .into_iter()
        .map(|r| {
            r.get::<Option<String>, _>(0)
                .unwrap_or_else(|| "NULL".into())
        })
        .collect()
}

fn outside(object: &str) -> Result<(), MigrateError> {
    Err(MigrateError::OutsideSchema {
        version: 1,
        object: object.into(),
    })
}

#[tokio::test]
async fn rw2_an_enum_value_added_outside_the_schema_is_refused_and_rolled_back() {
    within(120, async {
        let (db, mut owner) = ready().await;
        as_owner(db, "CREATE TYPE test_b.rw1_state AS ENUM ('one');").await;
        let bad = one_step(
            "rw2_bad_enum",
            "add_an_outside_enum_value",
            "ALTER TYPE test_b.rw1_state ADD VALUE 'two';",
            "SELECT 1",
        );
        assert_eq!(apply(&mut owner, &bad).await, outside("test_b.rw1_state"));
        assert_eq!(
            texts(db, "select enumlabel::text from pg_enum where enumtypid = 'test_b.rw1_state'::regtype order by enumsortorder").await,
            ["one"],
            "the outside enum value survived the refusal"
        );
    })
    .await
}

#[tokio::test]
async fn rw2_control_an_enum_value_added_inside_the_schema_is_accepted() {
    within(120, async {
        let (db, mut owner) = ready().await;
        as_owner(db, "CREATE TYPE test_a.rw2_state_ok AS ENUM ('one');").await;
        let good = one_step(
            "rw2_ctl_enum",
            "add_an_own_enum_value",
            "ALTER TYPE test_a.rw2_state_ok ADD VALUE 'two';",
            "SELECT 1",
        );
        apply(&mut owner, &good).await.expect("an own enum value");
        assert_eq!(
            texts(db, "select enumlabel::text from pg_enum where enumtypid = 'test_a.rw2_state_ok'::regtype order by enumsortorder").await,
            ["one", "two"]
        );
    })
    .await
}

#[tokio::test]
async fn rw2_a_rule_disabled_outside_the_schema_is_refused_and_rolled_back() {
    within(120, async {
        let (db, mut owner) = ready().await;
        as_owner(
            db,
            "CREATE TABLE test_b.rw1_rules(id integer); \
             CREATE RULE keep_positive AS ON INSERT TO test_b.rw1_rules WHERE (NEW.id < 0) DO INSTEAD NOTHING;",
        )
        .await;
        let bad = one_step(
            "rw2_bad_rule",
            "disable_an_outside_rule",
            "ALTER TABLE test_b.rw1_rules DISABLE RULE keep_positive;",
            "ALTER TABLE test_b.rw1_rules ENABLE RULE keep_positive;",
        );
        assert_eq!(
            apply(&mut owner, &bad).await,
            outside("test_b.rw1_rules.keep_positive")
        );
        assert_eq!(
            texts(db, "select ev_enabled::text from pg_rewrite where rulename = 'keep_positive' and ev_class = 'test_b.rw1_rules'::regclass").await,
            ["O"],
            "the outside rule stayed disabled"
        );
    })
    .await
}

#[tokio::test]
async fn rw2_control_a_rule_disabled_inside_the_schema_is_accepted() {
    within(120, async {
        let (db, mut owner) = ready().await;
        as_owner(
            db,
            "CREATE TABLE test_a.rw2_rules_ok(id integer); \
             CREATE RULE keep_positive_ok AS ON INSERT TO test_a.rw2_rules_ok WHERE (NEW.id < 0) DO INSTEAD NOTHING;",
        )
        .await;
        let good = one_step(
            "rw2_ctl_rule",
            "disable_an_own_rule",
            "ALTER TABLE test_a.rw2_rules_ok DISABLE RULE keep_positive_ok;",
            "ALTER TABLE test_a.rw2_rules_ok ENABLE RULE keep_positive_ok;",
        );
        apply(&mut owner, &good).await.expect("an own rule");
        assert_eq!(
            texts(db, "select ev_enabled::text from pg_rewrite where rulename = 'keep_positive_ok'").await,
            ["D"]
        );
    })
    .await
}

#[tokio::test]
async fn rw2_a_comment_on_an_outside_object_is_refused_and_rolled_back() {
    within(120, async {
        let (db, mut owner) = ready().await;
        as_owner(db, "CREATE TYPE test_b.rw2_note AS ENUM ('a');").await;
        let bad = one_step(
            "rw2_bad_comment",
            "comment_on_an_outside_object",
            "COMMENT ON TYPE test_b.rw2_note IS 'set by a test_a step';",
            "COMMENT ON TYPE test_b.rw2_note IS NULL;",
        );
        assert_eq!(apply(&mut owner, &bad).await, outside("test_b.rw2_note"));
        assert_eq!(
            texts(
                db,
                "select obj_description('test_b.rw2_note'::regtype, 'pg_type')"
            )
            .await,
            ["NULL"],
            "the outside comment survived the refusal"
        );
    })
    .await
}

#[tokio::test]
async fn rw2_control_a_comment_on_an_own_object_is_accepted() {
    within(120, async {
        let (db, mut owner) = ready().await;
        as_owner(db, "CREATE TYPE test_a.rw2_note_ok AS ENUM ('a');").await;
        let good = one_step(
            "rw2_ctl_comment",
            "comment_on_an_own_object",
            "COMMENT ON TYPE test_a.rw2_note_ok IS 'set by a test_a step';",
            "COMMENT ON TYPE test_a.rw2_note_ok IS NULL;",
        );
        apply(&mut owner, &good).await.expect("an own comment");
        assert_eq!(
            texts(
                db,
                "select obj_description('test_a.rw2_note_ok'::regtype, 'pg_type')"
            )
            .await,
            ["set by a test_a step"]
        );
    })
    .await
}

#[tokio::test]
async fn rw2_default_privileges_for_an_outside_schema_are_refused_and_rolled_back() {
    within(120, async {
        let (db, mut owner) = ready().await;
        let bad = one_step(
            "rw2_bad_defacl",
            "default_privileges_outside",
            "ALTER DEFAULT PRIVILEGES IN SCHEMA test_b GRANT SELECT ON TABLES TO cortex_base;",
            "ALTER DEFAULT PRIVILEGES IN SCHEMA test_b REVOKE SELECT ON TABLES FROM cortex_base;",
        );
        assert_eq!(
            apply(&mut owner, &bad).await,
            outside("default privileges in test_b")
        );
        assert_eq!(
            texts(db, "select count(*)::text from pg_default_acl where defaclnamespace = 'test_b'::regnamespace").await,
            ["0"],
            "the outside default privileges survived the refusal"
        );
    })
    .await
}

#[tokio::test]
async fn rw2_control_default_privileges_for_the_own_schema_are_accepted() {
    within(120, async {
        let (db, mut owner) = ready().await;
        let good = one_step(
            "rw2_ctl_defacl",
            "default_privileges_inside",
            "ALTER DEFAULT PRIVILEGES IN SCHEMA test_a GRANT SELECT ON TABLES TO cortex_base;",
            "ALTER DEFAULT PRIVILEGES IN SCHEMA test_a REVOKE SELECT ON TABLES FROM cortex_base;",
        );
        apply(&mut owner, &good).await.expect("own default privileges");
        assert_eq!(
            texts(db, "select count(*)::text from pg_default_acl where defaclnamespace = 'test_a'::regnamespace").await,
            ["1"]
        );
    })
    .await
}

/// A catalogue no release migration may change: refused whatever the step's schema.
#[tokio::test]
async fn rw2_a_change_to_a_catalogue_no_migration_may_touch_is_refused() {
    within(120, async {
        let (db, mut owner) = ready().await;
        let probe = "select count(*)::text from pg_largeobject_metadata";
        let before = texts(db, probe).await;
        let bad = one_step(
            "rw2_bad_never",
            "create_a_large_object",
            "SELECT lo_create(0);",
            "SELECT 1",
        );
        assert_eq!(
            apply(&mut owner, &bad).await,
            outside("pg_catalog.pg_largeobject_metadata")
        );
        assert_eq!(texts(db, probe).await, before, "the large object survived");
    })
    .await
}

/// The guard reads the server's own count of this transaction's catalogue writes; with
/// `track_counts` off there is no count, and the guard refuses to run rather than pass.
#[tokio::test]
async fn rw2_without_track_counts_the_guard_fails_closed() {
    within(180, async {
        let db = isolated_db("rw2_track_counts").await;
        let mut admin = db.admin().await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "ALTER DATABASE {} SET track_counts = off",
            db.database
        )))
        .execute(&mut admin)
        .await
        .expect("track_counts off for this database");
        let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
        assert_eq!(
            apply(&mut owner, &platform_migrations()).await,
            Err(MigrateError::Db(DbError::Other(
                "track_counts is off: the migration guard cannot see catalogue writes".into()
            )))
        );
    })
    .await
}
