//! F-1b rework 7, fix B (kai's row 690c50a0 and ruling T-2; sam's SAM-R2-01, combined by vera).
//! The maintenance functions resolve the project's list partition by its NAME, partitioned kind
//! and root edge, never by what it routes. A same-schema parent carrying project A's generated
//! name but routing LIST value B, or a partitioned DEFAULT parent with that name, let A's create
//! make a leaf under it and A's drop remove one, with another project's rows in it (the author
//! reproduced both on PostgreSQL 18.4, runs-rw7). Each wrong parent is planted by the owner (a
//! trusted-provisioning mistake), in a world of its own. Under the parent's lock, the parent must
//! now be a non-default partition of the root routing exactly this project, itself partitioned
//! by RANGE on one `date` column; anything else is a `partition_conflict` before any DDL: the
//! snapshot (catalogue and data) is unchanged, and the other project's row survives.
//!
//! The control: the true parent creates, re-runs as `exists`, and drops with a valid audit, and
//! project B's objects survive. The existing leaf-bound, default, owner, renamed and detached
//! controls stay F1-31's (`f1_31_identity.rs`), unchanged.
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

/// Plants `plant` (as the owner), then a past month's leaf under the parent now named for A,
/// holding a row of `other`; flags that month eligible for A.
async fn planted(
    tag: &str,
    plant: impl Fn(&str, &str) -> String,
    other: Option<uuid::Uuid>,
) -> (Maint, uuid::Uuid) {
    let w = maint(tag).await;
    let (a, b) = (uuid(w.a), uuid(w.b));
    let pa = templates::project_partition_name("p", &a);
    let pb = templates::project_partition_name("p", &b);
    w.as_owner(&plant(&pa, &pb)).await;
    let other = other.unwrap_or(b);
    w.make_leaf("test_a", w.a, -3).await;
    w.as_admin(&format!(
        "INSERT INTO test_a.p VALUES ('{other}', '{}', 1, 'not-a')",
        month(-3)
    ))
    .await;
    w.flag(w.a, -3, true).await;
    (w, other)
}

/// A's create and A's drop are both conflicts, and change nothing.
async fn both_conflict(w: &Maint, other: uuid::Uuid) {
    let a = uuid(w.a);
    assert_eq!(
        rows_of(w, other).await,
        1,
        "premise: the other project's row"
    );
    let before = w.snapshot().await;
    let mut tx = w.begin("test_a", Some(w.a)).await;
    assert_eq!(
        create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
        conflict(),
        "A's create under a parent that is not A's"
    );
    tx.rollback().await.ok();
    assert_eq!(w.snapshot().await, before, "the create changed something");
    let run = uuid::Uuid::now_v7();
    let mut tx = w.begin("test_a", Some(w.a)).await;
    assert_eq!(
        drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
        conflict(),
        "A's drop under a parent that is not A's"
    );
    tx.rollback().await.ok();
    assert_eq!(w.snapshot().await, before, "the drop changed something");
    assert_eq!(
        rows_of(w, other).await,
        1,
        "the other project's row survives"
    );
    assert_eq!(w.runs("test_a", run).await, 0, "no run row");
}

#[tokio::test]
async fn rw7_a_parent_named_for_a_but_routing_b_is_a_conflict_before_any_ddl() {
    within(180, async {
        let (w, other) = planted(
            "rw7_list_b",
            |pa, pb| {
                format!(
                    "ALTER TABLE test_a.p DETACH PARTITION test_a.{pa}; \
                     ALTER TABLE test_a.{pa} RENAME TO {pa}_real; \
                     ALTER TABLE test_a.{pb} RENAME TO {pa};"
                )
            },
            None,
        )
        .await;
        both_conflict(&w, other).await;
    })
    .await
}

#[tokio::test]
async fn rw7_a_partitioned_default_parent_named_for_a_is_a_conflict_before_any_ddl() {
    within(180, async {
        let (w, other) = planted(
            "rw7_default",
            |pa, _| {
                format!(
                    "ALTER TABLE test_a.p DETACH PARTITION test_a.p_default; \
                     ALTER TABLE test_a.p DETACH PARTITION test_a.{pa}; \
                     ALTER TABLE test_a.{pa} RENAME TO {pa}_real; \
                     CREATE TABLE test_a.{pa} PARTITION OF test_a.p DEFAULT PARTITION BY RANGE (month);"
                )
            },
            Some(uuid::Uuid::now_v7()),
        )
        .await;
        both_conflict(&w, other).await;
    })
    .await
}

/// A's own bound, but not the expected sub-partitioning: LIST on the month, or RANGE on a
/// column that is not a `date`. A conflict before any DDL, not an SQL error from the CREATE.
#[tokio::test]
async fn rw7_a_project_parent_with_the_wrong_range_strategy_or_key_is_a_conflict() {
    within(240, async {
        for (tag, by) in [("rw7_by_list", "LIST (month)"), ("rw7_by_id", "RANGE (id)")] {
            let w = maint(tag).await;
            let a = uuid(w.a);
            let pa = templates::project_partition_name("p", &a);
            w.as_owner(&format!(
                "ALTER TABLE test_a.p DETACH PARTITION test_a.{pa}; \
                 ALTER TABLE test_a.{pa} RENAME TO {pa}_real; \
                 CREATE TABLE test_a.{pa} PARTITION OF test_a.p FOR VALUES IN ('{a}') PARTITION BY {by};"
            ))
            .await;
            let before = w.snapshot().await;
            let mut tx = w.begin("test_a", Some(w.a)).await;
            assert_eq!(
                create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
                conflict(),
                "{tag}: A's create under a parent partitioned BY {by}"
            );
            tx.rollback().await.ok();
            assert_eq!(w.snapshot().await, before, "{tag}: something changed");
        }
    })
    .await
}

/// The control: the true parent creates, re-runs as `exists`, and drops with a valid audit; B's
/// partition, its leaf and its row survive.
#[tokio::test]
async fn rw7_control_the_true_project_parent_creates_and_drops_and_b_survives() {
    within(180, async {
        let w = maint("rw7_true_parent").await;
        let (a, b) = (uuid(w.a), uuid(w.b));
        w.make_leaf("test_a", w.b, -3).await;
        w.as_admin(&format!(
            "INSERT INTO test_a.p VALUES ('{b}', '{}', 1, 'b-row')",
            month(-3)
        ))
        .await;
        for want in ["created", "exists"] {
            let mut tx = w.begin("test_a", Some(w.a)).await;
            assert_eq!(
                create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
                Ok(want.into())
            );
            tx.commit().await.expect("commit");
        }
        w.make_leaf("test_a", w.a, -3).await;
        w.flag(w.a, -3, true).await;
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
        assert_eq!(
            w.relkind("test_a", &leaf(&b, -3)).await,
            Some("r".into()),
            "B's leaf survives"
        );
        assert_eq!(
            w.relkind("test_a", &templates::project_partition_name("p", &b))
                .await,
            Some("p".into()),
            "B's partition survives"
        );
        assert_eq!(rows_of(&w, b).await, 1, "B's row survives");
    })
    .await
}
