//! The database side of piece 6: every function runs in the caller's `ModuleTx` (the project is the transaction's). The order
//! of refusals is R-16 as written in the S3 YAML: STEP 0 first (every check that needs no database read, 422), then 404 (the path
//! resource, then body references), 403 (the target rule, the section rule), the idempotency lookup, 409, then the write.

use std::collections::BTreeSet;

use cortex_db::{DbError, ModuleTx};
use cortex_module::{AuditDetails, AuditSink, AuditTarget, Problem, Role, SectionName, WorkerKind};
use cortex_types::{Clock, ProjectId, RecordId, WorkerId, canonical_json};
use serde::Serialize;
use sqlx::Row;

use crate::request::{RecordCreate, RecordType, SourceKind};
use crate::validate::{Validated, validate_create};

/// Who is writing: taken from the chain's `Principal`.
#[derive(Debug, Clone)]
pub struct Actor {
    pub worker: WorkerId,
    pub kind: WorkerKind,
    pub role: Role,
}

/// What a store function needs besides the transaction.
pub struct Env<'a> {
    pub project: ProjectId,
    pub clock: &'a dyn Clock,
    pub audit: &'a dyn AuditSink,
}

/// `RecordRef` (S3): the answer to a write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Created {
    pub id: RecordId,
    pub created_at: String,
    pub section_revision: Option<i32>,
}

/// The lineage block of a record that has a state row (S3 `Lineage`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lineage {
    pub reason: String,
    pub by_kind: String,
    pub by_id: String,
    pub at: String,
}

/// `Record` (S3): one record with its state and lineage.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecordOut {
    pub id: RecordId,
    #[serde(rename = "type")]
    pub kind: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub fields: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceOut>,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<RecordId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage: Option<Lineage>,
    pub created_at: String,
    pub author_kind: String,
    pub author: String,
    pub provenance: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<RecordId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_revision: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceOut {
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<String>,
}

/// `SectionPointer` (S3): the CURRENT revision of a reserved section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionPointer {
    pub section: String,
    pub record_id: RecordId,
    pub revision: i32,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

/// RFC 3339 with microseconds and a `Z`, made by the database from a timestamptz column (no time crate needed).
macro_rules! ts {
    ($c:literal) => {
        concat!(
            "to_char(",
            $c,
            " at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')"
        )
    };
}

/// A statement's error by SQLSTATE: a CHECK the handler's own validation should have caught, or a malformed `valid_until`, is the
/// caller's 422; everything else follows the platform's mapping.
/// True for the SQLSTATEs that mean "the caller's value is outside what the database accepts": a check (23514), a malformed literal
/// (22P02) and the date-time classes (cox W1-39, the second line behind the validator): 22007 invalid format, 22008 field overflow,
/// 22009 time zone displacement out of range. They answer 422, never a 500.
#[must_use]
pub fn is_input_error_code(code: &str) -> bool {
    matches!(code, "23514" | "22007" | "22008" | "22009" | "22P02")
}

fn db(e: sqlx::Error) -> Problem {
    if let sqlx::Error::Database(d) = &e
        && d.code().as_deref().is_some_and(is_input_error_code)
    {
        return Problem::invalid_request(422, "A value is outside what this record allows.");
    }
    Problem::from(DbError::from(e))
}

fn exec(r: Result<&mut sqlx::PgConnection, DbError>) -> Result<&mut sqlx::PgConnection, Problem> {
    r.map_err(Problem::from)
}

fn type_name(t: RecordType) -> &'static str {
    match t {
        RecordType::Decision => "decision",
        RecordType::Lesson => "lesson",
        RecordType::Note => "note",
        RecordType::Status => "status",
        RecordType::Question => "question",
        RecordType::Blocker => "blocker",
        RecordType::Knowledge => "knowledge",
        RecordType::Conversation => "conversation",
        RecordType::WorkProduct => "work-product",
        RecordType::HumanMessage => "human-message",
        RecordType::DrillCanary => "drill-canary",
        RecordType::Drill => "drill",
    }
}

