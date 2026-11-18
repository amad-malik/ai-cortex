//! What boot assembles from: owned values that [`crate::seam::gather`] read in one transaction.
//! Boot's own handoff type keeps the assembler independent of the socket's view type, whose real
//! form arrives with `wave1/ports` (cox W1-10).

use cortex_module::{Role, SectionView, WorkerKind};
use cortex_types::{HandoffId, InstallationId, ProjectId, Timestamp, TokenId, WorkerId};

/// Section 1: the token's principal plus its current identity row (S3 `Whoami`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub worker: WorkerId,
    pub name: String,
    pub kind: WorkerKind,
    /// The CURRENT role from `IdentityApi` (the registry wins over anything stale).
    pub role: Role,
    pub job_roles: Vec<String>,
    pub project: ProjectId,
    pub installation: InstallationId,
    pub token: TokenId,
    pub expires_at: Timestamp,
    pub renew_due: bool,
}

/// A handoff's kind (S2 `handoffs.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Task,
    Consult,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Task => "task",
            Kind::Consult => "consult",
        }
    }
}

/// The two states the handoff lists return (`claimed_by`: claimed; `pending_for`: pending).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HState {
    Pending,
    Claimed,
}

impl HState {
    pub fn as_str(self) -> &'static str {
        match self {
            HState::Pending => "pending",
            HState::Claimed => "claimed",
        }
    }
}

/// One open handoff: a briefing line, not the packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub id: HandoffId,
    pub kind: Kind,
    pub state: HState,
    /// 1 to 100, larger is more urgent (S2, S3).
    pub priority: i32,
    pub created_by: WorkerId,
    pub created_at: Timestamp,
    /// The whole summary (at most 2048 bytes in S3); boot cuts it only in the floor form.
    pub summary: String,
}

/// Everything one boot shows, read in one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inputs {
    pub identity: Identity,
    pub orientation: Option<SectionView>,
    pub rules_project: Option<SectionView>,
    pub rules_worker: Option<SectionView>,
    /// `claimed_by`'s rows (newest claim first), then `pending_for`'s (cox W1-33); `state` tells
    /// them apart.
    pub handoffs: Vec<Handoff>,
    /// `claimed_by` returned its limit, so more may exist than boot knows of.
    pub more_claimed: bool,
    /// `pending_for` returned its limit.
    pub more_pending: bool,
}
