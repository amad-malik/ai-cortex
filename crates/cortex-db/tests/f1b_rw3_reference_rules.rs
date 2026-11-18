//! F-1b rework 3 (kai's row a87f8569): how the migration guard's coverage of relationship ends is
//! known to be complete, the same way as for catalogues in rework 2. Every reference column the
//! server declares for a `Rows` catalogue (`pg_get_catalog_foreign_keys()`) has exactly one rule in
//! the guard's table, and every `Changed` rule has an end the guard checks. A reference a newer
//! PostgreSQL adds fails this test. And the `initdb` objects the owner could remove, which the
//! guard's removal check includes, are computed by the server: with this fixture, exactly `public`.
//! Added with the table they test. Their failing form is shown in the rework's receipts by a
//! mutated copy of the table.
use std::collections::BTreeSet;

use cortex_db::migrate::{
    CatalogueRule, ReferenceRule, guard_catalogues, guard_changed_endpoints, guard_references,
    owned_initdb_sql,
};
use cortex_testkit::{test_db, within};

#[tokio::test]
async fn rw3_every_reference_of_a_rows_catalogue_has_exactly_one_rule() {
    within(60, async {
        let db = test_db().await;
        let mut owner = db.owner().await;
        let rows: Vec<String> = guard_catalogues()
            .into_iter()
            .filter(|(_, r)| *r == CatalogueRule::Rows)
            .map(|(n, _)| n.to_owned())
            .collect();
        let server: BTreeSet<(String, String)> = sqlx::query_as(
            "select fktable::text, array_to_string(fkcols, ',') \
             from pg_get_catalog_foreign_keys() where fktable::text = any($1)",
        )
        .bind(&rows)
        .fetch_all(&mut owner)
        .await
        .expect("the server's catalogue references")
        .into_iter()
        .collect();
        let table = guard_references();
        let named: BTreeSet<(String, String)> = table
            .iter()
            .map(|(t, c, _)| ((*t).to_owned(), (*c).to_owned()))
            .collect();
        assert_eq!(named.len(), table.len(), "a reference has two rules");
        let missing: Vec<_> = server.difference(&named).collect();
        let extra: Vec<_> = named.difference(&server).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "the guard's references and the server differ: not in the table {missing:?}; not on the server {extra:?}"
        );
        let changed: BTreeSet<(&str, &str)> = table
            .iter()
            .filter(|(_, _, r)| *r == ReferenceRule::Changed)
            .map(|(t, c, _)| (*t, *c))
            .collect();
        let checked: BTreeSet<(&str, &str)> = guard_changed_endpoints().into_iter().collect();
        assert_eq!(
            changed, checked,
            "a Changed reference without a checked end, or a checked end not in the table"
        );
    })
    .await
}

#[tokio::test]
async fn rw3_the_initdb_objects_the_owner_can_remove_are_exactly_public() {
    within(60, async {
        let db = test_db().await;
        let mut owner = db.owner().await;
        let owned: Vec<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "select o::int8 from ({}) s order by 1",
            owned_initdb_sql()
        )))
        .fetch_all(&mut owner)
        .await
        .expect("the owned initdb objects");
        assert_eq!(
            owned,
            [2200],
            "public (2200) only, with this fixture's topology"
        );
    })
    .await
}
