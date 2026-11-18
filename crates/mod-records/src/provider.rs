//! The `RecordsApi` provider. Every call runs in the caller's `ModuleTx`, delegated to this piece's role; the project is the
//! transaction's. `get`: the records of the project among the ids, in ANY state, each with its `state`; an unknown id or another
//! project's id is simply absent (R-5). `changed_since`: records CREATED after a keyset cursor on `(created_at, id)`; it reports no
//! lineage change (that feed is events, a later release). `section_current` is boot's read: `None` when never written or when the
//! current revision was invalidated, no fallback to an older one. Nothing here writes.
//!
//! **`changed_since` is ordered by CREATION, not by commit (hale O-1):** the id, and so `created_at`, is made when the statement runs and the
//! row is visible at commit; a record whose transaction commits after a reader has paged past its key is never seen by that reader. Do not
//! build an index feed on it without that in mind (mod-search reads the `records.pub_*` views instead). No consumer calls it in 0.2.001.

use async_trait::async_trait;
use cortex_db::ModuleTx;
use cortex_module::{
    PortError, RecordCursor, RecordPage, RecordState, RecordView, RecordsApi, SectionName,
    SectionView, SourceView,
};
use cortex_types::{ModuleId, RecordId, Sha256, Timestamp, WorkerId};
use sqlx::Row;

use crate::ID;

/// The supported range of a cursor, in microseconds since the epoch: 1970-01-01T00:00:00Z to 9999-12-31T23:59:59.999999Z (the one rule
/// for every date-time input, cox W1-39; the top is the end of the stored timestamps' range). Outside it the keyset arithmetic would be
/// a database error, so it is refused (`PortError::Invalid`).
const CURSOR_MIN_MICROS: i64 = 0;
const CURSOR_MAX_MICROS: i64 = 253_402_300_799_999_999;

/// A page is at most this many records (R-8's list maximum); a larger `limit` is cut to it, `0` is refused.
const PAGE_MAX: u32 = 200;

/// The columns of a `RecordView`, in one place (a literal, so it can be spliced into a query at compile time).
macro_rules! view_columns {
    () => {
        "r.id::text AS id, r.type, r.summary, r.body, r.fields::text AS fields, r.source_kind, r.source_ref, r.source_span, \
         coalesce(s.state, 'active') AS state, s.superseded_by::text AS superseded_by, r.author_kind, r.author_id, r.provenance, \
         r.section, r.section_revision, (extract(epoch FROM r.created_at) * 1000000)::bigint AS created_micros, \
         CASE WHEN isfinite(r.valid_until) THEN (extract(epoch FROM r.valid_until) * 1000000)::bigint END AS valid_micros"
    };
}

/// One row of `view_columns!` as a `RecordView`. A row that does not decode is a stored-data fault, never row data in an error.
fn view_of(row: &sqlx::postgres::PgRow) -> Result<RecordView, cortex_db::DbError> {
    let bad = |what: &str| cortex_db::DbError::Other(format!("stored record row: {what}"));
    let id: String = row.try_get("id")?;
    let fields: String = row.try_get("fields")?;
    let state: String = row.try_get("state")?;
    let superseded_by: Option<String> = row.try_get("superseded_by")?;
    let source_kind: Option<String> = row.try_get("source_kind")?;
    let source = match source_kind {
        Some(kind) => Some(SourceView {
            kind,
            reference: row.try_get("source_ref")?,
            span: row.try_get("source_span")?,
        }),
        None => None,
    };
    let valid_micros: Option<i64> = row.try_get("valid_micros")?;
    Ok(RecordView {
        id: RecordId::parse(&id).map_err(|_| bad("id"))?,
        kind: row.try_get("type")?,
        summary: row.try_get("summary")?,
        body: row.try_get("body")?,
        fields: serde_json::from_str(&fields).map_err(|_| bad("fields"))?,
        source,
        state: match state.as_str() {
            "active" => RecordState::Active,
            "superseded" => RecordState::Superseded,
            "invalidated" => RecordState::Invalidated,
            _ => return Err(bad("state")),
        },
        superseded_by: superseded_by
            .map(|s| RecordId::parse(&s).map_err(|_| bad("superseded_by")))
            .transpose()?,
        author_kind: row.try_get("author_kind")?,
        author: row.try_get("author_id")?,
        provenance: row.try_get("provenance")?,
        section: row.try_get("section")?,
        section_revision: row.try_get("section_revision")?,
        created_at: Timestamp::from_unix_micros(row.try_get("created_micros")?),
        valid_until: valid_micros.map(Timestamp::from_unix_micros),
    })
}

pub struct Provider {
    id: ModuleId,
}