fn source_kind_name(k: SourceKind) -> &'static str {
    match k {
        SourceKind::Conversation => "conversation",
        SourceKind::File => "file",
        SourceKind::Url => "url",
        SourceKind::Worker => "worker",
    }
}

/// True when every id names an object of the project (of the given kind when one is asked for).
async fn all_exist(
    tx: &mut ModuleTx,
    ids: &BTreeSet<String>,
    kind: Option<&'static str>,
) -> Result<bool, Problem> {
    if ids.is_empty() {
        return Ok(true);
    }
    let list: Vec<String> = ids.iter().cloned().collect();
    let found: i64 = match kind {
        Some(k) => sqlx::query_scalar(
            "SELECT count(*) FROM platform.objects WHERE kind = $2 AND id = ANY($1::text[]::uuid[])",
        )
        .bind(list)
        .bind(k)
        .fetch_one(exec(tx.executor())?)
        .await
        .map_err(db)?,
        None => sqlx::query_scalar("SELECT count(*) FROM platform.objects WHERE id = ANY($1::text[]::uuid[])")
            .bind(list)
            .fetch_one(exec(tx.executor())?)
            .await
            .map_err(db)?,
    };
    Ok(usize::try_from(found).is_ok_and(|n| n == ids.len()))
}

/// Phase 404: the ids in the body name something in this project. An unknown id and another project's id look the same (R-5).
async fn check_references(tx: &mut ModuleTx, req: &RecordCreate) -> Result<(), Problem> {
    if let Some(a) = &req.affects {
        let ids: BTreeSet<String> = a.iter().map(ToString::to_string).collect();
        if !all_exist(tx, &ids, None).await? {
            return Err(Problem::not_found());
        }
    }
    let mut records: BTreeSet<String> = BTreeSet::new();
    if let Some(r) = req.reply_to {
        records.insert(r.to_string());
    }
    if let Some(src) = &req.source
        && src.kind == SourceKind::Conversation
        // A reference that is not a record id never gets here: step 0 (`check_source`) refused it with 422.
        && let Ok(r) = RecordId::parse(&src.reference)
    {
        records.insert(r.to_string());
    }
    if !all_exist(tx, &records, Some("record")).await? {
        return Err(Problem::not_found());
    }
    if let Some(h) = req.handoff_id {
        let ids: BTreeSet<String> = [h.to_string()].into();
        if !all_exist(tx, &ids, Some("handoff")).await? {
            return Err(Problem::not_found());
        }
    }
    Ok(())
}

