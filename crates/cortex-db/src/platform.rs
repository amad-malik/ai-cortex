//! `platform.objects` and `platform.refs` (CONTRACTS §3; S2 §6). Module roles may `INSERT` and
//! `SELECT` them, never `UPDATE` or `DELETE` (F1-14). Both run under the current module's role, in
//! its project (row security), through `executor()`, so the lock order's seal applies.
use crate::error::DbError;
use crate::tx::ModuleTx;

fn project(tx: &ModuleTx) -> Option<uuid::Uuid> {
    tx.ctx
        .project
        .map(|p| uuid::Uuid::parse_str(&p.to_string()).expect("a ProjectId is a UUID"))
}

pub async fn insert_object(tx: &mut ModuleTx, id: uuid::Uuid, kind: &str) -> Result<(), DbError> {
    let project = project(tx);
    let module = tx.current.as_str().to_owned();
    sqlx::query(
        "INSERT INTO platform.objects (id, project_id, kind, module) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(project)
    .bind(kind)
    .bind(module)
    .execute(tx.executor()?)
    .await
    .map(|_| ())
    .map_err(DbError::from_sqlx)
}

pub async fn insert_ref(
    tx: &mut ModuleTx,
    from: uuid::Uuid,
    to: uuid::Uuid,
    kind: &str,
) -> Result<(), DbError> {
    let project = project(tx);
    sqlx::query(
        "INSERT INTO platform.refs (project_id, from_id, to_id, kind) VALUES ($1, $2, $3, $4)",
    )
    .bind(project)
    .bind(from)
    .bind(to)
    .bind(kind)
    .execute(tx.executor()?)
    .await
    .map(|_| ())
    .map_err(DbError::from_sqlx)
}

/// Locators only.
pub async fn refs_to(
    tx: &mut ModuleTx,
    to: uuid::Uuid,
    kind: &str,
) -> Result<Vec<uuid::Uuid>, DbError> {
    sqlx::query_scalar(
        "SELECT from_id FROM platform.refs WHERE to_id = $1 AND kind = $2 ORDER BY from_id",
    )
    .bind(to)
    .bind(kind)
    .fetch_all(tx.executor()?)
    .await
    .map_err(DbError::from_sqlx)
}
