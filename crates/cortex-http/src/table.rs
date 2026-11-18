//! The exported route table (Q4; X-07; Wave 1 item F).
//!
//! `cortex-http` emits a minimal table from the `Routes` the pieces declare: the method, the path,
//! the owning piece, the scope, the target rule (the VARIANT STRING, `"None"` included, never null),
//! whether the route writes, whether it needs an `Idempotency-Key`, whether it declares a query, its
//! body cap, and the problem statuses the CHAIN itself can answer for it. sam's X-07 compares this
//! export with the S3 twin; [`diff`] is the same comparison, so a piece can run it on its own routes.
//!
//! What is compared STRICTLY: the set of operations (and that none is declared twice), the owning
//! piece (not for the server's own open routes), the scope, the target, the write kind, the key (a
//! header named `Idempotency-Key` that is `required: true`), the query, the open class and, where the
//! twin declares one, the body cap (`x-max-body`). A twin value that is missing or has the wrong
//! type is a drift of its own, and so is a construct this reader does not interpret (a path-item
//! `parameters`, a parameter reference that resolves to nothing): never a silent pass. A missing or
//! unknown `x-target` is NEVER read as `"None"`.
//!
//! THE STATUSES. The export claims, per kind of route, the statuses the chain's OWN code can answer
//! (the guard, and a write's decode, key, transaction and idempotency steps): not a handler's own
//! refusals (a 404 for a path id, a target's 403), and not the router's 404 and 405 for a path. The
//! list is derived from `chain.rs`, `problem.rs`, `idem.rs` and S0's `From<DbError>`, and
//! `tests/f_chain_statuses.rs` drives a plain read, a query read and a write through the real chain
//! and compares what comes back with it, in both directions (only a write's 409 and 429 need a
//! database; that file names the cells that cover them).
//!
//! Against the twin the rule is explicit and every part of it is quoted from the twin's own text:
//! a status the export claims must be listed by the operation, or be one the twin states once for
//! every token route ([`UNIVERSAL_R9`], [`UNIVERSAL_R16`]), or be covered by the operation's
//! `default` when that is the shared `Problem` ([`DEFAULT_COVERS`]); a chain status the operation
//! LISTS that the export does not claim is a drift, except [`RESERVED`] on a read. The twin's statuses on an
//! open route are the server's own handler's (piece 11) and are not compared. Nothing is widened to
//! make the pinned twin pass: it passes because it says these things, once, in R-9, R-16 and `Problem`.
use std::collections::{BTreeMap, BTreeSet};

use cortex_module::{Routes, TargetRule};
use serde_json::{Value, json};

use crate::chain::OpenRoute;

/// What the chain itself can answer on an authenticated READ: the token (401, 503 when the verifier or
/// the wait is unavailable), the body cap (413) and an unreadable body (400) on every route, the scope
/// (403), a query string the route did not declare, or a declared one that does not parse (422), and a
/// panic caught under the guard (500). The interim limiter WAITS for a place, so it adds no status.
const READ: [u16; 7] = [400, 401, 403, 413, 422, 500, 503];
/// A WRITE adds the key (428, and 400 for a bad one), a lock held by another request (429, the only 429
/// the chain makes today), an already-issued secret (409), and a database that cannot answer (503,
/// 500). A write's 400 and 422 are also its strict decoding.
const WRITE: [u16; 10] = [400, 401, 403, 409, 413, 422, 428, 429, 500, 503];
/// Every status the chain answers for some kind of route.
const UNIVERSE: [u16; 10] = WRITE;

/// Statuses the twin states ONCE for every token route (R-9, "Every token route may also answer"), so
/// an operation need not list them.
pub const UNIVERSAL_R9: [u16; 5] = [400, 401, 403, 422, 429];
/// The chain's own body-cap step (R-16, "the chain's own steps first"); not listed per operation either.
pub const UNIVERSAL_R16: [u16; 1] = [413];
/// What the twin's `default` response (the shared `Problem`) covers: "Any other refusal: 400, 405 and 415
/// invalid_request, 409 and 500 internal_error". Only the ones the chain can answer.
pub const DEFAULT_COVERS: [u16; 3] = [400, 409, 500];
/// Listed by the twin on every authenticated operation, but the chain answers it only for a WRITE
/// today (a lock held by another request): the per-worker limiter (M-8) will answer it for all. The
/// exception is for READS only: a write that does not claim 429 is a drift.
pub const RESERVED: [u16; 1] = [429];

