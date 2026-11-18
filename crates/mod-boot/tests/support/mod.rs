//! Shared fixtures for the mod-boot tests (`mvp/tests/mod-boot.md`, the shared fixture): project A
//! with the lead L and the workers W1 (job role `qa`) and W2 (job role `qa-lead`); project B,
//! whose every text holds a canary. A [`World`] holds both projects; [`Reads`] is the fake behind
//! boot's seam, answering only for the project of the transaction it stands for, as the ports'
//! row-level security does.
#![allow(dead_code)] // each test binary uses part of it

pub mod schema;

use async_trait::async_trait;
use cortex_module::{
    HandoffKind, HandoffTo, HandoffView, PortError, Principal, ProjectView, Role, SectionName,
    SectionView, WorkerKind, WorkerView,
};
use cortex_types::{
    HandoffId, Id, InstallationId, ProjectId, RecordId, Sha256, TeamId, Timestamp, WorkerId,
};
use mod_boot::seam::BootReads;
use mod_boot::{HState, Handoff, Identity, Inputs, Kind};
use serde_json::{Value, json};

/// A fixed UUIDv7 text: `01930000-0000-7000-8000-<tag><n>` with `tag` 4 hex characters.
pub fn uuid(tag: u16, n: u64) -> String {
    format!("01930000-0000-7000-8000-{tag:04x}{n:08x}")
}
pub fn id<T: Id>(tag: u16, n: u64) -> T {
    T::parse(&uuid(tag, n)).expect("a fixed UUIDv7")
}

pub const T_PROJECT: u16 = 0x0001;
pub const T_WORKER: u16 = 0x0002;
pub const T_TEAM: u16 = 0x0003;
pub const T_TOKEN: u16 = 0x0004;
pub const T_RECORD: u16 = 0x0005;
pub const T_HANDOFF: u16 = 0x0006;
pub const T_INSTALL: u16 = 0x0007;

/// 2026-11-06T12:00:00Z plus `secs`.
pub fn at(secs: i64) -> Timestamp {
    Timestamp::from_unix_micros((1_793_966_400 + secs) * 1_000_000)
}

pub fn project_a() -> ProjectId {
    id(T_PROJECT, 0xa)
}
pub fn project_b() -> ProjectId {
    id(T_PROJECT, 0xb)
}
pub fn installation() -> InstallationId {
    id(T_INSTALL, 1)
}
pub fn lead() -> WorkerId {
    id(T_WORKER, 0x10)
}
pub fn w1() -> WorkerId {
    id(T_WORKER, 0x11)
}
pub fn w2() -> WorkerId {
    id(T_WORKER, 0x12)
}
pub fn lead_b() -> WorkerId {
    id(T_WORKER, 0xb0)
}
pub fn team_a() -> TeamId {
    id(T_TEAM, 0xa)
}

/// The canary every text of project B carries.
pub const CANARY: &str = "CANARY-B-7f3e";

/// A digest whose hex is `n` repeated (two hex digits per byte).
pub fn sha(n: u8) -> Sha256 {
    Sha256([n; 32])
}

pub fn section(
    rec: u64,
    revision: i32,
    body: &str,
    fields: Value,
    author: WorkerId,
) -> SectionView {
    SectionView {
        record_id: id(T_RECORD, rec),
        revision,
        body: body.to_owned(),
        body_sha256: sha(rec as u8),
        fields,
        author,
        created_at: at(-3600),
    }
}

/// An orientation of `body` with these reading-path entries.
pub fn orientation(rec: u64, revision: i32, body: &str, path: &[String]) -> SectionView {
    section(rec, revision, body, json!({ "reading_path": path }), lead())
}

/// A rules layer pointing at `path`.
pub fn rules(rec: u64, revision: i32, path: &str) -> SectionView {
    section(rec, revision, "", json!({ "source_path": path }), lead())
}

/// Five short reading-path entries.
pub fn path5() -> Vec<String> {
    (1..=5).map(|i| format!("/path/to/docs/a/{i}.md")).collect()
}

/// W1 of project A, a member of team A with job role `qa`.
pub fn identity_w1() -> Identity {
    Identity {
        worker: w1(),
        name: "W1".to_owned(),
        kind: WorkerKind::Worker,
        role: Role::Member(Some(team_a())),
        job_roles: vec!["qa".to_owned()],
        project: project_a(),
        installation: installation(),
        token: id(T_TOKEN, 0x11),
        expires_at: at(14 * 86_400),
        renew_due: false,
    }
}

pub fn handoff(n: u64, state: HState, priority: i32, created: i64, summary: &str) -> Handoff {
    Handoff {
        id: id(T_HANDOFF, n),
        kind: Kind::Task,
        state,
        priority,
        created_by: lead(),
        created_at: at(created),
        summary: summary.to_owned(),
    }
}

