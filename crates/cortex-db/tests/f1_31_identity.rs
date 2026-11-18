//! F1-31 categories 6 and 7 (TESTS r7; A7 r4 item 2 step 3). The relation found under the
//! computed name must be exactly the expected leaf; there is no ATTACH path; a relation that is
//! not the exact leaf is a `partition_conflict`, and nothing else is touched. r6 (vera): a
//! replacement blocked by the held lock is a serialisation outcome, asserted by what each side
//! sees after the wait. r2 (vera's F1B-B2-04): the race is coordinated INSIDE the function, at
//! a test gate after validation and before any DDL, so a late native DROP lock cannot stand in
//! for the validation locks; the competitor's wait is bound to T1 and to the target relation;
//! and a lock-skipping variant lets the replacement through while T1 waits at the gate. A leaf
//! with the right name and parent but the wrong bounds, or the wrong owner, is a conflict too.
//! RED (b2): the template and the b1 core are `todo!()`.
use std::time::Duration;

use cortex_db::maintenance::MaintError;
use cortex_db::maintenance::templates;
use cortex_testkit::maint::{
    Maint, Mutation, PAUSE_CLASS, PAUSE_KEY, Refusal, SPEC_A, audit, create, drop, leaf, maint,
    maint_with, month, refused, uuid, variant, variants,
};
use cortex_testkit::{sqlstate, within};
use sqlx::Connection;

fn conflict() -> Result<String, Refusal> {
    Err(refused(MaintError::Conflict))
}

#[tokio::test]
async fn cat06_a_rerun_on_the_exact_partition_returns_exists() {
    within(180, async {
        let w = maint("c06ok").await;
        let a = uuid(w.a);
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            Ok("created".into())
        );
        tx.commit().await.expect("commit");
        let before = w.snapshot().await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            Ok("exists".into())
        );
        tx.commit().await.expect("commit");
        assert_eq!(w.snapshot().await, before, "a re-run changed something");
    })
    .await
}

/// A same-name ordinary table, with a row that would fit the month (so an ATTACH could succeed).
async fn plant_same_name_table(w: &cortex_testkit::maint::Maint) -> String {
    let a = uuid(w.a);
    let name = leaf(&a, 1);
    w.as_owner(&format!(
        "CREATE TABLE test_a.{name} (LIKE test_a.p); \
         INSERT INTO test_a.{name} VALUES ('{a}', '{}', 1, 'planted');",
        month(1)
    ))
    .await;
    name
}

#[tokio::test]
async fn cat06_a_same_name_ordinary_table_is_a_conflict_and_is_untouched() {
    within(180, async {
        let w = maint("c06bad").await;
        let name = plant_same_name_table(&w).await;
        let before = w.snapshot().await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(uuid(w.a)), Some(&month(1))).await,
            conflict()
        );
        tx.rollback().await.ok();
        assert_eq!(
            w.snapshot().await,
            before,
            "the table, its rows or pg_inherits changed"
        );
        assert_eq!(w.relkind("test_a", &name).await.as_deref(), Some("r"));
        assert_eq!(w.parent_of("test_a", &name).await, None, "it was attached");
    })
    .await
}

#[tokio::test]
async fn cat06_control_an_attaching_template_attaches_the_planted_table() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(&base, &SPEC_A, Mutation::AttachOnCollision)
            .expect("the variant: its marker exists");
        let w = maint_with("c06ctl", v).await;
        let name = plant_same_name_table(&w).await;
        let a = uuid(w.a);
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            Ok("exists".into())
        );
        tx.commit().await.expect("commit");
        assert_eq!(
            w.parent_of("test_a", &name).await,
            Some(format!(
                "test_a.{}",
                templates::project_partition_name("p", &a)
            )),
            "the attaching variant did not attach the planted table"
        );
    })
    .await
}

#[tokio::test]
async fn cat07_the_exact_leaf_drops_and_an_absent_month_is_absent() {
    within(180, async {
        let w = maint("c07ok").await;
        let a = uuid(w.a);
        w.make_leaf("test_a", w.a, -3).await;
        w.flag(w.a, -3, true).await;
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            Ok("dropped".into())
        );
        audit(&mut tx, a, run, "drop").await.expect("audit");
        tx.commit().await.expect("commit");
        assert_eq!(w.relkind("test_a", &leaf(&a, -3)).await, None);
        let before = w.snapshot().await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(
                &mut tx,
                "test_a",
                Some(a),
                Some(&month(-4)),
                uuid::Uuid::now_v7()
            )
            .await,
            Ok("absent".into())
        );
        tx.commit()
            .await
            .expect("a zero-change run commits with no audit");
        assert_eq!(
            w.snapshot().await,
            before,
            "an absent month changed something"
        );
    })
    .await
}