/// The eight rules, to tell a twin value that is one of them from one that is not.
const RULES: [TargetRule; 8] = [
    TargetRule::None,
    TargetRule::SelfWorker,
    TargetRule::SelfOrProjectLead,
    TargetRule::NotProjectLead,
    TargetRule::RetireTarget,
    TargetRule::TokenTarget,
    TargetRule::Lineage,
    TargetRule::TaskAddressee,
];

/// One exported route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub piece: String,
    pub method: String,
    pub path: String,
    pub scope: String,
    pub target: String,
    pub write: bool,
    pub idempotency_key: bool,
    pub max_body: u64,
    pub query: bool,
    pub open: bool,
    pub statuses: Vec<u16>,
}

/// What differs between the export and the twin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// The twin has the operation; the export does not.
    MissingOperation { method: String, path: String },
    /// The export has a route the twin does not.
    ExtraRoute { method: String, path: String },
    /// The export declares the same method and path twice.
    Duplicate { method: String, path: String },
    Scope {
        method: String,
        path: String,
        exported: String,
        twin: String,
    },
    /// Includes a twin value that is missing or is not one of the eight variants: it is never read as `None`.
    Target {
        method: String,
        path: String,
        exported: String,
        twin: String,
    },
    Write {
        method: String,
        path: String,
        exported: bool,
        twin: bool,
    },
    IdempotencyKey {
        method: String,
        path: String,
        exported: bool,
        twin: bool,
    },
    /// The route is declared by a different piece than the twin's `x-piece` (not for open routes: the
    /// twin's `health` is the server's own).
    Piece {
        method: String,
        path: String,
        exported: String,
        twin: String,
    },
    /// The twin declares a body cap (`x-max-body`) and the export's differs, e.g. a route that forgot
    /// `.max_body()` and kept `Routes`' own default.
    MaxBody {
        method: String,
        path: String,
        exported: u64,
        twin: u64,
    },
    /// The twin declares a query parameter on the operation iff the export says `.query()`.
    Query {
        method: String,
        path: String,
        exported: bool,
        twin: bool,
    },
    /// The twin's `x-class` is `open` iff the export marks the route open.
    Open {
        method: String,
        path: String,
        exported: bool,
        twin: bool,
    },
    /// A field of the twin operation that the comparison needs is missing or has the wrong type.
    TwinField {
        method: String,
        path: String,
        field: &'static str,
    },
    /// Chain-level statuses on which the two disagree.
    Statuses {
        method: String,
        path: String,
        only_exported: Vec<u16>,
        only_twin: Vec<u16>,
    },
}

/// The variant string of a rule. ONE `match`, no wildcard arm.
pub fn target_name(rule: TargetRule) -> &'static str {
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

/// The table of every route the pieces declare and every open route, sorted by path then method.
pub fn export(pieces: &[(&str, &Routes)], open: &[OpenRoute]) -> Vec<Row> {
    let mut rows = Vec::new();
    for (piece, routes) in pieces {
        for e in routes.entries() {
            let statuses = if e.write {
                WRITE.to_vec()
            } else {
                READ.to_vec()
            };
            rows.push(Row {
                piece: (*piece).to_owned(),
                method: e.method.as_str().to_owned(),
                path: e.path.to_owned(),
                scope: e.scope.to_owned(),
                target: target_name(e.target).to_owned(),
                write: e.write,
                idempotency_key: e.write,
                max_body: e.max_body,
                query: e.query,
                open: false,
                statuses,
            });
        }
    }
    for o in open {
        // An open route has no token, no scope and no target; the chain answers no problem on it.
        rows.push(Row {
            piece: "open".to_owned(),
            method: o.method.as_str().to_owned(),
            path: o.path.to_owned(),
            scope: "none".to_owned(),
            target: target_name(TargetRule::None).to_owned(),
            write: false,
            idempotency_key: false,
            max_body: 0,
            query: false,
            open: true,
            statuses: Vec::new(),
        });
    }
    rows.sort_by(|a, b| (&a.path, &a.method).cmp(&(&b.path, &b.method)));
    rows
}

/// The stable JSON form: `{"routes": [ {..}, .. ]}`, rows in the order given, the target a string.
pub fn to_json(rows: &[Row]) -> Value {
    let list: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "piece": r.piece,
                "method": r.method,
                "path": r.path,
                "scope": r.scope,
                "target": r.target,
                "write": r.write,
                "idempotency_key": r.idempotency_key,
                "max_body": r.max_body,
                "query": r.query,
                "open": r.open,
                "statuses": r.statuses,
            })
        })
        .collect();
    json!({ "routes": list })
}

