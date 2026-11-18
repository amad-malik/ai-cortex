//! RED for piece 6, DB layer (tests/mod-records.md REC-04, 11, 12, 29 and the lineage tables): the migration applies to
//! a fresh database through the migration guard, and the pack's constraints, triggers, row-level security and grants
//! hold under the module role. Needs `supervise.sh`'s throwaway Postgres (CV2_PG_PORT, CV2_PG_PASSWORD).

use std::sync::Arc;

use cortex_db::migrate::{apply, platform_migrations, revert_last};
use cortex_db::{DbPool, ModuleTx, OwnerConn, TxOptions};
use cortex_module::Piece;
use cortex_testkit::{TestDb, ctx, empty_db, sqlstate};
use cortex_types::{ProjectId, RecordId, SystemClock};
use mod_records::validate::stored_len;
use mod_records::{BOOTSTRAP_SQL, ID, ModRecords};
use serde_json::{Value, json};

struct World {
    db: TestDb,
    pool: DbPool,
    a: ProjectId,
    b: ProjectId,
}

async fn setup(tag: &str) -> World {
    setup_with(tag, "").await
}

/// `before_migrations` is SQL the superuser runs after the bootstrap and before any migration (a hostile configuration).
async fn setup_with(tag: &str, before_migrations: &str) -> World {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("bootstrap");
    if !before_migrations.is_empty() {
        sqlx::raw_sql(sqlx::AssertSqlSafe(before_migrations.to_owned()))
            .execute(&mut admin)
            .await
            .expect("the hostile configuration");
    }
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    apply(&mut owner, &ModRecords::new().migrations())
        .await
        .expect("the records migration applies through the guard");
    let pool = DbPool::connect(&db.db_config(&[ID], Arc::new(SystemClock)))
        .await
        .expect("pool");
    World {
        db,
        pool,
        a: ProjectId::new(&SystemClock),
        b: ProjectId::new(&SystemClock),
    }
}

async fn begin(w: &World, project: Option<ProjectId>) -> ModuleTx {
    ModuleTx::begin(&w.pool, ctx(ID, project), TxOptions::default())
        .await
        .expect("begin")
}

/// One record to insert, with the columns the checks care about.
struct Rec<'a> {
    project: ProjectId,
    kind: &'a str,
    section: Option<&'a str>,
    revision: Option<i32>,
    source_kind: Option<&'a str>,
    source_ref: Option<&'a str>,
    owner: Option<&'a str>,
    fields: Value,
}

impl<'a> Rec<'a> {
    fn note(project: ProjectId) -> Rec<'a> {
        Rec {
            project,
            kind: "note",
            section: None,
            revision: None,
            source_kind: None,
            source_ref: None,
            owner: None,
            fields: json!({}),
        }
    }
    fn knowledge(project: ProjectId) -> Rec<'a> {
        Rec {
            kind: "knowledge",
            source_kind: Some("file"),
            source_ref: Some("/x"),
            ..Rec::note(project)
        }
    }
}

async fn insert(tx: &mut ModuleTx, r: &Rec<'_>) -> Result<RecordId, sqlx::Error> {
    let id = RecordId::new(&SystemClock);
    sqlx::query(
        "INSERT INTO records.records (id, project_id, created_at, type, provenance, author_kind, author_id, written_by, summary, \
         summary_sha256, body_sha256, fields_sha256, fields, section, section_revision, source_kind, source_ref, owner) \
         VALUES ($1::uuid, $2::uuid, uuid_extract_timestamp($1::uuid), $3, 'worker_written', 'worker', 'w1', $1::uuid, 'summary', \
         sha256('s'::bytea), sha256('b'::bytea), sha256('f'::bytea), $4::jsonb, $5, $6, $7, $8, $9::uuid)",
    )
    .bind(id.to_string())
    .bind(r.project.to_string())
    .bind(r.kind)
    .bind(serde_json::to_string(&r.fields).expect("json"))
    .bind(r.section)
    .bind(r.revision)
    .bind(r.source_kind)
    .bind(r.source_ref)
    .bind(r.owner)
    .execute(tx.executor().expect("executor"))
    .await?;
    Ok(id)
}