/// Phase 403: only the project lead writes `rules-project` and `orientation`; `rules-worker:<id>` is written by that worker or the
/// project lead. Anything else that parses as a section is not writable by this caller.
pub fn check_section_write(who: &Actor, section: Option<&SectionName>) -> Result<(), Problem> {
    let allowed = match section {
        None => true,
        Some(SectionName::Orientation | SectionName::RulesProject) => {
            matches!(who.role, Role::ProjectLead)
        }
        Some(SectionName::RulesWorker(w)) => {
            matches!(who.role, Role::ProjectLead) || *w == who.worker
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(Problem::new(
            &crate::problems::SECTION_RESERVED,
            "This section is reserved: the project lead writes it (a worker may write its own rules-worker section).",
        ))
    }
}

/// The authorize phase of `records.log` (R-16 as written in the S3 YAML): STEP 0 first (every check that needs no database read
/// answers 422 at once: bounds, patterns, NUL, conditional body rules, handler-required fields), then the reads: 404 (the body's
/// references), then 403 (who may write the section). The route's authorize closure.
pub async fn authorize_log(
    tx: &mut ModuleTx,
    who: &Actor,
    req: &RecordCreate,
) -> Result<Validated, Problem> {
    let v = validate_create(req)?;
    check_references(tx, req).await?;
    check_section_write(who, v.section.as_ref())?;
    Ok(v)
}

/// The execute phase: the 409 phase is empty for a write; the insert, the section revision under the section row lock, the platform
/// object and refs, and the audit row of a reserved write (last: the audit seals the transaction's lock order).
pub async fn execute_log(
    tx: &mut ModuleTx,
    env: &Env<'_>,
    who: &Actor,
    req: &RecordCreate,
    v: &Validated,
) -> Result<Created, Problem> {
    let id = RecordId::new(env.clock);
    let project = env.project.to_string();
    let fields = req
        .fields
        .clone()
        .map(serde_json::Value::Object)
        .unwrap_or_else(|| serde_json::json!({}));
    let canonical = canonical_json(&fields).map_err(|_| {
        Problem::invalid_request(422, "fields cannot be written in canonical JSON.")
    })?;
    let fields_text = serde_json::to_string(&fields)
        .map_err(|_| Problem::invalid_request(422, "fields cannot be encoded."))?;

    // The section's next revision, taken under the section row lock (Q-8): insert the row if it is new, then lock it.
    let (section, revision) = match &v.section {
        Some(name) => {
            let name = name.to_string();
            sqlx::query(
                "INSERT INTO records.sections (project_id, section) VALUES ($1::uuid, $2) ON CONFLICT DO NOTHING",
            )
            .bind(&project)
            .bind(&name)
            .execute(exec(tx.executor())?)
            .await
            .map_err(db)?;
            let last: i32 = sqlx::query_scalar(
                "SELECT last_revision FROM records.sections WHERE project_id = $1::uuid AND section = $2 FOR UPDATE",
            )
            .bind(&project)
            .bind(&name)
            .fetch_one(exec(tx.executor())?)
            .await
            .map_err(db)?;
            (Some(name), Some(last + 1))
        }
        None => (None, None),
    };

    let author_kind = match who.kind {
        WorkerKind::Worker => "worker",
        WorkerKind::Service => "service",
    };
    let body = req.body.as_deref().filter(|b| !b.is_empty());
    let src = req.source.as_ref();
    let created_at: String = sqlx::query_scalar(concat!(
        "INSERT INTO records.records (id, project_id, created_at, type, provenance, author_kind, author_id, written_by, summary, body, \
         summary_sha256, body_sha256, fields_sha256, fields, reply_to, handoff_id, section, section_revision, valid_until, \
         source_kind, source_ref, source_span) \
         VALUES ($1::uuid, $2::uuid, uuid_extract_timestamp($1::uuid), $3, 'worker_written', $4, $5, $6::uuid, $7, $8, \
         sha256(convert_to($7, 'UTF8')), sha256(convert_to(coalesce($8, ''), 'UTF8')), sha256($9::bytea), $10::jsonb, \
         $11::uuid, $12::uuid, $13, $14, $15::timestamptz, $16, $17, $18) RETURNING ",
        ts!("created_at")
    ))
    .bind(id.to_string())
    .bind(&project)
    .bind(type_name(req.kind))
    .bind(author_kind)
    .bind(who.worker.to_string())
    .bind(who.worker.to_string())
    .bind(&req.summary)
    .bind(body)
    .bind(canonical)
    .bind(fields_text)
    .bind(req.reply_to.map(|r| r.to_string()))
    .bind(req.handoff_id.map(|h| h.to_string()))
    .bind(section.as_deref())
    .bind(revision)
    .bind(req.valid_until.as_deref())
    .bind(src.map(|s| source_kind_name(s.kind)))
    .bind(src.map(|s| s.reference.as_str()))
    .bind(src.and_then(|s| s.span.as_deref()))
    .fetch_one(exec(tx.executor())?)
    .await
    .map_err(db)?;

    if let Some(name) = &section {
        sqlx::query(
            "UPDATE records.sections SET last_revision = $3, current_record_id = $4::uuid WHERE project_id = $1::uuid AND section = $2",
        )
        .bind(&project)
        .bind(name)
        .bind(revision)
        .bind(id.to_string())
        .execute(exec(tx.executor())?)
        .await
        .map_err(db)?;
    }

    // Every record is a platform object; the citations are insert-only refs (affects, reply_to, cites).
    let me = uuid::Uuid::parse_str(&id.to_string())
        .map_err(|_| Problem::from(DbError::Other("record id".into())))?;
    cortex_db::objects::insert(tx, me, "record")
        .await
        .map_err(Problem::from)?;
    let to_uuid = |s: &str| {
        uuid::Uuid::parse_str(s).map_err(|_| Problem::from(DbError::Other("cited id".into())))
    };
    if let Some(affects) = &req.affects {
        let unique: BTreeSet<String> = affects.iter().map(ToString::to_string).collect();
        for a in unique {
            cortex_db::refs::insert(tx, me, to_uuid(&a)?, "affects")
                .await
                .map_err(Problem::from)?;
        }
    }
    if let Some(r) = req.reply_to {
        cortex_db::refs::insert(tx, me, to_uuid(&r.to_string())?, "reply_to")
            .await
            .map_err(Problem::from)?;
    }
    if let Some(s) = src
        && s.kind == SourceKind::Conversation
    {
        cortex_db::refs::insert(tx, me, to_uuid(&s.reference)?, "cites")
            .await
            .map_err(Problem::from)?;
    }

    if section.is_some() {
        env.audit
            .record(
                tx,
                "section.write_reserved",
                AuditTarget::Record(id),
                AuditDetails::new(),
            )
            .await
            .map_err(Problem::from)?;
    }
    Ok(Created {
        id,
        created_at,
        section_revision: revision,
    })
}

fn id_of(row: &sqlx::postgres::PgRow, col: &str) -> Result<Option<RecordId>, Problem> {
    let t: Option<String> = row.try_get(col).map_err(db)?;
    t.map(|t| RecordId::parse(&t).map_err(|_| Problem::from(DbError::Other("stored id".into()))))
        .transpose()
}

/// One record with its state and lineage; `None` when it names nothing in the project (another project's id looks the same).
pub async fn show(tx: &mut ModuleTx, id: RecordId) -> Result<Option<RecordOut>, Problem> {
    let row = sqlx::query(concat!(
        "SELECT r.id::text AS id, r.type, r.summary, r.body, r.fields::text AS fields, r.source_kind, r.source_ref, r.source_span, \
         coalesce(s.state, 'active') AS state, s.superseded_by::text AS superseded_by, s.reason, s.by_kind AS s_by_kind, \
         s.by_id AS s_by_id, ",
        ts!("s.at"),
        " AS s_at, ",
        ts!("r.created_at"),
        " AS created_at, r.author_kind, r.author_id, r.provenance, r.reply_to::text AS reply_to, r.handoff_id::text AS handoff_id, ",
        ts!("r.valid_until"),
        " AS valid_until, r.section, r.section_revision \
         FROM records.records r LEFT JOIN records.state s ON s.project_id = r.project_id AND s.record_id = r.id \
         WHERE r.id = $1::uuid"
    ))
    .bind(id.to_string())
    .fetch_optional(exec(tx.executor())?)
    .await
    .map_err(db)?;
    let Some(row) = row else { return Ok(None) };
    let fields_text: String = row.try_get("fields").map_err(db)?;
    let fields = serde_json::from_str(&fields_text)
        .map_err(|_| Problem::from(DbError::Other("stored fields".into())))?;
    let source_kind: Option<String> = row.try_get("source_kind").map_err(db)?;
    let source = match source_kind {
        Some(kind) => Some(SourceOut {
            kind,
            reference: row.try_get("source_ref").map_err(db)?,
            span: row.try_get("source_span").map_err(db)?,
        }),
        None => None,
    };
    let reason: Option<String> = row.try_get("reason").map_err(db)?;
    let lineage = match reason {
        Some(reason) => Some(Lineage {
            reason,
            by_kind: row.try_get("s_by_kind").map_err(db)?,
            by_id: row.try_get("s_by_id").map_err(db)?,
            at: row.try_get("s_at").map_err(db)?,
        }),
        None => None,
    };
    Ok(Some(RecordOut {
        id,
        kind: row.try_get("type").map_err(db)?,
        summary: row.try_get("summary").map_err(db)?,
        body: row.try_get("body").map_err(db)?,
        fields,
        source,
        state: row.try_get("state").map_err(db)?,
        superseded_by: id_of(&row, "superseded_by")?,
        lineage,
        created_at: row.try_get("created_at").map_err(db)?,
        author_kind: row.try_get("author_kind").map_err(db)?,
        author: row.try_get("author_id").map_err(db)?,
        provenance: row.try_get("provenance").map_err(db)?,
        reply_to: id_of(&row, "reply_to")?,
        handoff_id: row.try_get("handoff_id").map_err(db)?,
        valid_until: row.try_get("valid_until").map_err(db)?,
        section: row.try_get("section").map_err(db)?,
        section_revision: row.try_get("section_revision").map_err(db)?,
    }))
}

/// The current revision of a reserved section; `None` when never written or when the current revision was invalidated (no
/// fallback to an older revision, CHANGES section 20).
pub async fn section_current(
    tx: &mut ModuleTx,
    name: &SectionName,
) -> Result<Option<SectionPointer>, Problem> {
    let row = sqlx::query(
        "SELECT r.id::text AS id, r.section_revision, encode(r.body_sha256, 'hex') AS sha, r.fields->>'source_path' AS source_path \
         FROM records.sections sec JOIN records.records r ON r.project_id = sec.project_id AND r.id = sec.current_record_id \
         WHERE sec.section = $1",
    )
    .bind(name.to_string())
    .fetch_optional(exec(tx.executor())?)
    .await
    .map_err(db)?;
    let Some(row) = row else { return Ok(None) };
    let record_id =
        id_of(&row, "id")?.ok_or_else(|| Problem::from(DbError::Other("stored id".into())))?;
    Ok(Some(SectionPointer {
        section: name.to_string(),
        record_id,
        revision: row.try_get("section_revision").map_err(db)?,
        sha256: row.try_get("sha").map_err(db)?,
        source_path: row.try_get("source_path").map_err(db)?,
    }))
}

/// What the lineage routes read about the target record before they authorise: its author, its state and its section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageTarget {
    pub id: RecordId,
    pub author_kind: String,
    pub author_id: String,
    pub state: String,
    pub section: Option<String>,
    pub section_revision: Option<i32>,
}

