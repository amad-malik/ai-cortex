//! MS-00, the first test (kai's order): how fast is the keyword query, under the search role, through the `security_barrier` views?
//!
//! **THE RULE OF 0.2.001 (r2, 6 Nov 2026; vera refused the leakproof statement, cox 16:13Z):** the keyword query, unchanged, keeps a p95 of at
//! most 300 ms, COLD and WARM, for a project of up to [`MS00_CAP_RECORDS`] records: the LARGEST measured size that passes, rounded down to a
//! round number, not interpolated. Measured (corpus asserted, unflagged): 20,000 records p95 52.0 ms; 40,000 p95 203.8 ms; **60,000 p95
//! 370.8 ms (fails)**; 80,000 p95 479.0 ms; **100,000 p95 499.2 ms (a stated limit of 0.2.001)**. Why: `FORCE ROW LEVEL SECURITY` plus a
//! `@@` function that is not leakproof keep the `records_tsv` GIN index out of the plan (a Parallel Seq Scan of the project); a faster
//! design is post-0.2.001 and needs its own security review (MS-00-REPORT.md, 'after 0.2.001').
//!
//! **IF THIS CELL FAILS AT THE CAP (cox, 6 Nov 16:49Z):** the margin is thin (two clean runs at 40,000 records gave a cold p95 of 203.8 and 238.0
//! ms and a warm p95 of 199.6 and 266.3 ms, so the worse margin to 300 ms is 11%), and a loaded machine can fail it. Rerun it ONCE on an idle VM
//! (nobody else's database run, nothing else building). Two failures lower the cap one step, to 30,000 records, measured first (the same
//! command with `MS00_ROWS=30000`), and `mod-search-receipts/MS-00-REPORT.md` is restated; no argument.
//!
//! Run with no environment, the cell loads exactly [`MS00_CAP_RECORDS`] records and ASSERTS the rule (about 1.5 minutes): it fails if the
//! query is slower than the cap allows. Run with `MS00_ROWS` listing sizes (for example `MS00_ROWS=40000,60000,80000`) it is a MEASUREMENT of
//! those sizes; the rule is asserted for every listed size at or below the cap, and sizes above it are reported, not asserted.
//!
//! Corpus (the definition's): project A has N records (prose of 200 to 2,000 characters, exactly 1% over 20,000 characters, ASSERTED before
//! timing), project B has 1,000. 100 two-word queries as the search role in project A; `EXPLAIN (ANALYZE, BUFFERS)` of 5 of them; p50 and p95
//! of a COLD pass (the first read after the load and an ANALYZE, with the buffers as the load left them) and of a WARM pass (the same queries again).
//!
//! ```text
//! CV2_DB=1 CV2_ADMIT_LOG=1 cv2-admit timeout --kill-after=120 780 crates/cortex-testkit/supervise.sh \
//!     cargo test -p mod-search --test srch_perf -- --nocapture
//! MS00_ROWS=40000,60000,80000 CV2_DB=1 CV2_ADMIT_LOG=1 cv2-admit timeout --kill-after=120 1500 crates/cortex-testkit/supervise.sh \
//!     cargo test -p mod-search --test srch_perf -- --nocapture
//! ```
mod support;

use std::time::Instant;

use cortex_db::{ModuleTx, TxOptions};
use cortex_testkit::ctx;
use mod_search::query::SQL;
use sqlx::Row;
use support::{World, setup};

const VOCAB: i64 = 5_000;

/// The cap of 0.2.001: records per project up to which the unflagged keyword query keeps p95 <= 300 ms (cold and warm). The largest measured
/// size that passes (40,000: 203.8 ms cold, 199.6 ms warm; 60,000 fails at 370.8 / 379.4), rounded down to a round number; NOT interpolated.
const MS00_CAP_RECORDS: i64 = 40_000;