/// The SQLSTATE of a refused insert, in a transaction of its own (a failed statement aborts the transaction).
async fn refused(w: &World, project: ProjectId, r: Rec<'_>) -> String {
    let mut tx = begin(w, Some(project)).await;
    let e = insert(&mut tx, &r)
        .await
        .expect_err("the insert must be refused");
    sqlstate(&e).unwrap_or_else(|| format!("no sqlstate: {e}"))
}

async fn accepted(w: &World, project: ProjectId, r: Rec<'_>) {
    let mut tx = begin(w, Some(project)).await;
    insert(&mut tx, &r).await.expect("the insert is accepted");
    tx.commit().await.expect("commit");
}

async fn count(tx: &mut ModuleTx, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("count")
}

#[tokio::test]
async fn db01_the_migration_applies_through_the_guard_and_reverts() {
    let w = setup("db01").await;
    let mut admin = w.db.admin().await;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'records' AND c.relkind = 'r' ORDER BY 1",
    )
    .fetch_all(&mut admin)
    .await
    .expect("tables");
    assert_eq!(
        tables,
        [
            "lineage_counters",
            "records",
            "sections",
            "state",
            "state_log"
        ]
    );
    let forced: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'records' AND c.relkind = 'r' AND c.relrowsecurity AND c.relforcerowsecurity",
    )
    .fetch_one(&mut admin)
    .await
    .expect("rls");
    assert_eq!(
        forced, 5,
        "row-level security is enabled and forced on all five tables"
    );
    let views: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'records' AND c.relkind = 'v' ORDER BY 1",
    )
    .fetch_all(&mut admin)
    .await
    .expect("views");
    assert_eq!(
        views,
        ["pub_sources", "pub_state"],
        "the search views live inside schema records (W1-14)"
    );
    let mut owner = OwnerConn::connect(&w.db.owner_config())
        .await
        .expect("owner");
    revert_last(&mut owner, &ModRecords::new().migrations())
        .await
        .expect("the down step reverts");
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_namespace WHERE nspname = 'records'")
            .fetch_one(&mut admin)
            .await
            .expect("schema");
    assert_eq!(left, 0, "nothing of the piece is left after the revert");
}

#[tokio::test]
async fn db02_records_are_immutable_for_the_module_and_for_the_owner() {
    let w = setup("db02").await;
    let mut tx = begin(&w, Some(w.a)).await;
    let id = insert(&mut tx, &Rec::note(w.a)).await.expect("insert");
    tx.commit().await.expect("commit");

    let mut tx = begin(&w, Some(w.a)).await;
    let e = sqlx::query("UPDATE records.records SET summary = 'x'")
        .execute(tx.executor().expect("executor"))
        .await
        .expect_err("UPDATE is refused to the module role");
    assert_eq!(sqlstate(&e).as_deref(), Some("42501"));
    let mut tx = begin(&w, Some(w.a)).await;
    let e = sqlx::query("DELETE FROM records.records")
        .execute(tx.executor().expect("executor"))
        .await
        .expect_err("DELETE is refused to the module role");
    assert_eq!(sqlstate(&e).as_deref(), Some("42501"));

    // The owner is held to row-level security too (FORCE) and the trigger refuses its UPDATE.
    let mut owner = w.db.owner().await;
    sqlx::query("SELECT set_config('cortex.project_id', $1, false)")
        .bind(w.a.to_string())
        .execute(&mut owner)
        .await
        .expect("setting");
    let e = sqlx::query("UPDATE records.records SET summary = 'x' WHERE id = $1::uuid")
        .bind(id.to_string())
        .execute(&mut owner)
        .await
        .expect_err("the trigger refuses the owner");
    assert_eq!(sqlstate(&e).as_deref(), Some("42501"));
}

#[tokio::test]
async fn db03_projects_are_isolated_by_row_level_security() {
    let w = setup("db03").await;
    accepted(&w, w.a, Rec::note(w.a)).await;
    let mut tx = begin(&w, Some(w.a)).await;
    assert_eq!(
        count(&mut tx, "SELECT count(*) FROM records.records").await,
        1
    );
    let mut tx = begin(&w, Some(w.b)).await;
    assert_eq!(
        count(&mut tx, "SELECT count(*) FROM records.records").await,
        0,
        "B sees nothing of A"
    );
    let mut tx = begin(&w, None).await;
    assert_eq!(
        count(&mut tx, "SELECT count(*) FROM records.records").await,
        0,
        "no project, no rows"
    );
    // A's transaction cannot write a row for B.
    assert_eq!(refused(&w, w.a, Rec::note(w.b)).await, "42501");
}

