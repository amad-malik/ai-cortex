//! mike's F1B-R0-07: the F1-31 "nothing changed" witness sees a count-preserving change of the
//! data and a change of a relation's owner, so a refusal or a successful no-op (`exists`) that
//! changed values could not pass as unchanged. It needs no `cortex-db` behaviour (only the
//! testkit and raw SQL), so it can pass at RED; it is a guard, not a RED.
use cortex_testkit::maint::snapshot_of;
use cortex_testkit::{isolated_db, within};

#[tokio::test]
async fn snapshot_sees_a_same_count_value_change_and_an_owner_change() {
    within(120, async {
        let db = isolated_db("snapguard").await;
        let mut admin = db.admin().await;
        sqlx::raw_sql("INSERT INTO test_a.t (project_id, v) VALUES (gen_random_uuid(), 'before')")
            .execute(&mut admin)
            .await
            .expect("seed");
        let first = snapshot_of(&db).await;
        assert_eq!(snapshot_of(&db).await, first, "the witness is not stable");
        sqlx::raw_sql("UPDATE test_a.t SET v = 'after'")
            .execute(&mut admin)
            .await
            .expect("same-count update");
        let second = snapshot_of(&db).await;
        assert_eq!(first.relations, second.relations);
        assert_ne!(
            first.rows, second.rows,
            "a count-preserving update was not seen"
        );
        sqlx::raw_sql("ALTER TABLE test_a.locks OWNER TO cortex_base")
            .execute(&mut admin)
            .await
            .expect("owner change");
        assert_ne!(
            snapshot_of(&db).await.relations,
            second.relations,
            "an owner change was not seen"
        );
    })
    .await
}
