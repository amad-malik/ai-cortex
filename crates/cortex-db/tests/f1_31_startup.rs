//! F1-31 categories 12, 13 and 15 (TESTS r7; A7 r4 item 2, "Startup verification" and
//! "Lifecycle"). `check_definer_functions` is the startup check and `doctor`'s. Each planted case
//! has its own database. **Checker off** (r6, vera): each planted fixture is also asserted to be
//! real, case by case: a changed one is present and callable, an extra one is listable, a missing
//! one is absent from `pg_proc`.
//!
//! **The owner's ACL entry (vera's R0 note, r8 list F1):** PostgreSQL's owner has implicit grant
//! options, and the stored grant-option bit of its own entry is not inferred from ownership. The
//! check normalises explicitly: the owner's entry must be present with `EXECUTE`, and its stored
//! grantability is not compared. Every other entry must be exactly `(cortex_m_<module>, EXECUTE,
//! not grantable)`, and a NULL ACL refuses.
//! RED (b2): `check_definer_functions`, the template and the b1 core are `todo!()`.
use cortex_db::maintenance::templates;
use cortex_db::maintenance::{DefinerError, FnId, MaintError};
use cortex_testkit::maint::{Maint, SPEC_A, maint};
use cortex_testkit::within;

const CREATE: &str = "test_a.create_partition(uuid, date)";

fn id(schema: &str, name: &str, args: &str) -> FnId {
    FnId {
        schema: schema.into(),
        name: name.into(),
        args: args.into(),
    }
}

fn create_id() -> FnId {
    id("test_a", "create_partition", "uuid, date")
}

/// Rewrites a function's body in place (`CREATE OR REPLACE` from its own definition, with a
/// comment added), keeping its signature, owner, ACL and settings.
fn touch_body(regprocedure: &str) -> String {
    format!(
        "DO $plant$ DECLARE d text; BEGIN \
         SELECT pg_get_functiondef('{regprocedure}'::regprocedure) INTO d; \
         EXECUTE regexp_replace(d, '\\$function\\$', E'$function$\\n-- planted', ''); END $plant$;"
    )
}

/// Calls `schema.create_partition(NULL, NULL)` as the superuser. r2 (mike's F1B-R0-08):
/// "callable" means the call succeeded, or the body ran and gave exactly its typed NULL-value
/// refusal (`P0C03`, `partition_bad_value`). Any other SQLSTATE, or a client or transport error,
/// is not evidence that the body ran.
async fn callable(w: &Maint, schema: &str) -> bool {
    let mut admin = w.db.admin().await;
    match sqlx::query(sqlx::AssertSqlSafe(format!(
        "select {schema}.create_partition(null, null)"
    )))
    .execute(&mut admin)
    .await
    {
        Ok(_) => true,
        Err(e) => e.as_database_error().is_some_and(|d| {
            d.code().as_deref() == Some(MaintError::BadValue.sqlstate())
                && d.message() == MaintError::BadValue.code()
        }),
    }
}

#[tokio::test]
async fn cat12_the_approved_set_starts() {
    within(180, async {
        let w = maint("c12ok").await;
        assert_eq!(
            w.check().await,
            Ok(()),
            "the approved set, with EXECUTE-only module grants, must start"
        );
    })
    .await
}