/// Load `n` records into `project` through the records role, in chunks, with generated prose.
async fn load(w: &World, project: cortex_types::ProjectId, n: i64) {
    let chunk = 10_000;
    let mut done = 0;
    while done < n {
        let take = chunk.min(n - done);
        let mut tx = ModuleTx::begin(&w.pool, ctx("records", Some(project)), TxOptions::default())
            .await
            .expect("begin");
        sqlx::query(
            "INSERT INTO records.records (id, project_id, created_at, type, provenance, author_kind, author_id, written_by, summary, body, \
             summary_sha256, body_sha256, fields_sha256) \
             SELECT g.id, $1::uuid, uuid_extract_timestamp(g.id), 'note', 'worker_written', 'worker', 'perf', $1::uuid, g.summary, g.body, \
                    sha256(convert_to(g.summary, 'UTF8')), sha256(convert_to(g.body, 'UTF8')), sha256('{}'::bytea) \
             FROM (SELECT uuidv7() AS id, \
                          (SELECT string_agg('w' || floor(random() * $4)::int, ' ') FROM generate_series(1, 4 + (i % 5))) AS summary, \
                          (SELECT string_agg('w' || floor(random() * $4)::int, ' ') \
                             FROM generate_series(1, CASE WHEN (i + $3) % 100 = 0 THEN 4500 ELSE 40 + ((i * 37 + $3) % 290) END)) AS body \
                   FROM generate_series(1, $2::int) AS i) g",
        )
        .bind(project.to_string())
        .bind(take)
        .bind(done)
        .bind(VOCAB as f64)
        .execute(tx.executor().expect("executor"))
        .await
        .expect("load a chunk");
        tx.commit().await.expect("commit");
        done += take;
    }
}

/// The corpus as loaded (sam MS7-02): the definition asks for prose of 200 to 2,000 characters with 1% over 20,000. Printed and ASSERTED before
/// any timing, so a generator that cannot produce the tail fails here instead of being measured.
async fn assert_corpus(w: &World, project: cortex_types::ProjectId, n: i64) {
    let mut c = w.db.admin().await;
    let (total, long, short_min, short_max, avg): (
        i64,
        i64,
        Option<i32>,
        Option<i32>,
        Option<f64>,
    ) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE char_length(body) > 20000), \
                min(char_length(body)) FILTER (WHERE char_length(body) <= 20000), \
                max(char_length(body)) FILTER (WHERE char_length(body) <= 20000), \
                avg(char_length(body))::float8 \
         FROM records.records WHERE project_id = $1::uuid",
    )
    .bind(project.to_string())
    .fetch_one(&mut c)
    .await
    .expect("corpus statistics");
    let (short_min, short_max) = (short_min.unwrap_or(0), short_max.unwrap_or(0));
    println!(
        "MS-00 corpus statistics for project {project}: {total} records, bodies over 20,000 characters {long} ({:.2}%), the others {short_min} to {short_max} characters, mean {:.0}",
        100.0 * long as f64 / total as f64,
        avg.unwrap_or(0.0)
    );
    assert_eq!(total, n, "the project holds the records asked for");
    assert_eq!(
        long,
        n / 100,
        "exactly 1% of the bodies are over 20,000 characters"
    );
    assert!(
        short_min >= 200 && short_max <= 2_000,
        "the other bodies are 200 to 2,000 characters: {short_min} to {short_max}"
    );
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    sorted[(((sorted.len() as f64) * p).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1)]
}

/// 100 deterministic two-word queries.
fn queries() -> Vec<String> {
    (0..100u64)
        .map(|i| {
            format!(
                "w{} w{}",
                (i * 7919 + 13) % VOCAB as u64,
                (i * 104_729 + 101) % VOCAB as u64
            )
        })
        .collect()
}

