//! F-1b rework 5 (kai's row fd150e5e and ruling T-1; vera's F1B-RW4-TRANSITIVE-DEFAULT-CONSTRAINT
//! on 0dc166f). Attaching a partition changes the DEFAULT partition's implicit constraint without
//! writing any row of it, so when that DEFAULT is in another schema the row-based guard cannot see
//! the change. The author reproduced it on PostgreSQL 18.4 without a topology check (runs-rw5).
//! Ruling T-1 admits schema-closed partition trees only, so the guard checks the STARTING topology:
//! before a step runs, any `pg_inherits` link (partition, DEFAULT partition, partition index,
//! inheritance child) joining two schemas refuses the step, naming the link.
//!
//! Each mixed start is built by the OWNER before the guard runs, in a database of its own (the
//! owner-role path the ruling names), so it cannot reach any other case. The new error is matched
//! by its `Debug` form, so these tests-only cases compile before the variant exists; that form
//! names the variant and both fields exactly.
use cortex_db::OwnerConn;
use cortex_db::maintenance::templates;
use cortex_db::migrate::{MigrateError, Migration, MigrationSet, apply, platform_migrations};
use cortex_testkit::maint::{SPEC_A, SPEC_B};
use cortex_testkit::{TestDb, isolated_db, module, within};
use sqlx::{Connection, Row};

fn step(module_id: &str, name: &'static str, up: &'static str, down: &'static str) -> MigrationSet {
    MigrationSet {
        module: module(module_id),
        schema: "test_a",
        steps: vec![Migration {
            version: 1,
            name,
            up,
            down,
        }],
    }
}

/// A database of its own, its platform migrations applied, and `setup` run by the owner.
async fn start(tag: &str, setup: &'static str) -> (TestDb, OwnerConn) {
    let db = isolated_db(tag).await;
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform migrations");
    let mut raw = db.owner().await;
    sqlx::raw_sql(setup)
        .execute(&mut raw)
        .await
        .expect("the starting topology, made by the owner before the guard runs");
    (db, owner)
}

async fn one(db: &TestDb, sql: &'static str) -> String {
    let mut admin = db.admin().await;
    let v: Option<String> = sqlx::query(sql)
        .fetch_one(&mut admin)
        .await
        .expect("probe")
        .get(0);
    v.unwrap_or_else(|| "NULL".into())
}

async fn ledger(db: &TestDb, module_id: &'static str) -> String {
    let mut admin = db.admin().await;
    sqlx::query_scalar::<_, i64>("select count(*) from platform.migrations where module = $1")
        .bind(module_id)
        .fetch_one(&mut admin)
        .await
        .expect("ledger")
        .to_string()
}

/// Whether the owner may insert `sql`'s row now (in a transaction that is rolled back).
async fn accepts(db: &TestDb, sql: &'static str) -> bool {
    let mut owner = db.owner().await;
    let mut tx = owner.begin().await.expect("begin");
    let ok = sqlx::query(sql).execute(&mut *tx).await.is_ok();
    tx.rollback().await.ok();
    ok
}

fn refused_at_start(result: Result<(), MigrateError>, link: &str) {
    let err = result.expect_err("a starting topology that is not schema-closed refuses the step");
    assert_eq!(
        format!("{err:?}"),
        format!("StartTopologyNotSchemaClosed {{ version: 1, link: {link:?} }}"),
    );
}

