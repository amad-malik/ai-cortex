//! The DB-free cells of the repair after vera's REWORK of mod-tokens at a3c5da3 (review row 18a0db66):
//!
//! - **V1** (the caller is read again in the changing transaction): `caller_is_current`, the pure comparison the
//!   handlers of `issue` and `revoke` make between the principal the verifier froze and the caller's CURRENT facts.
//! - **V2** (the audit names are the binding registry's): the four constants, and the host mint intent
//!   (`BootstrapRun`) that picks `token.issued` (first run) or `token.reissued` (re-run), W1-26.
//! - **V4** (a cursor instant has the W1-39 bounds before it reaches SQL): `decode_cursor` and its limits.
//!
//! The database halves of the same findings are in `route_caller_fresh.rs`, `db_verify.rs` (V3), `route_read.rs`
//! and `route_issue.rs`.
use cortex_module::{Principal, Role, WorkerKind, WorkerView};
use cortex_types::{ProjectId, SystemClock, TeamId, Timestamp, TokenId, WorkerId};
use mod_tokens::issue::{ACTION_ISSUED, ACTION_REISSUED, BootstrapRun, caller_is_current};
use mod_tokens::read::{CURSOR_MAX_MICROS, CURSOR_MIN_MICROS, decode_cursor, encode_cursor};
use mod_tokens::revoke::{ACTION_REVOKED, ACTION_SELF_REVOKED};

fn ids() -> (WorkerId, ProjectId, TeamId, TeamId, TokenId) {
    let c = SystemClock;
    (
        WorkerId::new(&c),
        ProjectId::new(&c),
        TeamId::new(&c),
        TeamId::new(&c),
        TokenId::new(&c),
    )
}

fn principal(worker: WorkerId, project: ProjectId, token: TokenId, role: Role) -> Principal {
    Principal {
        worker,
        project,
        kind: WorkerKind::Worker,
        role,
        token,
        expires_at: Timestamp::from_unix_micros(1_900_000_000_000_000),
        renew_due: false,
    }
}

fn view(worker: WorkerId, role: Role, kind: WorkerKind, active: bool) -> WorkerView {
    WorkerView {
        id: worker,
        name: "w".into(),
        kind,
        role,
        job_roles: vec![],
        active,
    }
}

// ---- V1 ------------------------------------------------------------------------------------------------

#[test]
fn rw_1_a_caller_who_is_unchanged_is_current() {
    let (w, p, t, _, tok) = ids();
    for role in [
        Role::ProjectLead,
        Role::TeamLead(t),
        Role::Member(Some(t)),
        Role::Member(None),
    ] {
        let pr = principal(w, p, tok, role);
        let now = view(w, role, WorkerKind::Worker, true);
        assert!(caller_is_current(&pr, Some(&now)), "{role:?}");
    }
}

#[test]
fn rw_2_every_change_of_the_callers_membership_is_not_current() {
    let (w, p, t, u, tok) = ids();
    let roles = [
        Role::ProjectLead,
        Role::TeamLead(t),
        Role::TeamLead(u),
        Role::Member(Some(t)),
        Role::Member(Some(u)),
        Role::Member(None),
    ];
    // Every ordered pair of different roles: the frozen role against the current one.
    for frozen in &roles {
        for now in &roles {
            let pr = principal(w, p, tok, *frozen);
            let cur = view(w, *now, WorkerKind::Worker, true);
            assert_eq!(
                caller_is_current(&pr, Some(&cur)),
                frozen == now,
                "frozen {frozen:?}, now {now:?}"
            );
        }
    }
}

#[test]
fn rw_3_a_retired_a_missing_or_another_caller_is_not_current() {
    let (w, p, t, _, tok) = ids();
    let (other, ..) = ids();
    let pr = principal(w, p, tok, Role::TeamLead(t));
    // Retired between the verification and the change.
    let retired = view(w, Role::TeamLead(t), WorkerKind::Worker, false);
    assert!(!caller_is_current(&pr, Some(&retired)));
    // Not found at all.
    assert!(!caller_is_current(&pr, None));
    // The facts of another worker (a wrong lookup is never a pass).
    let someone_else = view(other, Role::TeamLead(t), WorkerKind::Worker, true);
    assert!(!caller_is_current(&pr, Some(&someone_else)));
    // The kind changed (a worker is not a service).
    let service = view(w, Role::TeamLead(t), WorkerKind::Service, true);
    assert!(!caller_is_current(&pr, Some(&service)));
}

