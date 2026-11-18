//! The piece's SQL, in its own schema, for the read side and the claim. Every statement is a literal with
//! bind parameters (sqlx 0.9's `SqlSafeStr`), run on the caller's transaction (`ModuleTx::executor`), under
//! the piece's role and the transaction's project (row-level security): another project's row is invisible.
//!
//! Types are read as text (ids, timestamps, `jsonb` arrays) because the workspace's `sqlx` has no `uuid`
//! decode into our id types, no `chrono` and no `json` feature; each parse failure is an internal error that
//! carries no row data.

use cortex_db::{DbError, ModuleTx};
use cortex_module::Problem;
use cortex_types::{HandoffId, WorkerId};
use sqlx::Row;
use sqlx::postgres::PgRow;

use crate::body::Kind;
use crate::rules::State;
use crate::validate::To;

/// A stored value that does not fit its type: the CHECKs should have refused it. Internal; no row data.
pub(crate) fn bad_row<E>(_e: E) -> DbError {
    DbError::Other("mod_handoffs: a stored value does not fit its type".into())
}

/// A handoff row, as the read side and the claim see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row_ {
    pub id: HandoffId,
    pub kind: Kind,
    pub state: State,
    pub created_by: WorkerId,
    pub to: To,
    pub holder: Option<WorkerId>,
    pub rework_holder: Option<WorkerId>,
    pub summary: String,
    pub next_action: String,
    pub acceptance: String,
    pub verify: Option<String>,
    pub context: Option<String>,
    pub files: Vec<String>,
    pub blockers: Vec<String>,
    pub decisions: Vec<String>,
    pub priority: i32,
    pub due_at: Option<String>,
    pub round: i32,
    pub created_at: String,
    pub closed_at: Option<String>,
    pub close_reason: Option<String>,
    pub transition_seq: i32,
}

/// The one column list of a handoff row (23 columns, in the order [`Row_::from_pg`] reads them). The two-argument
/// form appends `$extra` after the 23rd column (a 24th, which the caller reads by index).
macro_rules! row_sql {
    ($tail:literal) => {
        row_sql!("", $tail)
    };
    ($extra:literal, $tail:literal) => {
        concat!(
            "SELECT h.id::text, h.kind, h.state, h.created_by::text, h.to_kind, h.to_ref, h.holder::text, \
             h.rework_holder::text, h.summary, h.next_action, h.acceptance, h.verify, h.context, \
             coalesce(h.files, '[]'::jsonb)::text, coalesce(h.blockers, '[]'::jsonb)::text, \
             coalesce(h.decisions, '[]'::jsonb)::text, h.priority, \
             to_char(h.due_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), h.round, \
             to_char(h.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
             to_char(h.closed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
             h.close_reason, h.transition_seq",
            $extra,
            " FROM handoffs.handoffs h",
            $tail
        )
    };
}

fn strings(json: &str) -> Result<Vec<String>, DbError> {
    serde_json::from_str(json).map_err(bad_row)
}

impl Row_ {
    pub(crate) fn from_pg(r: &PgRow) -> Result<Row_, DbError> {
        let s =
            |i: usize| -> Result<String, DbError> { r.try_get::<String, _>(i).map_err(bad_row) };
        let o = |i: usize| -> Result<Option<String>, DbError> {
            r.try_get::<Option<String>, _>(i).map_err(bad_row)
        };
        let worker = |t: String| WorkerId::parse(&t).map_err(bad_row);
        let kind = match s(1)?.as_str() {
            "task" => Kind::Task,
            "consult" => Kind::Consult,
            _ => return Err(bad_row(())),
        };
        let to = match (s(4)?.as_str(), s(5)?) {
            ("worker", id) => To::Worker(worker(id)?),
            ("job_role", role) => To::JobRole(role),
            _ => return Err(bad_row(())),
        };
        Ok(Row_ {
            id: HandoffId::parse(&s(0)?).map_err(bad_row)?,
            kind,
            state: State::parse(&s(2)?).ok_or_else(|| bad_row(()))?,
            created_by: worker(s(3)?)?,
            to,
            holder: o(6)?.map(worker).transpose()?,
            rework_holder: o(7)?.map(worker).transpose()?,
            summary: s(8)?,
            next_action: s(9)?,
            acceptance: s(10)?,
            verify: o(11)?,
            context: o(12)?,
            files: strings(&s(13)?)?,
            blockers: strings(&s(14)?)?,
            decisions: strings(&s(15)?)?,
            priority: r.try_get::<i32, _>(16).map_err(bad_row)?,
            due_at: o(17)?,
            round: r.try_get::<i32, _>(18).map_err(bad_row)?,
            created_at: s(19)?,
            closed_at: o(20)?,
            close_reason: o(21)?,
            transition_seq: r.try_get::<i32, _>(22).map_err(bad_row)?,
        })
    }
}

/// A handoff by id, in the transaction's project (another project's id is `None`).
pub async fn load(tx: &mut ModuleTx, id: HandoffId) -> Result<Option<Row_>, DbError> {
    let row = sqlx::query(row_sql!(" WHERE h.id = $1::uuid"))
        .bind(id.to_string())
        .fetch_optional(tx.executor()?)
        .await?;
    row.as_ref().map(Row_::from_pg).transpose()
}

/// The same, locked for the rest of the transaction: the write phase's single point of serialisation.
pub async fn load_for_update(tx: &mut ModuleTx, id: HandoffId) -> Result<Option<Row_>, DbError> {
    let row = sqlx::query(row_sql!(" WHERE h.id = $1::uuid FOR UPDATE"))
        .bind(id.to_string())
        .fetch_optional(tx.executor()?)
        .await?;
    row.as_ref().map(Row_::from_pg).transpose()
}

/// One evidence entry as stored: no inline bytes ever leave the table through the read side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRef_ {
    pub kind: String,
    pub reference: Option<String>,
    pub sha256: String,
    pub size: i32,
}