impl Provider {
    pub fn new() -> Provider {
        Provider {
            id: ModuleId::parse(ID).expect("a valid module id"),
        }
    }
}

impl Default for Provider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RecordsApi for Provider {
    async fn get(&self, tx: &mut ModuleTx, ids: &[RecordId]) -> Result<Vec<RecordView>, PortError> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let list: Vec<String> = ids.iter().map(ToString::to_string).collect();
        Ok(tx
            .delegate(&self.id, async move |t: &mut ModuleTx| {
                let rows = sqlx::query(concat!(
                    "SELECT ",
                    view_columns!(),
                    " FROM records.records r LEFT JOIN records.state s ON s.project_id = r.project_id AND s.record_id = r.id \
                     WHERE r.id = ANY($1::text[]::uuid[]) ORDER BY r.created_at, r.id"
                ))
                .bind(list)
                .fetch_all(t.executor()?)
                .await?;
                rows.iter().map(view_of).collect::<Result<Vec<_>, _>>()
            })
            .await?)
    }

    async fn changed_since(
        &self,
        tx: &mut ModuleTx,
        after: Option<RecordCursor>,
        limit: u32,
    ) -> Result<RecordPage, PortError> {
        if limit == 0 {
            return Err(PortError::Invalid("limit is at least 1"));
        }
        let limit = i64::from(limit.min(PAGE_MAX));
        let (micros, id) = match after {
            Some(c) => {
                let m = c.at.unix_micros();
                if !(CURSOR_MIN_MICROS..=CURSOR_MAX_MICROS).contains(&m) {
                    return Err(PortError::Invalid(
                        "the cursor is outside the supported time range",
                    ));
                }
                (Some(m), Some(c.id.to_string()))
            }
            None => (None, None),
        };
        Ok(tx
            .delegate(&self.id, async move |t: &mut ModuleTx| {
                // One row more than asked: it says whether another page exists. The cursor is a position, never authority.
                let rows = sqlx::query(concat!(
                    "SELECT ",
                    view_columns!(),
                    " FROM records.records r LEFT JOIN records.state s ON s.project_id = r.project_id AND s.record_id = r.id \
                     WHERE ($1::bigint IS NULL OR (r.created_at, r.id) > (timestamptz 'epoch' + $1::bigint * interval '1 microsecond', $2::uuid)) \
                     ORDER BY r.created_at, r.id LIMIT $3"
                ))
                .bind(micros)
                .bind(id)
                .bind(limit + 1)
                .fetch_all(t.executor()?)
                .await?;
                let mut records = rows.iter().map(view_of).collect::<Result<Vec<_>, _>>()?;
                let more = i64::try_from(records.len()).is_ok_and(|n| n > limit);
                if more {
                    records.pop();
                }
                let next = if more {
                    records.last().map(|r| RecordCursor { at: r.created_at, id: r.id })
                } else {
                    None
                };
                Ok(RecordPage { records, next })
            })
            .await?)
    }

    async fn section_current(
        &self,
        tx: &mut ModuleTx,
        name: &SectionName,
    ) -> Result<Option<SectionView>, PortError> {
        let name = name.to_string();
        Ok(tx
            .delegate(&self.id, async move |t: &mut ModuleTx| {
                let row = sqlx::query(
                    "SELECT r.id::text AS id, r.section_revision, coalesce(r.body, '') AS body, r.body_sha256, r.fields::text AS fields, \
                     r.author_id, (extract(epoch FROM r.created_at) * 1000000)::bigint AS micros \
                     FROM records.sections sec JOIN records.records r ON r.project_id = sec.project_id AND r.id = sec.current_record_id \
                     WHERE sec.section = $1",
                )
                .bind(name)
                .fetch_optional(t.executor()?)
                .await?;
                let Some(row) = row else { return Ok(None) };
                let bad = || cortex_db::DbError::Other("stored section row".into());
                let id: String = row.try_get("id")?;
                let digest: Vec<u8> = row.try_get("body_sha256")?;
                let fields: String = row.try_get("fields")?;
                let author: String = row.try_get("author_id")?;
                Ok(Some(SectionView {
                    record_id: RecordId::parse(&id).map_err(|_| bad())?,
                    revision: row.try_get("section_revision")?,
                    body: row.try_get("body")?,
                    body_sha256: Sha256(<[u8; 32]>::try_from(digest.as_slice()).map_err(|_| bad())?),
                    fields: serde_json::from_str(&fields).map_err(|_| bad())?,
                    author: WorkerId::parse(&author).map_err(|_| bad())?,
                    created_at: Timestamp::from_unix_micros(row.try_get("micros")?),
                }))
            })
            .await?)
    }
}