/// `LineageState` (S3): the answer to a supersede, an invalidation or an undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LineageState {
    pub id: RecordId,
    pub state: String,
}

/// The 404 phase of the lineage routes: the record in the path, or `None` when it names nothing in the project (another project's
/// id looks the same). Reads its author, its state and its section, for the target rule and the state machine.
pub async fn lineage_target(
    tx: &mut ModuleTx,
    id: RecordId,
) -> Result<Option<LineageTarget>, Problem> {
    let row = sqlx::query(
        "SELECT r.author_kind, r.author_id, coalesce(s.state, 'active') AS state, r.section, r.section_revision \
         FROM records.records r LEFT JOIN records.state s ON s.project_id = r.project_id AND s.record_id = r.id \
         WHERE r.id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_optional(exec(tx.executor())?)
    .await
    .map_err(db)?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(LineageTarget {
        id,
        author_kind: row.try_get("author_kind").map_err(db)?,
        author_id: row.try_get("author_id").map_err(db)?,
        state: row.try_get("state").map_err(db)?,
        section: row.try_get("section").map_err(db)?,
        section_revision: row.try_get("section_revision").map_err(db)?,
    }))
}

/// True when the record exists in the project (the `by` of a supersede).
pub async fn record_exists(tx: &mut ModuleTx, id: RecordId) -> Result<bool, Problem> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM records.records WHERE id = $1::uuid")
        .bind(id.to_string())
        .fetch_one(exec(tx.executor())?)
        .await
        .map_err(db)?;
    Ok(n == 1)
}

