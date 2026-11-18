//! The six ports and their view types (section 1.5).
//!
//! **The rules every port obeys.**
//! - **Direction:** the consumer calls the provider synchronously, inside the caller's
//!   `ModuleTx`. The provider runs its SQL through `tx.delegate(&provider, …)`, under its own role,
//!   in the same transaction. A port call never opens a second transaction, sees the caller's
//!   writes, and a failure rolls back both.
//! - **Lifetime:** nothing outlives the call. Results are owned values.
//! - **Project and actor:** from `tx.context()` only. No port method takes a project or an actor.
//! - **Errors:** [`PortError`], codes only, never row text. `Db` poisons the caller's transaction.
//! - **Tables:** a port reads its provider's tables through the provider's code only.
//!
//! The two auth-hook ports run before a `Ctx` exists: `TokenVerifier` owns its transaction, and
//! `ScopePolicy` is pure.
//!
//! **The view types are PROVISIONAL** where marked: their fields come from the S2 data pack
//! (nemo) and the S3 API pack (cox), and are fixed at the freeze.

use std::sync::Arc;

use async_trait::async_trait;
use cortex_db::{DbError, ModuleTx};
use cortex_types::{
    HandoffId, InstallationId, ProjectId, RecordId, ScopeName, Secret, Sha256, TeamId, Timestamp,
    TokenId, WorkerId,
};

use crate::manifest::Port;

/// A port's error: a code, never row text.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PortError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(&'static str),
    #[error("invalid: {0}")]
    Invalid(&'static str),
    #[error("forbidden")]
    Forbidden,
    #[error("unavailable")]
    Unavailable,
    /// Poisons the caller's transaction (S1).
    #[error(transparent)]
    Db(#[from] DbError),
}

/// Why `TokenVerifier::verify` refused (mira's C5). A wrong secret is `Unknown`.
#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("no token")]
    Missing,
    #[error("malformed token")]
    Malformed,
    #[error("unknown token")]
    Unknown,
    #[error("expired token")]
    Expired,
    #[error("revoked token")]
    Revoked,
    #[error("retired worker")]
    Retired,
    #[error("token store unavailable")]
    Unavailable,
}

/// A worker or a service (an automation); the same type for the caller and for a target
/// (sophi's F-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkerKind {
    Worker,
    Service,
}

/// The caller's role in its project, with its team where it has one (sophi's F-3), so that
/// team-bound target rules stay pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    ProjectLead,
    TeamLead(TeamId),
    Member(Option<TeamId>),
}

/// Who is calling: made only by `TokenVerifier::verify` (mira's C6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub worker: WorkerId,
    /// The token's project; the chain copies it into `Ctx::project` (r5).
    pub project: ProjectId,
    pub kind: WorkerKind,
    pub role: Role,
    /// The token's id, never the token.
    pub token: TokenId,
    pub expires_at: Timestamp,
    pub renew_due: bool,
}

/// The row a target rule is checked against; the handler fills it from the row it read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Target {
    pub worker: Option<WorkerId>,
    /// The target worker's kind (sophi's F-5): "not to services" is then a pure rule.
    pub kind: Option<WorkerKind>,
    pub team: Option<TeamId>,
    pub lead: Option<WorkerId>,
    /// True when the handoff being created is a consult, which any principal that passed the scope
    /// check may send (W1-9; `mod-handoffs-PORT-TYPES-r0.md`). **Set only by `handoffs.create`**, from the
    /// decoded top-level body field `kind` (`"consult"` or `"task"`), and nothing else; **read only by
    /// `TaskAddressee`**. Note that the S3 body has two fields called `kind`: this flag comes from the
    /// handoff's own, never from the addressee's `to.kind`. Default `false`, and a plain `bool`: no
    /// route that is not a handoff create has a reason to set it.
    pub consult: bool,
}

