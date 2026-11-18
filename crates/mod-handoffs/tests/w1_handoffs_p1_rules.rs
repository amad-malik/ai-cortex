//! Wave 1, mod-handoffs P1 (row 2a86598d): the pure rules. DB-free. Test definitions:
//! `mvp/tests/mod-handoffs.md` r3.3, HAND-100 (the 180-cell table), HAND-50 to HAND-52 and HAND-60 (who may
//! review), HAND-20, HAND-21, HAND-24 and HAND-28 (the one addressing predicate), HAND-102 (state words),
//! rule 9 (the order of refusals, S3 R-16: target, self-review, state, holder, the lead's reason).
//!
//! `decide` is given the answer of the TARGET rule (`Ctx::authorize`, the policy's) as a bool; here a
//! reference oracle written from the prose table of the test definitions supplies it, so the table is
//! checked against two independent encodings. RED against the stub: every function is `todo!()`.
use cortex_types::WorkerId;
use mod_handoffs::rules::{Allowed, Caller, Facts, Refusal, State, Verb, decide, may_claim};
use mod_handoffs::validate::To;

fn w(n: u8) -> WorkerId {
    WorkerId::parse(&format!("01930000-0000-7000-8000-0000000000{n:02x}")).expect("a UUIDv7")
}

const CREATOR: u8 = 1;
const LEAD: u8 = 2;
const HOLDER: u8 = 3;
const ADDRESSEE: u8 = 4;
const STRANGER: u8 = 5;

const STATES: [State; 5] = [
    State::Pending,
    State::Claimed,
    State::Returned,
    State::Accepted,
    State::Cancelled,
];
const VERBS: [Verb; 6] = [
    Verb::Claim,
    Verb::Release,
    Verb::Return,
    Verb::Accept,
    Verb::Rework,
    Verb::Cancel,
];

/// HAND-100's six actor classes, as the id of the worker who calls.
const CLASSES: [(&str, u8); 6] = [
    ("ADDRESSEE", ADDRESSEE),
    ("HOLDER", HOLDER),
    ("AUTHOR", HOLDER),
    ("CREATOR", CREATOR),
    ("LEAD", LEAD),
    ("STRANGER", STRANGER),
];

fn facts(state: State, caller: u8) -> Facts {
    let held = matches!(state, State::Claimed | State::Returned | State::Accepted);
    let returned = matches!(state, State::Returned | State::Accepted);
    Facts {
        state,
        caller: w(caller),
        creator: w(CREATOR),
        holder: held.then(|| w(HOLDER)),
        author: returned.then(|| w(HOLDER)),
        project_lead: w(LEAD),
    }
}

/// The policy's answer for the route's declared rule, from the prose of HAND-100's table: a claim by a
/// worker who holds the addressed role (the addressee and the holder both hold `dev`); `None` for release
/// and return; `SelfOrProjectLead` (accept, rework) and `Lineage` (cancel) for the creator or the lead.
fn target_allowed(verb: Verb, caller: u8) -> bool {
    match verb {
        Verb::Claim => caller == ADDRESSEE || caller == HOLDER,
        Verb::Release | Verb::Return => true,
        Verb::Accept | Verb::Rework | Verb::Cancel => caller == CREATOR || caller == LEAD,
    }
}

/// The expected outcome of one cell, written from the table in the test definitions (not from `decide`).
fn expected(verb: Verb, state: State, caller: u8) -> Result<Allowed, Refusal> {
    let target = target_allowed(verb, caller);
    match verb {
        Verb::Claim => {
            if !target {
                Err(Refusal::TargetNotAllowed)
            } else {
                match state {
                    State::Pending => Ok(Allowed::Yes),
                    State::Claimed => Err(Refusal::AlreadyClaimed),
                    _ => Err(Refusal::InvalidTransition),
                }
            }
        }
        Verb::Release => {
            if state != State::Claimed {
                Err(Refusal::InvalidTransition)
            } else if caller == HOLDER || caller == LEAD {
                Ok(Allowed::Yes)
            } else {
                Err(Refusal::NotHolder)
            }
        }
        Verb::Return => {
            if state != State::Claimed {
                Err(Refusal::InvalidTransition)
            } else if caller == HOLDER {
                Ok(Allowed::Yes)
            } else {
                Err(Refusal::NotHolder)
            }
        }
        Verb::Accept | Verb::Rework => {
            if !target {
                Err(Refusal::TargetNotAllowed)
            } else if state != State::Returned {
                Err(Refusal::InvalidTransition)
            } else {
                Ok(Allowed::Yes)
            }
        }
        Verb::Cancel => {
            if !target {
                Err(Refusal::TargetNotAllowed)
            } else if !matches!(state, State::Pending | State::Claimed | State::Returned) {
                Err(Refusal::InvalidTransition)
            } else {
                Ok(Allowed::Yes)
            }
        }
    }
}

