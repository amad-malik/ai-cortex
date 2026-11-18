//! F1-31 categories 3 and 4 (TESTS r7; A7 r4 item 2, "Lifecycle"). No module or app role owns a
//! maintenance function, is a member of `cortex_owner`, or can change one. r6 (vera): a module
//! role's `GRANT EXECUTE … TO PUBLIC` is not a `42501` oracle (a non-owner without a grant option
//! gets a warning and grants nothing); the oracle is the unchanged ACL and PUBLIC still denied.
//! RED (b2): the template and the b1 core are `todo!()`.
use cortex_testkit::maint::{Refusal, exec, maint};
use cortex_testkit::{sqlstate, within};

const FN: &str = "test_a.create_partition(uuid, date)";

fn denied(r: Result<(), Refusal>) -> bool {
    r == Err(Refusal::Sql("42501".into()))
}

#[tokio::test]
async fn cat03_no_module_or_app_role_can_become_or_belongs_to_the_owner() {
    within(180, async {
        let w = maint("c03").await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert!(
            denied(exec(&mut tx, "set role cortex_owner").await),
            "cortex_m_test_a became cortex_owner"
        );
        tx.rollback().await.ok();
        let mut app = w.db.login("cortex_app").await;
        let e = sqlx::query("set role cortex_owner")
            .execute(&mut app)
            .await
            .expect_err("cortex_app became cortex_owner");
        assert_eq!(sqlstate(&e).as_deref(), Some("42501"));
        let mut admin = w.db.admin().await;
        let direct: i64 = sqlx::query_scalar(
            "select count(*) from pg_auth_members m where m.roleid = 'cortex_owner'::regrole \
             and m.member in ('cortex_m_test_a'::regrole, 'cortex_app'::regrole)",
        )
        .fetch_one(&mut admin)
        .await
        .expect("pg_auth_members");
        assert_eq!(direct, 0, "a direct membership of cortex_owner");
        let indirect: bool = sqlx::query_scalar(
            "select pg_has_role('cortex_m_test_a', 'cortex_owner', 'MEMBER') \
             or pg_has_role('cortex_app', 'cortex_owner', 'MEMBER')",
        )
        .fetch_one(&mut admin)
        .await
        .expect("pg_has_role");
        assert!(!indirect, "an indirect membership of cortex_owner");
    })
    .await
}

#[tokio::test]
async fn cat04_module_and_app_roles_cannot_alter_replace_or_regrant() {
    within(180, async {
        let w = maint("c04").await;
        let statements = [
            format!("alter function {FN} owner to cortex_m_test_a"),
            format!("alter function {FN} set search_path = public"),
            "create or replace function test_a.create_partition(project_id uuid, month date) \
             returns text language sql as $$ select 'x' $$"
                .to_owned(),
        ];
        for s in &statements {
            let mut tx = w.begin("test_a", Some(w.a)).await;
            assert!(denied(exec(&mut tx, s).await), "as cortex_m_test_a: {s}");
            tx.rollback().await.ok();
            let mut app = w.db.login("cortex_app").await;
            let e = sqlx::raw_sql(sqlx::AssertSqlSafe(s.clone()))
                .execute(&mut app)
                .await
                .expect_err("cortex_app changed the function");
            assert_eq!(sqlstate(&e).as_deref(), Some("42501"), "as cortex_app: {s}");
        }
        let acl_before = w.fn_acl(FN).await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        exec(
            &mut tx,
            &format!("grant execute on function {FN} to public"),
        )
        .await
        .expect("a warning, not an error: nothing is granted");
        tx.commit().await.expect("commit");
        assert_eq!(w.fn_acl(FN).await, acl_before, "the ACL changed");
        assert!(
            !w.can_execute("cortex_m_test_b", FN).await,
            "a role outside the module gained EXECUTE"
        );
    })
    .await
}

#[tokio::test]
async fn cat04_control_a_planted_grant_option_lets_the_module_grant_onward() {
    within(180, async {
        let w = maint("c04ctl").await;
        // Planted by the owner; the startup check is off (not run here: category 12 refuses it).
        w.as_owner(&format!(
            "grant execute on function {FN} to cortex_m_test_a with grant option"
        ))
        .await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        exec(
            &mut tx,
            &format!("grant execute on function {FN} to public"),
        )
        .await
        .expect("granted, with the planted option");
        tx.commit().await.expect("commit");
        assert!(
            w.can_execute("cortex_m_test_b", FN).await,
            "with a grant option the module grants onward, so the outside role gains EXECUTE"
        );
    })
    .await
}
