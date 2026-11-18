//! F1-31 categories 1 and 2 (TESTS r7; A7 r4 item 2 steps 1 and 2). Every case has its own
//! database; every refusal leaves the catalogue and the data unchanged (`Snapshot`). r2 (vera's
//! F1B-B2-01): a refusal is compared whole (`refused`): the SQLSTATE, the message exactly the
//! code, no DETAIL, no HINT. The two message controls show that comparison fails on a wrong
//! prefix and on a message carrying row data, with the SQLSTATE kept.
//! RED (b2): the template, the startup check and the b1 core are `todo!()`.
use cortex_db::maintenance::MaintError;
use cortex_db::maintenance::templates;
use cortex_testkit::maint::{
    Mutation, Refusal, SPEC_A, add_project, audit, create, drop, exec, leaf, maint, maint_with,
    month, refused, uuid, variant,
};
use cortex_testkit::within;
use cortex_types::ProjectId;

fn not_authorised() -> Result<String, Refusal> {
    Err(refused(MaintError::NotAuthorised))
}

#[tokio::test]
async fn cat01_a_transaction_creates_its_own_projects_month() {
    within(180, async {
        let w = maint("c01ok").await;
        let a = uuid(w.a);
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            Ok("created".into())
        );
        tx.commit().await.expect("commit");
        assert_eq!(
            w.relkind("test_a", &leaf(&a, 1)).await.as_deref(),
            Some("r")
        );
        assert_eq!(
            w.parent_of("test_a", &leaf(&a, 1)).await,
            Some(format!(
                "test_a.{}",
                templates::project_partition_name("p", &a)
            ))
        );
    })
    .await
}

#[tokio::test]
async fn cat01_another_unknown_or_closed_project_or_no_context_is_not_authorised() {
    within(180, async {
        let w = maint("c01no").await;
        let closed = ProjectId::new(&*w.clock);
        add_project(&mut w.db.owner().await, closed, "closed").await;
        let ghost = ProjectId::new(&*w.clock); // never added: it does not exist
        let before = w.snapshot().await;
        let cases = [
            (
                "B's project from A's transaction",
                Some(uuid(w.b)),
                Some(w.a),
            ),
            (
                "a nonexistent UUID from A's transaction",
                Some(uuid(ghost)),
                Some(w.a),
            ),
            ("no project context", Some(uuid(w.a)), None),
            (
                "a nonexistent project as the transaction's own",
                Some(uuid(ghost)),
                Some(ghost),
            ),
            (
                "a closed project as the transaction's own",
                Some(uuid(closed)),
                Some(closed),
            ),
        ];
        for (name, target, context) in cases {
            let mut tx = w.begin("test_a", context).await;
            assert_eq!(
                create(&mut tx, "test_a", target, Some(&month(1))).await,
                not_authorised(),
                "{name}"
            );
            tx.rollback().await.ok();
        }
        // An empty project context inside A's transaction.
        let mut tx = w.begin("test_a", Some(w.a)).await;
        exec(&mut tx, "select set_config('cortex.project_id', '', true)")
            .await
            .expect("empty the context");
        assert_eq!(
            create(&mut tx, "test_a", Some(uuid(w.a)), Some(&month(1))).await,
            not_authorised(),
            "an empty project context"
        );
        tx.rollback().await.ok();
        assert_eq!(
            w.snapshot().await,
            before,
            "a refusal changed the catalogue or the data"
        );
    })
    .await
}

#[tokio::test]
async fn cat01_control_without_the_project_binding_the_b_call_succeeds() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(&base, &SPEC_A, Mutation::ProjectBindingRemoved)
            .expect("the variant: its marker exists");
        let w = maint_with("c01ctl", v).await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(uuid(w.b)), Some(&month(1))).await,
            Ok("created".into()),
            "without the binding, A's transaction creates B's month"
        );
        tx.commit().await.expect("commit");
    })
    .await
}

#[tokio::test]
async fn cat02_each_window_end_is_accepted_for_create_and_drop() {
    within(180, async {
        let w = maint("c02ok").await;
        let a = uuid(w.a);
        for (schema, ends) in [("test_a", [0, 2]), ("test_b", [0, 1])] {
            for o in ends {
                let mut tx = w.begin(schema, Some(w.a)).await;
                assert_eq!(
                    create(&mut tx, schema, Some(a), Some(&month(o))).await,
                    Ok("created".into()),
                    "{schema} create at {}",
                    month(o)
                );
                tx.commit().await.expect("commit");
            }
        }
        // r2 (nemo's M-1 confirmation r1): test_a's drop window is records' (-inf, -1]: its upper
        // end, and a leaf fifty years old, which must never age out of it. test_b's is finite.
        for (schema, ends) in [("test_a", [-1, -600]), ("test_b", [-12, -6])] {
            for o in ends {
                w.make_leaf(schema, w.a, o).await;
                if schema == "test_a" {
                    w.flag(w.a, o, true).await;
                }
                let run = uuid::Uuid::now_v7();
                let mut tx = w.begin(schema, Some(w.a)).await;
                assert_eq!(
                    drop(&mut tx, schema, Some(a), Some(&month(o)), run).await,
                    Ok("dropped".into()),
                    "{schema} drop at {}",
                    month(o)
                );
                audit(&mut tx, a, run, "drop").await.expect("audit");
                tx.commit().await.expect("commit");
                assert_eq!(w.relkind(schema, &leaf(&a, o)).await, None);
            }
        }
        // The far end of the unbounded past: year 1 is inside test_a's drop envelope (no lower
        // limit, no overflow), so it is not a bad value; with no leaf there it is `absent`.
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(
                &mut tx,
                "test_a",
                Some(a),
                Some("0001-01-01"),
                uuid::Uuid::now_v7()
            )
            .await,
            Ok("absent".into()),
            "year 1 is inside an unbounded-past drop window"
        );
        tx.commit().await.expect("commit");
    })
    .await
}

