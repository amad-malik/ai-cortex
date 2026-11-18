//! Project-scoped audit list (S3 r2.7, R-4 and R-8).
//! The `next` cursor is opaque to clients: return it unchanged as `after`.
use axum::Json;
use axum::extract::State;
use cortex_db::DbError;
use cortex_module::route::{AppState, StrictQuery};
use cortex_module::{Ctx, Problem};
use cortex_types::{ModuleId, datetime};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
#[path = "../tests/support/read_tests.rs"]
mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Query {
    limit: Option<i64>,
    after: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct Page {
    items: Vec<Item>,
    next: Option<String>,
}

#[derive(Serialize)]
struct Item {
    id: String,
    at: String,
    action: String,
    actor_kind: String,
    actor_id: Option<String>,
    target: Value,
    outcome: String,
    request_id: Option<String>,
    details: Value,
}

type AuditRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    String,
);

fn bad_cursor() -> Problem {
    Problem::invalid_request(422, "The cursor is not valid for this list.")
}

// A list-specific position, never authority (Cox decision on fb38abcd).
fn encode_cursor(at: &str, id: &str) -> String {
    format!("a1|{at}|{id}")
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn decode_cursor(cursor: &str) -> Result<(String, String), Problem> {
    // 67 encoded bytes: tag, UTC timestamp with six fractional digits, and UUIDv7.
    if cursor.len() != 134
        || !cursor
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(bad_cursor());
    }
    let bytes = cursor
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let part = std::str::from_utf8(pair).map_err(|_| bad_cursor())?;
            u8::from_str_radix(part, 16).map_err(|_| bad_cursor())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let text = String::from_utf8(bytes).map_err(|_| bad_cursor())?;
    let mut parts = text.split('|');
    let (Some("a1"), Some(at), Some(id), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad_cursor());
    };
    if at.len() != 27 || at.as_bytes()[19] != b'.' || at.as_bytes()[26] != b'Z' {
        return Err(bad_cursor());
    }
    datetime::validate(at).map_err(|_| bad_cursor())?;
    let parsed = uuid::Uuid::parse_str(id).map_err(|_| bad_cursor())?;
    if parsed.hyphenated().to_string() != id
        || parsed.get_version_num() != 7
        || parsed.get_variant() != uuid::Variant::RFC4122
    {
        return Err(bad_cursor());
    }
    Ok((at.to_owned(), id.to_owned()))
}

fn object(raw: &str) -> Result<Value, Problem> {
    match serde_json::from_str::<Value>(raw) {
        Ok(value @ Value::Object(_)) => Ok(value),
        _ => Err(DbError::Other("invalid audit object".into()).into()),
    }
}

pub(crate) async fn list(
    State(state): State<AppState>,
    ctx: Ctx,
    StrictQuery(query): StrictQuery<Query>,
) -> Result<Json<Page>, Problem> {
    let limit = match query.limit {
        None => 50,
        Some(n) if (1..=200).contains(&n) => n,
        Some(_) => {
            return Err(Problem::invalid_request(
                422,
                "The limit must be from 1 to 200.",
            ));
        }
    };
    let (at, id) = query
        .after
        .as_deref()
        .map(decode_cursor)
        .transpose()?
        .unzip();
    let module = ModuleId::parse("audit").expect("valid module id");
    let mut tx = ctx.begin(&state.pool, &module).await?;
    let rows: Vec<AuditRow> = sqlx::query_as(
        "SELECT id::text, to_char(at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), action, actor_kind, actor_id, target::text, outcome, request_id::text, details::text \
         FROM audit.records WHERE project_id = $1::text::uuid \
         AND ($2::text IS NULL OR (at, id) > ($2::text::timestamptz, $3::text::uuid)) \
         ORDER BY at ASC, id ASC LIMIT $4",
    )
    .bind(ctx.project().to_string()).bind(at).bind(id).bind(limit + 1)
    .fetch_all(tx.executor()?).await.map_err(DbError::from)?;
    tx.commit().await?;
    let more = rows.len() as i64 > limit;
    let mut next = None;
    let mut items = Vec::new();
    for (id, at, action, actor_kind, actor_id, target, outcome, request_id, details) in
        rows.into_iter().take(limit as usize)
    {
        if more {
            next = Some(encode_cursor(&at, &id));
        }
        items.push(Item {
            id,
            at,
            action,
            actor_kind,
            actor_id,
            target: object(&target)?,
            outcome,
            request_id,
            details: object(&details)?,
        });
    }
    Ok(Json(Page { items, next }))
}
