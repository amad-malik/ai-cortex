//! The piece's own problem codes (S3 r2.5 `x-error-codes.pieces`), registered once through the manifest.
//! `affects_unknown` and `lineage_target_unknown` are RETIRED (R-5: an id that names nothing in the project is 404).

use cortex_module::ProblemSpec;

pub const TYPE_RESERVED: ProblemSpec = ProblemSpec {
    code: "type_reserved",
    status: 422,
    title: "Record type reserved",
    fix: "Use a type a worker may write: decision, lesson, note, status, question, blocker, knowledge or work-product.",
};
pub const SOURCE_REQUIRED: ProblemSpec = ProblemSpec {
    code: "source_required",
    status: 422,
    title: "Source required",
    fix: "A knowledge record needs a source {kind, ref}, except in a reserved section.",
};
pub const SOURCE_NOT_ALLOWED: ProblemSpec = ProblemSpec {
    code: "source_not_allowed",
    status: 422,
    title: "Source not allowed",
    fix: "Only a knowledge record carries a source; remove it.",
};
pub const LOCATOR_TOO_LONG: ProblemSpec = ProblemSpec {
    code: "locator_too_long",
    status: 422,
    title: "Locator too long",
    fix: "Keep each reading_path entry to 256 bytes.",
};
pub const WHY_REQUIRED: ProblemSpec = ProblemSpec {
    code: "why_required",
    status: 422,
    title: "Why required",
    fix: "A decision needs fields.why.",
};
pub const FILES_REQUIRED: ProblemSpec = ProblemSpec {
    code: "files_required",
    status: 422,
    title: "Files required",
    fix: "A work-product needs fields.files with at least one file.",
};
pub const SECTION_INVALID: ProblemSpec = ProblemSpec {
    code: "section_invalid",
    status: 422,
    title: "Section invalid",
    fix: "Use rules-project, rules-worker:<id> or orientation.",
};
pub const SECTION_RESERVED: ProblemSpec = ProblemSpec {
    code: "section_reserved",
    status: 403,
    title: "Section reserved",
    fix: "Only the project lead writes this section (a worker may write its own rules-worker section).",
};
pub const SELF_SUPERSEDE: ProblemSpec = ProblemSpec {
    code: "self_supersede",
    status: 422,
    title: "A record cannot supersede itself",
    fix: "Name a different record in `by`.",
};
pub const LINEAGE_STATE_CONFLICT: ProblemSpec = ProblemSpec {
    code: "lineage_state_conflict",
    status: 409,
    title: "Lineage state conflict",
    fix: "Read the record's state and act on that; an invalidation can be undone only while no higher section revision exists.",
};
pub const REASON_REQUIRED: ProblemSpec = ProblemSpec {
    code: "reason_required",
    status: 422,
    title: "Reason required",
    fix: "Send the reason this operation requires, in the form its documentation gives.",
};

/// Every code the piece registers, in the order the manifest lists them.
pub const ALL: [ProblemSpec; 11] = [
    TYPE_RESERVED,
    SOURCE_REQUIRED,
    SOURCE_NOT_ALLOWED,
    LOCATOR_TOO_LONG,
    WHY_REQUIRED,
    FILES_REQUIRED,
    SECTION_INVALID,
    SECTION_RESERVED,
    SELF_SUPERSEDE,
    LINEAGE_STATE_CONFLICT,
    REASON_REQUIRED,
];
