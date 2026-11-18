//! Shared fixtures for the mod-policy tests: principals, targets, and the ORACLE.
//!
//! The oracle is written from `mvp/tests/mod-identity-and-mod-policy.md` section 3 (the table text),
//! in a different style from `mod-policy`'s own code (explicit `if`s per rule, no shared helper), and it
//! never calls `Policy::rows()`. A test that compared the code with itself would prove nothing.
#![allow(dead_code)]

use cortex_module::{Principal, Role, Target, TargetRule, WorkerKind};
use cortex_types::{ProjectId, TeamId, Timestamp, TokenId, WorkerId};

pub fn wid(n: u8) -> WorkerId {
    WorkerId::parse(&format!("0193e6a0-0000-7000-8000-0000000001{n:02}")).expect("a v7 id")
}

pub fn tid(n: u8) -> TeamId {
    TeamId::parse(&format!("0193e6a0-0000-7000-8000-0000000002{n:02}")).expect("a v7 id")
}

// Worker numbers. LEAD is the project lead of the fixture project; OTHER is nobody in particular.
pub const LEAD: u8 = 1;
pub const TL1: u8 = 2; // team lead of team 1
pub const TL2: u8 = 3; // team lead of team 2
pub const M1: u8 = 4; // member of team 1
pub const M0: u8 = 5; // member in no team
pub const OTHER: u8 = 6;

pub fn team1() -> TeamId {
    tid(1)
}

pub fn team2() -> TeamId {
    tid(2)
}

pub fn principal(n: u8, kind: WorkerKind, role: Role) -> Principal {
    Principal {
        worker: wid(n),
        project: ProjectId::parse("0193e6a0-0000-7000-8000-0000000000a1").expect("a v7 id"),
        kind,
        role,
        token: TokenId::parse("0193e6a0-0000-7000-8000-0000000000aa").expect("a v7 id"),
        expires_at: Timestamp::from_unix_micros(4_102_444_799_000_000),
        renew_due: false,
    }
}

pub fn lead() -> Principal {
    principal(LEAD, WorkerKind::Worker, Role::ProjectLead)
}

pub fn team_lead_1() -> Principal {
    principal(TL1, WorkerKind::Worker, Role::TeamLead(team1()))
}

pub fn team_lead_2() -> Principal {
    principal(TL2, WorkerKind::Worker, Role::TeamLead(team2()))
}

pub fn member_1() -> Principal {
    principal(M1, WorkerKind::Worker, Role::Member(Some(team1())))
}

pub fn member_0() -> Principal {
    principal(M0, WorkerKind::Worker, Role::Member(None))
}

/// The five callers of the exhaustive domain: the lead, a team lead of each of two teams, a member of
/// team 1 and a member in no team.
pub fn callers() -> Vec<(&'static str, Principal)> {
    vec![
        ("project lead", lead()),
        ("team lead of team 1", team_lead_1()),
        ("team lead of team 2", team_lead_2()),
        ("member of team 1", member_1()),
        ("member in no team", member_0()),
    ]
}

pub fn tg(
    worker: Option<u8>,
    kind: Option<WorkerKind>,
    team: Option<u8>,
    lead: Option<u8>,
) -> Target {
    Target {
        worker: worker.map(wid),
        kind,
        team: team.map(tid),
        lead: lead.map(wid),
        consult: false,
    }
}

/// The small exhaustive domain of targets, relative to a caller `p`: the worker is nothing, the
/// caller, the project lead, or somebody else (4); the kind is unknown, worker or service (3); the
/// team is unknown, team 1 or team 2 (3); the lead is unknown, the project lead or somebody else (3).
/// 4 x 3 x 3 x 3 = 108 targets per caller, 5 callers, 8 rules: 4,320 evaluations.
pub fn domain(p: &Principal) -> Vec<Target> {
    let workers = [None, Some(p.worker), Some(wid(LEAD)), Some(wid(OTHER))];
    let kinds = [None, Some(WorkerKind::Worker), Some(WorkerKind::Service)];
    let teams = [None, Some(tid(1)), Some(tid(2))];
    let leads = [None, Some(wid(LEAD)), Some(wid(OTHER))];
    let mut out = Vec::new();
    for worker in workers {
        for kind in kinds {
            for team in teams {
                for lead in leads {
                    out.push(Target {
                        worker,
                        kind,
                        team,
                        lead,
                        consult: false,
                    });
                }
            }
        }
    }
    out
}

/// All eight rules. The `match` below has NO wildcard arm: a ninth variant stops this file compiling
/// until it is listed here (PO-14).
pub const RULES: [TargetRule; 8] = [
    TargetRule::None,
    TargetRule::SelfWorker,
    TargetRule::SelfOrProjectLead,
    TargetRule::NotProjectLead,
    TargetRule::RetireTarget,
    TargetRule::TokenTarget,
    TargetRule::Lineage,
    TargetRule::TaskAddressee,
];