#[tokio::test]
async fn cat07_relations_that_are_not_the_exact_leaf_are_conflicts() {
    within(180, async {
        let w = maint("c07bad").await;
        let (a, b) = (uuid(w.a), uuid(w.b));
        let a_part = templates::project_partition_name("p", &a);
        // Under the computed names of four eligible months in A's drop window:
        // -3: a partitioned parent; -4: the default partition renamed; -5: B's child renamed;
        // -6: a detached leaf.
        w.make_leaf("test_a", w.b, -5).await;
        w.make_leaf("test_a", w.a, -6).await;
        w.as_owner(&format!(
            "CREATE TABLE test_a.{} (LIKE test_a.p) PARTITION BY RANGE (month); \
             ALTER TABLE test_a.p_default RENAME TO {}; \
             ALTER TABLE test_a.{} RENAME TO {}; \
             ALTER TABLE test_a.{a_part} DETACH PARTITION test_a.{};",
            leaf(&a, -3),
            leaf(&a, -4),
            leaf(&b, -5),
            leaf(&a, -5),
            leaf(&a, -6)
        ))
        .await;
        for o in [-3, -4, -5, -6] {
            w.flag(w.a, o, true).await;
        }
        let before = w.snapshot().await;
        for (o, what) in [
            (-3, "a parent"),
            (-4, "the default partition"),
            (-5, "another project's child"),
            (-6, "a detached table"),
        ] {
            let mut tx = w.begin("test_a", Some(w.a)).await;
            assert_eq!(
                drop(
                    &mut tx,
                    "test_a",
                    Some(a),
                    Some(&month(o)),
                    uuid::Uuid::now_v7()
                )
                .await,
                conflict(),
                "{what}"
            );
            tx.rollback().await.ok();
        }
        assert_eq!(
            w.snapshot().await,
            before,
            "an unrelated object was touched"
        );
    })
    .await
}

#[tokio::test]
async fn cat07_control_without_the_ancestry_check_the_default_partition_is_dropped() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(&base, &SPEC_A, Mutation::AncestryCheckRemoved)
            .expect("the variant: its marker exists");
        let w = maint_with("c07ctl", v).await;
        let a = uuid(w.a);
        w.as_owner(&format!(
            "ALTER TABLE test_a.p_default RENAME TO {}",
            leaf(&a, -4)
        ))
        .await;
        w.flag(w.a, -4, true).await;
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-4)), run).await,
            Ok("dropped".into()),
            "without the ancestry check, the default partition is dropped"
        );
        audit(&mut tx, a, run, "drop").await.expect("audit");
        tx.commit().await.expect("commit");
        assert_eq!(w.relkind("test_a", &leaf(&a, -4)).await, None);
    })
    .await
}

// ---- r2 (vera's F1B-B2-04): bounds and owner -------------------------------------------------

/// A leaf under A's own list partition, under the computed name of `offset`, but spanning two
/// months (`offset` to `offset + 2`): the right name and parent, the wrong bounds.
async fn plant_wrong_bounds(w: &Maint, offset: i32) {
    let a = uuid(w.a);
    w.as_owner(&format!(
        "CREATE TABLE test_a.{} PARTITION OF test_a.{} FOR VALUES FROM ('{}') TO ('{}')",
        leaf(&a, offset),
        templates::project_partition_name("p", &a),
        month(offset),
        month(offset + 2)
    ))
    .await;
}

/// The exact leaf for `offset`, then handed to another owner.
async fn plant_wrong_owner(w: &Maint, offset: i32) {
    w.make_leaf("test_a", w.a, offset).await;
    w.as_admin(&format!(
        "ALTER TABLE test_a.{} OWNER TO cortex_base",
        leaf(&uuid(w.a), offset)
    ))
    .await;
}

#[tokio::test]
async fn cat06_a_same_parent_leaf_with_wrong_bounds_or_owner_is_a_conflict() {
    within(180, async {
        let w = maint("c06bo").await;
        let a = uuid(w.a);
        plant_wrong_bounds(&w, 1).await; // months 1 and 2
        // Healthy partner: this month, exact, created normally.
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(0))).await,
            Ok("created".into())
        );
        tx.commit().await.expect("commit");
        let before = w.snapshot().await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            conflict(),
            "a leaf with the right name and parent but two months' bounds"
        );
        tx.rollback().await.ok();
        assert_eq!(
            w.snapshot().await,
            before,
            "the wrong-bounds leaf or something else changed"
        );

        let w = maint("c06own").await;
        let a = uuid(w.a);
        plant_wrong_owner(&w, 1).await;
        w.make_leaf("test_a", w.a, 0).await; // healthy partner, owned by cortex_owner
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(0))).await,
            Ok("exists".into())
        );
        tx.commit().await.expect("commit");
        let before = w.snapshot().await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            conflict(),
            "the exact leaf, owned by another role"
        );
        tx.rollback().await.ok();
        assert_eq!(w.snapshot().await, before);
    })
    .await
}

