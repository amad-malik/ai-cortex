//! The piece's own problem codes (S3 r2.5 `x-error-codes.pieces`): registered once in the manifest,
//! minted by the registry at startup. Fixed texts: never a request value, never row data.

use cortex_module::ProblemSpec;

pub const NAME_INVALID: ProblemSpec = ProblemSpec {
    code: "name_invalid",
    status: 422,
    title: "Invalid name",
    fix: "Use 1 to 32 characters: a letter, then letters, digits or hyphens.",
};

pub const NAME_TAKEN: ProblemSpec = ProblemSpec {
    code: "name_taken",
    status: 409,
    title: "Name taken",
    fix: "Choose a name that no worker, team or project here has used before.",
};

pub const JOB_ROLES_INVALID: ProblemSpec = ProblemSpec {
    code: "job_roles_invalid",
    status: 422,
    title: "Invalid job roles",
    fix: "Send at most 8 different job roles, each a lowercase letter then up to 31 lowercase letters, digits or hyphens.",
};

pub const LEAD_PERMANENT: ProblemSpec = ProblemSpec {
    code: "lead_permanent",
    status: 409,
    title: "The project lead is permanent",
    fix: "The project lead cannot be retired.",
};

/// SHARED (cox W1-31, W1-34): the registry accepts one code from several pieces only when
/// (code, status, title, fix) are identical. The ONE source of the text is
/// `mvp/api/shared-problem-codes.json`; a test reads that file and compares.
pub const TARGET_RETIRED: ProblemSpec = ProblemSpec {
    code: "target_retired",
    status: 409,
    title: "Worker retired",
    fix: "Use an active worker.",
};

pub const TEAM_LEAD_TAKEN: ProblemSpec = ProblemSpec {
    code: "team_lead_taken",
    status: 409,
    title: "The team already has a team lead",
    fix: "Make the current team lead a member first.",
};

/// W1-51 (cox, 7 Nov): a worker who leads a team cannot be retired while leading, because the retired
/// worker would keep the team's one `team_lead` seat and nobody could move or demote it any more.
/// This piece's alone (not a shared code); the S3 twin gets it with r2.8 (pending item 5).
pub const TEAM_LEAD_IN_POST: ProblemSpec = ProblemSpec {
    code: "team_lead_in_post",
    status: 409,
    title: "A team lead cannot be retired",
    fix: "Move the worker to another team or make the worker a member, then retire.",
};

/// SHARED (cox W1-31, W1-34): the text is the one in `mvp/api/shared-problem-codes.json`.
pub const REASON_REQUIRED: ProblemSpec = ProblemSpec {
    code: "reason_required",
    status: 422,
    title: "Reason required",
    fix: "Send the reason this operation requires, in the form its documentation gives.",
};