/// W1's inputs with orientation and both rules layers set, and no handoff.
pub fn inputs_w1() -> Inputs {
    Inputs {
        identity: identity_w1(),
        orientation: Some(orientation(
            1,
            2,
            "What project A is.\nWho owns what.\n",
            &path5(),
        )),
        rules_project: Some(rules(2, 7, "/path/to/docs/a/RULES.md")),
        rules_worker: Some(rules(3, 1, "/path/to/docs/a/rules-w1.md")),
        handoffs: Vec::new(),
        more_claimed: false,
        more_pending: false,
    }
}

/// The worst case of S2 §7.2 at the admitted maxima: a 32-byte name and job roles, ten 256-byte
/// locators, ten-digit revisions, 2048-byte summaries, ten claimed and ten pending handoffs, a
/// 3072-byte orientation body.
pub fn worst_case() -> Inputs {
    let name = format!("W{}", "x".repeat(31));
    let role = |i: usize| format!("r{i}{}", "a".repeat(31 - i.to_string().len()));
    let loc = |i: usize| format!("/{i:03}{}", "p".repeat(252));
    let mut body = String::new();
    while body.len() < 3072 - 64 {
        body.push_str(&"o".repeat(63));
        body.push('\n');
    }
    body.push_str(&"e".repeat(3072 - body.len()));
    let summary = format!("{}\n{}", "s".repeat(1023), "t".repeat(1024));
    let mut handoffs = Vec::new();
    // claimed_by's order: newest claim first (109 is the newest).
    for n in (0..10).rev() {
        handoffs.push(handoff(100 + n, HState::Claimed, 100, n as i64, &summary));
    }
    for n in 0..10 {
        handoffs.push(handoff(200 + n, HState::Pending, 100, n as i64, &summary));
    }
    Inputs {
        identity: Identity {
            name,
            job_roles: (0..10).map(role).collect(),
            ..identity_w1()
        },
        orientation: Some(SectionView {
            revision: i32::MAX,
            ..orientation(1, i32::MAX, &body, &(0..10).map(loc).collect::<Vec<_>>())
        }),
        rules_project: Some(rules(2, i32::MAX, &loc(90))),
        rules_worker: Some(rules(3, i32::MAX, &loc(91))),
        handoffs,
        more_claimed: false,
        more_pending: false,
    }
}

/// Every line of `md` that starts a Markdown heading.
pub fn headings(md: &str) -> Vec<&str> {
    md.lines().filter(|l| l.starts_with('#')).collect()
}

/// The text of section `n` (`## n …` up to the next heading), heading included.
pub fn section_text(md: &str, n: u32) -> String {
    let head = format!("## {n} ");
    let mut out = String::new();
    let mut inside = false;
    for line in md.split_inclusive('\n') {
        if line.starts_with("## ") {
            inside = line.starts_with(&head);
        }
        if inside {
            out.push_str(line);
        }
    }
    out
}

// ---- The fake world behind the seam ----

/// One project's rows, as the providers would hold them.
#[derive(Clone, Default)]
pub struct ProjectRows {
    pub project: Option<ProjectView>,
    pub workers: Vec<WorkerView>,
    pub sections: Vec<(SectionName, SectionView)>,
    /// `claimed_by`'s answer per worker, in the provider's order (newest claim first).
    pub claimed: Vec<(WorkerId, Vec<HandoffView>)>,
    /// `pending_for`'s answer per worker, in the provider's order.
    pub pending: Vec<(WorkerId, Vec<HandoffView>)>,
}

/// Both projects.
#[derive(Clone, Default)]
pub struct World {
    pub a: ProjectRows,
    pub b: ProjectRows,
}

/// The fake reads for one transaction: its project is fixed, as `tx.context()` fixes it.
pub struct Reads {
    pub rows: ProjectRows,
    /// Every call, in order, for the "what did boot ask" checks.
    pub calls: Vec<String>,
    /// When set, every read answers this error (BOOT-52).
    pub fail: Option<PortError>,
}

impl Reads {
    pub fn for_project(w: &World, p: ProjectId) -> Reads {
        let rows = if Some(p) == w.a.project.as_ref().map(|v| v.id) {
            w.a.clone()
        } else if Some(p) == w.b.project.as_ref().map(|v| v.id) {
            w.b.clone()
        } else {
            ProjectRows::default()
        };
        Reads {
            rows,
            calls: Vec::new(),
            fail: None,
        }
    }

