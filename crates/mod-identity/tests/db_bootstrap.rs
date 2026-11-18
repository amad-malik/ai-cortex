//! `bootstrap_project` (BO-01 to BO-12), against a real database in a host-path transaction
//! (`ModuleTx::begin_host`, finn's item (d)). No route, no token, no audit row: the host path
//! (cox, piece 11) writes those around this call in the same transaction (W1-18).
//! Needs `supervise.sh`'s throwaway Postgres. RED: `bootstrap_project` is `todo!()`.
mod common;

use std::sync::Arc;

use common::seed::migrate_only;
use cortex_db::{DbPool, HostContext, ModuleTx, TxOptions};
use cortex_module::IdentityApi;
use cortex_testkit::{TestDb, ctx, sqlstate};
use cortex_types::{ProjectId, RequestId, SystemClock};
use mod_identity::{BootstrapError, Bootstrapped, bootstrap_project, find_project};

async fn setup(tag: &str) -> (TestDb, DbPool) {
    let db = migrate_only(tag).await;
    let pool = DbPool::connect(&db.db_config(&["test_a", mod_identity::ID], Arc::new(SystemClock)))
        .await
        .expect("pool");
    (db, pool)
}

async fn host_tx(pool: &DbPool) -> ModuleTx {
    ModuleTx::begin_host(
        pool,
        HostContext {
            uid: 1000,
            project: Some(ProjectId::new(&SystemClock)),
            request_id: RequestId::new(&SystemClock),
        },
        TxOptions::default(),
    )
    .await
    .expect("begin_host")
}

/// The host path as piece 11 will run it: (1) an install-level transaction asks `find_project` for the
/// project of that name; (2) a transaction whose project is that id (a re-run) or a fresh one (the first
/// run) runs `bootstrap_project`; on `OtherProject(p)` (a race lost) it repeats once with `p`. Commits on
/// success, rolls back on any error.
async fn run(pool: &DbPool, project: &str, lead: &str) -> Result<Bootstrapped, BootstrapError> {
    let mut id = {
        let mut tx0 = ModuleTx::begin_host(
            pool,
            HostContext {
                uid: 1000,
                project: None,
                request_id: RequestId::new(&SystemClock),
            },
            TxOptions::default(),
        )
        .await
        .expect("begin_host (install level)");
        let found = find_project(&mut tx0, project).await;
        let _ = tx0.rollback().await;
        found?.map(|f| f.project)
    };
    for _attempt in 0..2 {
        let mut tx = ModuleTx::begin_host(
            pool,
            HostContext {
                uid: 1000,
                project: Some(id.unwrap_or_else(|| ProjectId::new(&SystemClock))),
                request_id: RequestId::new(&SystemClock),
            },
            TxOptions::default(),
        )
        .await
        .expect("begin_host");
        match bootstrap_project(&mut tx, project, lead).await {
            Ok(b) => {
                assert_eq!(
                    tx.context().project,
                    Some(b.project),
                    "the context is the resolved project"
                );
                tx.commit().await.expect("commit");
                return Ok(b);
            }
            Err(BootstrapError::OtherProject(p)) => {
                let _ = tx.rollback().await;
                id = Some(p);
            }
            Err(e) => {
                let _ = tx.rollback().await;
                return Err(e);
            }
        }
    }
    panic!("bootstrap kept naming another project");
}

const TABLES: [&str; 6] = [
    "SELECT count(*) FROM identity.installations",
    "SELECT count(*) FROM identity.projects",
    "SELECT count(*) FROM identity.teams",
    "SELECT count(*) FROM identity.workers",
    "SELECT count(*) FROM identity.team_members",
    "SELECT count(*) FROM identity.names",
];

/// Row counts of the six tables and the sum of every worker's perm_gen.
async fn shape(db: &TestDb) -> (Vec<i64>, i64) {
    let mut admin = db.admin().await;
    let mut counts = Vec::new();
    for sql in TABLES {
        counts.push(
            sqlx::query_scalar(sql)
                .fetch_one(&mut admin)
                .await
                .expect("count"),
        );
    }
    let perm_sum: i64 =
        sqlx::query_scalar("SELECT coalesce(sum(perm_gen), 0)::bigint FROM identity.workers")
            .fetch_one(&mut admin)
            .await
            .expect("perm_gen");
    (counts, perm_sum)
}