// ---- V2 ------------------------------------------------------------------------------------------------

/// mod-audit's token actions (`wave1/mod-audit`, `validate_details`), written out: the binding registry.
const REGISTRY: [&str; 4] = [
    "token.issued",
    "token.reissued",
    "token.revoked",
    "token.self_revoked",
];

#[test]
fn rw_4_the_four_audit_names_are_the_binding_registrys() {
    assert_eq!(ACTION_ISSUED, "token.issued");
    assert_eq!(ACTION_REISSUED, "token.reissued");
    assert_eq!(ACTION_REVOKED, "token.revoked");
    assert_eq!(ACTION_SELF_REVOKED, "token.self_revoked");
    for a in [
        ACTION_ISSUED,
        ACTION_REISSUED,
        ACTION_REVOKED,
        ACTION_SELF_REVOKED,
    ] {
        assert!(REGISTRY.contains(&a), "{a}");
    }
}

#[test]
fn rw_5_a_first_bootstrap_run_is_issued_and_a_rerun_is_reissued() {
    assert_eq!(BootstrapRun::First.audit_action(), "token.issued");
    assert_eq!(BootstrapRun::Rerun.audit_action(), "token.reissued");
    assert_ne!(
        BootstrapRun::First.audit_action(),
        BootstrapRun::Rerun.audit_action()
    );
}

// ---- V4 ------------------------------------------------------------------------------------------------

fn some_id() -> TokenId {
    TokenId::new(&SystemClock)
}

#[test]
fn rw_6_the_limits_are_the_first_instant_of_1970_and_the_last_microsecond_of_9999() {
    assert_eq!(CURSOR_MIN_MICROS, 0, "1970-01-01T00:00:00Z");
    // 9999-12-31T23:59:59.999999Z in microseconds since the epoch.
    assert_eq!(CURSOR_MAX_MICROS, 253_402_300_799_999_999);
}

#[test]
fn rw_7_a_cursor_inside_the_bounds_decodes_and_round_trips() {
    for us in [
        CURSOR_MIN_MICROS,
        1,
        1_800_000_000_000_000,
        CURSOR_MAX_MICROS - 1,
        CURSOR_MAX_MICROS,
    ] {
        let id = some_id();
        let c = encode_cursor(us, &id.to_string());
        let (got_us, got_id) = decode_cursor(&c).expect("a cursor in range");
        assert_eq!((got_us, got_id), (us, id), "{us}");
    }
}

#[test]
fn rw_8_a_cursor_outside_the_bounds_is_422_before_any_sql() {
    for us in [
        -1_i64,
        -86_400_000_000,
        CURSOR_MAX_MICROS + 1,
        i64::MAX,
        i64::MIN,
        i64::MIN + 1,
    ] {
        let c = encode_cursor(us, &some_id().to_string());
        let p = decode_cursor(&c).expect_err("out of range");
        assert_eq!((p.status, p.code), (422, "invalid_request"), "{us}");
    }
}

#[test]
fn rw_9_the_old_cursor_refusals_hold() {
    let id = some_id().to_string();
    let hex = |s: &str| -> String { s.bytes().map(|b| format!("{b:02x}")).collect() };
    for (what, c) in [
        ("empty", String::new()),
        ("odd length", "abc".to_owned()),
        ("upper-case hex", encode_cursor(5, &id).to_uppercase()),
        ("no colon", hex(&format!("5{id}"))),
        ("not a number", hex(&format!("five:{id}"))),
        ("not a token id", hex("5:nope")),
        (
            "a number past i64",
            hex(&format!("99999999999999999999:{id}")),
        ),
    ] {
        let p = decode_cursor(&c).expect_err(what);
        assert_eq!(p.status, 422, "{what}");
    }
}