/// The latest return of a handoff, with its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnReport_ {
    pub round: i32,
    pub author: WorkerId,
    pub summary: String,
    pub risks: String,
    pub tests: Vec<String>,
    pub evidence: Vec<EvidenceRef_>,
    pub at: String,
}

pub async fn latest_return(
    tx: &mut ModuleTx,
    id: HandoffId,
) -> Result<Option<ReturnReport_>, DbError> {
    let row = sqlx::query(
        "SELECT r.id::text, r.round, r.author::text, r.summary, r.risks, coalesce(r.tests, '[]'::jsonb)::text, \
         to_char(r.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM handoffs.returns r WHERE r.handoff_id = $1::uuid ORDER BY r.round DESC LIMIT 1",
    )
    .bind(id.to_string())
    .fetch_optional(tx.executor()?)
    .await?;
    let Some(r) = row else { return Ok(None) };
    let return_id: String = r.try_get(0).map_err(bad_row)?;
    let evidence = sqlx::query(
        "SELECT e.kind, e.ref, encode(e.sha256, 'hex'), e.size FROM handoffs.evidence e \
         WHERE e.return_id = $1::uuid ORDER BY e.id",
    )
    .bind(return_id)
    .fetch_all(tx.executor()?)
    .await?
    .iter()
    .map(|e| {
        Ok(EvidenceRef_ {
            kind: e.try_get(0).map_err(bad_row)?,
            reference: e.try_get(1).map_err(bad_row)?,
            sha256: e.try_get(2).map_err(bad_row)?,
            size: e.try_get(3).map_err(bad_row)?,
        })
    })
    .collect::<Result<Vec<_>, DbError>>()?;
    Ok(Some(ReturnReport_ {
        round: r.try_get(1).map_err(bad_row)?,
        author: WorkerId::parse(&r.try_get::<String, _>(2).map_err(bad_row)?).map_err(bad_row)?,
        summary: r.try_get(3).map_err(bad_row)?,
        risks: r.try_get(4).map_err(bad_row)?,
        tests: strings(&r.try_get::<String, _>(5).map_err(bad_row)?)?,
        evidence,
        at: r.try_get(6).map_err(bad_row)?,
    }))
}

/// The latest review of a handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review_ {
    pub decision: String,
    pub by: WorkerId,
    pub at: String,
    pub notes: Option<String>,
    pub self_review: bool,
}

pub async fn latest_review(tx: &mut ModuleTx, id: HandoffId) -> Result<Option<Review_>, DbError> {
    let row = sqlx::query(
        "SELECT v.verdict, v.reviewer::text, \
         to_char(v.at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), v.notes, v.self_review \
         FROM handoffs.reviews v WHERE v.handoff_id = $1::uuid ORDER BY v.round DESC LIMIT 1",
    )
    .bind(id.to_string())
    .fetch_optional(tx.executor()?)
    .await?;
    row.map(|r| {
        Ok(Review_ {
            decision: r.try_get(0).map_err(bad_row)?,
            by: WorkerId::parse(&r.try_get::<String, _>(1).map_err(bad_row)?).map_err(bad_row)?,
            at: r.try_get(2).map_err(bad_row)?,
            notes: r.try_get(3).map_err(bad_row)?,
            self_review: r.try_get(4).map_err(bad_row)?,
        })
    })
    .transpose()
}

