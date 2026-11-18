//! The real [`Ledger`] (piece 11 `serve` S4b, mira): one read-only transaction under the `platform` module role, in which
//! `cortex_db::migrate::verify` compares each compiled and enabled migration set with `platform.migrations` (W1-41: `cortex_m_platform` is the
//! one role that can read the ledger, and `serve` holds only the app secret).
//!
//! - **Unreachable** is only "the pool could not get a connection" (`DbError::Unavailable`). A database that answers but will not let the
//!   `platform` role in (the role is missing, or the session ended) is reachable and every set is `Database`-faulted: the server is up and NOT
//!   READY, not "down".
//! - It writes nothing and takes no lock; the transaction is rolled back, so the pool connection goes straight back.
//! - A statement timeout bounds one read, so a stuck database cannot hold the (unauthenticated) health routes for ever.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cortex_db::migrate::verify;
use cortex_db::{
    Actor, DbError, DbPool, MigrateError, MigrationSet, ModuleTx, TxContext, TxOptions,
};
use cortex_types::{ModuleId, RequestId, SystemClock};

use crate::probe::{Ledger, LedgerFault, LedgerRead};

/// One ledger read may take this long inside the database.
pub const READ_TIMEOUT: Duration = Duration::from_secs(2);

pub struct DbLedger {
    pool: Arc<DbPool>,
    /// `platform`'s set first, then the enabled pieces' sets in migration order.
    sets: Vec<MigrationSet>,
    timeout: Duration,
}

impl DbLedger {
    pub fn new(pool: Arc<DbPool>, sets: Vec<MigrationSet>) -> Self {
        Self::with_timeout(pool, sets, READ_TIMEOUT)
    }

    pub fn with_timeout(pool: Arc<DbPool>, sets: Vec<MigrationSet>, timeout: Duration) -> Self {
        DbLedger {
            pool,
            sets,
            timeout,
        }
    }
}

fn fault(e: &MigrateError) -> LedgerFault {
    match e {
        MigrateError::OutOfOrder => LedgerFault::OutOfOrder,
        _ => LedgerFault::Database,
    }
}

#[async_trait]
impl Ledger for DbLedger {
    async fn read(&self) -> LedgerRead {
        let platform = ModuleId::parse("platform").expect("a valid module id");
        let ctx = TxContext {
            module: platform.clone(),
            project: None,
            actor: Actor::System(platform),
            request_id: RequestId::new(&SystemClock),
        };
        let opts = TxOptions {
            statement_timeout: Some(self.timeout),
        };
        let mut tx = match ModuleTx::begin(&self.pool, ctx, opts).await {
            Ok(tx) => tx,
            Err(DbError::Unavailable) => return LedgerRead::Unreachable,
            // The pool's module registry was built without `platform`: refused before any connection (a wiring defect).
            Err(DbError::UnknownModule) => return LedgerRead::PlatformNotRegistered,
            // The database answered but will not let the platform role in: reachable, and nothing could be checked.
            Err(_) => {
                return LedgerRead::Answered(
                    self.sets
                        .iter()
                        .map(|s| (s.module.as_str().to_owned(), Err(LedgerFault::Database)))
                        .collect(),
                );
            }
        };
        let mut states = Vec::with_capacity(self.sets.len());
        for set in &self.sets {
            let state = verify(&mut tx, set).await.map_err(|e| fault(&e));
            states.push((set.module.as_str().to_owned(), state));
        }
        // Read-only: nothing to commit. The rollback hands the connection straight back to the pool.
        let _ = tx.rollback().await;
        LedgerRead::Answered(states)
    }
}
