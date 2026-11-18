//! The keyword query over the two S2 revision 8 views, and the bounded way to run it.
//!
//! - **What it reads:** `records.pub_sources` and `records.pub_state` only (the role `cortex_m_search` holds SELECT on nothing else). Both
//!   views filter on the project setting themselves and fail closed, so the token's project is the only project a search can see.
//! - **The query:** `websearch_to_tsquery('english', $q)`: the words are ANDed, a quoted phrase keeps its order, `-word` excludes, `or`
//!   widens; it never raises a syntax error and never takes raw tsquery text (S4 §7.1). Rank is `ts_rank` (summary weight A over body weight
//!   B); the order is the contract's: rank descending, then `created_at` descending, then id.
//! - **Hidden always:** a `drill-canary` record (`hidden_by_default`) and a record past its `valid_until` (the injected clock). Not active
//!   records (superseded, invalidated) are left out unless `include_history`.
//! - **Bounds (v1 incident 912073e2):** a statement timeout, a whole-request budget, and the cap of two, all of which answer 503 or 429 at
//!   once rather than queue.
//! - **O-1:** this module never calls `RecordsApi::changed_since` (it orders by creation, not by commit); it has no index to feed.
//!
//! INTERIM(ponytail): AND-only websearch keyword query, English config, replaced in 0.2.002.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use cortex_db::{DbError, DbPool, ModuleTx, TxContext, TxOptions};
use cortex_module::Problem;
use sqlx::Row;

use crate::cap::Cap;
use crate::response::{Hit, SearchResult};
use crate::validate::Validated;

/// The three limits v1 learned the hard way (incident 912073e2): a statement timeout (v1 3,000 ms) and a whole-request budget (v1 4,500 ms).
/// The third, the cap of two, is [`Cap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub statement_timeout: Duration,
    pub budget: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            statement_timeout: Duration::from_millis(3_000),
            budget: Duration::from_millis(4_500),
        }
    }
}

/// `$1` the query, `$2` the types (NULL: all), `$3` since, `$4` until (text, already checked as RFC 3339 and parsed by the database; a
/// text parameter is never compared with a timestamp without the cast), `$5` include history, `$6` the limit.
pub const SQL: &str = "\
WITH q AS (SELECT websearch_to_tsquery('english', $1) AS tq) \
SELECT s.id::text AS id, s.type, s.summary, ts_rank(s.tsv, q.tq)::float8 AS rank, \
       to_char(s.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at, \
       coalesce(st.state, 'active') AS state, s.tsv_truncated AS truncated \
FROM records.pub_sources s \
CROSS JOIN q \
LEFT JOIN records.pub_state st ON st.record_id = s.id \
WHERE s.tsv @@ q.tq \
  AND NOT s.hidden_by_default \
  AND (s.valid_until IS NULL OR s.valid_until > platform.now()) \
  AND ($2::text[] IS NULL OR s.type = ANY($2)) \
  AND ($3::text IS NULL OR s.created_at >= $3::text::timestamptz) \
  AND ($4::text IS NULL OR s.created_at < $4::text::timestamptz) \
  AND ($5::boolean OR st.record_id IS NULL) \
ORDER BY rank DESC, s.created_at DESC, s.id \
LIMIT $6";

/// Run the search inside the caller's transaction (the search role, the token's project). No bound is applied here: [`answer`] adds them.
pub async fn run(tx: &mut ModuleTx, v: &Validated) -> Result<Vec<Hit>, Problem> {
    let types: Option<Vec<String>> =
        (!v.types.is_empty()).then(|| v.types.iter().map(|t| t.as_str().to_owned()).collect());
    let rows = sqlx::query(SQL)
        .bind(&v.query)
        .bind(types)
        .bind(v.since.as_deref())
        .bind(v.until.as_deref())
        .bind(v.history)
        .bind(i64::from(v.limit))
        .fetch_all(tx.executor().map_err(Problem::from)?)
        .await
        .map_err(db)?;
    rows.iter().map(hit).collect()
}