fn conflict() -> Problem {
    Problem::new(
        &crate::problems::LINEAGE_STATE_CONFLICT,
        "The record is not in a state this operation applies to, or a higher section revision exists.",
    )
}

/// A lineage reason: free text of 1 to 512 bytes, required for a supersede, an invalidation AND an undo. Length and NUL are
/// `invalid_request`; absent, empty or blank is `reason_required` (the field's own code comes last, R-16).
fn check_reason(reason: Option<&str>) -> Result<&str, Problem> {
    let r = reason.unwrap_or("");
    if r.len() > 512 {
        return Err(Problem::invalid_request(
            422,
            "reason is at most 512 bytes.",
        ));
    }
    if r.contains('\0') {
        return Err(Problem::invalid_request(
            422,
            "reason must not contain a NUL character.",
        ));
    }
    if r.trim().is_empty() {
        return Err(Problem::new(
            &crate::problems::REASON_REQUIRED,
            "A reason of 1 to 512 bytes is required.",
        ));
    }
    Ok(r)
}

fn by_kind(who: &Actor) -> &'static str {
    match who.kind {
        WorkerKind::Worker => "worker",
        WorkerKind::Service => "service",
    }
}

/// The first statement of every lineage change: the project's counter row, taken FOR UPDATE by the increment. All lineage changes of a
/// project serialise on it, so the state a change reads (the replacement is active, the target is in the state it expects) cannot be
/// changed by another lineage change until this one commits: two records superseding each other at once cannot both pass (hale Q-1,
/// cox W1-36). Lock order: counter row, then the record's state row, then the section row, then the audit row last. A section WRITE takes
/// only the section row, so there is no cycle. (The S2 comment on `records.state_log` lists the counter last; the order that matters for
/// deadlock freedom is that all three lineage paths use the same one.)
async fn next_seq(tx: &mut ModuleTx, project: &str) -> Result<i64, Problem> {
    sqlx::query_scalar(
        "INSERT INTO records.lineage_counters (project_id, last_seq) VALUES ($1::uuid, 1) \
         ON CONFLICT (project_id) DO UPDATE SET last_seq = records.lineage_counters.last_seq + 1 RETURNING last_seq",
    )
    .bind(project)
    .fetch_one(exec(tx.executor())?)
    .await
    .map_err(db)
}