    fn check(&mut self, call: String) -> Result<(), PortError> {
        self.calls.push(call);
        match &self.fail {
            Some(PortError::Unavailable) => Err(PortError::Unavailable),
            Some(PortError::NotFound) => Err(PortError::NotFound),
            Some(PortError::Forbidden) => Err(PortError::Forbidden),
            Some(PortError::Invalid(s)) => Err(PortError::Invalid(s)),
            Some(PortError::Conflict(s)) => Err(PortError::Conflict(s)),
            Some(PortError::Db(_)) => Err(PortError::Unavailable),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl BootReads for Reads {
    async fn project(&mut self) -> Result<ProjectView, PortError> {
        self.check("project".into())?;
        self.rows.project.clone().ok_or(PortError::NotFound)
    }
    async fn worker(&mut self, w: &WorkerId) -> Result<Option<WorkerView>, PortError> {
        self.check(format!("worker {w}"))?;
        Ok(self.rows.workers.iter().find(|v| v.id == *w).cloned())
    }
    async fn section(&mut self, name: &SectionName) -> Result<Option<SectionView>, PortError> {
        self.check(format!("section {name}"))?;
        Ok(self
            .rows
            .sections
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone()))
    }
    async fn claimed_by(
        &mut self,
        w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        self.check(format!("claimed_by {w} {limit}"))?;
        Ok(first(&self.rows.claimed, w, limit))
    }
    async fn pending_for(
        &mut self,
        w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        self.check(format!("pending_for {w} {limit}"))?;
        Ok(first(&self.rows.pending, w, limit))
    }
}

/// The worker's list, cut to `limit` as the provider cuts it.
fn first(lists: &[(WorkerId, Vec<HandoffView>)], w: &WorkerId, limit: u32) -> Vec<HandoffView> {
    let mut v = lists
        .iter()
        .find(|(k, _)| k == w)
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    v.truncate(limit as usize);
    v
}

pub fn worker_view(id: WorkerId, name: &str, role: Role, job_roles: &[&str]) -> WorkerView {
    WorkerView {
        id,
        name: name.to_owned(),
        kind: WorkerKind::Worker,
        role,
        job_roles: job_roles.iter().map(|s| (*s).to_owned()).collect(),
        active: true,
    }
}

/// The shared fixture: A with L, W1 (`qa`) and W2 (`qa-lead`), orientation and project rules set;
/// B with a canary in every text.
pub fn world() -> World {
    let pa = ProjectView {
        id: project_a(),
        name: "project-a".into(),
        state: "active".into(),
        lead: lead(),
        installation: installation(),
        token_lifetime_days: 90,
    };
    let pb = ProjectView {
        id: project_b(),
        name: format!("project-{CANARY}"),
        state: "active".into(),
        lead: lead_b(),
        installation: installation(),
        token_lifetime_days: 90,
    };
    let a = ProjectRows {
        project: Some(pa),
        workers: vec![
            worker_view(lead(), "L", Role::ProjectLead, &[]),
            worker_view(w1(), "W1", Role::Member(Some(team_a())), &["qa"]),
            worker_view(w2(), "W2", Role::Member(Some(team_a())), &["qa-lead"]),
        ],
        sections: vec![
            (
                SectionName::Orientation,
                orientation(1, 2, "What project A is.\n", &path5()),
            ),
            (
                SectionName::RulesProject,
                rules(2, 7, "/path/to/docs/a/RULES.md"),
            ),
        ],
        claimed: Vec::new(),
        pending: Vec::new(),
    };
    let canary_path: Vec<String> = (1..=5).map(|i| format!("/{CANARY}/{i}")).collect();
    let b = ProjectRows {
        project: Some(pb),
        workers: vec![
            worker_view(lead_b(), &format!("L{CANARY}"), Role::ProjectLead, &[]),
            worker_view(w1(), &format!("W1{CANARY}"), Role::ProjectLead, &["canary"]),
        ],
        sections: vec![
            (
                SectionName::Orientation,
                section(
                    0xb1,
                    9,
                    &format!("{CANARY} orientation\n"),
                    json!({ "reading_path": canary_path }),
                    lead_b(),
                ),
            ),
            (
                SectionName::RulesProject,
                section(
                    0xb2,
                    9,
                    "",
                    json!({ "source_path": format!("/{CANARY}/RULES.md") }),
                    lead_b(),
                ),
            ),
        ],
        claimed: Vec::new(),
        pending: Vec::new(),
    };
    World { a, b }
}

/// W1's principal in project A.
pub fn principal_w1() -> Principal {
    Principal {
        worker: w1(),
        project: project_a(),
        kind: WorkerKind::Worker,
        role: Role::Member(Some(team_a())),
        token: id(T_TOKEN, 0x11),
        expires_at: at(14 * 86_400),
        renew_due: false,
    }
}

/// A view as `open_for` returns it (the real `HandoffView`, `wave1/ports`).
pub fn view(n: u64, to: HandoffTo, state: &str, priority: i32) -> HandoffView {
    HandoffView {
        id: id::<HandoffId>(T_HANDOFF, n),
        kind: HandoffKind::Task,
        from: lead(),
        to,
        state: state.to_owned(),
        priority,
        summary: format!("handoff {n}"),
        created_at: at(n as i64),
    }
}

/// The record id text of fixture record `n`.
pub fn record(n: u64) -> RecordId {
    id(T_RECORD, n)
}