#[tokio::test]
async fn db04_source_and_owner_checks_are_the_databases_backstop() {
    let w = setup("db04").await;
    let none = |p| Rec {
        source_kind: None,
        source_ref: None,
        ..Rec::knowledge(p)
    };
    assert_eq!(
        refused(&w, w.a, none(w.a)).await,
        "23514",
        "knowledge without a source"
    );
    let note_with_source = Rec {
        source_kind: Some("file"),
        source_ref: Some("/x"),
        ..Rec::note(w.a)
    };
    assert_eq!(
        refused(&w, w.a, note_with_source).await,
        "23514",
        "a note cannot carry a source"
    );
    let kind_no_ref = Rec {
        source_ref: None,
        ..Rec::knowledge(w.a)
    };
    assert_eq!(
        refused(&w, w.a, kind_no_ref).await,
        "23514",
        "a source kind without a ref"
    );
    let bad_kind = Rec {
        source_kind: Some("blog"),
        ..Rec::knowledge(w.a)
    };
    assert_eq!(
        refused(&w, w.a, bad_kind).await,
        "23514",
        "an unknown source kind"
    );
    let owner = "01930000-0000-7000-8000-0000000000aa";
    let note_owner = Rec {
        owner: Some(owner),
        ..Rec::note(w.a)
    };
    assert_eq!(
        refused(&w, w.a, note_owner).await,
        "23514",
        "only knowledge may carry an owner"
    );
    accepted(
        &w,
        w.a,
        Rec {
            owner: Some(owner),
            ..Rec::knowledge(w.a)
        },
    )
    .await;
    accepted(&w, w.a, Rec::knowledge(w.a)).await;
}

#[tokio::test]
async fn db05_the_reserved_sections_waive_the_source_and_nothing_else_does() {
    let w = setup("db05").await;
    let w1 = "rules-worker:01930000-0000-7000-8000-0000000000aa";
    for (i, sec) in ["orientation", "rules-project", w1].into_iter().enumerate() {
        let r = Rec {
            source_kind: None,
            source_ref: None,
            section: Some(sec),
            revision: Some(1),
            ..Rec::knowledge(w.a)
        };
        accepted(&w, w.a, r).await;
        let _ = i;
    }
    let scratch = Rec {
        source_kind: None,
        source_ref: None,
        section: Some("scratch"),
        revision: Some(1),
        ..Rec::knowledge(w.a)
    };
    assert_eq!(
        refused(&w, w.a, scratch).await,
        "23514",
        "the waiver covers the three reserved sections only"
    );
    let note_src = Rec {
        section: Some("orientation"),
        revision: Some(2),
        source_kind: Some("file"),
        source_ref: Some("/x"),
        ..Rec::note(w.a)
    };
    assert_eq!(
        refused(&w, w.a, note_src).await,
        "23514",
        "a note in orientation cannot carry a source"
    );
    // A section and its revision go together; a revision number is never repeated.
    let no_rev = Rec {
        source_kind: None,
        source_ref: None,
        section: Some("orientation"),
        revision: None,
        ..Rec::knowledge(w.a)
    };
    assert_eq!(refused(&w, w.a, no_rev).await, "23514");
    let dup = Rec {
        source_kind: None,
        source_ref: None,
        section: Some("orientation"),
        revision: Some(1),
        ..Rec::knowledge(w.a)
    };
    assert_eq!(
        refused(&w, w.a, dup).await,
        "23505",
        "the unique revision index holds"
    );
}