/// The part every lineage change shares after the state row is written: the section pointer, the gapless log row (its `seq` was taken
/// first, by `next_seq`), the audit row.
#[allow(clippy::too_many_arguments)]
async fn after_state_change(
    tx: &mut ModuleTx,
    env: &Env<'_>,
    who: &Actor,
    target: &LineageTarget,
    seq: i64,
    from: &str,
    to: &str,
    superseded_by: Option<RecordId>,
    reason: &str,
    action: &'static str,
) -> Result<(), Problem> {
    let project = env.project.to_string();
    if let (Some(section), Some(revision)) = (&target.section, target.section_revision) {
        let row = sqlx::query(
            "SELECT last_revision, current_record_id::text AS current_id FROM records.sections \
             WHERE project_id = $1::uuid AND section = $2 FOR UPDATE",
        )
        .bind(&project)
        .bind(section)
        .fetch_one(exec(tx.executor())?)
        .await
        .map_err(db)?;
        let last: i32 = row.try_get("last_revision").map_err(db)?;
        let current: Option<String> = row.try_get("current_id").map_err(db)?;
        let me = target.id.to_string();
        match to {
            // An invalidation clears the pointer only when this IS the current revision; no fallback to an older one.
            "invalidated" if current.as_deref() == Some(me.as_str()) => {
                sqlx::query("UPDATE records.sections SET current_record_id = NULL WHERE project_id = $1::uuid AND section = $2")
                    .bind(&project)
                    .bind(section)
                    .execute(exec(tx.executor())?)
                    .await
                    .map_err(db)?;
            }
            // An undo is refused when ANY higher revision exists (in any state); otherwise this revision is current again.
            "active" => {
                if last > revision {
                    return Err(conflict());
                }
                sqlx::query("UPDATE records.sections SET current_record_id = $3::uuid WHERE project_id = $1::uuid AND section = $2")
                    .bind(&project)
                    .bind(section)
                    .bind(&me)
                    .execute(exec(tx.executor())?)
                    .await
                    .map_err(db)?;
            }
            _ => {}
        }
    }
    sqlx::query(
        "INSERT INTO records.state_log (project_id, seq, record_id, from_state, to_state, superseded_by, reason, by_kind, by_id) \
         VALUES ($1::uuid, $2, $3::uuid, $4, $5, $6::uuid, $7, $8, $9)",
    )
    .bind(&project)
    .bind(seq)
    .bind(target.id.to_string())
    .bind(from)
    .bind(to)
    .bind(superseded_by.map(|b| b.to_string()))
    .bind(reason)
    .bind(by_kind(who))
    .bind(who.worker.to_string())
    .execute(exec(tx.executor())?)
    .await
    .map_err(db)?;
    // The audit row is last: it seals the transaction's lock order.
    env.audit
        .record(
            tx,
            action,
            AuditTarget::Record(target.id),
            AuditDetails::new(),
        )
        .await
        .map_err(Problem::from)
}