#[tokio::test]
async fn bo_01_02_03_first_run_creates_everything_and_a_rerun_changes_nothing() {
    let (db, pool) = setup("mi_bo_first").await;
    let first = run(&pool, "Helix", "KAI").await.expect("first run");
    assert!(first.created && !first.reissued());
    let after = shape(&db).await;
    assert_eq!(
        after.0,
        vec![1, 1, 0, 1, 0, 2],
        "1 installation, 1 project, 0 teams, 1 worker, 0 members, 2 names"
    );
    let mut admin = db.admin().await;
    let (kind, state, roles, mode, perm): (String, String, Vec<String>, String, i64) = sqlx::query_as(
        "SELECT kind, state, job_roles, mode, perm_gen FROM identity.workers WHERE id = $1::uuid",
    )
    .bind(first.lead.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("lead");
    assert_eq!(
        (
            kind.as_str(),
            state.as_str(),
            roles.len(),
            mode.as_str(),
            perm
        ),
        ("worker", "active", 0, "model", 0)
    );
    let (lead_of, inst, plabel, llabel): (String, String, String, String) = sqlx::query_as(
        "SELECT p.lead_worker_id::text, p.installation_id::text, pn.label, wn.label \
         FROM identity.projects p \
         JOIN identity.names pn ON pn.entity_kind = 'project' AND pn.entity_id = p.id AND pn.current \
         JOIN identity.names wn ON wn.entity_kind = 'worker' AND wn.entity_id = p.lead_worker_id AND wn.current \
         WHERE p.id = $1::uuid",
    )
    .bind(first.project.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("project");
    assert_eq!(lead_of, first.lead.to_string());
    assert_eq!(inst, first.installation.to_string());
    assert_eq!(
        (plabel.as_str(), llabel.as_str()),
        ("Helix", "KAI"),
        "names are kept as typed"
    );
    // BO-02 and BO-03: a re-run (by name key, other capitals) gives the same ids and changes no row.
    for (p, l) in [("Helix", "KAI"), ("helix", "kai"), ("HELIX", "Kai")] {
        let again = run(&pool, p, l).await.expect("re-run");
        assert_eq!(
            (again.installation, again.project, again.lead),
            (first.installation, first.project, first.lead)
        );
        assert!(!again.created && again.reissued());
        assert_eq!(
            shape(&db).await,
            after,
            "{p}/{l}: every table has the same rows"
        );
    }
}

#[tokio::test]
async fn bo_04_a_different_lead_for_an_existing_project_is_a_conflict() {
    let (db, pool) = setup("mi_bo_conflict").await;
    run(&pool, "helix", "kai").await.expect("first run");
    let before = shape(&db).await;
    assert_eq!(
        run(&pool, "helix", "bob").await,
        Err(BootstrapError::Conflict)
    );
    assert_eq!(shape(&db).await, before, "nothing changed");
}

#[tokio::test]
async fn bo_05_invalid_names_write_no_row_anywhere() {
    let (db, pool) = setup("mi_bo_invalid").await;
    let long = "a".repeat(33);
    for bad in [
        "",
        long.as_str(),
        "ab c",
        "1abc",
        "\u{212A}evin",
        "kai\n",
        "a_b",
        "a\0b",
    ] {
        assert_eq!(
            run(&pool, bad, "kai").await,
            Err(BootstrapError::Invalid),
            "project {bad:?}"
        );
        assert_eq!(
            run(&pool, "helix", bad).await,
            Err(BootstrapError::Invalid),
            "lead {bad:?}"
        );
    }
    assert_eq!(
        shape(&db).await.0,
        vec![0, 0, 0, 0, 0, 0],
        "no installation either"
    );
    // 32 characters is accepted for both names.
    let ok = "a".repeat(32);
    run(&pool, &ok, &ok).await.expect("32 characters are valid");
}

#[tokio::test]
async fn bo_06_a_fault_after_the_project_or_after_the_lead_leaves_no_row() {
    let (db, pool) = setup("mi_bo_fault").await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(
        "CREATE FUNCTION identity.boom() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected fault' USING ERRCODE = 'P0001'; END $$;",
    )
    .execute(&mut admin)
    .await
    .expect("fault function");
    // After the project row: the lead insert fails.
    sqlx::raw_sql("CREATE TRIGGER boom BEFORE INSERT ON identity.workers FOR EACH ROW EXECUTE FUNCTION identity.boom()")
        .execute(&mut admin)
        .await
        .expect("trigger on workers");
    assert!(matches!(
        run(&pool, "helix", "kai").await,
        Err(BootstrapError::Db(_))
    ));
    assert_eq!(
        shape(&db).await.0,
        vec![0, 0, 0, 0, 0, 0],
        "no installation, project, worker or name"
    );
    sqlx::raw_sql("DROP TRIGGER boom ON identity.workers")
        .execute(&mut admin)
        .await
        .expect("drop");
    // After the lead row: the names insert fails.
    sqlx::raw_sql("CREATE TRIGGER boom BEFORE INSERT ON identity.names FOR EACH ROW EXECUTE FUNCTION identity.boom()")
        .execute(&mut admin)
        .await
        .expect("trigger on names");
    assert!(matches!(
        run(&pool, "helix", "kai").await,
        Err(BootstrapError::Db(_))
    ));
    assert_eq!(shape(&db).await.0, vec![0, 0, 0, 0, 0, 0]);
    sqlx::raw_sql("DROP TRIGGER boom ON identity.names")
        .execute(&mut admin)
        .await
        .expect("drop");
    // Cleared, the same call works: nothing was half-written.
    run(&pool, "helix", "kai").await.expect("after the fault");
}

#[tokio::test]
async fn bo_07_two_simultaneous_calls_give_one_project_and_one_lead() {
    let (db, pool) = setup("mi_bo_race").await;
    let (a, b) = tokio::join!(run(&pool, "helix", "kai"), run(&pool, "Helix", "KAI"));
    let (a, b) = (a.expect("a"), b.expect("b: no unique violation leaks out"));
    assert_eq!(
        (a.installation, a.project, a.lead),
        (b.installation, b.project, b.lead),
        "both return the same ids"
    );
    assert_eq!(
        [a.created, b.created].iter().filter(|c| **c).count(),
        1,
        "exactly one created"
    );
    assert_eq!(shape(&db).await.0, vec![1, 1, 0, 1, 0, 2]);
}

#[tokio::test]
async fn bo_08_never_a_second_installation() {
    let (db, pool) = setup("mi_bo_inst").await;
    run(&pool, "helix", "kai").await.expect("first");
    run(&pool, "second", "alice").await.expect("second project");
    assert_eq!(shape(&db).await.0[0], 1, "still one installation");
    let mut tx = host_tx(&pool).await;
    let r = tx
        .delegate(&cortex_types::ModuleId::parse("platform").expect("id"), async |t: &mut ModuleTx| -> Result<(), cortex_db::DbError> {
            sqlx::query("INSERT INTO identity.installations (id, label) VALUES (gen_random_uuid(), 'again')")
                .execute(t.executor()?)
                .await?;
            Ok(())
        })
        .await;
    assert!(r.is_err(), "a raw second installation row is refused");
    let _ = tx.rollback().await;
    // The unique index itself, as the superuser: 23505 installations_singleton.
    let mut admin = db.admin().await;
    let e = sqlx::query(
        "INSERT INTO identity.installations (id, label) VALUES (gen_random_uuid(), 'again')",
    )
    .execute(&mut admin)
    .await
    .expect_err("a second row");
    assert_eq!(sqlstate(&e).as_deref(), Some("23505"));
}

#[tokio::test]
async fn bo_09_it_needs_the_host_path() {
    let (db, pool) = setup("mi_bo_host").await;
    // A module transaction without a project, and one with a project: both refused, nothing written.
    for project in [None, Some(ProjectId::new(&SystemClock))] {
        let mut tx = ModuleTx::begin(&pool, ctx(mod_identity::ID, project), TxOptions::default())
            .await
            .expect("begin");
        let r = bootstrap_project(&mut tx, "helix", "kai").await;
        assert!(
            matches!(r, Err(BootstrapError::Context) | Err(BootstrapError::Db(_))),
            "{r:?}"
        );
        let _ = tx.rollback().await;
    }
    // A host transaction that names no project: refused before any SQL.
    let mut tx = ModuleTx::begin_host(
        &pool,
        HostContext {
            uid: 1000,
            project: None,
            request_id: RequestId::new(&SystemClock),
        },
        TxOptions::default(),
    )
    .await
    .expect("begin_host");
    assert_eq!(
        bootstrap_project(&mut tx, "helix", "kai").await,
        Err(BootstrapError::Context)
    );
    let _ = tx.rollback().await;
    assert_eq!(shape(&db).await.0, vec![0, 0, 0, 0, 0, 0]);
}

#[tokio::test]
async fn bo_11_12_a_second_project_with_its_own_lead() {
    let (db, pool) = setup("mi_bo_second").await;
    let first = run(&pool, "helix", "kai").await.expect("helix");
    let before_first = {
        let mut admin = db.admin().await;
        sqlx::query_as::<_, (String, i64)>("SELECT label, perm_gen FROM identity.names n JOIN identity.workers w ON w.id = n.entity_id WHERE n.entity_kind = 'worker'")
            .fetch_all(&mut admin)
            .await
            .expect("first rows")
    };
    let second = run(&pool, "second", "alice")
        .await
        .expect("second project and lead");
    assert!(second.created);
    assert_ne!(second.project, first.project);
    assert_eq!(second.installation, first.installation, "one installation");
    // The same worker name in another project is accepted (names are unique per project).
    let third = run(&pool, "third", "kai")
        .await
        .expect("kai again, in the third project");
    assert!(third.created && third.lead != first.lead);
    // A re-run of `second` is a no-op; a different lead for it is a conflict.
    let shape_now = shape(&db).await;
    assert_eq!(
        run(&pool, "second", "alice").await.expect("re-run").lead,
        second.lead
    );
    assert_eq!(shape(&db).await, shape_now);
    assert_eq!(
        run(&pool, "second", "bob").await,
        Err(BootstrapError::Conflict)
    );
    assert_eq!(
        shape(&db).await.0,
        vec![1, 3, 0, 3, 0, 6],
        "1 installation, 3 projects and leads, 6 names"
    );
    // The first project's rows are unchanged.
    let after_first = {
        let mut admin = db.admin().await;
        sqlx::query_as::<_, (String, i64)>("SELECT label, perm_gen FROM identity.names n JOIN identity.workers w ON w.id = n.entity_id WHERE n.entity_kind = 'worker' AND w.project_id = $1::uuid")
            .bind(first.project.to_string())
            .fetch_all(&mut admin)
            .await
            .expect("rows")
    };
    assert_eq!(after_first, before_first);
    // BO-12: each project sees only its own rows under the module role, and has one lead in no team.
    for (p, expect_name) in [(first.project, "kai"), (second.project, "alice")] {
        let mut tx = ModuleTx::begin(&pool, ctx(mod_identity::ID, Some(p)), TxOptions::default())
            .await
            .expect("begin");
        let names: Vec<String> =
            sqlx::query_scalar("SELECT label FROM identity.names WHERE entity_kind = 'worker'")
                .fetch_all(tx.executor().expect("executor"))
                .await
                .expect("names");
        assert_eq!(
            names,
            vec![expect_name.to_owned()],
            "project {p} sees its own lead only"
        );
        let members: i64 = sqlx::query_scalar("SELECT count(*) FROM identity.team_members")
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect("members");
        assert_eq!(members, 0, "the lead is outside any team");
        let _ = tx.rollback().await;
    }
}

#[tokio::test]
async fn bo_07b_two_simultaneous_calls_for_a_new_project_in_an_existing_installation() {
    let (db, pool) = setup("mi_bo_race2").await;
    run(&pool, "helix", "kai")
        .await
        .expect("the installation exists now");
    // Widen the window: the lead insert sleeps. Without the lock on the installation row both calls
    // would find no `second` project and both would create it (the second then fails on the unique
    // project name); with it, the second call waits and finds the first one's work.
    {
        let mut admin = db.admin().await;
        sqlx::raw_sql(
            "CREATE FUNCTION identity.slow() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(0.5); RETURN NEW; END $$; \
             CREATE TRIGGER slow BEFORE INSERT ON identity.workers FOR EACH ROW EXECUTE FUNCTION identity.slow();",
        )
        .execute(&mut admin)
        .await
        .expect("slow trigger");
    }
    let (a, b) = tokio::join!(run(&pool, "second", "alice"), run(&pool, "Second", "ALICE"));
    let (a, b) = (a.expect("a"), b.expect("b: no unique violation leaks out"));
    assert_eq!(
        (a.project, a.lead),
        (b.project, b.lead),
        "both return the same ids"
    );
    assert_eq!(
        [a.created, b.created].iter().filter(|c| **c).count(),
        1,
        "exactly one created it"
    );
    assert_eq!(
        shape(&db).await.0,
        vec![1, 2, 0, 2, 0, 4],
        "one installation, two projects, two leads"
    );
}

/// ID2C-01 (sam): on a re-run the function must not return success for project P while the transaction's
/// context and `cortex.project_id` name another id: the host path's next steps (token, audit) are
/// project-scoped and would run in the wrong project.
#[tokio::test]
async fn bo_01b_a_rerun_never_succeeds_in_a_transaction_that_names_another_project() {
    let (db, pool) = setup("mi_bo_rerun_ctx").await;
    let first = run(&pool, "helix", "kai").await.expect("first run");
    let before = shape(&db).await;
    // A fresh id Q on a re-run: refused, nothing changed.
    let mut tx = host_tx(&pool).await;
    let r = bootstrap_project(&mut tx, "helix", "kai").await;
    assert_eq!(
        r,
        Err(BootstrapError::OtherProject(first.project)),
        "the transaction names a fresh project"
    );
    let _ = tx.rollback().await;
    // The right id: success, and the delegated identity lookup in the SAME transaction finds the project.
    let mut tx = ModuleTx::begin_host(
        &pool,
        HostContext {
            uid: 1000,
            project: Some(first.project),
            request_id: RequestId::new(&SystemClock),
        },
        TxOptions::default(),
    )
    .await
    .expect("begin_host");
    let b = bootstrap_project(&mut tx, "helix", "kai")
        .await
        .expect("re-run in the right project");
    assert_eq!(b.project, first.project);
    assert_eq!(
        tx.context().project,
        Some(b.project),
        "returned project and transaction context agree"
    );
    let seen = mod_identity::Provider::new()
        .project(&mut tx)
        .await
        .expect("delegated lookup in the same tx");
    assert_eq!(
        seen.id, b.project,
        "the delegated IdentityApi sees the project the function returned"
    );
    let _ = tx.rollback().await;
    assert_eq!(shape(&db).await, before);
}

/// ID2C-02 (sam): an ordinary module transaction that merely CARRIES a host actor is not a host-path
/// transaction: refused as `Context`, and the transaction stays usable.
#[tokio::test]
async fn bo_09b_a_module_transaction_carrying_a_host_actor_is_refused_as_context() {
    let (db, pool) = setup("mi_bo_fake_host").await;
    let mut c = ctx(mod_identity::ID, Some(ProjectId::new(&SystemClock)));
    c.actor = cortex_db::Actor::HostOperator { uid: 1000 };
    let mut tx = ModuleTx::begin(&pool, c, TxOptions::default())
        .await
        .expect("begin");
    assert_eq!(
        bootstrap_project(&mut tx, "helix", "kai").await,
        Err(BootstrapError::Context)
    );
    // Still usable: a plain read works after the refusal.
    let one: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("usable");
    assert_eq!(one, 1);
    let _ = tx.rollback().await;
    assert_eq!(shape(&db).await.0, vec![0, 0, 0, 0, 0, 0]);
}

#[tokio::test]
async fn find_project_is_read_only_by_name_key_and_host_only() {
    let (db, pool) = setup("mi_bo_find").await;
    let host = |project| HostContext {
        uid: 1000,
        project,
        request_id: RequestId::new(&SystemClock),
    };
    let mut tx = ModuleTx::begin_host(&pool, host(None), TxOptions::default())
        .await
        .expect("begin_host");
    assert_eq!(
        find_project(&mut tx, "helix").await,
        Ok(None),
        "empty database"
    );
    assert_eq!(
        find_project(&mut tx, "a b").await,
        Err(BootstrapError::Invalid)
    );
    let _ = tx.rollback().await;
    let first = run(&pool, "Helix", "kai").await.expect("first");
    let before = shape(&db).await;
    let mut tx = ModuleTx::begin_host(&pool, host(None), TxOptions::default())
        .await
        .expect("begin_host");
    for name in ["helix", "HELIX", "Helix"] {
        let f = find_project(&mut tx, name)
            .await
            .expect("ok")
            .expect("found");
        assert_eq!(
            (f.project, f.lead, f.installation),
            (first.project, first.lead, first.installation)
        );
    }
    assert_eq!(find_project(&mut tx, "other").await, Ok(None));
    let _ = tx.rollback().await;
    assert_eq!(shape(&db).await, before, "read-only");
    // Not a host transaction: Context.
    let mut tx = ModuleTx::begin(
        &pool,
        ctx(mod_identity::ID, Some(first.project)),
        TxOptions::default(),
    )
    .await
    .expect("begin");
    assert_eq!(
        find_project(&mut tx, "helix").await,
        Err(BootstrapError::Context)
    );
    let _ = tx.rollback().await;
}