/// Every route declares one (cox, 20:35Z), named in the S3 YAML as `x-target` (R-17); the
/// handler checks it with `ScopePolicy::target`, in its transaction, against the row it read.
/// The set is exactly sophi's eight (table 3.2 in `mvp/tests/mod-identity-and-mod-policy.md`;
/// known delta 3). What each rule allows is mod-policy's truth table, not S0's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetRule {
    /// No row target (table 3.2 `None`; R-17).
    None,
    /// The caller is the target worker (table 3.2 `SelfWorker`; R-17).
    SelfWorker,
    /// The caller is the target worker, or the project lead (table 3.2 `SelfOrProjectLead`; R-17).
    SelfOrProjectLead,
    /// The target worker and the lead are both known and differ (table 3.2 `NotProjectLead`; R-17).
    NotProjectLead,
    /// Who may retire a worker: never oneself or the lead (table 3.2 `RetireTarget`; R-17).
    RetireTarget,
    /// Who may issue, revoke or inspect a worker's token (table 3.2 `TokenTarget`; R-17).
    TokenTarget,
    /// Who may supersede or invalidate a record (table 3.2 `Lineage`; R-17).
    Lineage,
    /// Who may address a task handoff (table 3.2 `TaskAddressee`; R-17).
    TaskAddressee,
}

/// `TokenVerifier` (mod-tokens): called by the chain once per request.
///
/// Its own transaction: the digest lookup, the role from `IdentityApi`; it writes nothing but
/// `last_used`, and commits whatever happens to the request (C6). The bearer is a `Secret`: never
/// logged, never in an error, never stored (only its digest).
#[async_trait]
pub trait TokenVerifier: Send + Sync {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError>;
}

/// `ScopePolicy` (mod-policy): pure, no I/O.
pub trait ScopePolicy: Send + Sync {
    /// An unknown role or scope: `false`. The scope only (C7).
    fn allows(&self, p: &Principal, scope: &ScopeName) -> bool;
    /// Pure; the handler passes the row it read.
    fn target(&self, p: &Principal, rule: TargetRule, t: &Target) -> bool;
}

/// What an audit row is about: ids only, never text. PROVISIONAL (mod-audit's pack).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditTarget {
    None,
    Project(ProjectId),
    Team(TeamId),
    Worker(WorkerId),
    Token(TokenId),
    Record(RecordId),
    Handoff(HandoffId),
}

// ---- Audit details (delta 5, part 2) ----

/// What an audit detail may refer to: an id, never text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailRef {
    Project(ProjectId),
    Team(TeamId),
    Worker(WorkerId),
    Token(TokenId),
    Record(RecordId),
    Handoff(HandoffId),
}

impl DetailRef {
    /// The id's canonical text: lowercase, hyphenated.
    fn text(&self) -> String {
        match self {
            DetailRef::Project(v) => v.to_string(),
            DetailRef::Team(v) => v.to_string(),
            DetailRef::Worker(v) => v.to_string(),
            DetailRef::Token(v) => v.to_string(),
            DetailRef::Record(v) => v.to_string(),
            DetailRef::Handoff(v) => v.to_string(),
        }
    }
}

macro_rules! detail_ref_from {
    ($($id:ident => $variant:ident),+ $(,)?) => {$(
        impl From<$id> for DetailRef {
            fn from(v: $id) -> Self {
                DetailRef::$variant(v)
            }
        }
    )+};
}
detail_ref_from!(
    ProjectId => Project,
    TeamId => Team,
    WorkerId => Worker,
    TokenId => Token,
    RecordId => Record,
    HandoffId => Handoff,
);

