//! F1-31 categories 10 and 11 (TESTS r7; A7 r4 item 2 step 6). The created child's access is
//! set, then verified, inside the function. r6 (vera): the testkit-only variant plants a bad
//! grant or a permissive policy after the repair and before the verify. Vera's R0 note (r8 list
//! F2): category 10's outside reader, `cortex_m_test_b`, has schema `USAGE` on `test_a` and
//! project A's context, so a refusal can only come from the child's own ACL and the verify-off
//! control really returns rows.
//! RED (b2): the template and the b1 core are `todo!()`.
use cortex_db::maintenance::MaintError;
use cortex_db::maintenance::templates;
use cortex_testkit::maint::{
    Maint, Mutation, RETENTION_ROLE, Refusal, SPEC_A, count, create, exec, leaf, maint, maint_with,
    month, refused, uuid, variant,
};
use cortex_testkit::within;

fn unsafe_access() -> Result<String, Refusal> {
    Err(refused(MaintError::AccessUnsafe))
}

fn is_denied<T: std::fmt::Debug>(r: &Result<T, Refusal>) -> bool {
    matches!(r, Err(Refusal::Sql(code)) if code == "42501")
}

/// Creates A's month 1 and inserts `rows` rows for A through the parent.
async fn child_with_rows(w: &Maint, rows: i64) -> String {
    let a = uuid(w.a);
    let mut tx = w.begin("test_a", Some(w.a)).await;
    assert_eq!(
        create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
        Ok("created".into())
    );
    for i in 0..rows {
        exec(
            &mut tx,
            &format!(
                "insert into test_a.p (project_id, month, id, v) values ('{a}', '{}', {i}, 'row')",
                month(1)
            ),
        )
        .await
        .expect("insert through the parent");
    }
    tx.commit().await.expect("commit");
    leaf(&a, 1)
}

async fn outside_reader_can_see_schema(w: &Maint) {
    w.as_owner("GRANT USAGE ON SCHEMA test_a TO cortex_m_test_b")
        .await;
}

#[tokio::test]
async fn cat10_the_child_gets_exactly_the_modules_dml_set_despite_a_default_grant() {
    within(180, async {
        let w = maint("c10ok").await;
        w.as_owner(
            "ALTER DEFAULT PRIVILEGES FOR ROLE cortex_owner GRANT SELECT ON TABLES TO PUBLIC",
        )
        .await;
        let child = child_with_rows(&w, 1).await;
        let not_owner: Vec<String> = w
            .rel_acl("test_a", &child)
            .await
            .into_iter()
            .filter(|e| !e.starts_with("cortex_owner "))
            .collect();
        assert_eq!(
            not_owner,
            vec![
                "cortex_m_test_a INSERT false".to_owned(),
                "cortex_m_test_a SELECT false".to_owned(),
                "cortex_m_test_ret DELETE false".to_owned(),
            ],
            "the child's ACL is not exactly the spec's per-role grants (E3)"
        );
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            count(&mut tx, &format!("select count(*) from test_a.{child}")).await,
            Ok(1),
            "the module role reads its own rows"
        );
        tx.rollback().await.ok();
        outside_reader_can_see_schema(&w).await;
        let mut tx = w.begin("test_b", Some(w.a)).await;
        let r = count(&mut tx, &format!("select count(*) from test_a.{child}")).await;
        assert!(is_denied(&r), "the outside role read the child: {r:?}");
        tx.rollback().await.ok();
        // r2 (E3, mike's F1B-R0-09): DELETE is the retention role's alone.
        let mut tx = w.begin("test_a", Some(w.a)).await;
        let r = exec(&mut tx, &format!("delete from test_a.{child}")).await;
        assert!(
            is_denied(&r),
            "the module role deleted from the child: {r:?}"
        );
        tx.rollback().await.ok();
        let mut tx = w.begin("test_ret", Some(w.a)).await;
        exec(&mut tx, &format!("delete from test_a.{child}"))
            .await
            .expect("the retention role deletes");
        tx.commit().await.expect("commit");
        let mut admin = w.db.admin().await;
        let left: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "select count(*) from test_a.{child}"
        )))
        .fetch_one(&mut admin)
        .await
        .expect("count");
        assert_eq!(left, 0, "the retention role's delete did not take");
        // nemo's confirmation (3): no membership shortcut between the module and retention roles,
        // and DELETE on data gives the retention role no partition-maintenance EXECUTE.
        let shortcut: bool = sqlx::query_scalar(
            "select pg_has_role('cortex_m_test_a', 'cortex_m_test_ret', 'MEMBER') \
             or pg_has_role('cortex_m_test_ret', 'cortex_m_test_a', 'MEMBER')",
        )
        .fetch_one(&mut admin)
        .await
        .expect("pg_has_role");
        assert!(
            !shortcut,
            "a membership shortcut between module and retention roles"
        );
        for f in [
            "test_a.create_partition(uuid, date)",
            "test_a.drop_partition(uuid, date, uuid)",
        ] {
            assert!(
                !w.can_execute(RETENTION_ROLE, f).await,
                "the retention role can execute {f}"
            );
        }
    })
    .await
}

