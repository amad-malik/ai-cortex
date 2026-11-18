//! RED for piece 7, DB cells for isolation and the search role's privileges (MS-01 to MS-03; the shared negative matrix NEG-00 to NEG-09 is
//! the slot runner's and the chain's: here is the database half). Needs `supervise.sh`'s throwaway Postgres.
mod support;

use mod_search::query::run;
use mod_search::validate::check;
use serde_json::{Value, json};
use support::{Rec, World, search_tx, seed, setup};

async fn ids_for(w: &World, project: Option<cortex_types::ProjectId>, body: Value) -> Vec<String> {
    let v = check(
        Some("application/json"),
        &serde_json::to_vec(&body).expect("json"),
    )
    .expect("valid");
    let mut tx = search_tx(w, project).await;
    let hits = run(&mut tx, &v)
        .await
        .unwrap_or_else(|p| panic!("{} {}", p.code, p.detail));
    tx.commit().await.expect("commit");
    hits.into_iter().map(|h| h.id).collect()
}

#[tokio::test]
async fn ms01_a_search_in_one_project_never_sees_another_projects_canary_in_a_summary_or_a_body() {
    let w = setup("ms01").await;
    let a1 = seed(
        &w,
        w.a,
        &Rec::note(1, 1, "ordinary plan", Some("nothing special here")),
    )
    .await;
    let b_sum = seed(&w, w.b, &Rec::note(2, 2, "zebracanary summary", None)).await;
    let b_body = seed(
        &w,
        w.b,
        &Rec::note(3, 3, "ordinary plan", Some("zebracanary hidden in a body")),
    )
    .await;
    // B sees its own canary, in the summary and in the body.
    let mut b = ids_for(&w, Some(w.b), json!({"query": "zebracanary"})).await;
    b.sort();
    let mut want = vec![b_sum.clone(), b_body.clone()];
    want.sort();
    assert_eq!(b, want);
    // A sees none of it, by any spelling of the query, with or without history.
    for q in [
        "zebracanary",
        "zebracanary or ordinary",
        "\"zebracanary summary\"",
        "zebra*",
        "ordinary -nothing",
    ] {
        for hist in [false, true] {
            let got = ids_for(&w, Some(w.a), json!({"query": q, "include_history": hist})).await;
            assert!(
                !got.contains(&b_sum) && !got.contains(&b_body),
                "query {q:?}: {got:?}"
            );
        }
    }
    // A's own record is found, and B's record of the same words is not A's.
    assert_eq!(
        ids_for(&w, Some(w.a), json!({"query": "ordinary plan"})).await,
        vec![a1]
    );
}

async fn denied(w: &World, sql: &'static str) -> Option<String> {
    let mut tx = search_tx(w, Some(w.a)).await;
    let r = sqlx::query(sql)
        .execute(tx.executor().expect("executor"))
        .await;
    match r {
        Ok(_) => None,
        Err(sqlx::Error::Database(d)) => d.code().map(|c| c.to_string()),
        Err(e) => panic!("not a database error: {e}"),
    }
}

#[tokio::test]
async fn ms02_the_search_role_reads_the_two_views_and_nothing_else() {
    let w = setup("ms02").await;
    seed(&w, w.a, &Rec::note(1, 1, "visible", None)).await;
    // The two views work.
    for ok in [
        "SELECT count(*) FROM records.pub_sources",
        "SELECT count(*) FROM records.pub_state",
    ] {
        assert_eq!(denied(&w, ok).await, None, "{ok}");
    }
    // Every base table of the schema, every write on the views, the generated-column functions: permission denied (42501).
    for sql in [
        "SELECT count(*) FROM records.records",
        "SELECT count(*) FROM records.state",
        "SELECT count(*) FROM records.state_log",
        "SELECT count(*) FROM records.sections",
        "SELECT count(*) FROM records.lineage_counters",
        "INSERT INTO records.pub_state (record_id, project_id, state, changed_at) VALUES (gen_random_uuid(), gen_random_uuid(), 'invalidated', now())",
        "UPDATE records.pub_sources SET summary = 'x'",
        "DELETE FROM records.pub_sources",
        "DELETE FROM records.pub_state",
        "SELECT records.search_vector('a', 'b')",
        "SELECT records.search_vector_partial('a', 'b')",
        "CREATE TABLE records.sneaky (a int)",
    ] {
        assert_eq!(denied(&w, sql).await.as_deref(), Some("42501"), "{sql}");
    }
}

#[tokio::test]
async fn ms03_with_no_project_or_a_project_without_rows_a_search_returns_nothing() {
    let w = setup("ms03").await;
    seed(&w, w.a, &Rec::note(1, 1, "findable words", None)).await;
    assert!(
        ids_for(&w, None, json!({"query": "findable"}))
            .await
            .is_empty(),
        "an empty project setting sees nothing"
    );
    let other = cortex_types::ProjectId::new(&cortex_types::SystemClock);
    assert!(
        ids_for(&w, Some(other), json!({"query": "findable"}))
            .await
            .is_empty()
    );
    assert_eq!(
        ids_for(&w, Some(w.a), json!({"query": "findable"}))
            .await
            .len(),
        1
    );
}