/// The grammar of an entry key and of a code: `^[a-z][a-z0-9_]{0,31}$`. No dot, colon, hyphen,
/// space, quote, backslash, control character or capital, and at most 32 characters, so a token
/// (`ctx2_`, 32 hex, a dot, 43 characters) fails twice, and the text needs no JSON escaping.
fn is_word(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

/// A reason code that passed the grammar. It has no public constructor: `AuditDetails::code` is the
/// only way in, and it validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailCode(String);

impl DetailCode {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One audit detail value: an id, a validated code or a boolean. There is no variant that holds a
/// free string, a number or a nested value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailValue {
    Ref(DetailRef),
    Code(DetailCode),
    Flag(bool),
}

/// The `details` of an audit row: a closed type (delta 5; kai's rule: a token-shaped string must be
/// unrecordable). Ids, codes and booleans only, under keys of the same grammar: at most
/// [`AuditDetails::MAX_ENTRIES`] entries, no key twice. Every builder is fallible, and a refusal is
/// `PortError::Invalid` with a fixed message: the value is never truncated, escaped or echoed.
///
/// The provider stores [`AuditDetails::render_json`] in `audit.records.details`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditDetails(Vec<(&'static str, DetailValue)>);

impl AuditDetails {
    /// Far under the table's 16 KiB check: at most 16 entries of at most 32 + 36 characters.
    pub const MAX_ENTRIES: usize = 16;

    /// No details: most audited operations have none.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    fn push(mut self, key: &'static str, v: DetailValue) -> Result<Self, PortError> {
        if !is_word(key) {
            return Err(PortError::Invalid("audit detail key"));
        }
        if self.0.len() >= Self::MAX_ENTRIES {
            return Err(PortError::Invalid("audit detail entries"));
        }
        if self.0.iter().any(|(k, _)| *k == key) {
            return Err(PortError::Invalid("audit detail key repeated"));
        }
        self.0.push((key, v));
        Ok(self)
    }

    /// An id (of a project, team, worker, token, record or handoff), recorded as its UUID text.
    pub fn id(self, key: &'static str, v: impl Into<DetailRef>) -> Result<Self, PortError> {
        self.push(key, DetailValue::Ref(v.into()))
    }

    /// A reason code. `^[a-z][a-z0-9_]{0,31}$`, anything else is `PortError::Invalid`.
    pub fn code(self, key: &'static str, v: &str) -> Result<Self, PortError> {
        if !is_word(v) {
            return Err(PortError::Invalid("audit detail code"));
        }
        self.push(key, DetailValue::Code(DetailCode(v.to_owned())))
    }

    /// A boolean.
    pub fn flag(self, key: &'static str, v: bool) -> Result<Self, PortError> {
        self.push(key, DetailValue::Flag(v))
    }

    /// The entries, in the order they were added.
    pub fn entries(&self) -> &[(&'static str, DetailValue)] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The JSON object the provider stores, in entry order: `{"reason":"lost_key","self_review":true}`.
    /// Keys, codes and ids are all inside `[a-z0-9_-]`, so nothing is escaped, because nothing needs to be.
    pub fn render_json(&self) -> String {
        let mut out = String::from("{");
        for (i, (key, value)) in self.0.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(key);
            out.push_str("\":");
            match value {
                DetailValue::Ref(r) => {
                    out.push('"');
                    out.push_str(&r.text());
                    out.push('"');
                }
                DetailValue::Code(c) => {
                    out.push('"');
                    out.push_str(c.as_str());
                    out.push('"');
                }
                DetailValue::Flag(b) => out.push_str(if *b { "true" } else { "false" }),
            }
        }
        out.push('}');
        out
    }
}

/// `AuditSink` (mod-audit): inside the caller's transaction, a plain top-level INSERT (T-2), so
/// the operation and its audit row commit or roll back together.
///
/// **Committed operations only (DV-3, kai e351c8ca):** 0.2.001 audits what committed, so there is no
/// outcome parameter; the provider writes `allowed`. A refusal is a structured log line (the
/// chain's), not a row; a later denial log is a separate method, so this signature does not change twice.
/// `details` is the closed [`AuditDetails`]: ids, codes and booleans, never text.
#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn record(
        &self,
        tx: &mut ModuleTx,
        action: &'static str,
        target: AuditTarget,
        details: AuditDetails,
    ) -> Result<(), PortError>;
}