/// Who closed a handoff, from the closing row of the transition log: `(actor_kind, actor_id)`.
pub async fn closed_by(
    tx: &mut ModuleTx,
    id: HandoffId,
) -> Result<Option<(String, String)>, DbError> {
    let row = sqlx::query(
        "SELECT t.actor_kind, t.actor_id FROM handoffs.transitions t \
         WHERE t.handoff_id = $1::uuid AND t.to_state IN ('accepted', 'cancelled') ORDER BY t.seq DESC LIMIT 1",
    )
    .bind(id.to_string())
    .fetch_optional(tx.executor()?)
    .await?;
    row.map(|r| {
        Ok((
            r.try_get(0).map_err(bad_row)?,
            r.try_get(1).map_err(bad_row)?,
        ))
    })
    .transpose()
}

/// What a list asks for. `me` and `roles` are the caller's, read through `IdentityApi` before the query.
pub struct ListFilter<'a> {
    pub state: Option<&'a str>,
    pub holder: Option<WorkerId>,
    pub created_by: Option<WorkerId>,
    pub mine: bool,
    pub me: WorkerId,
    pub me_active: bool,
    pub roles: &'a [String],
    pub lead: WorkerId,
    /// The keyset position: `(priority, created_at, id)` of the last item of the previous page.
    pub after: Option<(i32, &'a str, HandoffId)>,
    pub limit: i64,
}

/// The list: every handoff of the project in `x-order` (priority descending, then `created_at`, then `id`),
/// filtered; with `mine`, only the ones the caller holds, may claim, may review, or created and had
/// cancelled by someone else in the last 7 days. The claim predicate is the one in `rules::may_claim`.
pub async fn list(tx: &mut ModuleTx, f: &ListFilter<'_>) -> Result<Vec<Row_>, Problem> {
    let (ap, ac, ai) = match &f.after {
        Some((p, c, i)) => (Some(*p), Some((*c).to_owned()), Some(i.to_string())),
        None => (None, None, None),
    };
    let rows = sqlx::query(row_sql!(
        " LEFT JOIN LATERAL (SELECT r.author FROM handoffs.returns r WHERE r.handoff_id = h.id \
             ORDER BY r.round DESC LIMIT 1) lr ON true \
          LEFT JOIN LATERAL (SELECT t.actor_id FROM handoffs.transitions t WHERE t.handoff_id = h.id \
             AND t.to_state IN ('accepted', 'cancelled') ORDER BY t.seq DESC LIMIT 1) closer ON true \
          WHERE ($1::text IS NULL OR h.state = $1) \
            AND ($2::text IS NULL OR h.holder = $2::uuid) \
            AND ($3::text IS NULL OR h.created_by = $3::uuid) \
            AND (NOT $4::boolean OR ( \
                 (h.state = 'claimed' AND h.holder = $5::uuid) \
              OR (h.state = 'pending' AND $6::boolean AND ( \
                    (h.rework_holder IS NOT NULL AND h.rework_holder = $5::uuid) \
                 OR (h.rework_holder IS NULL AND ((h.to_kind = 'worker' AND h.to_ref = $5::text) \
                                               OR (h.to_kind = 'job_role' AND h.to_ref = ANY($7::text[])))))) \
              OR (h.state = 'returned' AND (h.created_by = $5::uuid OR $5::uuid = $8::uuid) \
                    AND NOT (lr.author = $5::uuid AND NOT (h.created_by = $5::uuid AND $5::uuid = $8::uuid))) \
              OR (h.state = 'cancelled' AND h.created_by = $5::uuid \
                    AND h.closed_at > platform.now() - interval '7 days' AND closer.actor_id <> $5::text))) \
            AND ($9::int IS NULL OR h.priority < $9 OR (h.priority = $9 \
                 AND (h.created_at > $10::timestamptz OR (h.created_at = $10::timestamptz AND h.id > $11::uuid)))) \
          ORDER BY h.priority DESC, h.created_at ASC, h.id ASC LIMIT $12"
    ))
    .bind(f.state.map(str::to_owned))
    .bind(f.holder.map(|w| w.to_string()))
    .bind(f.created_by.map(|w| w.to_string()))
    .bind(f.mine)
    .bind(f.me.to_string())
    .bind(f.me_active)
    .bind(f.roles.to_vec())
    .bind(f.lead.to_string())
    .bind(ap)
    .bind(ac)
    .bind(ai)
    .bind(f.limit)
    .fetch_all(tx.executor()?)
    .await
    .map_err(crate::problem::time_db)?;
    rows.iter()
        .map(Row_::from_pg)
        .collect::<Result<Vec<_>, DbError>>()
        .map_err(Problem::from)
}