async fn timed(w: &World, project: cortex_types::ProjectId, qs: &[String]) -> Vec<f64> {
    let mut ms = Vec::new();
    for q in qs {
        let mut tx = ModuleTx::begin(&w.pool, ctx("search", Some(project)), TxOptions::default())
            .await
            .expect("begin");
        let t = Instant::now();
        let rows = sqlx::query(SQL)
            .bind(q)
            .bind(None::<Vec<String>>)
            .bind(None::<&str>)
            .bind(None::<&str>)
            .bind(false)
            .bind(10_i64)
            .fetch_all(tx.executor().expect("executor"))
            .await
            .expect("the query runs");
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(rows.len() <= 10);
        tx.commit().await.expect("commit");
    }
    ms
}

/// Measure one corpus size. Returns the findings (empty when the rule holds); the caller asserts after every size has been printed.
async fn one(rows: i64) -> Vec<String> {
    let mut findings = Vec::new();
    let w = setup(&format!("ms00_{rows}")).await;
    let t = Instant::now();
    load(&w, w.a, rows).await;
    load(&w, w.b, 1_000).await;
    let mut admin = w.db.admin().await;
    sqlx::query("ANALYZE records.records")
        .execute(&mut admin)
        .await
        .expect("analyze");
    let sizes: (String, String) = sqlx::query_as(
        "SELECT pg_size_pretty(pg_table_size('records.records')), pg_size_pretty(pg_relation_size('records.records_tsv'))",
    )
    .fetch_one(&mut admin)
    .await
    .expect("sizes");
    println!(
        "MS-00 corpus: project A {rows} records, project B 1000; loaded in {:.1} s; table {}, GIN index {}",
        t.elapsed().as_secs_f64(),
        sizes.0,
        sizes.1
    );
    assert_corpus(&w, w.a, rows).await;
    assert_corpus(&w, w.b, 1_000).await;
    let qs = queries();
    let mut cold = timed(&w, w.a, &qs).await;
    let mut warm = timed(&w, w.a, &qs).await;
    cold.sort_by(f64::total_cmp);
    warm.sort_by(f64::total_cmp);
    println!(
        "MS-00 {rows} records: COLD p50 {:.1} ms p95 {:.1} ms max {:.1} ms | WARM p50 {:.1} ms p95 {:.1} ms max {:.1} ms",
        pct(&cold, 0.5),
        pct(&cold, 0.95),
        cold[cold.len() - 1],
        pct(&warm, 0.5),
        pct(&warm, 0.95),
        warm[warm.len() - 1]
    );
    let mut index_used = true;
    for q in qs.iter().step_by(20) {
        let mut tx = ModuleTx::begin(&w.pool, ctx("search", Some(w.a)), TxOptions::default())
            .await
            .expect("begin");
        let plan: Vec<String> = sqlx::query(sqlx::AssertSqlSafe(format!(
            "EXPLAIN (ANALYZE, BUFFERS) {SQL}"
        ))) // a constant of this crate, not request data
        .bind(q)
        .bind(None::<Vec<String>>)
        .bind(None::<&str>)
        .bind(None::<&str>)
        .bind(false)
        .bind(10_i64)
        .fetch_all(tx.executor().expect("executor"))
        .await
        .expect("explain")
        .iter()
        .map(|r| r.get::<String, _>(0))
        .collect();
        let text = plan.join("\n");
        println!("MS-00 {rows} records, EXPLAIN (ANALYZE, BUFFERS) for {q:?}:\n{text}\n");
        index_used &= text.contains("records_tsv");
        tx.commit().await.expect("commit");
    }
    println!("MS-00 {rows} records: records_tsv GIN index in every plan: {index_used}");
    // The rule holds for every size at or below the cap; sizes above it are measurements (stated limits), not assertions.
    if rows <= MS00_CAP_RECORDS && !(pct(&cold, 0.95) <= 300.0 && pct(&warm, 0.95) <= 300.0) {
        findings.push(format!(
            "p95 above 300 ms at {rows} records, within the cap of {MS00_CAP_RECORDS} (cold {:.1}, warm {:.1})",
            pct(&cold, 0.95),
            pct(&warm, 0.95)
        ));
    }
    if !index_used && rows >= 20_000 {
        findings.push(format!(
            "the records_tsv GIN index is not in the plan at {rows} records"
        ));
    }
    findings
}

