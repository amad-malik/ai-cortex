//! F-1b rework 4 (kai's row a79384e3; vera's F1B-RW3-AUTO-EXTENSION-REFERENCE on c247eb1): a false
//! refusal. `ALTER FUNCTION … DEPENDS ON EXTENSION` and `NO DEPENDS ON EXTENSION` change only the own
//! function's dependency (pg_depend deptype 'x', AUTO_EXTENSION): the function is not made a member
//! of the extension, and the extension is not changed. A step of the function's schema doing either
//! is accepted. True membership (`ALTER EXTENSION … ADD/DROP`, deptype 'e') stays refused: those
//! cases and their controls are rework 3's (`f1b_rw3_endpoints.rs`), unchanged.
use cortex_db::OwnerConn;
use cortex_db::migrate::{Migration, MigrationSet, apply, platform_migrations};
use cortex_testkit::{TestDb, module, test_db, within};
use sqlx::Row;

fn step(module_id: &str, name: &'static str, up: &'static str, down: &'static str) -> MigrationSet {
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

/// The extension and two own functions, made once per binary: `rw4_g` already depends on `tcn`
/// (deptype 'x'), `rw4_f` does not. Neither is a member.
static SETUP: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn ready(db: &TestDb) -> OwnerConn {
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform migrations");
    SETUP
        .get_or_init(|| async {
            let mut raw = db.owner().await;
            sqlx::raw_sql(
                "CREATE EXTENSION tcn SCHEMA test_b; \
                 CREATE FUNCTION test_a.rw4_f() RETURNS int LANGUAGE sql AS $$ SELECT 1 $$; \
                 CREATE FUNCTION test_a.rw4_g() RETURNS int LANGUAGE sql AS $$ SELECT 2 $$; \
                 ALTER FUNCTION test_a.rw4_g() DEPENDS ON EXTENSION tcn;",
            )
            .execute(&mut raw)
            .await
            .expect("owner setup");
        })
        .await;
    owner
}

/// The function's dependency types on `tcn`, e.g. `"x"`, or `""` for none.
async fn deps(db: &TestDb, f: &'static str) -> String {
    let q: &'static str = match f {
        "rw4_f" => {
            "select coalesce(string_agg(d.deptype::text, ',' order by d.deptype), '') from pg_depend d \
                    where d.classid = 'pg_proc'::regclass and d.objid = 'test_a.rw4_f()'::regprocedure \
                    and d.refobjid = (select oid from pg_extension where extname = 'tcn')"
        }
        "rw4_g" => {
            "select coalesce(string_agg(d.deptype::text, ',' order by d.deptype), '') from pg_depend d \
                    where d.classid = 'pg_proc'::regclass and d.objid = 'test_a.rw4_g()'::regprocedure \
                    and d.refobjid = (select oid from pg_extension where extname = 'tcn')"
        }
        _ => unreachable!(),
    };
    let mut admin = db.admin().await;
    sqlx::query(q)
        .fetch_one(&mut admin)
        .await
        .expect("pg_depend")
        .get(0)
}

#[tokio::test]
async fn rw4_an_own_function_depending_on_an_extension_is_accepted() {
    within(120, async {
        let db = test_db().await;
        let mut owner = ready(db).await;
        assert_eq!(
            deps(db, "rw4_f").await,
            "",
            "premise: rw4_f has no dependency on tcn"
        );
        let good = step(
            "rw4_ctl_depends",
            "own_function_depends_on_extension",
            "ALTER FUNCTION test_a.rw4_f() DEPENDS ON EXTENSION tcn;",
            "ALTER FUNCTION test_a.rw4_f() NO DEPENDS ON EXTENSION tcn;",
        );
        apply(&mut owner, &good)
            .await
            .expect("an own function's AUTO_EXTENSION dependency");
        assert_eq!(
            deps(db, "rw4_f").await,
            "x",
            "an AUTO_EXTENSION dependency, not membership"
        );
    })
    .await
}

#[tokio::test]
async fn rw4_an_own_function_no_longer_depending_on_an_extension_is_accepted() {
    within(120, async {
        let db = test_db().await;
        let mut owner = ready(db).await;
        assert_eq!(
            deps(db, "rw4_g").await,
            "x",
            "premise: rw4_g depends on tcn (x)"
        );
        let good = step(
            "rw4_ctl_no_depends",
            "own_function_no_depends_on_extension",
            "ALTER FUNCTION test_a.rw4_g() NO DEPENDS ON EXTENSION tcn;",
            "ALTER FUNCTION test_a.rw4_g() DEPENDS ON EXTENSION tcn;",
        );
        apply(&mut owner, &good)
            .await
            .expect("removing an own function's AUTO_EXTENSION dependency");
        assert_eq!(deps(db, "rw4_g").await, "", "the dependency is gone");
    })
    .await
}