#[tokio::test]
async fn cat07_a_same_parent_leaf_with_wrong_bounds_or_owner_is_a_conflict() {
    within(180, async {
        let w = maint("c07bo").await;
        let a = uuid(w.a);
        plant_wrong_bounds(&w, -5).await; // months -5 and -4
        plant_wrong_owner(&w, -3).await;
        w.make_leaf("test_a", w.a, -7).await; // healthy partner
        for o in [-7, -5, -3] {
            w.flag(w.a, o, true).await;
        }
        let before = w.snapshot().await;
        for (o, what) in [(-5, "wrong bounds"), (-3, "wrong owner")] {
            let mut tx = w.begin("test_a", Some(w.a)).await;
            assert_eq!(
                drop(
                    &mut tx,
                    "test_a",
                    Some(a),
                    Some(&month(o)),
                    uuid::Uuid::now_v7()
                )
                .await,
                conflict(),
                "{what}: never a later permission error, never a drop"
            );
            tx.rollback().await.ok();
        }
        assert_eq!(w.snapshot().await, before, "a refusal changed something");
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-7)), run).await,
            Ok("dropped".into()),
            "the healthy partner"
        );
        audit(&mut tx, a, run, "drop").await.expect("audit");
        tx.commit().await.expect("commit");
    })
    .await
}

#[tokio::test]
async fn cat06_control_without_the_bounds_or_owner_check_the_planted_leaf_passes_as_exists() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        for (tag, m) in [
            ("c06ctl_bounds", Mutation::BoundsCheckRemoved),
            ("c06ctl_owner", Mutation::OwnerCheckRemoved),
        ] {
            let v = variant(&base, &SPEC_A, m).expect("the variant: its marker exists");
            let w = maint_with(tag, v).await;
            if m == Mutation::BoundsCheckRemoved {
                plant_wrong_bounds(&w, 1).await;
            } else {
                plant_wrong_owner(&w, 1).await;
            }
            let mut tx = w.begin("test_a", Some(w.a)).await;
            assert_eq!(
                create(&mut tx, "test_a", Some(uuid(w.a)), Some(&month(1))).await,
                Ok("exists".into()),
                "{m:?}: the planted leaf was not taken for the expected one"
            );
            tx.rollback().await.ok();
        }
    })
    .await
}

// ---- r2 (vera's F1B-B2-04): the validation locks, coordinated inside the function ------------

/// The pid waiting at the pause gate (the two-int advisory lock), polled for up to 10 s.
async fn waiter_at_gate(w: &Maint) -> i32 {
    let mut admin = w.db.admin().await;
    for _ in 0..200 {
        let pid: Option<i32> = sqlx::query_scalar(
            "select pid from pg_locks where locktype = 'advisory' and classid = $1::oid \
             and objid = $2::oid and objsubid = 2 and not granted limit 1",
        )
        .bind(PAUSE_CLASS)
        .bind(PAUSE_KEY)
        .fetch_optional(&mut admin)
        .await
        .expect("pg_locks");
        if let Some(pid) = pid {
            return pid;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("INVALID: T1 never reached the gate after validation; nothing was measured")
}

/// Holds the gate: a superuser transaction holding the pause lock until it commits.
async fn hold_gate(w: &Maint) -> sqlx::PgConnection {
    let mut gate = w.db.admin().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "BEGIN; SELECT pg_advisory_xact_lock({PAUSE_CLASS}, {PAUSE_KEY});"
    )))
    .execute(&mut gate)
    .await
    .expect("the gate");
    gate
}

/// Starts T2, the owner, renaming the leaf; returns its pid and its task.
async fn competitor(
    w: &Maint,
    name: String,
) -> (i32, tokio::task::JoinHandle<Result<(), Option<String>>>) {
    let mut t2 = w.db.owner().await;
    let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut t2)
        .await
        .expect("pid");
    let task = tokio::spawn(async move {
        let r = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE test_a.{name} RENAME TO replaced"
        )))
        .execute(&mut t2)
        .await;
        t2.close().await.ok();
        r.map(|_| ()).map_err(|e| sqlstate(&e))
    });
    (pid, task)
}