/// vera's exact case: an own LIST parent, an owned empty DEFAULT partition of it in `test_b`, and
/// an own new child attached for a new bound. Refused before the step runs, and nothing changes:
/// the outside DEFAULT's partition constraint (the semantic oracle) and what it accepts.
#[tokio::test]
async fn rw5_an_outside_default_partition_refuses_the_own_attach() {
    within(120, async {
        let (db, mut owner) = start(
            "rw5_outside_default",
            "CREATE TABLE test_a.rw5_p (k int) PARTITION BY LIST (k); \
             CREATE TABLE test_b.rw5_d PARTITION OF test_a.rw5_p DEFAULT; \
             CREATE TABLE test_a.rw5_c (k int);",
        )
        .await;
        let constraint = "select pg_get_partition_constraintdef('test_b.rw5_d'::regclass)";
        let children = "select string_agg(inhrelid::regclass::text, ', ' order by inhrelid::regclass::text) \
                        from pg_inherits where inhparent = 'test_a.rw5_p'::regclass";
        assert_eq!(one(&db, constraint).await, "NULL", "premise: no sibling bound yet");
        assert!(
            accepts(&db, "INSERT INTO test_b.rw5_d VALUES (1)").await,
            "premise: the DEFAULT takes k = 1"
        );
        let attach = step(
            "rw5_attach",
            "own_child_attached_to_own_parent",
            "ALTER TABLE test_a.rw5_p ATTACH PARTITION test_a.rw5_c FOR VALUES IN (1);",
            "ALTER TABLE test_a.rw5_p DETACH PARTITION test_a.rw5_c;",
        );
        refused_at_start(
            apply(&mut owner, &attach).await,
            "test_b.rw5_d is the DEFAULT partition of test_a.rw5_p",
        );
        assert_eq!(one(&db, constraint).await, "NULL", "the DEFAULT's constraint is unchanged");
        assert!(
            accepts(&db, "INSERT INTO test_b.rw5_d VALUES (1)").await,
            "the DEFAULT still takes k = 1"
        );
        assert_eq!(one(&db, children).await, "test_b.rw5_d", "rolled back: not attached");
        assert_eq!(ledger(&db, "rw5_attach").await, "0", "not recorded");
    })
    .await
}

/// The control: the same tree with its DEFAULT in the module's own schema. The attach is
/// accepted, and the own DEFAULT's constraint changes exactly as the outside one would have.
#[tokio::test]
async fn rw5_an_own_default_partition_in_the_same_schema_is_accepted() {
    within(120, async {
        let (db, mut owner) = start(
            "rw5_own_default",
            "CREATE TABLE test_a.rw5_p (k int) PARTITION BY LIST (k); \
             CREATE TABLE test_a.rw5_d PARTITION OF test_a.rw5_p DEFAULT; \
             CREATE TABLE test_a.rw5_c (k int);",
        )
        .await;
        let constraint = "select pg_get_partition_constraintdef('test_a.rw5_d'::regclass)";
        assert_eq!(
            one(&db, constraint).await,
            "NULL",
            "premise: no sibling bound yet"
        );
        let attach = step(
            "rw5_attach",
            "own_child_attached_to_own_parent",
            "ALTER TABLE test_a.rw5_p ATTACH PARTITION test_a.rw5_c FOR VALUES IN (1);",
            "ALTER TABLE test_a.rw5_p DETACH PARTITION test_a.rw5_c;",
        );
        apply(&mut owner, &attach)
            .await
            .expect("a schema-closed tree's attach");
        assert_eq!(
            one(&db, constraint).await,
            "(NOT ((k IS NOT NULL) AND (k = 1)))",
            "the own DEFAULT's constraint changed"
        );
        assert!(
            !accepts(&db, "INSERT INTO test_a.rw5_d VALUES (1)").await,
            "the own DEFAULT no longer takes k = 1"
        );
        assert_eq!(ledger(&db, "rw5_attach").await, "1", "recorded");
    })
    .await
}