/// The HTTP methods a path item can hold an operation under.
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// One twin operation, and the path item it sits in.
struct TwinOp<'a> {
    item: &'a Value,
    op: &'a Value,
}

/// Every operation of the twin, keyed by upper-case method and path. An operation is an object under a
/// method key; it counts without an `operationId`.
fn twin_operations(twin: &Value) -> BTreeMap<(String, String), TwinOp<'_>> {
    let mut out = BTreeMap::new();
    if let Some(paths) = twin.get("paths").and_then(Value::as_object) {
        for (path, item) in paths {
            for method in METHODS {
                if let Some(op) = item.get(method).filter(|o| o.is_object()) {
                    out.insert(
                        (method.to_ascii_uppercase(), path.clone()),
                        TwinOp { item, op },
                    );
                }
            }
        }
    }
    out
}

/// An operation's parameters with every `#/components/parameters/..` reference resolved.
struct Params<'a> {
    list: Vec<&'a Value>,
    /// A reference that resolves to nothing, or is not a components reference.
    dangling: bool,
}

fn parameters<'a>(twin: &'a Value, op: &'a Value) -> Params<'a> {
    let mut out = Params {
        list: Vec::new(),
        dangling: false,
    };
    let Some(list) = op.get("parameters").and_then(Value::as_array) else {
        return out;
    };
    for p in list {
        match p.get("$ref").and_then(Value::as_str) {
            Some(r) => match r
                .strip_prefix("#/components/parameters/")
                .and_then(|name| twin.pointer(&format!("/components/parameters/{name}")))
            {
                Some(found) => out.list.push(found),
                None => out.dangling = true,
            },
            None => out.list.push(p),
        }
    }
    out
}