/// nemo's confirmation (3), the extra-grant rollback control: an extra DELETE for the module role,
/// planted after the repair, is refused by the verify, and no child remains.
#[tokio::test]
async fn cat10_an_extra_module_delete_is_unsafe_and_no_child_remains() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(&base, &SPEC_A, Mutation::ExtraModuleDelete)
            .expect("the variant: its hook exists");
        let w = maint_with("c10extra", v).await;
        let a = uuid(w.a);
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            unsafe_access()
        );
        tx.rollback().await.ok();
        assert_eq!(w.relkind("test_a", &leaf(&a, 1)).await, None);
    })
    .await
}

/// r2 (E3): every grant is part of the atomic repair-and-verify. With the retention role's GRANT
/// line removed from the repair and the verify kept, the verify sees the missing DELETE and
/// refuses, and no child remains: a grant added after the function returned would be outside it.
#[tokio::test]
async fn cat10_a_missing_retention_grant_is_unsafe_and_no_child_remains() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(&base, &SPEC_A, Mutation::RetentionGrantOmitted)
            .expect("the variant: its grant marker exists");
        let w = maint_with("c10ret", v).await;
        let a = uuid(w.a);
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            unsafe_access()
        );
        tx.rollback().await.ok();
        assert_eq!(w.relkind("test_a", &leaf(&a, 1)).await, None);
    })
    .await
}

#[tokio::test]
async fn cat10_a_public_grant_planted_before_the_verify_is_unsafe_and_no_child_remains() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(&base, &SPEC_A, Mutation::PlantPublicSelect { verify: true })
            .expect("the variant: its marker exists");
        let w = maint_with("c10bad", v).await;
        let a = uuid(w.a);
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            unsafe_access()
        );
        tx.rollback().await.ok();
        assert_eq!(w.relkind("test_a", &leaf(&a, 1)).await, None);
    })
    .await
}

#[tokio::test]
async fn cat10_control_with_the_verify_off_the_planted_grant_reaches_the_child() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(
            &base,
            &SPEC_A,
            Mutation::PlantPublicSelect { verify: false },
        )
        .expect("the variant: its markers exist");
        let w = maint_with("c10ctl", v).await;
        let child = child_with_rows(&w, 2).await;
        assert!(
            w.rel_acl("test_a", &child)
                .await
                .contains(&"PUBLIC SELECT false".to_owned()),
            "the planted grant is not on the child"
        );
        outside_reader_can_see_schema(&w).await;
        let mut tx = w.begin("test_b", Some(w.a)).await;
        assert_eq!(
            count(&mut tx, &format!("select count(*) from test_a.{child}")).await,
            Ok(2),
            "with the verify off, the outside role reads the child's rows"
        );
        tx.rollback().await.ok();
    })
    .await
}