/// Every link kind, each direction, refuses ANY step (here an unrelated own table) and names the
/// first link; with the link removed by the owner, the same step in the same database is accepted.
#[tokio::test]
async fn rw5_every_cross_schema_link_refuses_any_step_until_it_is_gone() {
    within(240, async {
        let cases: [(&str, &str, &str, &str); 3] = [
            (
                "rw5_outside_parent",
                "CREATE TABLE test_b.rw5_q (k int) PARTITION BY LIST (k); \
                 CREATE TABLE test_a.rw5_x PARTITION OF test_b.rw5_q FOR VALUES IN (1);",
                "test_a.rw5_x is a partition of test_b.rw5_q",
                "ALTER TABLE test_b.rw5_q DETACH PARTITION test_a.rw5_x;",
            ),
            (
                "rw5_partition_index",
                "CREATE TABLE test_b.rw5_q (k int) PARTITION BY LIST (k); \
                 CREATE INDEX rw5_q_idx ON test_b.rw5_q (k); \
                 CREATE TABLE test_a.rw5_y (k int); \
                 CREATE INDEX rw5_a_idx ON test_a.rw5_y (k); \
                 ALTER TABLE test_b.rw5_q ATTACH PARTITION test_a.rw5_y FOR VALUES IN (1);",
                "test_a.rw5_a_idx is a partition index of test_b.rw5_q_idx",
                "ALTER TABLE test_b.rw5_q DETACH PARTITION test_a.rw5_y;",
            ),
            (
                "rw5_inheritance",
                "CREATE TABLE test_a.rw5_base (k int); \
                 CREATE TABLE test_b.rw5_heir () INHERITS (test_a.rw5_base);",
                "test_b.rw5_heir is an inheritance child of test_a.rw5_base",
                "ALTER TABLE test_b.rw5_heir NO INHERIT test_a.rw5_base;",
            ),
        ];
        for (tag, setup, link, unlink) in cases {
            let (db, mut owner) = start(tag, setup).await;
            let own = step(
                "rw5_unrelated",
                "an_unrelated_own_table",
                "CREATE TABLE test_a.rw5_t (k int);",
                "DROP TABLE test_a.rw5_t;",
            );
            refused_at_start(apply(&mut owner, &own).await, link);
            assert_eq!(
                one(
                    &db,
                    "select coalesce(to_regclass('test_a.rw5_t')::text, 'absent')"
                )
                .await,
                "absent",
                "{tag}: rolled back"
            );
            assert_eq!(
                ledger(&db, "rw5_unrelated").await,
                "0",
                "{tag}: not recorded"
            );
            let mut raw = db.owner().await;
            sqlx::raw_sql(unlink)
                .execute(&mut raw)
                .await
                .expect("the owner removes the link");
            apply(&mut owner, &own)
                .await
                .unwrap_or_else(|e| panic!("{tag}: a schema-closed start runs the step: {e}"));
            assert_eq!(ledger(&db, "rw5_unrelated").await, "1", "{tag}: recorded");
        }
    })
    .await
}

/// Ruling T-1's lifecycle, shown from the template: the run-time maintenance functions create a
/// child only in the spec's own schema, under a parent resolved in that schema and required to be
/// the root's child, and they attach, detach, inherit or move nothing.
#[test]
fn rw5_the_maintenance_template_makes_same_schema_children_only() {
    for spec in [&SPEC_A, &SPEC_B] {
        let (s, root) = (spec.schema, spec.root_table);
        let up = templates::partition_maintenance(spec).up;
        let lines: Vec<&str> = up.lines().map(str::trim).collect();
        let linking: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| {
                let l = l.to_ascii_lowercase();
                // DDL forms only: the template reads `pg_inherits`, which makes no link.
                [
                    "partition of",
                    "attach partition",
                    "detach partition",
                    "inherits (",
                    "inherits(",
                    " inherit ",
                    "set schema",
                ]
                .iter()
                .any(|w| l.contains(w))
            })
            .collect();
        assert_eq!(
            linking,
            [format!(
                "EXECUTE format('CREATE TABLE %I.%I PARTITION OF %s FOR VALUES FROM (%L) TO (%L)', '{s}', v_name, v_parent, v_from, v_to);"
            )],
            "{s}: the only link the template makes"
        );
        let parents: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| l.starts_with("v_parent :="))
            .collect();
        assert!(!parents.is_empty(), "{s}: the parent is resolved");
        for l in parents {
            // Resolved in the spec's schema, or cleared when it is not the root's partition (a
            // NULL parent is a conflict: nothing is created).
            assert!(
                l == "v_parent := NULL;"
                    || l.starts_with(&format!(
                        "v_parent := to_regclass(format('%I.%I', '{s}', '{root}_' ||"
                    )),
                "{s}: the parent is resolved in the spec's schema: {l}"
            );
        }
        for l in lines.iter().filter(|l| l.starts_with("v_root :=")) {
            assert_eq!(
                *l,
                format!("v_root := '{s}.{root}'::regclass;"),
                "{s}: the root"
            );
        }
        assert!(
            lines.contains(
                &"WHERE i.inhrelid = v_parent AND i.inhparent = v_root AND c.relkind = 'p') THEN"
            ),
            "{s}: the parent must be the root's own partition"
        );
    }
}