/// The worker's CLAIMED handoffs for `HandoffsApi::claimed_by`, newest claim first, then id (W1-33), with each row's
/// `created_at` in microseconds. A claimed row is written only when it leaves `claimed` (lease renewal is not
/// built in 0.2.001), so its `updated_at` is its claim time; a post-.001 lease renewal must give the claim time
/// its own column before it touches `updated_at`.
pub async fn claimed_by(
    tx: &mut ModuleTx,
    w: WorkerId,
    limit: i64,
) -> Result<Vec<(Row_, i64)>, DbError> {
    let rows = sqlx::query(row_sql!(
        ", (extract(epoch FROM h.created_at) * 1000000)::bigint",
        " WHERE h.state = 'claimed' AND h.holder = $1::uuid \
          ORDER BY h.updated_at DESC, h.id ASC LIMIT $2"
    ))
    .bind(w.to_string())
    .bind(limit)
    .fetch_all(tx.executor()?)
    .await?;
    rows.iter()
        .map(|r| Ok((Row_::from_pg(r)?, r.try_get::<i64, _>(23).map_err(bad_row)?)))
        .collect()
}

/// The PENDING handoffs `w` may claim for `HandoffsApi::pending_for` (W1-33), in `x-order` (priority descending,
/// then `created_at`, then id): after a rework only the rework holder; otherwise the worker address or a held
/// job role. An inactive worker may claim nothing. Each row comes with its `created_at` in microseconds.
pub async fn pending_for(
    tx: &mut ModuleTx,
    w: WorkerId,
    active: bool,
    roles: &[String],
    limit: i64,
) -> Result<Vec<(Row_, i64)>, DbError> {
    let rows = sqlx::query(row_sql!(
        ", (extract(epoch FROM h.created_at) * 1000000)::bigint",
        " WHERE h.state = 'pending' AND $2::boolean AND ( \
              (h.rework_holder IS NOT NULL AND h.rework_holder = $1::uuid) \
           OR (h.rework_holder IS NULL AND ((h.to_kind = 'worker' AND h.to_ref = $1::text) \
                                         OR (h.to_kind = 'job_role' AND h.to_ref = ANY($3::text[]))))) \
          ORDER BY h.priority DESC, h.created_at ASC, h.id ASC LIMIT $4"
    ))
    .bind(w.to_string())
    .bind(active)
    .bind(roles.to_vec())
    .bind(limit)
    .fetch_all(tx.executor()?)
    .await?;
    rows.iter()
        .map(|r| Ok((Row_::from_pg(r)?, r.try_get::<i64, _>(23).map_err(bad_row)?)))
        .collect()
}

/// The claim's write, on a row this transaction holds locked: the new `transition_seq` and `incarnation`.
pub async fn claim_write(
    tx: &mut ModuleTx,
    id: HandoffId,
    by: WorkerId,
) -> Result<(i32, i64), DbError> {
    let row = sqlx::query(
        "UPDATE handoffs.handoffs SET state = 'claimed', holder = $2::uuid, incarnation = incarnation + 1, \
         lease_renewed_at = platform.now(), updated_at = platform.now(), transition_seq = transition_seq + 1 \
         WHERE id = $1::uuid AND state = 'pending' RETURNING transition_seq, incarnation",
    )
    .bind(id.to_string())
    .bind(by.to_string())
    .fetch_one(tx.executor()?)
    .await?;
    Ok((
        row.try_get(0).map_err(bad_row)?,
        row.try_get(1).map_err(bad_row)?,
    ))
}

/// One row of the transition log (S2 `handoffs.transitions`), actor kind `worker`.
pub struct LogRow<'a> {
    pub id: HandoffId,
    pub seq: i32,
    pub from: Option<&'a str>,
    pub to: &'a str,
    pub actor: WorkerId,
    pub incarnation: i64,
    pub reason: Option<&'a str>,
}

