//! F1-31 categories 8 and 9, template level (TESTS r7; A7 r4 item 2 step 4). `drop_eligible` is
//! evaluated under the lock; false refuses with `partition_not_eligible`. Each module applies its
//! own predicate. The records rules (non-empty, pinned, a failed sink batch) are M-1's
//! consumer-owned cases, and the events rule is M-2's; they are not here.
//! RED (b2): the template and the b1 core are `todo!()`.
use cortex_db::maintenance::MaintError;
use cortex_db::maintenance::templates;
use cortex_testkit::maint::{
    Mutation, SPEC_A, audit, drop, leaf, maint, maint_with, month, refused, uuid, variant,
};
use cortex_testkit::within;

#[tokio::test]
async fn cat08_drop_eligible_true_lets_the_drop_run() {
    within(180, async {
        let w = maint("c08ok").await;
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
    })
    .await
}

#[tokio::test]
async fn cat08_drop_eligible_false_is_not_eligible_and_the_partition_survives() {
    within(180, async {
        let w = maint("c08no").await;
        let a = uuid(w.a);
        w.make_leaf("test_a", w.a, -3).await;
        w.make_leaf("test_a", w.a, -4).await;
        w.flag(w.a, -3, false).await; // -4 has no flag at all: also false
        let before = w.snapshot().await;
        for o in [-3, -4] {
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
                Err(refused(MaintError::NotEligible)),
                "month {}",
                month(o)
            );
            tx.rollback().await.ok();
        }
        assert_eq!(w.snapshot().await, before, "a refusal changed something");
    })
    .await
}

#[tokio::test]
async fn cat08_control_without_the_predicate_call_it_drops_anyway() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(&base, &SPEC_A, Mutation::PredicateCallRemoved)
            .expect("the variant: its marker exists");
        let w = maint_with("c08ctl", v).await;
        let a = uuid(w.a);
        w.make_leaf("test_a", w.a, -3).await;
        w.flag(w.a, -3, false).await;
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            Ok("dropped".into()),
            "without the predicate call, an ineligible month is dropped"
        );
        audit(&mut tx, a, run, "drop").await.expect("audit");
        tx.commit().await.expect("commit");
    })
    .await
}

#[tokio::test]
async fn cat09_each_module_applies_its_own_predicate() {
    within(180, async {
        let w = maint("c09").await;
        let a = uuid(w.a);
        // Seven months back is in both drop windows. test_b's rule (six months old) allows it;
        // test_a's rule (a flag, absent here) does not.
        w.make_leaf("test_a", w.a, -7).await;
        w.make_leaf("test_b", w.a, -7).await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(
                &mut tx,
                "test_a",
                Some(a),
                Some(&month(-7)),
                uuid::Uuid::now_v7()
            )
            .await,
            Err(refused(MaintError::NotEligible)),
            "test_b's rule was applied to test_a"
        );
        tx.rollback().await.ok();
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_b", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_b", Some(a), Some(&month(-7)), run).await,
            Ok("dropped".into()),
            "test_b's own rule"
        );
        audit(&mut tx, a, run, "drop").await.expect("audit");
        tx.commit().await.expect("commit");
        assert_eq!(
            w.relkind("test_a", &leaf(&a, -7)).await.as_deref(),
            Some("r")
        );
        assert_eq!(w.relkind("test_b", &leaf(&a, -7)).await, None);
    })
    .await
}