/// STEP 0 of a supersede (no database read): the reason (length and NUL, then required), then `by` is not the record itself.
pub fn step0_supersede(
    target: RecordId,
    by: RecordId,
    reason: Option<&str>,
) -> Result<String, Problem> {
    let reason = check_reason(reason)?.to_owned();
    if by == target {
        return Err(Problem::new(
            &crate::problems::SELF_SUPERSEDE,
            "A record cannot supersede itself.",
        ));
    }
    Ok(reason)
}

/// STEP 0 of an invalidation or an undo: the reason.
pub fn step0_invalidate(reason: Option<&str>) -> Result<String, Problem> {
    Ok(check_reason(reason)?.to_owned())
}

/// The authorize-phase reads of a supersede (R-16): step 0, then 404 for the record in the path, then 404 for `by`. The 403 target
/// rule is the route's `Ctx::authorize` on what this returns.
pub async fn authorize_supersede(
    tx: &mut ModuleTx,
    target: RecordId,
    by: RecordId,
    reason: Option<&str>,
) -> Result<(LineageTarget, String), Problem> {
    let reason = step0_supersede(target, by, reason)?;
    let t = lineage_target(tx, target)
        .await?
        .ok_or_else(Problem::not_found)?;
    if !record_exists(tx, by).await? {
        return Err(Problem::not_found());
    }
    Ok((t, reason))
}

/// The authorize-phase reads of an invalidation or an undo: step 0, then 404 for the record in the path.
pub async fn authorize_invalidate(
    tx: &mut ModuleTx,
    target: RecordId,
    reason: Option<&str>,
) -> Result<(LineageTarget, String), Problem> {
    let reason = step0_invalidate(reason)?;
    let t = lineage_target(tx, target)
        .await?
        .ok_or_else(Problem::not_found)?;
    Ok((t, reason))
}