#[tokio::test]
async fn db06_the_lineage_tables_enforce_reason_and_append_only() {
    let w = setup("db06").await;
    let mut tx = begin(&w, Some(w.a)).await;
    let r1 = insert(&mut tx, &Rec::note(w.a)).await.expect("r1");
    let r2 = insert(&mut tx, &Rec::note(w.a)).await.expect("r2");
    tx.commit().await.expect("commit");

    async fn state(w: &World, sql: &str, binds: &[String]) -> String {
        let mut tx = begin(w, Some(w.a)).await;
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()));
        for b in binds {
            q = q.bind(b.clone());
        }
        let e = q
            .execute(tx.executor().expect("executor"))
            .await
            .expect_err("refused");
        sqlstate(&e).unwrap_or_default()
    }
    let a = w.a.to_string();
    let (i1, i2) = (r1.to_string(), r2.to_string());
    let ins = "INSERT INTO records.state (project_id, record_id, state, superseded_by, by_kind, by_id, reason) \
               VALUES ($1::uuid, $2::uuid, $3, $4::uuid, 'worker', 'w1', $5)";
    // No reason, an empty reason and a 513-byte reason are all refused.
    let no_reason = "INSERT INTO records.state (project_id, record_id, state, by_kind, by_id) \
                     VALUES ($1::uuid, $2::uuid, 'invalidated', 'worker', 'w1')";
    assert_eq!(
        state(&w, no_reason, &[a.clone(), i1.clone()]).await,
        "23502"
    );
    let inv = |reason: String| {
        [
            a.clone(),
            i1.clone(),
            "invalidated".into(),
            String::new(),
            reason,
        ]
    };
    let _ = inv;
    let mut tx = begin(&w, Some(w.a)).await;
    let e = sqlx::query(
        "INSERT INTO records.state (project_id, record_id, state, by_kind, by_id, reason) \
         VALUES ($1::uuid, $2::uuid, 'invalidated', 'worker', 'w1', '')",
    )
    .bind(&a)
    .bind(&i1)
    .execute(tx.executor().expect("executor"))
    .await
    .expect_err("empty reason");
    assert_eq!(sqlstate(&e).as_deref(), Some("23514"));
    let mut tx = begin(&w, Some(w.a)).await;
    let e = sqlx::query(
        "INSERT INTO records.state (project_id, record_id, state, by_kind, by_id, reason) \
         VALUES ($1::uuid, $2::uuid, 'invalidated', 'worker', 'w1', repeat('x', 513))",
    )
    .bind(&a)
    .bind(&i1)
    .execute(tx.executor().expect("executor"))
    .await
    .expect_err("513-byte reason");
    assert_eq!(sqlstate(&e).as_deref(), Some("23514"));
    // superseded needs superseded_by, and a record cannot supersede itself.
    let mut tx = begin(&w, Some(w.a)).await;
    let e = sqlx::query(
        "INSERT INTO records.state (project_id, record_id, state, by_kind, by_id, reason) \
         VALUES ($1::uuid, $2::uuid, 'superseded', 'worker', 'w1', 'r')",
    )
    .bind(&a)
    .bind(&i1)
    .execute(tx.executor().expect("executor"))
    .await
    .expect_err("superseded without successor");
    assert_eq!(sqlstate(&e).as_deref(), Some("23514"));
    assert_eq!(
        state(
            &w,
            ins,
            &[
                a.clone(),
                i1.clone(),
                "superseded".into(),
                i1.clone(),
                "r".into()
            ]
        )
        .await,
        "23514"
    );

    // A good lineage row and its log row; the log is append-only and its sequence unique.
    let mut tx = begin(&w, Some(w.a)).await;
    sqlx::query(ins)
        .bind(&a)
        .bind(&i1)
        .bind("superseded")
        .bind(&i2)
        .bind("replaced")
        .execute(tx.executor().expect("executor"))
        .await
        .expect("state row");
    sqlx::query(
        "INSERT INTO records.state_log (project_id, seq, record_id, from_state, to_state, superseded_by, by_kind, by_id, reason) \
         VALUES ($1::uuid, 1, $2::uuid, 'active', 'superseded', $3::uuid, 'worker', 'w1', 'replaced')",
    )
    .bind(&a)
    .bind(&i1)
    .bind(&i2)
    .execute(tx.executor().expect("executor"))
    .await
    .expect("log row");
    tx.commit().await.expect("commit");
    assert_eq!(
        state(&w, "UPDATE records.state_log SET reason = 'x'", &[]).await,
        "42501"
    );
    assert_eq!(
        state(&w, "DELETE FROM records.state_log", &[]).await,
        "42501"
    );
    let dup = "INSERT INTO records.state_log (project_id, seq, record_id, from_state, to_state, by_kind, by_id, reason) \
               VALUES ($1::uuid, 1, $2::uuid, 'active', 'invalidated', 'worker', 'w1', 'why')";
    assert_eq!(state(&w, dup, &[a.clone(), i2.clone()]).await, "23505");
}

