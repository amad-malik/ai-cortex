//! F-1b rework 9, item 2 (kai's row 61c6f360 and T-2 Addendum 3: the partition spec names its
//! LIST key column). The parent check compares the parent's bound TEXT, FOR VALUES IN
//! ('<project>'); that means "this project" only if the root is LIST-partitioned by the project
//! column. A root listed by another column, with a parent bound to the project's text, passed it.
//! The root's partition key must now be exactly the spec's `list_column`, one column.
//!
//! Each world is the owner's: the fixture's root is renamed away and replaced by one listed by
//! `listed_by` (with the fixture's row security and grant), with A's and B's parents bound to
//! their projects' text and ranged by month. Listed by `owner_id`: a conflict; listed by
//! `project_id` (the spec's column): the paired control.
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

async fn world(tag: &str, listed_by: &str) -> Maint {
    let w = maint(tag).await;
    let (a, b) = (uuid(w.a), uuid(w.b));
    let pa = templates::project_partition_name("p", &a);
    let pb = templates::project_partition_name("p", &b);
    let policy = "project_id = nullif(current_setting('cortex.project_id', true), '')::uuid";
    w.as_owner(&format!(
        "ALTER TABLE test_a.p RENAME TO p_orig; \
         ALTER TABLE test_a.{pa} RENAME TO {pa}_orig; \
         ALTER TABLE test_a.{pb} RENAME TO {pb}_orig; \
         CREATE TABLE test_a.p (project_id uuid NOT NULL, month date NOT NULL, id bigint NOT NULL, \
           v text NOT NULL, owner_id uuid) PARTITION BY LIST ({listed_by}); \
         ALTER TABLE test_a.p ENABLE ROW LEVEL SECURITY; ALTER TABLE test_a.p FORCE ROW LEVEL SECURITY; \
         CREATE POLICY by_project ON test_a.p USING ({policy}) WITH CHECK ({policy}); \
         GRANT SELECT, INSERT ON test_a.p TO cortex_m_test_a; \
         CREATE TABLE test_a.{pa} PARTITION OF test_a.p FOR VALUES IN ('{a}') PARTITION BY RANGE (month); \
         CREATE TABLE test_a.{pb} PARTITION OF test_a.p FOR VALUES IN ('{b}') PARTITION BY RANGE (month);"
    ))
    .await;
    w.make_leaf("test_a", w.a, -3).await;
    w.flag(w.a, -3, true).await;
    w.make_leaf("test_a", w.b, -3).await;
    w.as_admin(&format!(
        "INSERT INTO test_a.p (project_id, month, id, v, owner_id) VALUES ('{b}', '{}', 1, 'b-row', '{b}')",
        month(-3)
    ))
    .await;
    w
}

#[tokio::test]
async fn rw9_a_root_listed_by_another_column_is_a_conflict() {
    within(180, async {
        let w = world("rw9_listed_by_owner", "owner_id").await;
        let (a, b) = (uuid(w.a), uuid(w.b));
        let before = w.snapshot().await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            conflict(),
            "A's create under a root listed by owner_id"
        );
        tx.rollback().await.ok();
        assert_eq!(w.snapshot().await, before, "the create changed something");
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            conflict(),
            "A's drop under a root listed by owner_id"
        );
        tx.rollback().await.ok();
        assert_eq!(w.snapshot().await, before, "the drop changed something");
        assert_eq!(w.runs("test_a", run).await, 0, "no run row");
        assert_eq!(rows_of(&w, b).await, 1, "B's row survives");
    })
    .await
}

/// The paired control: the same world listed by `project_id`, the spec's column: create, re-run
/// as `exists`, drop with a valid audit; B survives.
#[tokio::test]
async fn rw9_control_the_same_root_listed_by_the_project_column_creates_and_drops() {
    within(180, async {
        let w = world("rw9_listed_by_project", "project_id").await;
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