/// How a twin value reads in a [`Drift::Target`]: the string itself, or `(missing)`, or its JSON.
fn describe(v: Option<&Value>) -> String {
    match v {
        None => "(missing)".to_owned(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// The comparison with the S3 JSON twin.
pub fn diff(rows: &[Row], twin: &Value) -> Vec<Drift> {
    let ops = twin_operations(twin);
    let mut by_key: BTreeMap<(&str, &str), Vec<&Row>> = BTreeMap::new();
    for r in rows {
        by_key
            .entry((r.method.as_str(), r.path.as_str()))
            .or_default()
            .push(r);
    }
    let mut out = Vec::new();
    for (m, p) in ops.keys() {
        if !by_key.contains_key(&(m.as_str(), p.as_str())) {
            out.push(Drift::MissingOperation {
                method: m.clone(),
                path: p.clone(),
            });
        }
    }
    for (m, p) in by_key.keys() {
        if !ops.contains_key(&((*m).to_owned(), (*p).to_owned())) {
            out.push(Drift::ExtraRoute {
                method: (*m).to_owned(),
                path: (*p).to_owned(),
            });
        }
    }
    for ((m, p), list) in &by_key {
        if list.len() > 1 {
            out.push(Drift::Duplicate {
                method: (*m).to_owned(),
                path: (*p).to_owned(),
            });
        }
    }
    for ((m, p), found) in &ops {
        if let Some(list) = by_key.get(&(m.as_str(), p.as_str())) {
            compare(&mut out, twin, list[0], found);
        }
    }
    out
}

/// One exported row against its twin operation.
fn compare(out: &mut Vec<Drift>, twin: &Value, row: &Row, found: &TwinOp<'_>) {
    let (item, op) = (found.item, found.op);
    let (method, path) = (row.method.clone(), row.path.clone());
    let field = |out: &mut Vec<Drift>, field: &'static str| {
        out.push(Drift::TwinField {
            method: method.clone(),
            path: path.clone(),
            field,
        });
    };

    let twin_open = match op.get("x-class").and_then(Value::as_str) {
        Some(class) => class == "open",
        None => {
            field(out, "x-class");
            false
        }
    };
    if row.open != twin_open {
        out.push(Drift::Open {
            method: method.clone(),
            path: path.clone(),
            exported: row.open,
            twin: twin_open,
        });
    }

    // The owning piece. The twin's `health` is the server's own open routes, which are no piece.
    if !twin_open {
        match op.get("x-piece").and_then(Value::as_str) {
            Some(piece) if piece != row.piece => out.push(Drift::Piece {
                method: method.clone(),
                path: path.clone(),
                exported: row.piece.clone(),
                twin: piece.to_owned(),
            }),
            Some(_) => {}
            None => field(out, "x-piece"),
        }
    }

    match op.get("x-scope").and_then(Value::as_str) {
        Some(scope) if scope != row.scope => out.push(Drift::Scope {
            method: method.clone(),
            path: path.clone(),
            exported: row.scope.clone(),
            twin: scope.to_owned(),
        }),
        Some(_) => {}
        None => field(out, "x-scope"),
    }

    // X-07: the twin names the variant. A missing value, or a string that is not one of the eight, is a
    // drift; it is never read as `None`.
    let target = op.get("x-target");
    let known = target
        .and_then(Value::as_str)
        .filter(|t| RULES.iter().any(|r| target_name(*r) == *t));
    if known != Some(row.target.as_str()) {
        out.push(Drift::Target {
            method: method.clone(),
            path: path.clone(),
            exported: row.target.clone(),
            twin: describe(target),
        });
    }

    match op.get("x-mutates").and_then(Value::as_bool) {
        Some(writes) if writes != row.write => out.push(Drift::Write {
            method: method.clone(),
            path: path.clone(),
            exported: row.write,
            twin: writes,
        }),
        Some(_) => {}
        None => field(out, "x-mutates"),
    }

    // The body cap, where the twin declares one (every write does, and `POST /v2/search`): a route that
    // forgot `.max_body()` keeps `Routes`' own default and is a drift. A read the twin gives no cap makes
    // no claim, so the export's cap is not compared there.
    match op.get("x-max-body") {
        None => {}
        Some(v) => match v.as_u64() {
            Some(cap) if cap != row.max_body => out.push(Drift::MaxBody {
                method: method.clone(),
                path: path.clone(),
                exported: row.max_body,
                twin: cap,
            }),
            Some(_) => {}
            None => field(out, "x-max-body"),
        },
    }

    // Parameters. What this reader does not interpret is a drift, not a silent pass: a path-item
    // `parameters` (the pinned twin has none) and a reference that resolves to nothing.
    if item.get("parameters").is_some() {
        field(out, "path-item parameters");
    }
    let params = parameters(twin, op);
    if params.dangling {
        field(out, "parameters");
    }
    // The key is a header named `Idempotency-Key` that is REQUIRED; an optional one is not a key.
    let mut twin_key = false;
    let mut required_bad = false;
    for p in params.list.iter().filter(|p| {
        p["in"] == "header"
            && p["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case("idempotency-key"))
    }) {
        match p.get("required") {
            Some(Value::Bool(true)) => twin_key = true,
            Some(Value::Bool(false)) | None => {}
            Some(_) => required_bad = true,
        }
    }
    if required_bad {
        field(out, "required");
    }
    if row.idempotency_key != twin_key {
        out.push(Drift::IdempotencyKey {
            method: method.clone(),
            path: path.clone(),
            exported: row.idempotency_key,
            twin: twin_key,
        });
    }
    let twin_query = params.list.iter().any(|p| p["in"] == "query");
    if row.query != twin_query {
        out.push(Drift::Query {
            method: method.clone(),
            path: path.clone(),
            exported: row.query,
            twin: twin_query,
        });
    }

    // The statuses: see the module doc for the rule.
    let exported: BTreeSet<u16> = row.statuses.iter().copied().collect();
    let (only_exported, only_twin): (Vec<u16>, Vec<u16>) = if twin_open {
        // The server's own routes: the chain answers no problem on them, and what the twin lists there is
        // that handler's.
        (exported.iter().copied().collect(), Vec::new())
    } else {
        let explicit: BTreeSet<u16> = op
            .get("responses")
            .and_then(Value::as_object)
            .map(|m| m.keys().filter_map(|k| k.parse().ok()).collect())
            .unwrap_or_default();
        let default_is_problem = op
            .pointer("/responses/default/$ref")
            .and_then(Value::as_str)
            == Some("#/components/responses/Problem")
            && twin.pointer("/components/responses/Problem").is_some();
        let covered = |s: &u16| {
            explicit.contains(s)
                || UNIVERSAL_R9.contains(s)
                || UNIVERSAL_R16.contains(s)
                || (default_is_problem && DEFAULT_COVERS.contains(s))
        };
        (
            exported.iter().copied().filter(|s| !covered(s)).collect(),
            explicit
                .iter()
                .copied()
                .filter(|s| {
                    UNIVERSE.contains(s)
                        && !exported.contains(s)
                        && !(RESERVED.contains(s) && !row.write)
                })
                .collect(),
        )
    };
    if !only_exported.is_empty() || !only_twin.is_empty() {
        out.push(Drift::Statuses {
            method,
            path,
            only_exported,
            only_twin,
        });
    }
}