#[tokio::test]
async fn cat12_each_planted_function_fault_stops_startup_and_is_real_with_the_checker_off() {
    within(600, async {
        type Reality = for<'a> fn(&'a Maint) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + 'a>>;
        let present: Reality = |w| Box::pin(async move { w.function_exists(CREATE).await && callable(w, "test_a").await });
        let cases: Vec<(&str, &str, String, DefinerError, Reality)> = vec![
            (
                "missing",
                "owner",
                format!("DROP FUNCTION {CREATE}"),
                DefinerError::Missing(create_id()),
                |w| Box::pin(async move { !w.function_exists(CREATE).await }),
            ),
            (
                "invoker",
                "owner",
                format!("ALTER FUNCTION {CREATE} SECURITY INVOKER"),
                DefinerError::NotDefiner(create_id()),
                present,
            ),
            (
                "signature",
                "owner",
                format!(
                    "DROP FUNCTION {CREATE}; CREATE FUNCTION test_a.create_partition(project_id uuid, month text) \
                     RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, test_a, pg_temp \
                     AS $$ BEGIN RETURN 'x'; END $$; \
                     REVOKE ALL ON FUNCTION test_a.create_partition(uuid, text) FROM PUBLIC; \
                     GRANT EXECUTE ON FUNCTION test_a.create_partition(uuid, text) TO cortex_m_test_a;"
                ),
                DefinerError::Signature(create_id()),
                |w| Box::pin(async move { w.function_exists("test_a.create_partition(uuid, text)").await }),
            ),
            (
                "return type",
                "owner",
                format!(
                    "DROP FUNCTION {CREATE}; CREATE FUNCTION test_a.create_partition(project_id uuid, month date) \
                     RETURNS integer LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, test_a, pg_temp \
                     AS $$ BEGIN RETURN 1; END $$; \
                     REVOKE ALL ON FUNCTION {CREATE} FROM PUBLIC; GRANT EXECUTE ON FUNCTION {CREATE} TO cortex_m_test_a;"
                ),
                DefinerError::Signature(create_id()),
                present,
            ),
            (
                "an extra overload",
                "owner",
                "CREATE FUNCTION test_a.create_partition(project_id uuid, month date, extra integer) \
                 RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, test_a, pg_temp \
                 AS $$ BEGIN RETURN 'x'; END $$;"
                    .into(),
                DefinerError::Overloaded(id("test_a", "create_partition", "uuid, date, integer")),
                |w| Box::pin(async move { w.function_exists("test_a.create_partition(uuid, date, integer)").await }),
            ),
            (
                "an extra helper",
                "owner",
                "CREATE FUNCTION test_a.helper_x() RETURNS integer LANGUAGE plpgsql SECURITY DEFINER \
                 SET search_path = pg_catalog, test_a, pg_temp AS $$ BEGIN RETURN 1; END $$;"
                    .into(),
                DefinerError::Extra(id("test_a", "helper_x", "")),
                |w| Box::pin(async move { w.function_exists("test_a.helper_x()").await }),
            ),
            (
                "the body changed",
                "owner",
                touch_body(CREATE),
                DefinerError::Body(create_id()),
                present,
            ),
            (
                "a helper changed",
                "owner",
                touch_body("test_a.drop_eligible(uuid, date)"),
                DefinerError::Helper(id("test_a", "drop_partition", "uuid, date, uuid")),
                |w| Box::pin(async move { w.function_exists("test_a.drop_eligible(uuid, date)").await }),
            ),
            (
                "search_path changed",
                "owner",
                format!("ALTER FUNCTION {CREATE} SET search_path = public, pg_temp"),
                DefinerError::SearchPath(create_id()),
                present,
            ),
            (
                "owner changed",
                "admin",
                format!("ALTER FUNCTION {CREATE} OWNER TO cortex_base"),
                DefinerError::Owner(create_id()),
                present,
            ),
            (
                "a grantee added",
                "owner",
                format!("GRANT EXECUTE ON FUNCTION {CREATE} TO cortex_m_test_b"),
                DefinerError::Grantees(create_id()),
                present,
            ),
            (
                "the module's EXECUTE with grant option",
                "owner",
                format!("GRANT EXECUTE ON FUNCTION {CREATE} TO cortex_m_test_a WITH GRANT OPTION"),
                DefinerError::GrantOption(create_id()),
                present,
            ),
            (
                "a NULL ACL",
                "admin",
                format!("UPDATE pg_proc SET proacl = NULL WHERE oid = '{CREATE}'::regprocedure"),
                DefinerError::GrantOption(create_id()),
                present,
            ),
        ];
        for (n, (what, who, plant, want, real)) in cases.into_iter().enumerate() {
            let w = maint(&format!("c12_{n:02}")).await;
            match who {
                "admin" => w.as_admin(&plant).await,
                _ => w.as_owner(&plant).await,
            }
            let got = w.check().await;
            let errors = got.as_ref().err().cloned().unwrap_or_default();
            assert!(
                errors.contains(&want),
                "{what}: startup did not refuse with {want:?}; got {got:?}"
            );
            // The problem text names the identity.
            assert!(
                errors.iter().any(|e| e.problem().starts_with("definer_function_unsafe(")),
                "{what}: no definer_function_unsafe problem"
            );
            assert!(real(&w).await, "{what}: the planted fixture is not real with the checker off");
            if what == "a NULL ACL" {
                assert_eq!(w.fn_acl(CREATE).await, vec!["NULL".to_owned()], "the ACL is not NULL");
            }
        }
    })
    .await
}