/// HAND-100: 6 verbs x 5 states x 6 actor classes = 180 cells.
#[test]
fn the_transition_table_has_180_cells_and_decide_matches_every_one() {
    let mut cells = 0;
    let mut wrong = Vec::new();
    for verb in VERBS {
        for state in STATES {
            for (class, caller) in CLASSES {
                cells += 1;
                let f = facts(state, caller);
                let got = decide(verb, &f, target_allowed(verb, caller), true);
                let want = expected(verb, state, caller);
                if got != want {
                    wrong.push(format!(
                        "{verb:?} / {state:?} / {class}: got {got:?}, want {want:?}"
                    ));
                }
            }
        }
    }
    assert_eq!(cells, 180);
    assert!(
        wrong.is_empty(),
        "{} cells differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Rule 9 (S3 R-16): when two refusals apply, the first in the order wins.
#[test]
fn refusals_come_in_the_order_target_state_holder_reason() {
    // A stranger accepting a pending handoff: the target rule (403) before the state (409).
    let f = facts(State::Pending, STRANGER);
    assert_eq!(
        decide(Verb::Accept, &f, false, false),
        Err(Refusal::TargetNotAllowed)
    );
    // A non-addressee claiming a claimed handoff: 403 before AlreadyClaimed.
    let f = facts(State::Claimed, STRANGER);
    assert_eq!(
        decide(Verb::Claim, &f, false, false),
        Err(Refusal::TargetNotAllowed)
    );
    // A stranger returning a pending handoff: the state (409 invalid_transition) before the holder (not_holder).
    let f = facts(State::Pending, STRANGER);
    assert_eq!(
        decide(Verb::Return, &f, true, false),
        Err(Refusal::InvalidTransition)
    );
    // The lead releasing a PENDING handoff with no reason: the state before the reason.
    let f = facts(State::Pending, LEAD);
    assert_eq!(
        decide(Verb::Release, &f, true, false),
        Err(Refusal::InvalidTransition)
    );
}

/// HAND-62: the lead (who is not the holder) needs a reason to release; the holder does not.
#[test]
fn release_by_the_lead_needs_a_reason_the_holder_does_not() {
    let by_lead = facts(State::Claimed, LEAD);
    assert_eq!(
        decide(Verb::Release, &by_lead, true, false),
        Err(Refusal::ReasonRequired)
    );
    assert_eq!(
        decide(Verb::Release, &by_lead, true, true),
        Ok(Allowed::Yes)
    );
    let by_holder = facts(State::Claimed, HOLDER);
    assert_eq!(
        decide(Verb::Release, &by_holder, true, false),
        Ok(Allowed::Yes)
    );
    // The lead who IS the holder is the holder: no reason needed.
    let mut lead_holds = facts(State::Claimed, LEAD);
    lead_holds.holder = Some(w(LEAD));
    assert_eq!(
        decide(Verb::Release, &lead_holds, true, false),
        Ok(Allowed::Yes)
    );
    let stranger = facts(State::Claimed, STRANGER);
    assert_eq!(
        decide(Verb::Release, &stranger, true, true),
        Err(Refusal::NotHolder)
    );
}

// ---- who may review (HAND-50 to HAND-52, HAND-60) ----

const W1: u8 = 11;
const W2: u8 = 12;
const W3: u8 = 13;
const W4: u8 = 14;
const L: u8 = 20;
const TL: u8 = 21;

/// One row of HAND-50: author, creator, lead, then what each caller gets on `accept` and on `rework`.
struct Case {
    name: &'static str,
    author: u8,
    creator: u8,
    lead: u8,
    callers: &'static [(u8, Result<Allowed, Refusal>)],
}

const YES: Result<Allowed, Refusal> = Ok(Allowed::Yes);
const SELF: Result<Allowed, Refusal> = Ok(Allowed::SelfReview);
const NOT_ELIGIBLE: Result<Allowed, Refusal> = Err(Refusal::TargetNotAllowed);
const AUTHOR_BARRED: Result<Allowed, Refusal> = Err(Refusal::SelfReviewForbidden);

#[test]
fn who_may_review_the_six_cases_of_hand_50() {
    let cases = [
        Case {
            name: "a",
            author: W1,
            creator: L,
            lead: L,
            callers: &[
                (L, YES),
                (W1, NOT_ELIGIBLE),
                (W2, NOT_ELIGIBLE),
                (W4, NOT_ELIGIBLE),
            ],
        },
        Case {
            name: "b",
            author: W1,
            creator: W3,
            lead: L,
            callers: &[
                (W3, YES),
                (L, YES),
                (W1, NOT_ELIGIBLE),
                (W2, NOT_ELIGIBLE),
                (W4, NOT_ELIGIBLE),
            ],
        },
        Case {
            name: "c",
            author: W3,
            creator: W3,
            lead: L,
            callers: &[(L, YES), (W3, AUTHOR_BARRED), (W1, NOT_ELIGIBLE)],
        },
        Case {
            name: "d",
            author: L,
            creator: W3,
            lead: L,
            callers: &[(W3, YES), (L, AUTHOR_BARRED), (W1, NOT_ELIGIBLE)],
        },
        Case {
            name: "e",
            author: L,
            creator: L,
            lead: L,
            callers: &[(L, SELF), (W1, NOT_ELIGIBLE)],
        },
        Case {
            name: "f",
            author: W1,
            creator: TL,
            lead: L,
            callers: &[(TL, YES), (L, YES), (W1, NOT_ELIGIBLE), (W2, NOT_ELIGIBLE)],
        },
    ];
    for case in cases {
        for verb in [Verb::Accept, Verb::Rework] {
            for (caller, want) in case.callers {
                // The policy's SelfOrProjectLead: true for the creator and for the project lead.
                let eligible = *caller == case.creator || *caller == case.lead;
                let f = Facts {
                    state: State::Returned,
                    caller: w(*caller),
                    creator: w(case.creator),
                    holder: Some(w(case.author)),
                    author: Some(w(case.author)),
                    project_lead: w(case.lead),
                };
                assert_eq!(
                    decide(verb, &f, eligible, false),
                    *want,
                    "case {} / {verb:?} / caller {caller}",
                    case.name
                );
            }
        }
    }
}

/// The self-review exception is only for author = creator = project lead (the ruling): never when one of
/// the three differs, and only on a returned handoff.
#[test]
fn self_review_needs_all_three_to_be_one_worker() {
    let mut f = Facts {
        state: State::Returned,
        caller: w(L),
        creator: w(L),
        holder: Some(w(L)),
        author: Some(w(L)),
        project_lead: w(L),
    };
    assert_eq!(
        decide(Verb::Accept, &f, true, false),
        Ok(Allowed::SelfReview)
    );
    assert_eq!(
        decide(Verb::Rework, &f, true, false),
        Ok(Allowed::SelfReview)
    );
    f.creator = w(W3);
    assert_eq!(
        decide(Verb::Accept, &f, true, false),
        Err(Refusal::SelfReviewForbidden)
    );
    f.creator = w(L);
    f.project_lead = w(TL);
    assert_eq!(
        decide(Verb::Accept, &f, true, false),
        Err(Refusal::SelfReviewForbidden)
    );
    // Not a returned handoff: the self-review rule is not asked; the state is.
    let mut g = Facts {
        state: State::Pending,
        ..f.clone()
    };
    g.project_lead = w(L);
    assert_eq!(
        decide(Verb::Accept, &g, true, false),
        Err(Refusal::InvalidTransition)
    );
}

/// HAND-60: review rights follow the CURRENT project lead, because the fact is read when it happens.
#[test]
fn the_current_project_lead_is_the_one_who_counts() {
    let f = |caller: u8, lead: u8| Facts {
        state: State::Returned,
        caller: w(caller),
        creator: w(W3),
        holder: Some(w(W1)),
        author: Some(w(W1)),
        project_lead: w(lead),
    };
    // L was the lead; W2 is now. The creator W3 and the new lead W2 are eligible; L is not.
    let eligible = |caller: u8, lead: u8| caller == W3 || caller == lead;
    assert_eq!(
        decide(Verb::Accept, &f(W2, W2), eligible(W2, W2), false),
        Ok(Allowed::Yes)
    );
    assert_eq!(
        decide(Verb::Accept, &f(L, W2), eligible(L, W2), false),
        Err(Refusal::TargetNotAllowed)
    );
}

// ---- the refusal words and the state words ----

#[test]
fn every_refusal_maps_to_its_code_and_status() {
    let table = [
        (Refusal::TargetNotAllowed, "target_not_allowed", 403),
        (Refusal::SelfReviewForbidden, "self_review_forbidden", 403),
        (Refusal::AlreadyClaimed, "already_claimed", 409),
        (Refusal::InvalidTransition, "invalid_transition", 409),
        (Refusal::NotHolder, "not_holder", 409),
        (Refusal::ReasonRequired, "reason_required", 422),
    ];
    for (r, code, status) in table {
        assert_eq!((r.code(), r.status()), (code, status));
    }
}

#[test]
fn state_words_round_trip_and_open_means_not_terminal() {
    let words = [
        (State::Pending, "pending", true),
        (State::Claimed, "claimed", true),
        (State::Returned, "returned", true),
        (State::Accepted, "accepted", false),
        (State::Cancelled, "cancelled", false),
    ];
    for (s, word, open) in words {
        assert_eq!(s.as_str(), word);
        assert_eq!(State::parse(word), Some(s));
        assert_eq!(s.is_open(), open, "{word}");
    }
    for bad in ["", "Pending", "suspended", "done", "pending "] {
        assert_eq!(State::parse(bad), None, "{bad:?}");
    }
}

// ---- the one addressing predicate (HAND-20, HAND-21, HAND-24, HAND-28) ----

fn caller(n: u8, roles: &[&str], active: bool) -> Caller {
    Caller {
        id: w(n),
        job_roles: roles.iter().map(|r| (*r).to_owned()).collect(),
        active,
    }
}

#[test]
fn a_worker_address_matches_that_worker_only() {
    let to = To::Worker(w(W1));
    assert!(may_claim(&to, None, &caller(W1, &[], true)));
    assert!(!may_claim(&to, None, &caller(W2, &["dev"], true)));
}

#[test]
fn a_job_role_address_matches_a_holder_of_exactly_that_role() {
    let to = To::JobRole("dev".to_owned());
    assert!(may_claim(&to, None, &caller(W1, &["dev"], true)));
    assert!(may_claim(&to, None, &caller(W2, &["qa", "dev"], true)));
    assert!(!may_claim(&to, None, &caller(W3, &["qa"], true)));
    assert!(!may_claim(&to, None, &caller(W4, &[], true)));
    // R-15: exact and case-sensitive, no substring, no alias.
    assert!(!may_claim(&to, None, &caller(W1, &["Dev"], true)));
    assert!(!may_claim(&to, None, &caller(W1, &["developer"], true)));
    assert!(!may_claim(&to, None, &caller(W1, &["dev-lead"], true)));
}

/// HAND-24 / HAND-21: an inactive (retired) worker claims nothing.
#[test]
fn an_inactive_worker_claims_nothing() {
    assert!(!may_claim(
        &To::Worker(w(W1)),
        None,
        &caller(W1, &["dev"], false)
    ));
    assert!(!may_claim(
        &To::JobRole("dev".to_owned()),
        None,
        &caller(W1, &["dev"], false)
    ));
}

/// HAND-55: after a rework only the worker who returned it may claim, whatever the original address.
#[test]
fn after_a_rework_only_the_returner_may_claim() {
    let to = To::JobRole("dev".to_owned());
    assert!(may_claim(&to, Some(w(W1)), &caller(W1, &["dev"], true)));
    assert!(
        !may_claim(&to, Some(w(W1)), &caller(W2, &["dev"], true)),
        "another holder of the role"
    );
    // The returner may claim even if it has since lost the role; the rework lock is the address now.
    assert!(may_claim(&to, Some(w(W1)), &caller(W1, &[], true)));
    // Not if it has been retired.
    assert!(!may_claim(&to, Some(w(W1)), &caller(W1, &["dev"], false)));
    // A worker address behaves the same.
    assert!(!may_claim(
        &To::Worker(w(W2)),
        Some(w(W1)),
        &caller(W2, &[], true)
    ));
}