#[tokio::test]
async fn cat11_the_child_has_enable_force_and_its_own_policy_directly_and_through_the_parent() {
    within(180, async {
        let w = maint("c11ok").await;
        let child = child_with_rows(&w, 2).await;
        let mut admin = w.db.admin().await;
        let flags: (bool, bool) = sqlx::query_as(
            "select relrowsecurity, relforcerowsecurity from pg_class where oid = $1::regclass",
        )
        .bind(format!("test_a.{child}"))
        .fetch_one(&mut admin)
        .await
        .expect("flags");
        assert_eq!(flags, (true, true), "ENABLE and FORCE");
        let policies: Vec<(String, String)> = sqlx::query_as(
            "select cmd, qual from pg_policies where schemaname = 'test_a' and tablename = $1",
        )
        .bind(&child)
        .fetch_all(&mut admin)
        .await
        .expect("policies");
        assert_eq!(policies.len(), 1, "exactly one policy: {policies:?}");
        assert_eq!(policies[0].0, "ALL");
        assert!(
            // GREEN (finn): PostgreSQL deparses NULLIF in capitals, so the text is compared case-blind.
            policies[0].1.to_lowercase().contains("nullif")
                && policies[0].1.contains("cortex.project_id"),
            "not the nullif project policy: {}",
            policies[0].1
        );
        for (table, what) in [(child.as_str(), "the child"), ("p", "the parent")] {
            let q = format!("select count(*) from test_a.{table}");
            for (ctx, want) in [(Some(w.a), 2), (Some(w.b), 0), (None, 0)] {
                let mut tx = w.begin("test_a", ctx).await;
                assert_eq!(
                    count(&mut tx, &q).await,
                    Ok(want),
                    "{what}, context {ctx:?}"
                );
                tx.rollback().await.ok();
            }
            let mut tx = w.begin("test_b", Some(w.a)).await;
            let r = count(&mut tx, &q).await;
            assert!(is_denied(&r), "test_b read {what}: {r:?}");
            tx.rollback().await.ok();
            for s in [
                format!("update test_a.{table} set v = 'x'"),
                format!("delete from test_a.{table}"),
                format!("truncate test_a.{table}"),
                format!("alter table test_a.{table} owner to cortex_m_test_a"),
            ] {
                let mut tx = w.begin("test_a", Some(w.a)).await;
                let r = exec(&mut tx, &s).await;
                assert!(is_denied(&r), "{s}: {r:?}");
                tx.rollback().await.ok();
            }
        }
    })
    .await
}

#[tokio::test]
async fn cat11_a_permissive_policy_planted_before_the_verify_is_unsafe_and_no_child_remains() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(
            &base,
            &SPEC_A,
            Mutation::PlantPermissivePolicy { verify: true },
        )
        .expect("the variant: its marker exists");
        let w = maint_with("c11bad", v).await;
        let a = uuid(w.a);
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            unsafe_access()
        );
        tx.rollback().await.ok();
        assert_eq!(w.relkind("test_a", &leaf(&a, 1)).await, None);
    })
    .await
}

#[tokio::test]
async fn cat11_control_with_the_verify_off_a_direct_read_with_b_context_returns_as_rows() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(
            &base,
            &SPEC_A,
            Mutation::PlantPermissivePolicy { verify: false },
        )
        .expect("the variant: its markers exist");
        let w = maint_with("c11ctl", v).await;
        let child = child_with_rows(&w, 2).await;
        let mut tx = w.begin("test_a", Some(w.b)).await;
        assert_eq!(
            count(&mut tx, &format!("select count(*) from test_a.{child}")).await,
            Ok(2),
            "with the verify off, B's context reads A's rows directly from the child"
        );
        tx.rollback().await.ok();
    })
    .await
}
