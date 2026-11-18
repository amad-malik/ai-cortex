//! The piece's problem codes. Both are SHARED with other pieces (W1-31): the title and the fix are the ones in
//! `mvp/api/shared-problem-codes.json` (cox W1-34), word for word, and a cell reads that file.

use cortex_module::ProblemSpec;

/// 422: `tokens.revoke` without a reason.
pub const REASON_REQUIRED: ProblemSpec = ProblemSpec {
    code: "reason_required",
    status: 422,
    title: "Reason required",
    fix: "Send the reason this operation requires, in the form its documentation gives.",
};

/// 409: `tokens.issue` for a retired worker.
pub const TARGET_RETIRED: ProblemSpec = ProblemSpec {
    code: "target_retired",
    status: 409,
    title: "Worker retired",
    fix: "Use an active worker.",
};
