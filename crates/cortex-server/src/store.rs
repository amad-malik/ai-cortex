//! The real `BootstrapStore` (contract r0.4 section 5; W1-35 A, W1-18): the host path as TWO host transactions.
//!
//! (a) `find`: an install-level `begin_host` (project `None`), `mod_identity::find_project`, rolled back; then, when the project
//!     exists, a read-only transaction under that project to learn whether the named lead is its lead, only a worker of it, or
//!     unknown.
//! (b) `create_or_reissue`: `begin_host` with `Some(found id)` for a re-run or `Some(fresh id)` for the first run, then
//!     `bootstrap_project`, the token mint and the one `project.bootstrapped` audit row, all in THAT transaction; `OtherProject(p)`
//!     (a race lost, a stale find) rolls back and repeats with `p`, at most [`MAX_ATTEMPTS`] times. The transaction is kept open
//!     until `commit` (called only after the token file was written) or `rollback`.
//!

use async_trait::async_trait;
use cortex_db::{DbPool, HostContext, ModuleTx, TxOptions};
use cortex_module::{AuditDetails, AuditSink, AuditTarget, IdentityApi};
use cortex_types::{Clock, ProjectId, RequestId, Secret, WorkerId};
use mod_identity::{BootstrapError, FoundProject, bootstrap_project, find_project, labels};

use crate::bootstrap::{BootstrapStore, Found, StoreError};

/// At most this many attempts of the second transaction (the first plus the retries after `OtherProject`).
pub const MAX_ATTEMPTS: u32 = 3;

/// Why a mint failed. No detail: it can come from the entropy source or the database and must never carry row data.
#[derive(Debug, PartialEq, Eq)]
pub struct MintError;

/// Mints the lead's token INSIDE the open host transaction. `reissue` is true for a re-run. The real implementation is mod-tokens'
/// `issue_in_tx` (finn); the seam exists because piece 3 is not merged yet and so a failing mint can be tested.
#[async_trait]
pub trait Minter: Send + Sync {
    async fn mint(
        &self,
        tx: &mut cortex_db::ModuleTx,
        lead: WorkerId,
        reissue: bool,
    ) -> Result<Secret, MintError>;
}

pub struct DbStore<'a> {
    pool: &'a DbPool,
    identity: &'a dyn IdentityApi,
    audit: &'a dyn AuditSink,
    minter: &'a dyn Minter,
    clock: &'a dyn Clock,
    /// The operator's uid at the host socket (recorded as the actor of the host transaction).
    uid: u32,
    /// What `find` saw: the project's id for the second transaction (a re-run), or `None` (the first run: a fresh id).
    found: Option<FoundProject>,
    /// The open second transaction, from `create_or_reissue` until `commit` or `rollback`.
    tx: Option<ModuleTx>,
}

impl<'a> DbStore<'a> {
    pub fn new(
        pool: &'a DbPool,
        identity: &'a dyn IdentityApi,
        audit: &'a dyn AuditSink,
        minter: &'a dyn Minter,
        clock: &'a dyn Clock,
        uid: u32,
    ) -> Self {
        DbStore {
            pool,
            identity,
            audit,
            minter,
            clock,
            uid,
            found: None,
            tx: None,
        }
    }

    async fn begin(&self, project: Option<ProjectId>) -> Result<ModuleTx, StoreError> {
        ModuleTx::begin_host(
            self.pool,
            HostContext {
                uid: self.uid,
                project,
                request_id: RequestId::new(self.clock),
            },
            TxOptions::default(),
        )
        .await
        .map_err(|_| StoreError("could not begin the host transaction".into()))
    }
}

/// An error that carries no name, id or row data.
fn err(text: &str) -> StoreError {
    StoreError(text.to_owned())
}

#[async_trait]
impl BootstrapStore for DbStore<'_> {
    /// Read only. Nothing is written and nothing stays open.
    async fn find(&mut self, project: &str, lead: &str) -> Result<Found, StoreError> {
        self.found = None;
        let mut tx0 = self.begin(None).await?;
        let found = find_project(&mut tx0, project).await;
        let _ = tx0.rollback().await;
        let found = match found {
            Ok(f) => f,
            Err(_) => return Err(err("could not read the project state")),
        };
        let Some(f) = found else {
            return Ok(Found::Nothing);
        };
        self.found = Some(f);
        // Is the named lead the project's lead, only a worker of it, or unknown? Read under the project's own row-level security.
        let want = labels::name_key(lead);
        let mut tx1 = self.begin(Some(f.project)).await?;
        let verdict = async {
            let leader = self
                .identity
                .worker(&mut tx1, &f.lead)
                .await
                .map_err(|_| err("could not read the project state"))?;
            if leader.as_ref().and_then(|w| labels::name_key(&w.name)) == want && want.is_some() {
                return Ok(Found::ProjectAndLead);
            }
            let roster = self
                .identity
                .roster(&mut tx1)
                .await
                .map_err(|_| err("could not read the project state"))?;
            if roster
                .iter()
                .any(|w| labels::name_key(&w.name) == want && want.is_some())
            {
                Ok(Found::LeadNotProjectLead)
            } else {
                Ok(Found::ProjectOnly)
            }
        }
        .await;
        let _ = tx1.rollback().await;
        verdict
    }

    /// The second host transaction: create or find, mint, audit. Kept open for `commit`.
    async fn create_or_reissue(&mut self, project: &str, lead: &str) -> Result<Secret, StoreError> {
        let mut id = self.found.map(|f| f.project);
        for _attempt in 0..MAX_ATTEMPTS {
            let mut tx = self
                .begin(Some(id.unwrap_or_else(|| ProjectId::new(self.clock))))
                .await?;
            match bootstrap_project(&mut tx, project, lead).await {
                Ok(b) => {
                    let token = match self.minter.mint(&mut tx, b.lead, !b.created).await {
                        Ok(t) => t,
                        Err(MintError) => {
                            let _ = tx.rollback().await;
                            return Err(err("could not mint the token"));
                        }
                    };
                    let details = AuditDetails::new()
                        .id("project", b.project)
                        .and_then(|d| d.id("lead", b.lead))
                        .and_then(|d| d.flag("created", b.created))
                        .and_then(|d| d.flag("reissued", !b.created));
                    let Ok(details) = details else {
                        let _ = tx.rollback().await;
                        return Err(err("could not build the audit record"));
                    };
                    if self
                        .audit
                        .record(
                            &mut tx,
                            "project.bootstrapped",
                            AuditTarget::Project(b.project),
                            details,
                        )
                        .await
                        .is_err()
                    {
                        let _ = tx.rollback().await;
                        return Err(err("could not write the audit record"));
                    }
                    self.tx = Some(tx);
                    return Ok(token);
                }
                Err(BootstrapError::OtherProject(p)) => {
                    // A race lost or a stale find: the project exists under another id. Begin again with that id.
                    let _ = tx.rollback().await;
                    id = Some(p);
                }
                Err(BootstrapError::Conflict) => {
                    let _ = tx.rollback().await;
                    return Err(err("conflict"));
                }
                Err(_) => {
                    let _ = tx.rollback().await;
                    return Err(err("could not create the project or re-issue the token"));
                }
            }
        }
        Err(err("could not create the project: too many attempts"))
    }

    async fn commit(&mut self) -> Result<(), StoreError> {
        match self.tx.take() {
            Some(tx) => tx.commit().await.map_err(|_| err("could not commit")),
            None => Err(err("nothing to commit")),
        }
    }

    async fn rollback(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.rollback().await;
        }
    }
}