#[tokio::test]
async fn db07_a_created_at_that_is_not_the_ids_timestamp_is_refused() {
    let w = setup("db07").await;
    let mut tx = begin(&w, Some(w.a)).await;
    let id = RecordId::new(&SystemClock);
    let e = sqlx::query(
        "INSERT INTO records.records (id, project_id, created_at, type, provenance, author_kind, author_id, written_by, summary, \
         summary_sha256, body_sha256, fields_sha256) \
         VALUES ($1::uuid, $2::uuid, now() + interval '1 day', 'note', 'worker_written', 'worker', 'w1', $1::uuid, 's', \
         sha256('s'::bytea), sha256('b'::bytea), sha256('f'::bytea))",
    )
    .bind(id.to_string())
    .bind(w.a.to_string())
    .execute(tx.executor().expect("executor"))
    .await
    .expect_err("refused");
    assert_eq!(sqlstate(&e).as_deref(), Some("23514"));
}

/// The Rust bound on `fields` is the database's own form: `octet_length(fields::text)`.
#[tokio::test]
async fn db08_stored_len_equals_the_databases_octet_length_of_fields() {
    let w = setup("db08").await;
    let samples = [
        json!({}),
        json!({"a": 1}),
        json!({"a": [1, 2, {"b": "c"}], "z": null, "t": true, "f": false}),
        json!({"\u{e9}": "\u{fc}\u{20ac}\u{1f600}"}),
        json!({"k": "line\nbreak \"q\" \\ \u{1} / tab\t"}),
        json!({"list": [[], {}, [[]], [{}], "x"], "n": [-7, 0, 123456789, 1.5]}),
    ];
    for v in samples {
        let mut tx = begin(&w, Some(w.a)).await;
        let text = serde_json::to_string(&v).expect("json");
        let stored: i32 = sqlx::query_scalar("SELECT octet_length($1::jsonb::text)")
            .bind(text)
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect("octet_length");
        assert_eq!(stored_len(&v) as i32, stored, "stored length of {v}");
    }
}