#[tokio::test]
async fn cat13_a_sealed_migration_with_a_new_approved_version_passes() {
    within(180, async {
        let w = maint("c13ok").await;
        assert_eq!(w.check().await, Ok(()), "startup");
        // The new approved version: the template for a wider create window, installed by a
        // migration (as cortex_owner) and pinned by the new inventory.
        let mut spec = SPEC_A.clone();
        spec.create_window.latest = 3;
        let new = templates::partition_maintenance(&spec);
        w.as_owner(
            "DROP TRIGGER IF EXISTS maintenance_audited ON test_a.maintenance_runs; \
             DROP TABLE IF EXISTS test_a.maintenance_runs; \
             DROP FUNCTION IF EXISTS test_a.create_partition(uuid, date); \
             DROP FUNCTION IF EXISTS test_a.drop_partition(uuid, date, uuid); \
             DROP FUNCTION IF EXISTS test_a.maintenance_audit();",
        )
        .await;
        w.as_owner(&new.up).await;
        let old_inventory = w.inventory();
        let mut inv: Vec<_> = old_inventory
            .iter()
            .filter(|f| f.schema != "test_a" || f.name == templates::ELIGIBLE_FN)
            .cloned()
            .collect();
        let world = Maint {
            sql_a: new.clone(),
            ..w
        };
        inv.extend(
            world
                .inventory()
                .into_iter()
                .filter(|f| f.schema == "test_a" && f.name != templates::ELIGIBLE_FN),
        );
        assert_eq!(
            world.check_with(&inv).await,
            Ok(()),
            "the new approved version"
        );
        assert!(
            world.check_with(&old_inventory).await.is_err(),
            "the old pins accepted the new bodies"
        );
    })
    .await
}

#[tokio::test]
async fn cat13_an_owner_change_outside_the_lifecycle_is_reported_by_doctor_and_refused_at_startup()
{
    within(180, async {
        let w = maint("c13bad").await;
        assert_eq!(w.check().await, Ok(()), "startup");
        // While running: an owner-level change, outside a sealed migration.
        w.as_owner(&format!(
            "ALTER FUNCTION {CREATE} SET search_path = public, pg_temp"
        ))
        .await;
        let doctor = w.check().await;
        assert!(
            doctor
                .as_ref()
                .is_err_and(|e| e.contains(&DefinerError::SearchPath(create_id()))),
            "doctor did not report it: {doctor:?}"
        );
        assert!(w.check().await.is_err(), "the next startup did not refuse");
    })
    .await
}

