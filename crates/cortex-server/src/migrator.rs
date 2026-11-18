//! The real `Migrator` (contract r0.4 section 5): applies each module's migration set under the OWNER role, forward only, each step in
//! its own transaction (`cortex_db::migrate::apply`). It reports how many steps were new, so a current schema is a no-op that says 0, and
//! on a failure it names the version of the step that failed. Nothing it prints can carry SQL, a password, a user name or a path.

use async_trait::async_trait;
use cortex_db::{MigrateError, MigrationSet, OwnerConn};

use crate::migrate::{Migrator, StepFailed};

pub struct DbMigrator {
    owner: OwnerConn,
    sets: Vec<MigrationSet>,
}

impl DbMigrator {
    pub fn new(owner: OwnerConn, sets: Vec<MigrationSet>) -> Self {
        DbMigrator { owner, sets }
    }

    /// The versions of `module` recorded in the ledger; empty when the ledger does not exist yet.
    async fn recorded(&mut self, module: &str) -> Result<Vec<i32>, ()> {
        let conn = self.owner.conn();
        let exists: bool =
            sqlx::query_scalar("SELECT to_regclass('platform.migrations') IS NOT NULL")
                .fetch_one(&mut *conn)
                .await
                .map_err(|_| ())?;
        if !exists {
            return Ok(Vec::new());
        }
        sqlx::query_scalar("SELECT version FROM platform.migrations WHERE module = $1")
            .bind(module)
            .fetch_all(conn)
            .await
            .map_err(|_| ())
    }
}

/// The version a [`MigrateError`] names, if it names one.
fn named_version(e: &MigrateError) -> Option<u32> {
    match e {
        MigrateError::OutsideSchema { version, .. }
        | MigrateError::StartTopologyNotSchemaClosed { version, .. }
        | MigrateError::StepLeftTopologyNotSchemaClosed { version, .. }
        | MigrateError::StepLeftHoldableCursor { version, .. }
        | MigrateError::StepLeftOpenCursor { version, .. }
        | MigrateError::ChecksumChanged { version } => Some(*version),
        _ => None,
    }
}

#[async_trait]
impl Migrator for DbMigrator {
    async fn apply(&mut self, module: &str) -> Result<u32, StepFailed> {
        let Some(idx) = self.sets.iter().position(|s| s.module.as_str() == module) else {
            return Err(StepFailed(0));
        };
        let before = self.recorded(module).await.map_err(|()| StepFailed(0))?;
        let result = cortex_db::migrate::apply(&mut self.owner, &self.sets[idx]).await;
        match result {
            Ok(()) => {
                let after = self.recorded(module).await.map_err(|()| StepFailed(0))?;
                Ok(after.len().saturating_sub(before.len()) as u32)
            }
            Err(e) => {
                if let Some(v) = named_version(&e) {
                    return Err(StepFailed(v));
                }
                // A plain database error names no step: the failing one is the first step of the set the ledger does not hold (steps
                // are applied in order, each committed on its own).
                // If the ledger cannot be read either (the connection died inside the step), the failing step is UNKNOWN: 0, never a guess
                // (hale F-1: an empty default named step 1, a step that was applied and committed).
                let Ok(done) = self.recorded(module).await else {
                    return Err(StepFailed(0));
                };
                let first_missing = self.sets[idx]
                    .steps
                    .iter()
                    .map(|s| s.version)
                    .find(|v| !done.contains(&(*v as i32)))
                    .unwrap_or(0);
                Err(StepFailed(first_missing))
            }
        }
    }
}