/// W1-14: the search role has USAGE on schema records and SELECT on the two views, nothing else; no function in the schema
/// is executable through PUBLIC.
#[tokio::test]
async fn db09_the_search_role_reads_only_the_two_views_and_calls_no_function() {
    let w = setup("db09").await;
    let mut tx = begin(&w, Some(w.a)).await;
    let _live = insert(&mut tx, &Rec::knowledge(w.a)).await.expect("live");
    let gone = insert(&mut tx, &Rec::note(w.a)).await.expect("gone");
    sqlx::query(
        "INSERT INTO records.state (project_id, record_id, state, by_kind, by_id, reason) \
         VALUES ($1::uuid, $2::uuid, 'invalidated', 'worker', 'w1', 'because')",
    )
    .bind(w.a.to_string())
    .bind(gone.to_string())
    .execute(tx.executor().expect("executor"))
    .await
    .expect("invalidate");
    tx.commit().await.expect("commit");
    accepted(&w, w.b, Rec::note(w.b)).await;

    let mut c = w.db.admin().await;
    async fn as_search(c: &mut sqlx::PgConnection, project: Option<ProjectId>) {
        sqlx::query("RESET ROLE")
            .execute(&mut *c)
            .await
            .expect("reset");
        sqlx::query("SELECT set_config('cortex.project_id', $1, false)")
            .bind(project.map(|p| p.to_string()).unwrap_or_default())
            .execute(&mut *c)
            .await
            .expect("setting");
        sqlx::query("SET ROLE cortex_m_search")
            .execute(&mut *c)
            .await
            .expect("set role");
    }
    async fn n(c: &mut sqlx::PgConnection, sql: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(&mut *c)
            .await
            .expect("count")
    }
    as_search(&mut c, Some(w.a)).await;
    assert_eq!(
        n(&mut c, "SELECT count(*) FROM records.pub_sources").await,
        2,
        "A's two records"
    );
    assert_eq!(
        n(&mut c, "SELECT count(*) FROM records.pub_state").await,
        1,
        "one non-active record"
    );
    for sql in [
        "SELECT count(*) FROM records.records",
        "SELECT count(*) FROM records.state",
        "SELECT count(*) FROM records.state_log",
        "SELECT count(*) FROM records.sections",
        "SELECT records.search_vector('a', 'b')::text",
        "SELECT records.search_vector_partial('a', 'b')::text",
    ] {
        let e = sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(&mut c)
            .await
            .expect_err("refused to the search role");
        assert_eq!(sqlstate(&e).as_deref(), Some("42501"), "{sql}");
        // the failed statement leaves the session usable
    }
    as_search(&mut c, Some(w.b)).await;
    assert_eq!(
        n(&mut c, "SELECT count(*) FROM records.pub_sources").await,
        1,
        "B sees only B's"
    );
    as_search(&mut c, None).await;
    assert_eq!(
        n(&mut c, "SELECT count(*) FROM records.pub_sources").await,
        0,
        "no project, no rows"
    );
    sqlx::query("RESET ROLE")
        .execute(&mut c)
        .await
        .expect("reset");

    // The catalogue: no function in the schema is executable through PUBLIC (a NULL proacl IS the default, PUBLIC execute).
    let open: i64 = n(
        &mut c,
        "SELECT count(*) FROM pg_proc p JOIN pg_namespace ns ON ns.oid = p.pronamespace WHERE ns.nspname = 'records' \
         AND (p.proacl IS NULL OR EXISTS (SELECT 1 FROM aclexplode(p.proacl) a WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE'))",
    )
    .await;
    assert_eq!(
        open, 0,
        "REVOKE EXECUTE ... FROM PUBLIC on every function in the schema"
    );
    for f in [
        "records.search_vector(text, text)",
        "records.search_vector_partial(text, text)",
    ] {
        let q = format!(
            "SELECT has_function_privilege('cortex_m_records', '{f}', 'EXECUTE')::int::bigint"
        );
        assert_eq!(n(&mut c, &q).await, 1, "the module role keeps {f}");
        let q = format!(
            "SELECT has_function_privilege('cortex_m_search', '{f}', 'EXECUTE')::int::bigint"
        );
        assert_eq!(n(&mut c, &q).await, 0, "the search role has not {f}");
    }
    assert_eq!(
        n(
            &mut c,
            "SELECT has_schema_privilege('cortex_m_search', 'records', 'USAGE')::int::bigint"
        )
        .await,
        1
    );
    assert_eq!(
        n(
            &mut c,
            "SELECT has_schema_privilege('cortex_m_search', 'records', 'CREATE')::int::bigint"
        )
        .await,
        0
    );
    assert_eq!(
        n(
            &mut c,
            "SELECT has_table_privilege('cortex_m_search', 'records.records', 'SELECT')::int::bigint"
        )
        .await,
        0
    );
    assert_eq!(
        n(
            &mut c,
            "SELECT has_table_privilege('cortex_m_search', 'records.pub_state', 'SELECT')::int::bigint"
        )
        .await,
        1
    );
}

/// Cox's addition to W1-14: the REVOKE covers the functions that exist when it runs. A function added by a LATER migration would
/// again be executable through PUBLIC. This cell walks `pg_proc` for the schema, so the next such function fails here.
/// Rule for every future migration in schema `records`: after creating a function, `REVOKE EXECUTE ON FUNCTION ... FROM PUBLIC`
/// and grant back only what a module role needs (a trigger function needs no grant).
#[tokio::test]
async fn db10_the_search_role_can_execute_no_function_in_schema_records() {
    let w = setup("db10").await;
    let mut c = w.db.admin().await;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_proc p JOIN pg_namespace ns ON ns.oid = p.pronamespace WHERE ns.nspname = 'records'",
    )
    .fetch_one(&mut c)
    .await
    .expect("functions");
    assert!(
        total >= 4,
        "the walk is not vacuous: {total} functions in the schema"
    );
    let open: Vec<String> = sqlx::query_scalar(
        "SELECT p.oid::regprocedure::text FROM pg_proc p JOIN pg_namespace ns ON ns.oid = p.pronamespace \
         WHERE ns.nspname = 'records' AND has_function_privilege('cortex_m_search', p.oid, 'EXECUTE') ORDER BY 1",
    )
    .fetch_all(&mut c)
    .await
    .expect("walk");
    assert!(open.is_empty(), "the search role can execute: {open:?}");
    // And through PUBLIC: no ACL entry for the pseudo-role, and no NULL ACL (the default, which is PUBLIC execute).
    let public_open: Vec<String> = sqlx::query_scalar(
        "SELECT p.oid::regprocedure::text FROM pg_proc p JOIN pg_namespace ns ON ns.oid = p.pronamespace \
         WHERE ns.nspname = 'records' AND (p.proacl IS NULL OR EXISTS \
         (SELECT 1 FROM aclexplode(p.proacl) a WHERE a.grantee = 0 AND a.privilege_type = 'EXECUTE')) ORDER BY 1",
    )
    .fetch_all(&mut c)
    .await
    .expect("walk public");
    assert!(
        public_open.is_empty(),
        "executable through PUBLIC: {public_open:?}"
    );
}

