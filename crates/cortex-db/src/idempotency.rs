//! The idempotency helper (CONTRACTS §3; S2 §5, K13). Its rows are held by `cortex_m_platform`
//! and reached only through this helper, by delegation (L-7, A7 item 1c). It takes
//! `AdvisoryClass::Idempotency` first (the lock order's step 1).
use std::time::Duration;

use cortex_types::{ModuleId, Sha256, TokenId, WorkerId};
use sqlx::Row;

use crate::error::DbError;
use crate::tx::{AdvisoryClass, ModuleTx};

/// The helper's rows are held by this module's role (L-7).
fn platform() -> ModuleId {
    ModuleId::parse("platform").expect("a valid module id")
}

fn uuid_of(s: &str) -> Result<uuid::Uuid, DbError> {
    uuid::Uuid::parse_str(s).map_err(|_| DbError::Other("not a UUID".into()))
}

/// The key's lock: a 32-bit FNV-1a over the principal and the key, so that two concurrent requests
/// with one key run one after the other (S2 §5 step 1).
fn lock_key(principal: &str, key: &str) -> i32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in principal
        .bytes()
        .chain(std::iter::once(0))
        .chain(key.bytes())
    {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h as i32
}

/// What the row lookup found, inside the platform delegation.
enum Lookup {
    Reserved(i64),
    Existing {
        project: Option<uuid::Uuid>,
        route: String,
        body: Vec<u8>,
        status: Option<i32>,
        response: Option<String>,
    },
}

/// An `Idempotency-Key`: **8 to 128 printable ASCII characters** (0x20 to 0x7E), as frozen S2 §5
/// requires (mike's F1B-R0-01). Anything else is `IdemError::BadKey`. A stored row is bound to
/// the principal, the project, the route and the body hash (S2 §5, "Binding and replay").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdemKey(pub(crate) String);

impl IdemKey {
    pub fn parse(s: &str) -> Result<Self, IdemError> {
        if (8..=128).contains(&s.len()) && s.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            Ok(IdemKey(s.to_owned()))
        } else {
            Err(IdemError::BadKey)
        }
    }
}

/// The stored response. A secret-bearing response stores only a marker.
#[derive(Debug, Clone, PartialEq)]
pub enum StoredResponse {
    Body(serde_json::Value),
    SecretNotStored { token_id: TokenId },
}

/// What `begin` found.
#[derive(Debug)]
pub enum Idem {
    /// A completed earlier request with this key and body: return its response.
    Replay {
        status: u16,
        response: StoredResponse,
    },
    /// A new request: do the work, then `finish`.
    Proceed(IdemToken),
}