#[tokio::test]
async fn cat15_an_unapproved_function_from_a_migration_is_caught_at_startup() {
    within(180, async {
        // A stated boundary, not a containment test: a migration runs as cortex_owner and is a
        // trusted release artifact. What it leaves behind is detected at startup.
        let w = maint("c15").await;
        w.as_owner(
            "CREATE FUNCTION test_a.rogue() RETURNS integer LANGUAGE plpgsql SECURITY DEFINER \
             SET search_path = pg_catalog, test_a, pg_temp AS $$ BEGIN RETURN 1; END $$;",
        )
        .await;
        let got = w.check().await;
        assert!(
            got.as_ref()
                .is_err_and(|e| e.contains(&DefinerError::Extra(id("test_a", "rogue", "")))),
            "the unapproved function was not caught: {got:?}"
        );
    })
    .await
}

/// The OID of `regprocedure`, or None.
async fn oid_of(w: &Maint, regprocedure: &str) -> Option<i64> {
    let mut admin = w.db.admin().await;
    sqlx::query_scalar("select to_regprocedure($1)::oid::bigint")
        .bind(regprocedure)
        .fetch_one(&mut admin)
        .await
        .expect("to_regprocedure")
}

/// vera's F1B-B2-02: A's `create_partition` moved into a fresh owner-owned schema outside the
/// module schemas (test_b already has its own function of that name and signature, so moving
/// there would fail as a duplicate before the check ran). The moved object is the same OID; the
/// old identity is gone; test_b's own function is untouched.
#[tokio::test]
async fn cat12_a_function_moved_out_of_its_schema_is_moved_and_real() {
    within(180, async {
        let w = maint("c12moved").await;
        let original = oid_of(&w, CREATE)
            .await
            .expect("A's function before the move");
        let b_before = oid_of(&w, "test_b.create_partition(uuid, date)").await;
        w.as_owner(&format!(
            "CREATE SCHEMA test_moved; ALTER FUNCTION {CREATE} SET SCHEMA test_moved"
        ))
        .await;
        assert_eq!(
            oid_of(&w, CREATE).await,
            None,
            "the old identity is still there"
        );
        assert_eq!(
            oid_of(&w, "test_moved.create_partition(uuid, date)").await,
            Some(original),
            "the moved object is not the original"
        );
        assert_eq!(
            oid_of(&w, "test_b.create_partition(uuid, date)").await,
            b_before,
            "test_b's own function changed"
        );
        let got = w.check().await;
        assert!(
            got.as_ref()
                .is_err_and(|e| e.contains(&DefinerError::Moved(create_id()))),
            "startup did not refuse with Moved: {got:?}"
        );
        assert!(
            callable(&w, "test_moved").await,
            "the moved function is not callable"
        );
    })
    .await
}

/// vera's E8 hardening partners. The owner's entry is required with EXECUTE, and only its stored
/// grant-option bit is ignored: with the bit stored set, the approved set still starts; with the
/// owner's entry absent, startup refuses.
#[tokio::test]
async fn cat12_the_owners_entry_is_required_but_its_stored_grant_bit_is_ignored() {
    within(180, async {
        let w = maint("c12own_bit").await;
        w.as_admin(&format!(
            "UPDATE pg_proc SET proacl = '{{cortex_owner=X*/cortex_owner,cortex_m_test_a=X/cortex_owner}}' \
             WHERE oid = '{CREATE}'::regprocedure"
        ))
        .await;
        assert_eq!(
            w.fn_acl(CREATE).await,
            vec!["cortex_m_test_a EXECUTE false, cortex_owner EXECUTE true".to_owned()],
            "the plant did not store the owner's grant bit"
        );
        assert_eq!(w.check().await, Ok(()), "a stored owner grant bit must not refuse");

        let w = maint("c12own_gone").await;
        w.as_admin(&format!(
            "UPDATE pg_proc SET proacl = '{{cortex_m_test_a=X/cortex_owner}}' \
             WHERE oid = '{CREATE}'::regprocedure"
        ))
        .await;
        let got = w.check().await;
        assert!(
            got.as_ref().is_err_and(|e| e.contains(&DefinerError::Grantees(create_id()))),
            "an ACL without the owner's EXECUTE entry was accepted: {got:?}"
        );
    })
    .await
}