/// hale F-2 (cox W1-36): revision 7 gave the search role USAGE on schema `records`, so table ACLs are the only barrier. They are exact
/// only if `cortex_owner` has no default privileges. This cell seeds the worst ordinary configuration BEFORE the migrations and asserts
/// the exact ACLs of the five tables and the two views.
#[tokio::test]
async fn db11_hostile_default_privileges_cannot_widen_the_exact_grants() {
    let w = setup_with(
        "db11",
        "ALTER DEFAULT PRIVILEGES FOR ROLE cortex_owner GRANT ALL ON TABLES TO PUBLIC; \
         ALTER DEFAULT PRIVILEGES FOR ROLE cortex_owner GRANT ALL ON FUNCTIONS TO PUBLIC;",
    )
    .await;
    let mut c = w.db.admin().await;
    // Every grant on a relation of the schema except the owner's own: (relation, grantee, privilege).
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT c.relname::text, CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END, a.privilege_type \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         CROSS JOIN LATERAL aclexplode(coalesce(c.relacl, acldefault('r', c.relowner))) a \
         WHERE n.nspname = 'records' AND c.relkind IN ('r', 'v') AND a.grantee <> c.relowner ORDER BY 1, 2, 3",
    )
    .fetch_all(&mut c)
    .await
    .expect("acls");
    let mut want: Vec<(String, String, String)> = vec![];
    for (rel, role, privs) in [
        ("records", "cortex_m_records", &["INSERT", "SELECT"][..]),
        ("state_log", "cortex_m_records", &["INSERT", "SELECT"][..]),
        (
            "state",
            "cortex_m_records",
            &["INSERT", "SELECT", "UPDATE"][..],
        ),
        (
            "lineage_counters",
            "cortex_m_records",
            &["INSERT", "SELECT", "UPDATE"][..],
        ),
        (
            "sections",
            "cortex_m_records",
            &["INSERT", "SELECT", "UPDATE"][..],
        ),
        ("pub_sources", "cortex_m_search", &["SELECT"][..]),
        ("pub_state", "cortex_m_search", &["SELECT"][..]),
    ] {
        for p in privs {
            want.push((rel.to_owned(), role.to_owned(), (*p).to_owned()));
        }
    }
    want.sort();
    assert_eq!(
        rows, want,
        "the ACLs of the five tables and two views are exactly the pack's, whatever the owner's defaults"
    );
    for (role, rel, privilege, expected) in [
        ("cortex_m_search", "records.records", "SELECT", 0_i64),
        ("cortex_m_search", "records.state", "SELECT", 0),
        ("cortex_m_search", "records.pub_sources", "SELECT", 1),
        ("cortex_m_records", "records.records", "DELETE", 0),
        ("cortex_m_records", "records.records", "UPDATE", 0),
        ("cortex_m_records", "records.state_log", "DELETE", 0),
        ("cortex_m_records", "records.state", "DELETE", 0),
    ] {
        let q =
            format!("SELECT has_table_privilege('{role}', '{rel}', '{privilege}')::int::bigint");
        let got: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(q))
            .fetch_one(&mut c)
            .await
            .expect("privilege");
        assert_eq!(got, expected, "{role} {privilege} on {rel}");
    }
    // And the functions: still no PUBLIC execute (db10's walk, under the hostile default).
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_proc p JOIN pg_namespace ns ON ns.oid = p.pronamespace WHERE ns.nspname = 'records' \
         AND has_function_privilege('cortex_m_search', p.oid, 'EXECUTE')",
    )
    .fetch_one(&mut c)
    .await
    .expect("functions");
    assert_eq!(
        open, 0,
        "the search role executes no function in schema records, even with hostile function defaults"
    );
}