/// Proof that `begin` reserved the key in this transaction.
#[derive(Debug)]
pub struct IdemToken {
    pub(crate) id: i64,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum IdemError {
    /// 428: a `mutates: true` operation without a key.
    #[error("idempotency_key_required")]
    KeyRequired,
    /// 422: the key was used with another body.
    #[error("idempotency_key_reused")]
    KeyReused,
    /// 409: the earlier response carried a secret, which is not stored.
    #[error("already_issued")]
    AlreadyIssued { token_id: TokenId },
    #[error("invalid idempotency key")]
    BadKey,
    #[error(transparent)]
    Db(#[from] DbError),
}

/// 428 unless the operation does not mutate or the key is present.
pub fn require_key(mutates: bool, key: Option<&IdemKey>) -> Result<(), IdemError> {
    if mutates && key.is_none() {
        Err(IdemError::KeyRequired)
    } else {
        Ok(())
    }
}

/// Locks the key (class `Idempotency`), then replays, refuses or reserves. A row for this
/// principal and key with the same route, body AND project replays; a row with a different
/// route, body or project is `KeyReused` (422), and nothing runs (mike's F1B-R0-02).
/// A row reserved earlier in the same transaction and not yet finished is `KeyReused` (422
/// `idempotency_key_reused`) too, and nothing runs twice. S2 r9 §5 does not name this case: step 2
/// replays a stored response, which an unfinished row does not have, and step 3 is the 422 for a
/// different body or route. This is the conservative reading, for kai's ruling at R3.
pub async fn begin(
    tx: &mut ModuleTx,
    principal: WorkerId,
    key: &IdemKey,
    route: &str,
    body: Sha256,
) -> Result<Idem, IdemError> {
    let principal_text = principal.to_string();
    tx.advisory_lock(
        AdvisoryClass::Idempotency,
        lock_key(&principal_text, &key.0),
    )
    .await?;
    let principal = uuid_of(&principal_text)?;
    let project = match tx.ctx.project {
        Some(p) => Some(uuid_of(&p.to_string())?),
        None => None,
    };
    let (k, r, b) = (key.0.clone(), route.to_owned(), body.0.to_vec());
    let found = tx
        .delegate(&platform(), async |t| {
            let row = sqlx::query(
                "SELECT project_id, route, request_sha256, status, response::text \
                 FROM platform.idempotency WHERE principal_id = $1 AND key = $2",
            )
            .bind(principal)
            .bind(&k)
            .fetch_optional(t.executor()?)
            .await
            .map_err(DbError::from_sqlx)?;
            if let Some(row) = row {
                return Ok(Lookup::Existing {
                    project: row.try_get(0).map_err(DbError::from_sqlx)?,
                    route: row.try_get(1).map_err(DbError::from_sqlx)?,
                    body: row.try_get(2).map_err(DbError::from_sqlx)?,
                    status: row.try_get(3).map_err(DbError::from_sqlx)?,
                    response: row.try_get(4).map_err(DbError::from_sqlx)?,
                });
            }
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO platform.idempotency (principal_id, key, project_id, route, request_sha256) \
                 VALUES ($1, $2, $3, $4, $5) RETURNING id",
            )
            .bind(principal)
            .bind(&k)
            .bind(project)
            .bind(&r)
            .bind(&b)
            .fetch_one(t.executor()?)
            .await
            .map_err(DbError::from_sqlx)?;
            Ok(Lookup::Reserved(id))
        })
        .await?;
    match found {
        Lookup::Reserved(id) => Ok(Idem::Proceed(IdemToken { id })),
        Lookup::Existing {
            project: p,
            route: stored_route,
            body: stored_body,
            status,
            response,
        } => {
            if p != project || stored_route != route || stored_body != body.0 {
                return Err(IdemError::KeyReused);
            }
            let (Some(status), Some(response)) = (status, response) else {
                // Reserved earlier in this very transaction and not finished.
                return Err(IdemError::KeyReused);
            };
            let v: serde_json::Value = serde_json::from_str(&response)
                .map_err(|_| DbError::Other("a stored response is not JSON".into()))?;
            let status = u16::try_from(status)
                .map_err(|_| DbError::Other("a stored status is out of range".into()))?;
            if v.get("marker").and_then(|m| m.as_str()) == Some("secret_not_stored") {
                let token_id = v
                    .get("token_id")
                    .and_then(|t| t.as_str())
                    .and_then(|t| TokenId::parse(t).ok())
                    .ok_or_else(|| DbError::Other("a secret marker without a token id".into()))?;
                return Err(IdemError::AlreadyIssued { token_id });
            }
            let body = v.get("body").cloned().unwrap_or(serde_json::Value::Null);
            Ok(Idem::Replay {
                status,
                response: StoredResponse::Body(body),
            })
        }
    }
}

/// Stores the response for the reserved key, in the same transaction as the effect.
pub async fn finish(
    tx: &mut ModuleTx,
    token: IdemToken,
    status: u16,
    response: StoredResponse,
) -> Result<(), IdemError> {
    let stored = match &response {
        StoredResponse::Body(v) => serde_json::json!({ "body": v }),
        // S2 §5: a secret-bearing response stores only the marker and the token's id.
        StoredResponse::SecretNotStored { token_id } => {
            serde_json::json!({ "marker": "secret_not_stored", "token_id": token_id.to_string() })
        }
    };
    let text = stored.to_string();
    let n = tx
        .delegate(&platform(), async |t| {
            sqlx::query(
                "UPDATE platform.idempotency SET status = $2, response = $3::jsonb WHERE id = $1",
            )
            .bind(token.id)
            .bind(i32::from(status))
            .bind(&text)
            .execute(t.executor()?)
            .await
            .map(|r| r.rows_affected())
            .map_err(DbError::from_sqlx)
        })
        .await?;
    if n == 1 {
        Ok(())
    } else {
        Err(IdemError::Db(DbError::Other(
            "the reserved key is gone".into(),
        )))
    }
}

/// Deletes rows older than `age` by `platform.now()` (24 h in production). Returns the count.
pub async fn sweep(tx: &mut ModuleTx, age: Duration) -> Result<u64, DbError> {
    let micros = i64::try_from(age.as_micros()).unwrap_or(i64::MAX);
    tx.delegate(&platform(), async |t| {
        sqlx::query(
            "DELETE FROM platform.idempotency WHERE created_at < platform.now() - $1 * interval '1 microsecond'",
        )
        .bind(micros)
        .execute(t.executor()?)
        .await
        .map(|r| r.rows_affected())
        .map_err(DbError::from_sqlx)
    })
    .await
}
