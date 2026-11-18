//! The `x-cli` strings of the pinned S3 twin that no group registers YET, with the owner who adds his group when HIS piece
//! merges (cox, row 99c37d1b). An owner who registers a command deletes its row here in the same commit: a string that is both
//! registered and listed fails `tests/coverage.rs` (CV-03). Generated once from the twin `1d2c4b2f` (S3 r2.7) at the skeleton;
//! `tests/coverage.rs` re-reads the pinned twin, so a new pin shows the drift.

/// `(x-cli string exactly as the twin gives it, owner)`.
pub const NOT_YET: &[(&str, &str)] = &[
    (
        "cortex handoff accept <id>   (v1 form kept as alias: cortex-handoff --return <id> --decision accept; D-3, client owner marv)",
        "bob@helix",
    ),
    ("cortex handoff cancel <id> --reason <text>", "bob@helix"),
    ("cortex handoff claim <id>", "bob@helix"),
    ("cortex handoff create ...", "bob@helix"),
    ("cortex handoff mine | cortex handoff list", "bob@helix"),
    ("cortex handoff release <id> [--reason <text>]", "bob@helix"),
    ("cortex handoff return <id> ...", "bob@helix"),
    (
        "cortex handoff rework <id> --notes <text>   (alias: cortex-handoff --return <id> --decision rework)",
        "bob@helix",
    ),
    ("cortex handoff show <id>", "bob@helix"),
    ("cortex token inspect <id>", "finn@helix"),
    ("cortex token issue|reissue --worker <id>", "finn@helix"),
    ("cortex token list", "finn@helix"),
    ("cortex token revoke <id> --reason <code>", "finn@helix"),
    ("cortex token revoke-self", "finn@helix"),
    ("cortex whoami", "finn@helix"),
    ("cortex log <type> ... | cortex rules set", "nemo@helix"),
    (
        "cortex record invalidate <id> --reason <text> [--undo]   (alias: cortex invalidate)",
        "nemo@helix",
    ),
    ("cortex record show <id>", "nemo@helix"),
    (
        "cortex record supersede <id> --by <id> --reason <text>",
        "nemo@helix",
    ),
    ("cortex rules", "nemo@helix"),
    (
        "cortex search <q> [--type T]... [--limit N] [--since T] [--until T] [--history]   (alias: cortex-search; the v1-only flags --room --hall --graph --no-rerank are accepted, NOT sent, and noted in one stderr line: nix 2)",
        "nemo@helix",
    ),
    ("cortex boot [--budget N] [--format md|json]", "sophi@helix"),
    ("cortex projects", "sophi@helix"),
    ("cortex roster", "sophi@helix"),
    ("cortex team add <name>", "sophi@helix"),
    ("cortex worker add <name> --role R --mode M", "sophi@helix"),
    ("cortex worker move|promote|demote", "sophi@helix"),
    ("cortex worker retire <id> --reason <text>", "sophi@helix"),
];