pub fn rule_name(rule: TargetRule) -> &'static str {
    match rule {
        TargetRule::None => "None",
        TargetRule::SelfWorker => "SelfWorker",
        TargetRule::SelfOrProjectLead => "SelfOrProjectLead",
        TargetRule::NotProjectLead => "NotProjectLead",
        TargetRule::RetireTarget => "RetireTarget",
        TargetRule::TokenTarget => "TokenTarget",
        TargetRule::Lineage => "Lineage",
        TargetRule::TaskAddressee => "TaskAddressee",
    }
}

/// THE ORACLE for `target` (section 3.2), `consult` false (a task). Explicit `if`s per rule.
pub fn oracle(p: &Principal, rule: TargetRule, t: &Target) -> bool {
    let me = p.worker;
    let is_lead = matches!(p.role, Role::ProjectLead);
    let lead_of_team = match p.role {
        Role::TeamLead(team) => Some(team),
        _ => None,
    };
    match rule {
        TargetRule::None => true,
        TargetRule::SelfWorker => t.worker == Some(me),
        TargetRule::SelfOrProjectLead => is_lead || t.worker == Some(me),
        TargetRule::NotProjectLead => {
            // Ignores the caller; both fields needed.
            let (Some(w), Some(l)) = (t.worker, t.lead) else {
                return false;
            };
            w != l
        }
        TargetRule::RetireTarget => {
            // T.worker AND T.lead are needed for EVERY caller; never oneself, never the lead.
            let (Some(w), Some(l)) = (t.worker, t.lead) else {
                return false;
            };
            if w == me || w == l {
                return false;
            }
            if is_lead {
                return true;
            }
            if let Some(team) = lead_of_team {
                return t.team == Some(team);
            }
            false
        }
        TargetRule::TokenTarget => {
            let Some(_w) = t.worker else {
                return false;
            };
            if is_lead {
                return true;
            }
            if let Some(team) = lead_of_team {
                // Team lead: own team, and not a service.
                return t.team == Some(team) && t.kind == Some(WorkerKind::Worker);
            }
            false
        }
        TargetRule::Lineage => {
            if is_lead {
                return true;
            }
            if t.worker == Some(me) {
                return true; // the author
            }
            if let Some(team) = lead_of_team {
                return t.worker.is_some()
                    && t.kind == Some(WorkerKind::Worker)
                    && t.team == Some(team);
            }
            false
        }
        TargetRule::TaskAddressee => {
            if is_lead {
                return true; // any addressee, a job-role one included
            }
            if let Some(team) = lead_of_team {
                return t.worker.is_some() && t.team == Some(team);
            }
            false
        }
    }
}

/// THE ORACLE for `allows` (section 3.1), typed by hand: (scope, class in the S3 twin, project lead,
/// team lead, member). The kind check (R-12) is separate: a principal that is not a Worker gets false.
pub const MATRIX: [(&str, &str, bool, bool, bool); 20] = [
    ("self:read", "self", true, true, true),
    ("self:revoke", "self", true, true, true),
    ("roster:read", "work", true, true, true),
    ("team:create", "admin", true, false, false),
    ("team:assign", "admin", true, false, false),
    ("worker:create", "admin", true, false, false),
    ("worker:retire", "admin", true, true, false),
    ("token:issue", "admin", true, true, false),
    ("token:list", "admin", true, true, false),
    ("token:revoke", "admin", true, true, false),
    ("audit:read", "admin", true, false, false),
    ("records:read", "work", true, true, true),
    ("records:write", "work", true, true, true),
    ("records:lineage", "work", true, true, true),
    ("handoff:create", "work", true, true, true),
    ("handoff:read", "work", true, true, true),
    ("handoff:claim", "work", true, true, true),
    ("handoff:hold", "work", true, true, true),
    ("handoff:review", "work", true, true, true),
    ("handoff:manage", "work", true, true, true),
];

/// The S3 JSON twin, read at test time. `CORTEX_S3_TWIN` overrides the pinned file (read-only).
pub fn twin_path() -> String {
    std::env::var("CORTEX_S3_TWIN").unwrap_or_else(|_| {
        "/path/to/docs/mvp/wave1-pins-20261106-S3r26/S3/cortex-v2-skeleton.openapi.r2.6-a31fa244.json".to_owned()
    })
}

pub fn twin() -> serde_json::Value {
    let path = twin_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the S3 twin at {path} could not be read: {e}"));
    serde_json::from_str(&text).expect("the S3 twin is JSON")
}
