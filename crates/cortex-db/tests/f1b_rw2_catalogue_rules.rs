//! F-1b rework 2 (kai's row 23fb47be): how the migration guard's coverage is known to be complete.
//! The guard learns which catalogues a step changed from the server's own per-transaction write
//! counts, over every table in `pg_catalog`. It refuses any changed catalogue whose rule is `Never`,
//! or that has no rule. This test pins its rule table to the server: it fails when the server has a
//! catalogue the table lacks (a newer PostgreSQL), when the table names one the server lacks, when a
//! catalogue has two rules, or when the owner cannot read a `Rows` catalogue (its rows could not be
//! attributed). It is added with the table it tests. Its failing form is shown in the rework's
//! receipts by a mutated copy of the table.
use std::collections::BTreeSet;

use cortex_db::migrate::{CatalogueRule, guard_catalogues};
use cortex_testkit::{test_db, within};

#[tokio::test]
async fn rw2_every_server_catalogue_has_exactly_one_rule() {
    within(60, async {
        let db = test_db().await;
        let mut owner = db.owner().await;
        let server: BTreeSet<String> = sqlx::query_scalar(
            "select c.relname::text from pg_class c \
             where c.relnamespace = 'pg_catalog'::regnamespace and c.relkind = 'r'",
        )
        .fetch_all(&mut owner)
        .await
        .expect("the server's catalogues")
        .into_iter()
        .collect();
        let table = guard_catalogues();
        let named: BTreeSet<String> = table.iter().map(|(n, _)| (*n).to_owned()).collect();
        assert_eq!(named.len(), table.len(), "a catalogue has two rules");
        let missing: Vec<&String> = server.difference(&named).collect();
        let extra: Vec<&String> = named.difference(&server).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "the guard's table and the server differ: not in the table {missing:?}; not on the server {extra:?}"
        );
        for (name, rule) in &table {
            if *rule == CatalogueRule::Rows {
                let readable: bool =
                    sqlx::query_scalar("select has_table_privilege('cortex_owner', $1, 'SELECT')")
                        .bind(format!("pg_catalog.{name}"))
                        .fetch_one(&mut owner)
                        .await
                        .expect("privilege");
                assert!(readable, "the owner cannot read {name}, a Rows catalogue");
            }
        }
    })
    .await
}