/// A project, as `IdentityApi` shows it. `state` is PROVISIONAL (S2's states).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectView {
    pub id: ProjectId,
    pub name: String,
    pub state: String,
    /// The project lead (bob's ask for mod-handoffs; D-19).
    pub lead: WorkerId,
    /// The installation the project belongs to (delta 5; S3's `Whoami.installation`). Never an HTTP
    /// body field: `ProjectView` is not `Serialize`, and `GET /v2/projects/{id}` stays `{id, name}`.
    pub installation: InstallationId,
    /// How long the project's new tokens live, in days (S1 A3: 7 to 365, default 90). The tokens role cannot read
    /// `identity.projects`, so the provider carries it here, from the same row as `installation` (wave1/ports3). Never
    /// an HTTP body field, like `installation`.
    pub token_lifetime_days: i32,
}

/// A worker, as `IdentityApi` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerView {
    pub id: WorkerId,
    pub name: String,
    /// r5: a handler builds a `Target` from it (sophi's F-5).
    pub kind: WorkerKind,
    pub role: Role,
    /// bob's ask for mod-handoffs (D-19).
    pub job_roles: Vec<String>,
    pub active: bool,
}

/// `IdentityApi` (mod-identity): used by tokens, handoffs and boot. The project is the
/// transaction's.
#[async_trait]
pub trait IdentityApi: Send + Sync {
    async fn project(&self, tx: &mut ModuleTx) -> Result<ProjectView, PortError>;
    async fn worker(
        &self,
        tx: &mut ModuleTx,
        w: &WorkerId,
    ) -> Result<Option<WorkerView>, PortError>;
    async fn roster(&self, tx: &mut ModuleTx) -> Result<Vec<WorkerView>, PortError>;
}

/// A record's lineage state (S2 pack: `records.state`; a missing row means `Active`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordState {
    Active,
    Superseded,
    Invalidated,
}

/// A knowledge record's source (S2 pack: `source_kind`, `source_ref`, `source_span`). `span` is `None`
/// in 0.2.001.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceView {
    pub kind: String,
    pub reference: String,
    pub span: Option<String>,
}

/// A record as stored, with its lineage state (nemo's S2 revision 6 `records.sql`; W1-10 b). Records are
/// immutable, so there is no revision and no update time.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordView {
    pub id: RecordId,
    /// The type, hyphenated: decision, lesson, note, status, question, blocker, knowledge, work-product.
    pub kind: String,
    pub summary: String,
    /// `None` when the record has no body.
    pub body: Option<String>,
    /// Always a JSON object.
    pub fields: serde_json::Value,
    pub source: Option<SourceView>,
    pub state: RecordState,
    /// `Some` only when `state` is `Superseded`.
    pub superseded_by: Option<RecordId>,
    /// `worker`, `service`, `human` or `system`.
    pub author_kind: String,
    /// A worker uuid, `<source>:<id>` for a human, a module id for the system.
    pub author: String,
    pub provenance: String,
    pub section: Option<String>,
    pub section_revision: Option<i32>,
    pub created_at: Timestamp,
    pub valid_until: Option<Timestamp>,
}

/// Where `changed_since` resumes: a keyset position on `(created_at, id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordCursor {
    pub at: Timestamp,
    pub id: RecordId,
}

/// One page of `changed_since`.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordPage {
    pub records: Vec<RecordView>,
    pub next: Option<RecordCursor>,
}

/// A boot section's name (tate, r4): `orientation`, `rules-project` or `rules-worker:<id>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionName {
    Orientation,
    RulesProject,
    RulesWorker(WorkerId),
}

impl SectionName {
    /// The three forms only; anything else is `None`.
    pub fn parse(s: &str) -> Option<SectionName> {
        match s {
            "orientation" => Some(SectionName::Orientation),
            "rules-project" => Some(SectionName::RulesProject),
            _ => s
                .strip_prefix("rules-worker:")
                .and_then(|w| WorkerId::parse(w).ok())
                .map(SectionName::RulesWorker),
        }
    }
}

impl std::fmt::Display for SectionName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SectionName::Orientation => f.write_str("orientation"),
            SectionName::RulesProject => f.write_str("rules-project"),
            SectionName::RulesWorker(w) => write!(f, "rules-worker:{w}"),
        }
    }
}