/// The corpus sizes to run (sam MS7R2-01). No variable: the cap. Otherwise a NON-EMPTY comma-separated list of whole numbers, each at least 100 (so
/// the 1% of bodies over 20,000 characters exists). A malformed, empty or too small entry is an ERROR, never dropped: a typo must not turn this
/// cell into a run of nothing that passes.
fn sizes_from(var: Option<&str>) -> Result<Vec<i64>, String> {
    let Some(list) = var else {
        return Ok(vec![MS00_CAP_RECORDS]);
    };
    let mut sizes = Vec::new();
    for part in list.split(',') {
        let part = part.trim();
        let n: i64 = part.parse().map_err(|_| {
            format!("MS00_ROWS: {part:?} is not a whole number of records (in {list:?})")
        })?;
        if n < 100 {
            return Err(format!(
                "MS00_ROWS: {n} is below 100 records, too few for the 1% tail (in {list:?})"
            ));
        }
        sizes.push(n);
    }
    Ok(sizes)
}

// ---- sam MS7R2-01: a typo in MS00_ROWS must not turn the cap cell into a no-op that passes ----

#[test]
fn ms00_sizes_with_no_variable_are_the_cap_and_a_valid_list_is_taken_whole() {
    assert_eq!(sizes_from(None), Ok(vec![MS00_CAP_RECORDS]));
    assert_eq!(sizes_from(Some("40000")), Ok(vec![40_000]));
    assert_eq!(
        sizes_from(Some("40000,60000, 80000")),
        Ok(vec![40_000, 60_000, 80_000])
    );
    assert_eq!(sizes_from(Some(" 100000 ")), Ok(vec![100_000]));
}

#[test]
fn ms00_sizes_refuse_every_malformed_empty_or_non_positive_entry() {
    for bad in [
        "not-a-size",
        "",
        " ",
        ",",
        "40000,",
        ",40000",
        "40000,,60000",
        "40000,abc",
        "abc,40000",
        "4e4",
        "40_000",
        "40000.5",
        "0",
        "-5",
        "40000,-5",
        "99",
        "1 2",
        "40000 60000",
        "+",
        "NaN",
        "40000;60000",
    ] {
        let r = sizes_from(Some(bad));
        assert!(r.is_err(), "MS00_ROWS={bad:?} must be an error, got {r:?}");
    }
}

#[test]
fn ms00_sizes_never_come_back_empty() {
    for input in [
        None,
        Some("40000"),
        Some("40000,60000"),
        Some(""),
        Some("x"),
        Some(","),
        Some("0"),
    ] {
        if let Ok(v) = sizes_from(input) {
            assert!(!v.is_empty(), "an Ok list is never empty: {input:?}");
        }
    }
}

#[tokio::test]
async fn ms00_p95_is_within_300_ms_up_to_the_cap_of_40000_records() {
    let var = match std::env::var("MS00_ROWS") {
        Ok(v) => Some(v),
        Err(std::env::VarError::NotPresent) => None,
        Err(e) => panic!("MS00_ROWS: {e}"),
    };
    let sizes = sizes_from(var.as_deref()).unwrap_or_else(|e| panic!("{e}"));
    let mut findings = Vec::new();
    for n in sizes {
        findings.extend(one(n).await);
    }
    for f in &findings {
        println!("MS-00 FINDING: {f}");
    }
    // A plan without the GIN index is reported (it is the known cause above the cap), but it is not the rule.
    let p95_failed = findings.iter().any(|f| f.starts_with("p95"));
    assert!(
        !p95_failed,
        "the keyword query is slower than the 0.2.001 cap allows: {findings:?}"
    );
}