fn hit(r: &sqlx::postgres::PgRow) -> Result<Hit, Problem> {
    let get = |e: sqlx::Error| db(e);
    Ok(Hit {
        id: r.try_get("id").map_err(get)?,
        r#type: r.try_get("type").map_err(get)?,
        summary: r.try_get("summary").map_err(get)?,
        rank: r.try_get("rank").map_err(get)?,
        created_at: r.try_get("created_at").map_err(get)?,
        state: r.try_get("state").map_err(get)?,
        truncated: r.try_get("truncated").map_err(get)?,
    })
}

/// 503 `unavailable` with search's own text (a platform code, minted by `cortex-http`).
pub fn unavailable() -> Problem {
    Problem {
        code: "unavailable",
        status: 503,
        title: "Unavailable",
        detail: Cow::Borrowed(
            "Search cannot read the records right now or did not answer in time.",
        ),
        fix: "Retry later; if it repeats, report the request ID.",
        scope: None,
    }
}

/// A database error by kind. A search that cannot read the records views (no privilege, the view missing) is 503, never an empty success:
/// "every channel failed" is not an empty answer (MS-18). Everything else follows the platform's mapping.
fn db(e: sqlx::Error) -> Problem {
    if let sqlx::Error::Database(d) = &e
        && matches!(d.code().as_deref(), Some("42501" | "42P01"))
    {
        return unavailable();
    }
    Problem::from(DbError::from(e))
}

/// The whole answer, in the order S3 R-16 gives search: the caller has checked the request's bounds (422), THEN the cap (429), before any
/// database statement; then one read-only transaction under a statement timeout, all inside the request budget.
///
/// **The database work runs in its own task and keeps its place of the cap until the statement is over.** When the budget runs out, or the
/// client goes away and this future is dropped, the answer is 503 or nothing at once, but the statement the database is running is NOT
/// cancelled: dropping a `ModuleTx` mid-statement leaves its backend running (cortex-db has no cancel-on-drop; found by cells ms16 and ms17,
/// reported to cox and finn). So the place stays taken until the statement ends, and the statement timeout (v1: 3 s) bounds that: the cap
/// of two then still bounds the backends this route can have, and nothing leaks beyond one statement timeout.
pub async fn answer(
    pool: &Arc<DbPool>,
    tctx: TxContext,
    cap: &Cap,
    limits: &Limits,
    v: &Validated,
) -> Result<SearchResult, Problem> {
    let place = cap.try_enter()?;
    let hits = if v.empty_interval() {
        Vec::new()
    } else {
        let (pool, v, statement_timeout) = (Arc::clone(pool), v.clone(), limits.statement_timeout);
        let work = tokio::spawn(async move {
            let _place = place; // dropped when the statement is over, whatever became of the request
            let mut tx = ModuleTx::begin(
                &pool,
                tctx,
                TxOptions {
                    statement_timeout: Some(statement_timeout),
                },
            )
            .await
            .map_err(db_error)?;
            let hits = run(&mut tx, &v).await?;
            tx.commit().await.map_err(db_error)?;
            Ok::<_, Problem>(hits)
        });
        match tokio::time::timeout(limits.budget, work).await {
            Ok(Ok(r)) => r?,
            // The task panicked: nothing of the request is trusted.
            Ok(Err(_)) => return Err(internal()),
            Err(_) => return Err(unavailable()),
        }
    };
    Ok(SearchResult {
        hits,
        mode: "keyword",
    })
}

/// 500 `internal_error` (a platform code), fixed text.
fn internal() -> Problem {
    Problem {
        code: "internal_error",
        status: 500,
        title: "Internal error",
        detail: Cow::Borrowed("The server could not complete the request."),
        fix: "Retry later; if it repeats, report the request ID.",
        scope: None,
    }
}

/// A `DbError` as a problem, with a missing privilege read as an unreadable dependency (503), as [`db`] does for a statement.
fn db_error(e: DbError) -> Problem {
    match e {
        DbError::PermissionDenied => unavailable(),
        other => Problem::from(other),
    }
}