#[tokio::test]
async fn cat02_bad_values_are_refused_with_nothing_changed() {
    within(180, async {
        let w = maint("c02bad").await;
        let a = Some(uuid(w.a));
        // Leaves just outside each drop window exist and are eligible, so only the value check
        // can refuse them. r2: test_a's window has no lower end, so only its upper side (M and
        // M+1) is outside.
        for (schema, o) in [
            ("test_a", 0),
            ("test_a", 1),
            ("test_b", -13),
            ("test_b", -5),
        ] {
            w.make_leaf(schema, w.a, o).await;
            if schema == "test_a" {
                w.flag(w.a, o, true).await;
            }
        }
        let before = w.snapshot().await;
        let shared: Vec<(Option<uuid::Uuid>, Option<String>)> = vec![
            (None, Some(month(1))),
            (a, None),
            (a, Some("infinity".into())),
            (a, Some("-infinity".into())),
            (a, Some("0001-01-01".into())),
            (a, Some("9999-12-01".into())),
            (a, Some("2026-12-15".into())),
        ];
        let mut creates: Vec<(&str, Option<uuid::Uuid>, Option<String>)> = Vec::new();
        let mut drops = creates.clone();
        for schema in ["test_a", "test_b"] {
            for (p, m) in &shared {
                creates.push((schema, *p, m.clone()));
                // r2: year 1 is inside test_a's unbounded-past drop window (the positive test
                // above), so it is a bad value only for test_b's finite one.
                if !(schema == "test_a" && m.as_deref() == Some("0001-01-01")) {
                    drops.push((schema, *p, m.clone()));
                }
            }
        }
        // One month outside each end of each window.
        creates.extend([
            ("test_a", a, Some(month(-1))),
            ("test_a", a, Some(month(3))),
            ("test_b", a, Some(month(-1))),
            ("test_b", a, Some(month(2))),
        ]);
        drops.extend([
            ("test_a", a, Some(month(0))),
            ("test_a", a, Some(month(1))),
            ("test_b", a, Some(month(-13))),
            ("test_b", a, Some(month(-5))),
        ]);
        for (schema, p, m) in creates {
            let mut tx = w.begin(schema, Some(w.a)).await;
            assert_eq!(
                create(&mut tx, schema, p, m.as_deref()).await,
                Err(refused(MaintError::BadValue)),
                "{schema} create {p:?} {m:?}"
            );
            tx.rollback().await.ok();
        }
        for (schema, p, m) in drops {
            let mut tx = w.begin(schema, Some(w.a)).await;
            assert_eq!(
                drop(&mut tx, schema, p, m.as_deref(), uuid::Uuid::now_v7()).await,
                Err(refused(MaintError::BadValue)),
                "{schema} drop {p:?} {m:?}"
            );
            tx.rollback().await.ok();
        }
        assert_eq!(
            w.snapshot().await,
            before,
            "a refusal changed the catalogue or the data"
        );
    })
    .await
}

#[tokio::test]
async fn cat02_control_without_the_window_check_an_out_of_window_month_succeeds() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(&base, &SPEC_A, Mutation::WindowCheckRemoved)
            .expect("the variant: its marker exists");
        let w = maint_with("c02ctl", v).await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(uuid(w.a)), Some(&month(3))).await,
            Ok("created".into()),
            "without the window check, three months ahead is created"
        );
        tx.commit().await.expect("commit");
    })
    .await
}

/// vera's F1B-B2-01, the oracle's own controls: with the SQLSTATE kept, a wrong message prefix and
/// a message carrying row data (the project id) each fail the exact comparison every refusal
/// test uses, so neither could pass as a correct refusal.
#[tokio::test]
async fn cat02_control_a_wrong_prefix_or_leaking_message_fails_the_exact_refusal() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        for (tag, m) in [
            ("c02msg_prefix", Mutation::MessageWrongPrefix),
            ("c02msg_leak", Mutation::MessageLeaksRow),
        ] {
            let v = variant(&base, &SPEC_A, m).expect("the variant: its markers exist");
            let w = maint_with(tag, v).await;
            let a = uuid(w.a);
            let mut tx = w.begin("test_a", Some(w.a)).await;
            let r = create(&mut tx, "test_a", Some(a), Some("2026-12-15")).await;
            tx.rollback().await.ok();
            assert!(
                matches!(&r, Err(Refusal::Maint { code: MaintError::BadValue, .. })),
                "{m:?}: the SQLSTATE must be kept: {r:?}"
            );
            assert_ne!(
                r,
                Err(refused(MaintError::BadValue)),
                "{m:?}: the exact comparison did not see the changed message"
            );
            if m == Mutation::MessageLeaksRow {
                assert!(
                    matches!(&r, Err(Refusal::Maint { message, .. }) if message.contains(&a.to_string())),
                    "the leaking variant did not carry the project id: {r:?}"
                );
            }
        }
    })
    .await
}
