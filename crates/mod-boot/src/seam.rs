//! The reads, behind one seam. A port call needs a real `ModuleTx`, so a fake `IdentityApi`,
//! `RecordsApi` or `HandoffsApi` can only be called inside a database run. [`BootReads`] is the
//! four reads boot makes; [`PortReads`] passes them to the ports in the caller's transaction,
//! and the DB-free tests implement it over fakes.
//!
//! Every read is inside the token's project: the ports take it from `tx.context()` only, and no
//! read takes a project or a worker from the request (the worker is the principal's).

use async_trait::async_trait;
use cortex_db::{DbError, ModuleTx};
use cortex_module::{
    HandoffKind, HandoffView, HandoffsApi, IdentityApi, PortError, Principal, ProjectView,
    RecordsApi, SectionName, SectionView, WorkerView,
};
use cortex_types::WorkerId;

use crate::inputs::{HState, Handoff, Identity, Inputs, Kind};

/// How many handoffs boot asks for, per list (the cap per method, cox W1-33). More than any budget
/// can show; a list returned at this length means "there may be more", and its `more` line says
/// so.
pub const HANDOFF_LIMIT: u32 = 1000;

/// The four reads boot makes, in its one transaction.
#[async_trait]
pub trait BootReads: Send {
    async fn project(&mut self) -> Result<ProjectView, PortError>;
    async fn worker(&mut self, w: &WorkerId) -> Result<Option<WorkerView>, PortError>;
    async fn section(&mut self, name: &SectionName) -> Result<Option<SectionView>, PortError>;
    /// The worker's claimed handoffs, newest claim first, at most `limit` (W1-33).
    async fn claimed_by(&mut self, w: &WorkerId, limit: u32)
    -> Result<Vec<HandoffView>, PortError>;
    /// The pending handoffs the worker may claim: priority descending, `created_at` ascending, id;
    /// at most `limit` (W1-33).
    async fn pending_for(
        &mut self,
        w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError>;
}

/// The real reads: the ports, in the caller's `ModuleTx`.
pub struct PortReads<'a> {
    pub tx: &'a mut ModuleTx,
    pub identity: &'a dyn IdentityApi,
    pub records: &'a dyn RecordsApi,
    pub handoffs: &'a dyn HandoffsApi,
}

#[async_trait]
impl BootReads for PortReads<'_> {
    async fn project(&mut self) -> Result<ProjectView, PortError> {
        self.identity.project(self.tx).await
    }
    async fn worker(&mut self, w: &WorkerId) -> Result<Option<WorkerView>, PortError> {
        self.identity.worker(self.tx, w).await
    }
    async fn section(&mut self, name: &SectionName) -> Result<Option<SectionView>, PortError> {
        self.records.section_current(self.tx, name).await
    }
    async fn claimed_by(
        &mut self,
        w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        self.handoffs.claimed_by(self.tx, w, limit).await
    }
    async fn pending_for(
        &mut self,
        w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        self.handoffs.pending_for(self.tx, w, limit).await
    }
}

/// Read everything one boot shows. A port error stops the boot: no partial briefing (BOOT-52).
pub async fn gather(r: &mut dyn BootReads, p: &Principal) -> Result<Inputs, PortError> {
    let project = r.project().await?;
    // The transaction's project is the token's; a different answer is a provider defect, and boot
    // shows nothing rather than another project's identity.
    if project.id != p.project {
        return Err(PortError::NotFound);
    }
    let worker = r.worker(&p.worker).await?.ok_or(PortError::NotFound)?;
    let orientation = r.section(&SectionName::Orientation).await?;
    let rules_project = r.section(&SectionName::RulesProject).await?;
    let rules_worker = r.section(&SectionName::RulesWorker(p.worker)).await?;
    let claimed = r.claimed_by(&p.worker, HANDOFF_LIMIT).await?;
    let pending = r.pending_for(&p.worker, HANDOFF_LIMIT).await?;
    let more_claimed = claimed.len() >= HANDOFF_LIMIT as usize;
    let more_pending = pending.len() >= HANDOFF_LIMIT as usize;
    let mut handoffs = Vec::with_capacity(claimed.len() + pending.len());
    for (views, want) in [(&claimed, HState::Claimed), (&pending, HState::Pending)] {
        for v in views {
            let h = handoff_in(v)?;
            // A row in the wrong list is a provider defect too: refuse, never re-file it.
            if h.state != want {
                return Err(provider_defect(
                    "claimed_by or pending_for returned a row of the other state",
                ));
            }
            handoffs.push(h);
        }
    }
    Ok(Inputs {
        identity: Identity {
            worker: p.worker,
            name: worker.name,
            kind: p.kind,
            role: worker.role,
            job_roles: worker.job_roles,
            project: p.project,
            installation: project.installation,
            token: p.token,
            expires_at: p.expires_at,
            renew_due: p.renew_due,
        },
        orientation,
        rules_project,
        rules_worker,
        handoffs,
        more_claimed,
        more_pending,
    })
}

/// A provider broke the port's contract: 500 `internal_error` through the chain, no briefing.
fn provider_defect(why: &str) -> PortError {
    PortError::Db(DbError::Other(why.to_owned()))
}

/// One open handoff from the socket's view (bob's `mod-handoffs-PORT-TYPES-r0.md`, on
/// `wave1/ports`, cox W1-10). The lists return only `pending` and `claimed`; any other state is a
/// provider defect, and boot refuses the whole briefing with 500 rather than guess (the one
/// `PortError` that the chain answers as `internal_error` is a `Db` error with `Other`).
pub fn handoff_in(v: &HandoffView) -> Result<Handoff, PortError> {
    let state = match v.state.as_str() {
        "pending" => HState::Pending,
        "claimed" => HState::Claimed,
        _ => {
            return Err(provider_defect(
                "a handoff list returned a state other than pending or claimed",
            ));
        }
    };
    Ok(Handoff {
        id: v.id,
        kind: match v.kind {
            HandoffKind::Task => Kind::Task,
            HandoffKind::Consult => Kind::Consult,
        },
        state,
        priority: v.priority,
        created_by: v.from,
        created_at: v.created_at,
        summary: v.summary.clone(),
    })
}