/// The execute phase of a supersede (R-16): the 409 of the state, then ONE atomic statement on the state row (the race decides here: the loser's statement changes no row and is a 409), the
/// section pointer, the log row and the audit row. A section revision cannot be superseded at all (409): a section changes by a new revision and is withdrawn by invalidation.
pub async fn execute_supersede(
    tx: &mut ModuleTx,
    env: &Env<'_>,
    who: &Actor,
    target: &LineageTarget,
    by: RecordId,
    reason: &str,
) -> Result<LineageState, Problem> {
    // A section revision changes by a NEW revision and is withdrawn by invalidation; it is never superseded (cox W1-36, Q-2).
    if target.state != "active" || target.section.is_some() {
        return Err(conflict());
    }
    let seq = next_seq(tx, &env.project.to_string()).await?;
    // ONE statement decides: the replacement `by` must be ACTIVE (a record with an outgoing link can never be one, so a cycle is
    // impossible: cox W1-36, Q-1) and the target must still be active. Zero rows is a 409 and nothing was written.
    let won = sqlx::query(
        "INSERT INTO records.state (project_id, record_id, state, superseded_by, reason, by_kind, by_id) \
         SELECT $1::uuid, $2::uuid, 'superseded', $3::uuid, $4::text, $5::text, $6::text \
         WHERE NOT EXISTS (SELECT 1 FROM records.state b WHERE b.project_id = $1::uuid AND b.record_id = $3::uuid AND b.state <> 'active') \
         ON CONFLICT (project_id, record_id) DO UPDATE SET state = 'superseded', superseded_by = EXCLUDED.superseded_by, \
         reason = EXCLUDED.reason, by_kind = EXCLUDED.by_kind, by_id = EXCLUDED.by_id, at = platform.now() \
         WHERE records.state.state = 'active' AND NOT EXISTS \
         (SELECT 1 FROM records.state b WHERE b.project_id = EXCLUDED.project_id AND b.record_id = EXCLUDED.superseded_by AND b.state <> 'active') \
         RETURNING 1",
    )
    .bind(env.project.to_string())
    .bind(target.id.to_string())
    .bind(by.to_string())
    .bind(reason)
    .bind(by_kind(who))
    .bind(who.worker.to_string())
    .fetch_optional(exec(tx.executor())?)
    .await
    .map_err(db)?;
    if won.is_none() {
        return Err(conflict());
    }
    after_state_change(
        tx,
        env,
        who,
        target,
        seq,
        "active",
        "superseded",
        Some(by),
        reason,
        "record.superseded",
    )
    .await?;
    Ok(LineageState {
        id: target.id,
        state: "superseded".into(),
    })
}

/// Invalidate `target`, or undo an invalidation (`undo`). Same order as `supersede`. An undo works only on an invalidated record;
/// for a section revision it is a 409 when any higher revision exists in any state, decided under the section row lock.
pub async fn execute_invalidate(
    tx: &mut ModuleTx,
    env: &Env<'_>,
    who: &Actor,
    target: &LineageTarget,
    reason: &str,
    undo: bool,
) -> Result<LineageState, Problem> {
    let (need, from, to, action) = if undo {
        (
            "invalidated",
            "invalidated",
            "active",
            "record.invalidation_undone",
        )
    } else {
        ("active", "active", "invalidated", "record.invalidated")
    };
    if target.state != need {
        return Err(conflict());
    }
    let project = env.project.to_string();
    let seq = next_seq(tx, &project).await?;
    let changed = if undo {
        sqlx::query(
            "UPDATE records.state SET state = 'active', superseded_by = NULL, reason = $3, by_kind = $4, by_id = $5, at = platform.now() \
             WHERE project_id = $1::uuid AND record_id = $2::uuid AND state = 'invalidated' RETURNING 1",
        )
        .bind(&project)
        .bind(target.id.to_string())
        .bind(reason)
        .bind(by_kind(who))
        .bind(who.worker.to_string())
        .fetch_optional(exec(tx.executor())?)
        .await
        .map_err(db)?
    } else {
        sqlx::query(
            "INSERT INTO records.state (project_id, record_id, state, reason, by_kind, by_id) \
             VALUES ($1::uuid, $2::uuid, 'invalidated', $3, $4, $5) \
             ON CONFLICT (project_id, record_id) DO UPDATE SET state = 'invalidated', superseded_by = NULL, reason = EXCLUDED.reason, \
             by_kind = EXCLUDED.by_kind, by_id = EXCLUDED.by_id, at = platform.now() \
             WHERE records.state.state = 'active' RETURNING 1",
        )
        .bind(&project)
        .bind(target.id.to_string())
        .bind(reason)
        .bind(by_kind(who))
        .bind(who.worker.to_string())
        .fetch_optional(exec(tx.executor())?)
        .await
        .map_err(db)?
    };
    if changed.is_none() {
        return Err(conflict());
    }
    after_state_change(tx, env, who, target, seq, from, to, None, reason, action).await?;
    Ok(LineageState {
        id: target.id,
        state: to.into(),
    })
}
