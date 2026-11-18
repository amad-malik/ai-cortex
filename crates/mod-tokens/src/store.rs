//! The SQL side of verification: [`SqlStore`] implements [`VerifyStore`] over the piece's own schema and the
//! `IdentityApi` (S1 4.2; cells TOK-03, 10 to 12, 14 to 17, 19, 46, GEN-1 to GEN-7 on a database).
//!
//! **Two transactions per verification, both `ModuleTx` as module `tokens`.** The first has NO project: it calls
//! `tokens.verify_lookup(id)`, the private SECURITY DEFINER function that finds ONE token by its id before any
//! project is known (the project comes FROM the token), and commits. **This is the only mention of the function in
//! the crate** (TOK-20: a handler that called it could read another project's token). The second has the token's
//! project: it asks `IdentityApi::worker` for the worker's role, team and kind under that project's RLS, writes
//! `last_used_at` when due, and commits whatever happens to the request (C6).
//!
//! Every failure of the database is a [`StoreError`] (the verifier answers `Unavailable`); a stored value that does
//! not fit its type is the same, never a guess. Each transaction has a statement timeout.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use cortex_db::{Actor, DbError, DbPool, ModuleTx, TxContext, TxOptions};
use cortex_module::{IdentityApi, WorkerView};
use cortex_types::{Clock, ModuleId, ProjectId, RequestId, Timestamp, TokenId, WorkerId};
use sqlx::Row;
use sqlx::postgres::PgRow;

use crate::verify::{StoreError, StoredToken, TOUCH_INTERVAL_MICROS, VerifySession, VerifyStore};

/// The default statement timeout of a verification's transactions.
pub const DEFAULT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(5);

/// The two things the verifier needs that exist only after the server has assembled the pieces: the database pool
/// (built from the registry's module list) and the `IdentityApi` (a port, available when routes are built). The
/// piece binds them when it gets them; **until both are bound every verification is `Unavailable` (503), never a
/// pass** (fail closed).
#[derive(Default)]
pub struct Late {
    pool: OnceLock<Arc<DbPool>>,
    identity: OnceLock<Arc<dyn IdentityApi>>,
}

impl Late {
    /// Binds the pool once; `false` if it was already bound.
    pub fn bind_pool(&self, pool: Arc<DbPool>) -> bool {
        self.pool.set(pool).is_ok()
    }
    /// Binds the identity port once; `false` if it was already bound.
    pub fn bind_identity(&self, identity: Arc<dyn IdentityApi>) -> bool {
        self.identity.set(identity).is_ok()
    }
    /// Both are bound.
    pub fn ready(&self) -> bool {
        self.pool.get().is_some() && self.identity.get().is_some()
    }
}

/// The database store of `Verifier`.
pub struct SqlStore {
    late: Arc<Late>,
    clock: Arc<dyn Clock>,
    timeout: Duration,
}

impl SqlStore {
    /// A store whose pool and identity are known now.
    pub fn new(
        pool: Arc<DbPool>,
        identity: Arc<dyn IdentityApi>,
        clock: Arc<dyn Clock>,
    ) -> SqlStore {
        let late = Arc::new(Late::default());
        late.bind_pool(pool);
        late.bind_identity(identity);
        SqlStore::late(late, clock)
    }

    /// A store whose pool and identity are bound later (the piece's own wiring).
    pub fn late(late: Arc<Late>, clock: Arc<dyn Clock>) -> SqlStore {
        SqlStore {
            late,
            clock,
            timeout: DEFAULT_STATEMENT_TIMEOUT,
        }
    }

    /// Another statement timeout (a cell makes it short to see `Unavailable`).
    pub fn with_statement_timeout(mut self, timeout: Duration) -> SqlStore {
        self.timeout = timeout;
        self
    }
}

#[async_trait]
impl VerifyStore for SqlStore {
    async fn open(&self) -> Result<Box<dyn VerifySession>, StoreError> {
        let pool = self.late.pool.get().ok_or(StoreError)?.clone();
        let identity = self.late.identity.get().ok_or(StoreError)?.clone();
        Ok(Box::new(SqlSession {
            pool,
            identity,
            clock: self.clock.clone(),
            timeout: self.timeout,
            tx: None,
        }))
    }
}

/// One verification's database session: the lookup transaction is opened and closed inside `lookup`; the
/// project transaction is opened by `worker` and closed by `commit`.
struct SqlSession {
    pool: Arc<DbPool>,
    identity: Arc<dyn IdentityApi>,
    clock: Arc<dyn Clock>,
    timeout: Duration,
    tx: Option<ModuleTx>,
}

impl SqlSession {
    async fn begin(&self, project: Option<ProjectId>) -> Result<ModuleTx, StoreError> {
        let module = ModuleId::parse(crate::ID).map_err(|_| StoreError)?;
        let ctx = TxContext {
            actor: Actor::System(module.clone()),
            module,
            project,
            request_id: RequestId::new(&*self.clock),
        };
        ModuleTx::begin(
            &self.pool,
            ctx,
            TxOptions {
                statement_timeout: Some(self.timeout),
            },
        )
        .await
        .map_err(|_| StoreError)
    }
}