pub async fn log_transition(tx: &mut ModuleTx, r: &LogRow<'_>) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO handoffs.transitions (id, handoff_id, project_id, seq, from_state, to_state, actor_kind, \
         actor_id, incarnation, reason) \
         VALUES ($1::uuid, $2::uuid, nullif(current_setting('cortex.project_id', true), '')::uuid, $3, $4, $5, \
         'worker', $6, $7, $8)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(r.id.to_string())
    .bind(r.seq)
    .bind(r.from)
    .bind(r.to)
    .bind(r.actor.to_string())
    .bind(r.incarnation)
    .bind(r.reason)
    .execute(tx.executor()?)
    .await?;
    Ok(())
}

/// Insert a new handoff (`pending`, round 1, `transition_seq` 1, incarnation 0) and return nothing: the log
/// row for the creation is [`log_transition`]'s. Empty `files`, `blockers` and `decisions` are stored as NULL
/// (they count nothing against S2's aggregate CHECK, which counts the `jsonb` text of the arrays).
pub async fn insert_handoff(
    tx: &mut ModuleTx,
    id: HandoffId,
    creator: WorkerId,
    ok: &crate::validate::CreateOk,
) -> Result<(), Problem> {
    let (to_kind, to_ref) = match &ok.to {
        To::Worker(w) => ("worker", w.to_string()),
        To::JobRole(r) => ("job_role", r.clone()),
    };
    let arr = |v: &[String]| -> Result<Option<String>, DbError> {
        if v.is_empty() {
            Ok(None)
        } else {
            serde_json::to_string(v).map(Some).map_err(bad_row)
        }
    };
    sqlx::query(
        "INSERT INTO handoffs.handoffs (id, project_id, kind, state, created_by, to_kind, to_ref, priority, \
         summary, next_action, acceptance, verify, context, files, blockers, decisions, due_at, round, transition_seq) \
         VALUES ($1::uuid, nullif(current_setting('cortex.project_id', true), '')::uuid, $2, 'pending', $3::uuid, \
         $4, $5, $6, $7, $8, $9, $10, $11, $12::jsonb, $13::jsonb, $14::jsonb, $15::timestamptz, 1, 1)",
    )
    .bind(id.to_string())
    .bind(ok.kind.as_str())
    .bind(creator.to_string())
    .bind(to_kind)
    .bind(to_ref)
    .bind(ok.priority)
    .bind(&ok.summary)
    .bind(&ok.next_action)
    .bind(&ok.acceptance)
    .bind(&ok.verify)
    .bind(&ok.context)
    .bind(arr(&ok.files)?)
    .bind(arr(&ok.blockers)?)
    .bind(arr(&ok.decisions)?)
    .bind(&ok.due_at)
    .execute(tx.executor()?)
    .await
    .map_err(crate::problem::time_db)?;
    Ok(())
}

/// The latest return's author, for the self-review rule (known once a return exists).
pub async fn latest_return_author(
    tx: &mut ModuleTx,
    id: HandoffId,
) -> Result<Option<WorkerId>, DbError> {
    let a: Option<String> = sqlx::query_scalar(
        "SELECT r.author::text FROM handoffs.returns r WHERE r.handoff_id = $1::uuid ORDER BY r.round DESC LIMIT 1",
    )
    .bind(id.to_string())
    .fetch_optional(tx.executor()?)
    .await?;
    a.map(|t| WorkerId::parse(&t).map_err(bad_row)).transpose()
}

