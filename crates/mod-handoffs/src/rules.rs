//! The pure rules of the loop: who may do what in which state, and in which order a refusal is chosen
//! (S3 R-16). `tests/w1_handoffs_p1_rules.rs` is the spec (HAND-100, HAND-50 to HAND-52, HAND-20/21).
//!
//! Nothing here reads a row or the clock. A handler loads the facts, asks the policy for the TARGET
//! rule (`Ctx::authorize`) and passes that answer in as `target_allowed`; this module adds the rest of
//! the order: the self-review check, then the state (409), then the holder (409), then the lead's reason.

use cortex_types::WorkerId;

use crate::validate::To;

/// A handoff's state (S2: `suspended` is in the CHECK but unused in 0.2.001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Pending,
    Claimed,
    Returned,
    Accepted,
    Cancelled,
}

impl State {
    /// The stored word; any other word (including `suspended`) is not a state of this piece.
    pub fn parse(s: &str) -> Option<State> {
        match s {
            "pending" => Some(State::Pending),
            "claimed" => Some(State::Claimed),
            "returned" => Some(State::Returned),
            "accepted" => Some(State::Accepted),
            "cancelled" => Some(State::Cancelled),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Claimed => "claimed",
            State::Returned => "returned",
            State::Accepted => "accepted",
            State::Cancelled => "cancelled",
        }
    }

    /// Open: not terminal. `accepted` and `cancelled` are terminal.
    pub fn is_open(self) -> bool {
        matches!(self, State::Pending | State::Claimed | State::Returned)
    }
}

/// The six moves that change a handoff (create is not one: it has no earlier state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verb {
    Claim,
    Release,
    Return,
    Accept,
    Rework,
    Cancel,
}

/// What `decide` needs to know about the handoff and the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub state: State,
    pub caller: WorkerId,
    pub creator: WorkerId,
    pub holder: Option<WorkerId>,
    /// The author of the latest return: the holder who returned it (known once a return exists).
    pub author: Option<WorkerId>,
    pub project_lead: WorkerId,
}

/// Why a move is refused, in R-16's order: 403 (target, then self-review), 409 (state, then holder), 422.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    TargetNotAllowed,
    SelfReviewForbidden,
    AlreadyClaimed,
    InvalidTransition,
    NotHolder,
    ReasonRequired,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Refusal::TargetNotAllowed => "target_not_allowed",
            Refusal::SelfReviewForbidden => "self_review_forbidden",
            Refusal::AlreadyClaimed => "already_claimed",
            Refusal::InvalidTransition => "invalid_transition",
            Refusal::NotHolder => "not_holder",
            Refusal::ReasonRequired => "reason_required",
        }
    }

    pub fn status(self) -> u16 {
        match self {
            Refusal::TargetNotAllowed | Refusal::SelfReviewForbidden => 403,
            Refusal::AlreadyClaimed | Refusal::InvalidTransition | Refusal::NotHolder => 409,
            Refusal::ReasonRequired => 422,
        }
    }
}

/// A move that is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Allowed {
    /// An ordinary move.
    Yes,
    /// A review by the return's author who is also the creator and the project lead: the only case
    /// with no independent reviewer. The review row records `self_review = true`.
    SelfReview,
}

/// The 403 half of the order: the target rule's answer, then (accept and rework on a returned handoff)
/// the self-review rule. The route's `authorize` phase ends here, before the idempotency lookup (R-19).
pub fn authority(verb: Verb, f: &Facts, target_allowed: bool) -> Result<Allowed, Refusal> {
    // 403, the policy's answer for the route's declared rule. Release and return declare rule `None`,
    // for which the caller passes `true`: their holder checks are state, below.
    if !target_allowed {
        return Err(Refusal::TargetNotAllowed);
    }
    // 403, "never the author": only a review of a returned handoff, by the worker who returned it. The
    // one exception is the worker who is also the creator and the project lead (no one else can review).
    if matches!(verb, Verb::Accept | Verb::Rework)
        && f.state == State::Returned
        && f.author == Some(f.caller)
    {
        if f.creator == f.caller && f.project_lead == f.caller {
            return Ok(Allowed::SelfReview);
        }
        return Err(Refusal::SelfReviewForbidden);
    }
    Ok(Allowed::Yes)
}

/// The 409 and 422 half: the state, then the holder, then the lead's reason. The route's `execute` phase
/// runs it on the row it holds locked.
///
/// `reason_given` matters only to `Release` by the project lead who is not the holder.
pub fn transition(verb: Verb, f: &Facts, reason_given: bool) -> Result<(), Refusal> {
    // 409, the state.
    match verb {
        Verb::Claim => match f.state {
            State::Pending => {}
            State::Claimed => return Err(Refusal::AlreadyClaimed),
            _ => return Err(Refusal::InvalidTransition),
        },
        Verb::Release | Verb::Return => {
            if f.state != State::Claimed {
                return Err(Refusal::InvalidTransition);
            }
        }
        Verb::Accept | Verb::Rework => {
            if f.state != State::Returned {
                return Err(Refusal::InvalidTransition);
            }
        }
        Verb::Cancel => {
            if !f.state.is_open() {
                return Err(Refusal::InvalidTransition);
            }
        }
    }
    // 409, the holder; then 422, the lead's reason (it needs a read, so it is last).
    match verb {
        Verb::Return => {
            if f.holder != Some(f.caller) {
                return Err(Refusal::NotHolder);
            }
        }
        Verb::Release => {
            if f.holder == Some(f.caller) {
                // The holder releases with or without a reason.
            } else if f.project_lead == f.caller {
                if !reason_given {
                    return Err(Refusal::ReasonRequired);
                }
            } else {
                return Err(Refusal::NotHolder);
            }
        }
        Verb::Claim | Verb::Accept | Verb::Rework | Verb::Cancel => {}
    }
    Ok(())
}

/// The whole order for one move: [`authority`] then [`transition`]. `reason_given` matters only to
/// `Release` by the project lead who is not the holder.
pub fn decide(
    verb: Verb,
    f: &Facts,
    target_allowed: bool,
    reason_given: bool,
) -> Result<Allowed, Refusal> {
    let allowed = authority(verb, f, target_allowed)?;
    transition(verb, f, reason_given)?;
    Ok(allowed)
}

/// The caller, as the claim predicate sees them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub id: WorkerId,
    pub job_roles: Vec<String>,
    pub active: bool,
}

/// The ONE addressing predicate: `claim`, `mine` and `open_for` all call it (the v1 lesson). An
/// inactive caller may claim nothing. After a rework only the worker who returned it may claim,
/// whatever the original address; otherwise a worker address matches that worker, and a job-role
/// address matches a caller who holds exactly that role (case-sensitive, R-15).
pub fn may_claim(to: &To, rework_holder: Option<WorkerId>, c: &Caller) -> bool {
    if !c.active {
        return false;
    }
    if let Some(returner) = rework_holder {
        return returner == c.id;
    }
    match to {
        To::Worker(w) => *w == c.id,
        To::JobRole(role) => c.job_roles.iter().any(|r| r == role),
    }
}