fn row_to_token(r: &PgRow) -> Result<StoredToken, StoreError> {
    let e = |_: sqlx::Error| StoreError;
    let micros = |v: i64| Timestamp::from_unix_micros(v);
    let digest: Vec<u8> = r.try_get("digest").map_err(e)?;
    let id: String = r.try_get("id").map_err(e)?;
    let worker: String = r.try_get("worker_id").map_err(e)?;
    let project: String = r.try_get("project_id").map_err(e)?;
    Ok(StoredToken {
        id: TokenId::parse(&id).map_err(|_| StoreError)?,
        worker: WorkerId::parse(&worker).map_err(|_| StoreError)?,
        project: ProjectId::parse(&project).map_err(|_| StoreError)?,
        digest: digest.try_into().map_err(|_| StoreError)?,
        install_gen: r.try_get("install_gen").map_err(e)?,
        project_gen: r.try_get("project_gen").map_err(e)?,
        issued_at: micros(r.try_get("issued_us").map_err(e)?),
        expires_at: micros(r.try_get("expires_us").map_err(e)?),
        renew_after: micros(r.try_get("renew_us").map_err(e)?),
        last_used_at: r
            .try_get::<Option<i64>, _>("last_used_us")
            .map_err(e)?
            .map(micros),
        revoked_at: r
            .try_get::<Option<i64>, _>("revoked_us")
            .map_err(e)?
            .map(micros),
        revoke_reason: r.try_get("revoke_reason").map_err(e)?,
        worker_state: r.try_get("worker_state").map_err(e)?,
        current_install_gen: r.try_get("current_install_gen").map_err(e)?,
        current_project_gen: r.try_get("current_project_gen").map_err(e)?,
    })
}

#[async_trait]
impl VerifySession for SqlSession {
    async fn lookup(&mut self, id: &TokenId) -> Result<Option<StoredToken>, StoreError> {
        let mut tx = self.begin(None).await?;
        let row: Option<PgRow> = sqlx::query(
            "SELECT l.id::text AS id, l.worker_id::text AS worker_id, l.project_id::text AS project_id, \
                    l.digest AS digest, l.install_gen AS install_gen, l.project_gen AS project_gen, \
                    (extract(epoch FROM l.issued_at) * 1000000)::bigint AS issued_us, \
                    (extract(epoch FROM l.expires_at) * 1000000)::bigint AS expires_us, \
                    (extract(epoch FROM l.renew_after) * 1000000)::bigint AS renew_us, \
                    (extract(epoch FROM l.last_used_at) * 1000000)::bigint AS last_used_us, \
                    (extract(epoch FROM l.revoked_at) * 1000000)::bigint AS revoked_us, \
                    l.revoke_reason AS revoke_reason, l.worker_state AS worker_state, \
                    l.current_install_gen AS current_install_gen, l.current_project_gen AS current_project_gen \
             FROM tokens.verify_lookup($1::uuid) l",
        )
        .bind(id.to_string())
        .fetch_optional(tx.executor().map_err(|_| StoreError)?)
        .await
        .map_err(|e| {
            let _ = DbError::from(e);
            StoreError
        })?;
        tx.commit().await.map_err(|_| StoreError)?;
        row.as_ref().map(row_to_token).transpose()
    }

    async fn worker(
        &mut self,
        project: &ProjectId,
        w: &WorkerId,
    ) -> Result<Option<WorkerView>, StoreError> {
        if self.tx.is_none() {
            self.tx = Some(self.begin(Some(*project)).await?);
        }
        let tx = self.tx.as_mut().ok_or(StoreError)?;
        self.identity.worker(tx, w).await.map_err(|_| StoreError)
    }

    async fn touch(
        &mut self,
        project: &ProjectId,
        id: &TokenId,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        if self.tx.is_none() {
            self.tx = Some(self.begin(Some(*project)).await?);
        }
        let tx = self.tx.as_mut().ok_or(StoreError)?;
        // Only last_used_at; never first_used_at (OQ-T3). Exact integer arithmetic, no floating point.
        //
        // The one-minute rule is in the UPDATE, against the STORED value (vera's TOKENS-V3): the decision made on
        // the row read in the first transaction is only an optimisation, and two verifications that read the same
        // row write once. The same condition keeps the stamp from going backwards when completions arrive in the
        // other order (a stored stamp newer than `now - 60 s` is never overwritten by an older `now`). `<=` is
        // `touch_due`'s `>=` on the difference.
        sqlx::query(
            "UPDATE tokens.tokens \
             SET last_used_at = timestamptz 'epoch' + $2::bigint * interval '1 microsecond' \
             WHERE id = $1::uuid \
               AND (last_used_at IS NULL \
                    OR last_used_at <= timestamptz 'epoch' + ($2::bigint - $3::bigint) * interval '1 microsecond')",
        )
        .bind(id.to_string())
        .bind(now.unix_micros())
        .bind(TOUCH_INTERVAL_MICROS)
        .execute(tx.executor().map_err(|_| StoreError)?)
        .await
        .map_err(|_| StoreError)?;
        Ok(())
    }

    async fn commit(self: Box<Self>) -> Result<(), StoreError> {
        match self.tx {
            Some(tx) => tx.commit().await.map_err(|_| StoreError),
            None => Ok(()),
        }
    }
}