#[tokio::test]
async fn cat07_the_competitor_waits_on_the_validation_locks_then_sees_the_drop() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(&base, &SPEC_A, Mutation::PauseAfterValidation)
            .expect("the gate variant: its hook exists");
        let w = maint_with("c07race", v).await;
        let a = uuid(w.a);
        let name = leaf(&a, -3);
        w.make_leaf("test_a", w.a, -3).await;
        w.flag(w.a, -3, true).await;
        let mut admin = w.db.admin().await;
        let (leaf_oid, root_oid): (i64, i64) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select 'test_a.{name}'::regclass::oid::bigint, 'test_a.p'::regclass::oid::bigint"
        )))
        .fetch_one(&mut admin)
        .await
        .expect("oids");
        let gate = hold_gate(&w).await;
        let run = uuid::Uuid::now_v7();
        let mut t1 = w.begin("test_a", Some(w.a)).await;
        let t1_task = tokio::spawn(async move {
            let r = drop(&mut t1, "test_a", Some(a), Some(&month(-3)), run).await;
            (t1, r)
        });
        // Phase witness: T1 has validated, holds its locks, and waits at the gate.
        let t1_pid = waiter_at_gate(&w).await;
        let (t2_pid, t2_task) = competitor(&w, name.clone()).await;
        // The competitor's wait, bound to T1 and to the target relation.
        let mut bound = false;
        for _ in 0..200 {
            let (blockers, rels): (Vec<i32>, Vec<i64>) = sqlx::query_as(
                "select pg_blocking_pids($1), \
                 coalesce(array_agg(relation::bigint) filter (where relation is not null), '{}') \
                 from pg_locks where pid = $1 and not granted",
            )
            .bind(t2_pid)
            .fetch_one(&mut admin)
            .await
            .expect("the competitor's wait");
            if blockers.contains(&t1_pid) && rels.iter().any(|r| *r == leaf_oid || *r == root_oid) {
                bound = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            bound,
            "the competitor did not wait on T1's validation locks on the leaf or root"
        );
        // Release the gate; T1 drops and commits; then T2 sees its own outcome.
        gate.close().await.ok();
        let (mut t1, r) = t1_task.await.expect("T1");
        assert_eq!(r, Ok("dropped".into()));
        audit(&mut t1, a, run, "drop").await.expect("audit");
        t1.commit().await.expect("T1 commits");
        assert_eq!(
            t2_task.await.expect("T2"),
            Err(Some("42P01".into())),
            "after the wait, T2 sees the leaf gone (its own error)"
        );
        assert_eq!(w.relkind("test_a", &name).await, None);
        assert_eq!(w.relkind("test_a", "replaced").await, None);
    })
    .await
}

#[tokio::test]
async fn cat07_control_without_the_validation_locks_the_replacement_lands_while_t1_waits() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variants(
            &base,
            &SPEC_A,
            &[Mutation::PauseAfterValidation, Mutation::LockSkipped],
        )
        .expect("the variant: its hook and marker exist");
        let w = maint_with("c07ctl_lock", v).await;
        let a = uuid(w.a);
        let name = leaf(&a, -3);
        w.make_leaf("test_a", w.a, -3).await;
        w.flag(w.a, -3, true).await;
        let gate = hold_gate(&w).await;
        let mut t1 = w.begin("test_a", Some(w.a)).await;
        let t1_task = tokio::spawn(async move {
            let r = drop(
                &mut t1,
                "test_a",
                Some(a),
                Some(&month(-3)),
                uuid::Uuid::now_v7(),
            )
            .await;
            t1.rollback().await.ok();
            r
        });
        waiter_at_gate(&w).await;
        let (_, t2_task) = competitor(&w, name.clone()).await;
        let t2 = tokio::time::timeout(Duration::from_secs(10), t2_task).await;
        assert!(
            matches!(t2, Ok(Ok(Ok(())))),
            "without the validation locks, the replacement should land while T1 waits: {t2:?}"
        );
        assert_eq!(w.relkind("test_a", "replaced").await.as_deref(), Some("r"));
        gate.close().await.ok();
        let _ = t1_task.await;
    })
    .await
}

/// nemo's M-1 confirmation r1 (3): the project's LIST parent is created first, by the module's own
/// reviewed parent-creation function (an M-1 obligation), never by this template. So for an open
/// project with no list parent, `create_partition` refuses with `partition_conflict`, creates
/// nothing (no parent, no leaf), and never ATTACHes.
#[tokio::test]
async fn cat06_an_open_project_without_its_list_parent_is_a_conflict_and_nothing_is_created() {
    within(180, async {
        let w = maint("c06noparent").await;
        let c = cortex_types::ProjectId::new(&*w.clock);
        let cu = uuid(c);
        w.as_owner(&format!(
            "INSERT INTO identity_pub.projects (id, state) VALUES ('{cu}', 'open')"
        ))
        .await;
        let before = w.snapshot().await;
        let mut tx = w.begin("test_a", Some(c)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(cu), Some(&month(1))).await,
            conflict(),
            "the template must not create, or work around, a missing project parent"
        );
        tx.rollback().await.ok();
        assert_eq!(w.snapshot().await, before, "something was created");
        assert_eq!(
            w.relkind("test_a", &templates::project_partition_name("p", &cu))
                .await,
            None
        );
    })
    .await
}