/// A section's current revision (tate, r4). PROVISIONAL types for `fields`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionView {
    pub record_id: RecordId,
    pub revision: i32,
    pub body: String,
    pub body_sha256: Sha256,
    pub fields: serde_json::Value,
    pub author: WorkerId,
    pub created_at: Timestamp,
}

/// `RecordsApi` (mod-records): used by search and boot.
#[async_trait]
pub trait RecordsApi: Send + Sync {
    /// The records of this project among `ids`, in ANY state, each with its `state`; an id that names
    /// nothing in the project is simply absent (R-5). A consumer that wants only active records filters
    /// on `state == Active`.
    async fn get(&self, tx: &mut ModuleTx, ids: &[RecordId]) -> Result<Vec<RecordView>, PortError>;
    /// Records CREATED after the cursor, keyset on `(created_at, id)`, in that order, at most `limit`.
    /// It does not report lineage changes (supersede, invalidate, undo); a consumer re-reads `state`
    /// through `get`.
    async fn changed_since(
        &self,
        tx: &mut ModuleTx,
        after: Option<RecordCursor>,
        limit: u32,
    ) -> Result<RecordPage, PortError>;
    /// Boot's read. `None` when unset, or when the current revision is invalidated: no fallback
    /// to an older one (Q-4).
    async fn section_current(
        &self,
        tx: &mut ModuleTx,
        name: &SectionName,
    ) -> Result<Option<SectionView>, PortError>;
}

/// What a handoff is for (S3 `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffKind {
    Task,
    Consult,
}

/// Who a handoff was ADDRESSED to, as created. A rework does not rewrite it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffTo {
    Worker(WorkerId),
    /// A job-role label on S3's label grammar `^[a-z][a-z0-9-]{0,31}$` (R-15, R-18).
    JobRole(String),
}

/// An open handoff, as boot shows it: one briefing line, not the packet (bob's
/// `mod-handoffs-PORT-TYPES-r0.md`; W1-10 c).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffView {
    pub id: HandoffId,
    pub kind: HandoffKind,
    /// The creator (S3 `BootHandoff.created_by`).
    pub from: WorkerId,
    pub to: HandoffTo,
    /// `"pending"` (from `pending_for`) or `"claimed"` (from `claimed_by`).
    pub state: String,
    /// The S3 integer, 1 to 100, larger is more urgent.
    pub priority: i32,
    /// The whole summary, at most 2,048 bytes (S3). Boot cuts it and sets `summary_cut`.
    pub summary: String,
    pub created_at: Timestamp,
}

/// The most rows either `HandoffsApi` method returns, whatever `limit` the caller asks for (W1-33). The cap is
/// per method: it is a bound on one list, never on both together.
pub const HANDOFF_LIST_CAP: u32 = 1000;