/// A return: the `returns` row, its evidence in order, and the move to `returned`. The row is held locked.
/// Returns the new `transition_seq`.
pub async fn return_write(
    tx: &mut ModuleTx,
    row: &Row_,
    author: WorkerId,
    incarnation: i64,
    ok: &crate::validate::ReturnOk,
) -> Result<i32, DbError> {
    let return_id = uuid::Uuid::now_v7().to_string();
    let tests = serde_json::to_string(&ok.tests).map_err(bad_row)?;
    sqlx::query(
        "INSERT INTO handoffs.returns (id, handoff_id, project_id, round, author, incarnation, summary, risks, tests) \
         VALUES ($1::uuid, $2::uuid, nullif(current_setting('cortex.project_id', true), '')::uuid, $3, $4::uuid, $5, \
         $6, $7, $8::jsonb)",
    )
    .bind(&return_id)
    .bind(row.id.to_string())
    .bind(row.round)
    .bind(author.to_string())
    .bind(incarnation)
    .bind(&ok.summary)
    .bind(&ok.risks)
    .bind(tests)
    .execute(tx.executor()?)
    .await?;
    for e in &ok.evidence {
        sqlx::query(
            "INSERT INTO handoffs.evidence (id, return_id, project_id, kind, ref, inline, sha256, size) \
             VALUES ($1::uuid, $2::uuid, nullif(current_setting('cortex.project_id', true), '')::uuid, $3, $4, $5, $6, $7)",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(&return_id)
        .bind(e.kind.as_str())
        .bind(&e.reference)
        .bind(&e.inline)
        .bind(e.sha256.0.to_vec())
        .bind(i32::try_from(e.size).map_err(bad_row)?)
        .execute(tx.executor()?)
        .await?;
    }
    let seq: i32 = sqlx::query_scalar(
        "UPDATE handoffs.handoffs SET state = 'returned', updated_at = platform.now(), \
         transition_seq = transition_seq + 1 WHERE id = $1::uuid AND state = 'claimed' RETURNING transition_seq",
    )
    .bind(row.id.to_string())
    .fetch_one(tx.executor()?)
    .await?;
    Ok(seq)
}

/// A review row (`accept` or `rework`) for the handoff's current round.
pub async fn review_write(
    tx: &mut ModuleTx,
    id: HandoffId,
    round: i32,
    reviewer: WorkerId,
    verdict: &str,
    notes: Option<&str>,
    self_review: bool,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO handoffs.reviews (id, handoff_id, project_id, round, reviewer, verdict, notes, self_review) \
         VALUES ($1::uuid, $2::uuid, nullif(current_setting('cortex.project_id', true), '')::uuid, $3, $4::uuid, \
         $5, $6, $7)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(id.to_string())
    .bind(round)
    .bind(reviewer.to_string())
    .bind(verdict)
    .bind(notes)
    .bind(self_review)
    .execute(tx.executor()?)
    .await?;
    Ok(())
}

/// Accept: `returned` to `accepted`, closed now. Returns the new `transition_seq`.
pub async fn accept_write(tx: &mut ModuleTx, id: HandoffId) -> Result<i32, DbError> {
    Ok(sqlx::query_scalar(
        "UPDATE handoffs.handoffs SET state = 'accepted', closed_at = platform.now(), updated_at = platform.now(), \
         transition_seq = transition_seq + 1 WHERE id = $1::uuid AND state = 'returned' RETURNING transition_seq",
    )
    .bind(id.to_string())
    .fetch_one(tx.executor()?)
    .await?)
}

/// Rework: `returned` back to `pending`, round + 1, the holder cleared and `rework_holder` set to the worker
/// who returned it (the address stays as created). Returns the new `(transition_seq, round)`.
pub async fn rework_write(
    tx: &mut ModuleTx,
    id: HandoffId,
    returner: WorkerId,
) -> Result<(i32, i32), DbError> {
    let row = sqlx::query(
        "UPDATE handoffs.handoffs SET state = 'pending', holder = NULL, rework_holder = $2::uuid, round = round + 1, \
         updated_at = platform.now(), transition_seq = transition_seq + 1 \
         WHERE id = $1::uuid AND state = 'returned' RETURNING transition_seq, round",
    )
    .bind(id.to_string())
    .bind(returner.to_string())
    .fetch_one(tx.executor()?)
    .await?;
    Ok((
        row.try_get(0).map_err(bad_row)?,
        row.try_get(1).map_err(bad_row)?,
    ))
}

/// Release: `claimed` back to `pending`, the holder and the rework lock cleared (Q-17).
pub async fn release_write(tx: &mut ModuleTx, id: HandoffId) -> Result<i32, DbError> {
    Ok(sqlx::query_scalar(
        "UPDATE handoffs.handoffs SET state = 'pending', holder = NULL, rework_holder = NULL, \
         updated_at = platform.now(), transition_seq = transition_seq + 1 \
         WHERE id = $1::uuid AND state = 'claimed' RETURNING transition_seq",
    )
    .bind(id.to_string())
    .fetch_one(tx.executor()?)
    .await?)
}

/// Cancel: any open state to `cancelled`, closed now, with the reason.
pub async fn cancel_write(tx: &mut ModuleTx, id: HandoffId, reason: &str) -> Result<i32, DbError> {
    Ok(sqlx::query_scalar(
        "UPDATE handoffs.handoffs SET state = 'cancelled', closed_at = platform.now(), close_reason = $2, \
         updated_at = platform.now(), transition_seq = transition_seq + 1 \
         WHERE id = $1::uuid AND state IN ('pending', 'claimed', 'returned') RETURNING transition_seq",
    )
    .bind(id.to_string())
    .bind(reason)
    .fetch_one(tx.executor()?)
    .await?)
}
