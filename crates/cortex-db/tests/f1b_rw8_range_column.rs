//! F-1b rework 8 (kai's row 5e863984; the T-2 addendum: the partition spec names its range key
//! column). Rework 7's `parent` check pins RANGE, one column and the `date` type, not the
//! column's NAME: a project parent with A's exact LIST bound, ranged on ANOTHER date column of the
//! table, passes it. The parent must now be ranged on exactly the spec's `range_column`.
//!
//! Each world is the owner's: the root gains a second date column, `at`, and A's partition is
//! replaced by one with A's exact bound, ranged on `at` (the negative) or on `month` (the paired
//! control, which is the spec's column). B gets a leaf and a row of its own in both.
use cortex_db::maintenance::MaintError;
use cortex_db::maintenance::templates;
use cortex_testkit::maint::{
    Maint, Refusal, audit, create, drop, leaf, maint, month, refused, uuid,
};
use cortex_testkit::within;

fn conflict() -> Result<String, Refusal> {
    Err(refused(MaintError::Conflict))
}

async fn rows_of(w: &Maint, project: uuid::Uuid) -> i64 {
    let mut admin = w.db.admin().await;
    sqlx::query_scalar("select count(*) from test_a.p where project_id = $1")
        .bind(project)
        .fetch_one(&mut admin)
        .await
        .expect("rows")
}

/// The root gains `at date`; A's partition is replaced by one bound exactly to A and ranged on
/// `ranged_on`; a past month's leaf under it (A's, made by the owner) is flagged eligible; B gets
/// a leaf and a row.
async fn world(tag: &str, ranged_on: &str) -> Maint {
    let w = maint(tag).await;
    let (a, b) = (uuid(w.a), uuid(w.b));
    let pa = templates::project_partition_name("p", &a);
    w.as_owner(&format!(
        "ALTER TABLE test_a.p ADD COLUMN at date; \
         ALTER TABLE test_a.p DETACH PARTITION test_a.{pa}; \
         ALTER TABLE test_a.{pa} RENAME TO {pa}_real; \
         CREATE TABLE test_a.{pa} PARTITION OF test_a.p FOR VALUES IN ('{a}') PARTITION BY RANGE ({ranged_on});"
    ))
    .await;
    w.make_leaf("test_a", w.a, -3).await;
    w.flag(w.a, -3, true).await;
    w.make_leaf("test_a", w.b, -3).await;
    w.as_admin(&format!(
        "INSERT INTO test_a.p (project_id, month, id, v, at) VALUES ('{b}', '{m}', 1, 'b-row', '{m}')",
        m = month(-3)
    ))
    .await;
    w
}

#[tokio::test]
async fn rw8_a_project_parent_ranged_on_another_date_column_is_a_conflict() {
    within(180, async {
        let w = world("rw8_ranged_on_at", "at").await;
        let (a, b) = (uuid(w.a), uuid(w.b));
        let before = w.snapshot().await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            conflict(),
            "A's create under a parent ranged on `at`"
        );
        tx.rollback().await.ok();
        assert_eq!(w.snapshot().await, before, "the create changed something");
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            conflict(),
            "A's drop under a parent ranged on `at`"
        );
        tx.rollback().await.ok();
        assert_eq!(w.snapshot().await, before, "the drop changed something");
        assert_eq!(w.runs("test_a", run).await, 0, "no run row");
        assert_eq!(rows_of(&w, b).await, 1, "B's row survives");
    })
    .await
}

/// The paired control: the same world ranged on `month`, the spec's column: create, re-run as
/// `exists`, drop with a valid audit; B survives.
#[tokio::test]
async fn rw8_control_the_same_world_ranged_on_the_spec_column_creates_and_drops() {
    within(180, async {
        let w = world("rw8_ranged_on_month", "month").await;
        let (a, b) = (uuid(w.a), uuid(w.b));
        for want in ["created", "exists"] {
            let mut tx = w.begin("test_a", Some(w.a)).await;
            assert_eq!(
                create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
                Ok(want.into())
            );
            tx.commit().await.expect("commit");
        }
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            Ok("dropped".into())
        );
        audit(&mut tx, a, run, "drop").await.expect("audit");
        tx.commit().await.expect("commit with the audit");
        assert_eq!(w.runs("test_a", run).await, 1, "the run row");
        assert_eq!(w.audits(run).await, 1, "the audit row");
        assert_eq!(
            w.relkind("test_a", &leaf(&a, -3)).await,
            None,
            "A's leaf dropped"
        );
        assert_eq!(rows_of(&w, b).await, 1, "B's row survives");
    })
    .await
}