/// `HandoffsApi` (mod-handoffs): used by boot.
///
/// **Two methods, not one (W1-33; sam's BOOT12-01).** A claimed handoff and a pending one answer two different
/// questions with two different mandatory behaviours, so they are two calls: boot must always see the worker's
/// newest claimed handoff, and a flood of high-priority pending work must never push it out of a capped list.
/// The project and the actor come from `tx.context()` only; no other project's row ever appears. A provider
/// returns at most `min(limit, HANDOFF_LIST_CAP)` rows from each call, and `limit` is at least 1.
#[async_trait]
pub trait HandoffsApi: Send + Sync {
    /// The worker's CLAIMED handoffs (state `claimed`, holder `w`), NEWEST CLAIM FIRST, then id.
    async fn claimed_by(
        &self,
        tx: &mut ModuleTx,
        w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError>;

    /// The PENDING handoffs `w` may claim: addressed to `w` by id, or to a job role `w` holds (read through
    /// `IdentityApi::worker`), and, after a rework, only when `w` is the rework holder. Priority descending, then
    /// `created_at` ascending, then id (HAND-110's order, for pending only).
    async fn pending_for(
        &self,
        tx: &mut ModuleTx,
        w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError>;
}

/// The ports a piece may call: only those its manifest `uses`, each present.
#[derive(Clone, Default)]
pub struct Ports {
    identity: Option<Arc<dyn IdentityApi>>,
    records: Option<Arc<dyn RecordsApi>>,
    handoffs: Option<Arc<dyn HandoffsApi>>,
    token_verifier: Option<Arc<dyn TokenVerifier>>,
    scope_policy: Option<Arc<dyn ScopePolicy>>,
    audit: Option<Arc<dyn AuditSink>>,
}

impl Ports {
    pub fn identity(&self) -> Option<Arc<dyn IdentityApi>> {
        self.identity.clone()
    }
    pub fn records(&self) -> Option<Arc<dyn RecordsApi>> {
        self.records.clone()
    }
    pub fn handoffs(&self) -> Option<Arc<dyn HandoffsApi>> {
        self.handoffs.clone()
    }
    pub fn token_verifier(&self) -> Option<Arc<dyn TokenVerifier>> {
        self.token_verifier.clone()
    }
    pub fn scope_policy(&self) -> Option<Arc<dyn ScopePolicy>> {
        self.scope_policy.clone()
    }
    pub fn audit(&self) -> Option<Arc<dyn AuditSink>> {
        self.audit.clone()
    }

    /// Whether `port` is present.
    pub fn has(&self, port: Port) -> bool {
        match port {
            Port::Identity => self.identity.is_some(),
            Port::Records => self.records.is_some(),
            Port::Handoffs => self.handoffs.is_some(),
            Port::TokenVerifier => self.token_verifier.is_some(),
            Port::ScopePolicy => self.scope_policy.is_some(),
            Port::Audit => self.audit.is_some(),
        }
    }

    /// The same ports, keeping only `uses`.
    pub(crate) fn only(&self, uses: &[Port]) -> Ports {
        let keep = |p: Port| uses.contains(&p);
        Ports {
            identity: self.identity.clone().filter(|_| keep(Port::Identity)),
            records: self.records.clone().filter(|_| keep(Port::Records)),
            handoffs: self.handoffs.clone().filter(|_| keep(Port::Handoffs)),
            token_verifier: self
                .token_verifier
                .clone()
                .filter(|_| keep(Port::TokenVerifier)),
            scope_policy: self
                .scope_policy
                .clone()
                .filter(|_| keep(Port::ScopePolicy)),
            audit: self.audit.clone().filter(|_| keep(Port::Audit)),
        }
    }
}

/// Names the ports present; nothing else.
impl std::fmt::Debug for Ports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(Port::ALL.iter().filter(|p| self.has(**p)))
            .finish()
    }
}

/// What `Piece::provide` fills. The registry records which piece provided which port.
pub struct PortsBuilder {
    pub(crate) current: &'static str,
    pub(crate) provided: Vec<(Port, &'static str)>,
    pub(crate) ports: Ports,
}

impl PortsBuilder {
    pub(crate) fn new() -> Self {
        PortsBuilder {
            current: "",
            provided: Vec::new(),
            ports: Ports::default(),
        }
    }

    fn note(&mut self, port: Port) {
        self.provided.push((port, self.current));
    }

    pub fn identity(&mut self, p: Arc<dyn IdentityApi>) {
        self.note(Port::Identity);
        self.ports.identity.get_or_insert(p);
    }
    pub fn records(&mut self, p: Arc<dyn RecordsApi>) {
        self.note(Port::Records);
        self.ports.records.get_or_insert(p);
    }
    pub fn handoffs(&mut self, p: Arc<dyn HandoffsApi>) {
        self.note(Port::Handoffs);
        self.ports.handoffs.get_or_insert(p);
    }
    pub fn token_verifier(&mut self, p: Arc<dyn TokenVerifier>) {
        self.note(Port::TokenVerifier);
        self.ports.token_verifier.get_or_insert(p);
    }
    pub fn scope_policy(&mut self, p: Arc<dyn ScopePolicy>) {
        self.note(Port::ScopePolicy);
        self.ports.scope_policy.get_or_insert(p);
    }
    pub fn audit(&mut self, p: Arc<dyn AuditSink>) {
        self.note(Port::Audit);
        self.ports.audit.get_or_insert(p);
    }
}
