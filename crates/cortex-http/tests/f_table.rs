//! Wave 1 item F: the exported route table (Q4; sam's X-07) and its comparison with the S3 twin. DB-free.
//!
//! The strongest cell builds a synthetic `Routes` table FROM the pinned S3 r2.7 twin itself (33
//! operations on 30 paths) and requires that its export has zero drift against that same twin; the
//! drift cells then change one field at a time and require exactly that drift. A real piece's routes
//! take the same path through `export` and `diff`.
use std::collections::BTreeSet;

use axum::http::Method;
use axum::routing::get;
use cortex_http::chain::OpenRoute;
use cortex_http::table::{Drift, Row, diff, export, target_name, to_json};
use cortex_module::{Authorized, Reply, Routes, Target, TargetRule};
use serde_json::{Value, json};

const ALL: [TargetRule; 8] = [
    TargetRule::None,
    TargetRule::SelfWorker,
    TargetRule::SelfOrProjectLead,
    TargetRule::NotProjectLead,
    TargetRule::RetireTarget,
    TargetRule::TokenTarget,
    TargetRule::Lineage,
    TargetRule::TaskAddressee,
];

fn twin_path() -> String {
    std::env::var("CORTEX_S3_TWIN").unwrap_or_else(|_| {
        "/path/to/docs/mvp/wave1-pins-20261106-S3r27/S3/cortex-v2-skeleton.openapi.r2.7-1d2c4b2f.json".to_owned()
    })
}

fn twin() -> Value {
    let path = twin_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the S3 twin at {path} could not be read: {e}"));
    serde_json::from_str(&text).expect("the S3 twin is JSON")
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

fn rule_of(name: &str) -> TargetRule {
    *ALL.iter()
        .find(|r| target_name(**r) == name)
        .unwrap_or_else(|| panic!("the twin names a target {name:?} that is not one of the eight"))
}

/// Every operation of the twin: (method, path, the operation object).
fn operations(twin: &Value) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    for (path, item) in twin["paths"].as_object().expect("paths") {
        for (method, op) in item.as_object().expect("a path item") {
            if op.get("operationId").is_some() {
                out.push((method.to_uppercase(), path.clone(), op.clone()));
            }
        }
    }
    out
}

fn declares_query(twin: &Value, op: &Value) -> bool {
    op["parameters"].as_array().is_some_and(|ps| {
        ps.iter().any(|p| {
            let p = match p.get("$ref").and_then(Value::as_str) {
                Some(r) => &twin["components"]["parameters"][r.rsplit('/').next().expect("a ref")],
                None => p,
            };
            p["in"] == "query"
        })
    })
}

/// The pieces and the open routes a registry would hand over for the whole twin.
fn synthetic(twin: &Value) -> (Vec<(&'static str, Routes)>, Vec<OpenRoute>) {
    use std::collections::BTreeMap;
    let mut by_piece: BTreeMap<String, Routes> = BTreeMap::new();
    let mut open = Vec::new();
    for (method, path, op) in operations(twin) {
        let m = Method::from_bytes(method.as_bytes()).expect("a method");
        let (path, scope) = (leak(&path), leak(op["x-scope"].as_str().expect("x-scope")));
        if op["x-class"] == "open" {
            open.push(OpenRoute {
                method: m,
                path,
                handler: get(|| async { "ok" }),
            });
            continue;
        }
        let rule = rule_of(op["x-target"].as_str().expect("x-target"));
        let piece = op["x-piece"].as_str().expect("x-piece").to_owned();
        let routes = by_piece.remove(&piece).unwrap_or_default();
        let routes = if op["x-mutates"] == true {
            routes.write(
                m,
                path,
                scope,
                rule,
                |_tx, ctx, _b: &Value| {
                    Box::pin(async move { ctx.authorize(&Target::default(), ()) })
                },
                |_tx, _ctx, _a: Authorized<()>, _b: Value| {
                    Box::pin(async move { Ok(Reply::ok(json!({}))) })
                },
            )
        } else {
            let r = routes.read(m, path, scope, rule, || async { "x" });
            if declares_query(twin, &op) {
                r.query()
            } else {
                r
            }
        };
        // The twin's body cap, where it declares one: 16 KiB on most writes, `Routes`' default is 1 MiB.
        let routes = match op["x-max-body"].as_u64() {
            Some(cap) => routes.max_body(cap),
            None => routes,
        };
        by_piece.insert(piece, routes);
    }
    (
        by_piece.into_iter().map(|(k, v)| (leak(&k), v)).collect(),
        open,
    )
}

fn exported(twin: &Value) -> Vec<Row> {
    let (pieces, open) = synthetic(twin);
    let refs: Vec<(&str, &Routes)> = pieces.iter().map(|(n, r)| (*n, r)).collect();
    export(&refs, &open)
}

// ---- 1. the export of hand-built routes -----------------------------------------------------------

#[test]
fn f1_every_route_is_exported_with_its_declared_fields_sorted_by_path_then_method() {
    let routes = Routes::new()
        .write(
            Method::POST,
            "/v2/p/w",
            "records:write",
            TargetRule::Lineage,
            |_tx, ctx, _b: &Value| Box::pin(async move { ctx.authorize(&Target::default(), ()) }),
            |_tx, _ctx, _a: Authorized<()>, _b: Value| {
                Box::pin(async move { Ok(Reply::ok(json!({}))) })
            },
        )
        .max_body(64)
        .read(
            Method::GET,
            "/v2/p/list",
            "records:read",
            TargetRule::None,
            || async { "x" },
        )
        .query()
        .read(
            Method::GET,
            "/v2/p/a",
            "self:read",
            TargetRule::None,
            || async { "x" },
        );
    let open = vec![OpenRoute {
        method: Method::GET,
        path: "/v2/p/health",
        handler: get(|| async { "ok" }),
    }];
    let rows = export(&[("piece_p", &routes)], &open);
    let caps: Vec<u64> = routes.entries().map(|e| e.max_body).collect();
    let row = |piece: &str,
               method: &str,
               path: &str,
               scope: &str,
               target: &str,
               write: bool,
               query: bool,
               open: bool,
               max_body: u64,
               statuses: &[u16]| Row {
        piece: piece.to_owned(),
        method: method.to_owned(),
        path: path.to_owned(),
        scope: scope.to_owned(),
        target: target.to_owned(),
        write,
        idempotency_key: write,
        max_body,
        query,
        open,
        statuses: statuses.to_vec(),
    };
    // Sorted by path, then method: a, health, list, w.
    assert_eq!(
        rows,
        vec![
            row(
                "piece_p",
                "GET",
                "/v2/p/a",
                "self:read",
                "None",
                false,
                false,
                false,
                caps[2],
                &[400, 401, 403, 413, 422, 500, 503]
            ),
            row(
                "open",
                "GET",
                "/v2/p/health",
                "none",
                "None",
                false,
                false,
                true,
                0,
                &[]
            ),
            row(
                "piece_p",
                "GET",
                "/v2/p/list",
                "records:read",
                "None",
                false,
                true,
                false,
                caps[1],
                &[400, 401, 403, 413, 422, 500, 503]
            ),
            row(
                "piece_p",
                "POST",
                "/v2/p/w",
                "records:write",
                "Lineage",
                true,
                false,
                false,
                64,
                &[400, 401, 403, 409, 413, 422, 428, 429, 500, 503]
            ),
        ]
    );
}

#[test]
fn f2_target_name_is_the_variant_string_for_all_eight_and_equals_the_twins_set() {
    for rule in ALL {
        assert_eq!(
            target_name(rule),
            format!("{rule:?}"),
            "the name is the variant's own"
        );
    }
    let names: BTreeSet<&str> = ALL.iter().map(|r| target_name(*r)).collect();
    assert_eq!(names.len(), 8);
    let in_twin: BTreeSet<String> = operations(&twin())
        .into_iter()
        .filter_map(|(_, _, op)| op["x-target"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        names
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>(),
        in_twin
    );
}

#[test]
fn f3_the_json_is_stable_and_none_is_a_string_not_null() {
    let rows = exported(&twin());
    let j = to_json(&rows);
    let list = j["routes"].as_array().expect("a routes array");
    assert_eq!(list.len(), rows.len());
    let keys: Vec<&str> = list[0]
        .as_object()
        .expect("a row")
        .keys()
        .map(String::as_str)
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        [
            "idempotency_key",
            "max_body",
            "method",
            "open",
            "path",
            "piece",
            "query",
            "scope",
            "statuses",
            "target",
            "write"
        ],
        "exactly these fields, and no handler, closure or body type"
    );
    for r in list {
        assert!(
            r["target"].is_string(),
            "the target is the variant STRING: {r}"
        );
    }
    assert!(
        list.iter().any(|r| r["target"] == "None"),
        "None is the string \"None\""
    );
    // Sorted by path then method, and identical when exported twice.
    let order: Vec<(String, String)> = list
        .iter()
        .map(|r| {
            (
                r["path"].as_str().unwrap().to_owned(),
                r["method"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mut by_hand = order.clone();
    by_hand.sort();
    assert_eq!(order, by_hand, "sorted by path then method");
    assert_eq!(to_json(&exported(&twin())), j, "stable across runs");
}

// ---- 2. the comparison with the twin --------------------------------------------------------------

#[test]
fn f4_the_whole_twin_exported_has_no_drift_against_the_same_twin() {
    let twin = twin();
    let rows = exported(&twin);
    assert_eq!(rows.len(), 33, "33 operations");
    let paths: BTreeSet<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths.len(), 30, "on 30 paths");
    let pairs: BTreeSet<(&str, &str)> = rows
        .iter()
        .map(|r| (r.method.as_str(), r.path.as_str()))
        .collect();
    assert_eq!(pairs.len(), 33, "one row per method and path");
    assert_eq!(
        diff(&rows, &twin),
        vec![],
        "the export of the twin's own routes must match the twin"
    );
}

fn drift_after(change: impl FnOnce(&mut Vec<Row>)) -> Vec<Drift> {
    let twin = twin();
    let mut rows = exported(&twin);
    change(&mut rows);
    diff(&rows, &twin)
}

fn first_authenticated_write(rows: &[Row]) -> usize {
    rows.iter()
        .position(|r| r.write && !r.open)
        .expect("a write")
}

/// A plain GET read. `POST /v2/search` and `GET /v2/sections/{name}` are left out only so the cells
/// below pick the same, simplest operations each time.
fn first_authenticated_read(rows: &[Row]) -> usize {
    rows.iter()
        .position(|r| {
            r.method == "GET" && !r.write && !r.open && !r.query && r.path != "/v2/sections/{name}"
        })
        .expect("a plain read")
}

#[test]
fn f5_each_kind_of_drift_is_found_and_only_that_kind() {
    // A missing operation.
    let d = drift_after(|rows| {
        rows.remove(0);
    });
    assert!(
        matches!(d.as_slice(), [Drift::MissingOperation { .. }]),
        "{d:?}"
    );

    // An extra route.
    let d = drift_after(|rows| {
        let mut extra = rows[0].clone();
        extra.path = "/v2/not-in-the-twin".to_owned();
        rows.push(extra);
    });
    assert!(matches!(d.as_slice(), [Drift::ExtraRoute { .. }]), "{d:?}");

    // The same method and path twice.
    let d = drift_after(|rows| {
        let again = rows[0].clone();
        rows.push(again);
    });
    assert!(matches!(d.as_slice(), [Drift::Duplicate { .. }]), "{d:?}");

    // A changed scope.
    let d = drift_after(|rows| {
        let i = first_authenticated_read(rows);
        rows[i].scope = "audit:read".to_owned();
    });
    assert!(matches!(d.as_slice(), [Drift::Scope { .. }]), "{d:?}");

    // A changed target.
    let d = drift_after(|rows| {
        let i = first_authenticated_read(rows);
        rows[i].target = if rows[i].target == "TokenTarget" {
            "None"
        } else {
            "TokenTarget"
        }
        .to_owned();
    });
    assert!(matches!(d.as_slice(), [Drift::Target { .. }]), "{d:?}");

    // A write that is exported as a read, and the key with it.
    let d = drift_after(|rows| {
        let i = first_authenticated_write(rows);
        rows[i].write = false;
    });
    assert!(d.iter().any(|x| matches!(x, Drift::Write { .. })), "{d:?}");

    // A route that no longer asks for the key.
    let d = drift_after(|rows| {
        let i = first_authenticated_write(rows);
        rows[i].idempotency_key = false;
    });
    assert!(
        matches!(d.as_slice(), [Drift::IdempotencyKey { .. }]),
        "{d:?}"
    );

    // A chain-level status the export lacks (the twin lists 428 on every write).
    let d = drift_after(|rows| {
        let i = first_authenticated_write(rows);
        rows[i].statuses.retain(|s| *s != 428);
    });
    assert!(
        matches!(d.as_slice(), [Drift::Statuses { only_twin, .. }] if only_twin == &[428]),
        "{d:?}"
    );
}

#[test]
fn f6_a_missing_or_unknown_twin_target_is_a_drift_never_none() {
    // X-07: `None` is the explicit string; a missing or unknown value fails and is not coerced.
    let twin_full = twin();
    let rows = exported(&twin_full);
    let i = first_authenticated_read(&rows);
    let (method, path) = (rows[i].method.to_lowercase(), rows[i].path.clone());
    for bogus in [Value::Null, json!("Bogus"), json!("none")] {
        let mut t = twin_full.clone();
        t["paths"][&path][&method]["x-target"] = bogus.clone();
        let d = diff(&rows, &t);
        assert!(
            d.iter().any(|x| matches!(x, Drift::Target { .. })),
            "a twin target of {bogus} must be a drift, not read as None: {d:?}"
        );
    }
    let mut t = twin_full.clone();
    t["paths"][&path][&method]
        .as_object_mut()
        .unwrap()
        .remove("x-target");
    assert!(
        diff(&rows, &t)
            .iter()
            .any(|x| matches!(x, Drift::Target { .. })),
        "an absent x-target is a drift"
    );
}

#[test]
fn f7_a_query_that_only_one_side_declares_is_a_drift() {
    let d = drift_after(|rows| {
        let i = first_authenticated_read(rows);
        rows[i].query = true;
    });
    assert!(
        matches!(
            d.as_slice(),
            [Drift::Query {
                exported: true,
                twin: false,
                ..
            }]
        ),
        "{d:?}"
    );
    let d = drift_after(|rows| {
        let i = rows
            .iter()
            .position(|r| r.query)
            .expect("a read that declares a query");
        rows[i].query = false;
    });
    assert!(
        matches!(
            d.as_slice(),
            [Drift::Query {
                exported: false,
                twin: true,
                ..
            }]
        ),
        "{d:?}"
    );
}

#[test]
fn f8_open_must_agree_with_the_twins_class() {
    let d = drift_after(|rows| {
        let i = rows.iter().position(|r| r.open).expect("an open route");
        rows[i].open = false;
    });
    assert!(
        d.iter().any(|x| matches!(
            x,
            Drift::Open {
                exported: false,
                twin: true,
                ..
            }
        )),
        "{d:?}"
    );
    let d = drift_after(|rows| {
        let i = first_authenticated_read(rows);
        rows[i].open = true;
    });
    assert!(
        d.iter().any(|x| matches!(
            x,
            Drift::Open {
                exported: true,
                twin: false,
                ..
            }
        )),
        "{d:?}"
    );
}

#[test]
fn f9_a_twin_field_that_is_missing_or_mistyped_is_its_own_drift() {
    let twin_full = twin();
    let rows = exported(&twin_full);
    let i = first_authenticated_write(&rows);
    let (method, path) = (rows[i].method.to_lowercase(), rows[i].path.clone());
    for (field, bogus) in [
        ("x-mutates", json!("yes")),
        ("x-scope", json!(7)),
        ("x-class", Value::Null),
    ] {
        let mut t = twin_full.clone();
        t["paths"][&path][&method][field] = bogus.clone();
        let d = diff(&rows, &t);
        assert!(
            d.iter()
                .any(|x| matches!(x, Drift::TwinField { field: f, .. } if *f == field)),
            "{field} = {bogus}: {d:?}"
        );
    }
    for field in ["x-mutates", "x-scope", "x-class"] {
        let mut t = twin_full.clone();
        t["paths"][&path][&method]
            .as_object_mut()
            .unwrap()
            .remove(field);
        let d = diff(&rows, &t);
        assert!(
            d.iter()
                .any(|x| matches!(x, Drift::TwinField { field: f, .. } if *f == field)),
            "{field} removed: {d:?}"
        );
    }
}

/// Rows for the twin, the row `pick` chooses, and the diff after `edit` has changed that operation.
fn drift_twin_op(pick: fn(&[Row]) -> usize, edit: impl FnOnce(&mut Value)) -> Vec<Drift> {
    let twin_full = twin();
    let rows = exported(&twin_full);
    let i = pick(&rows);
    let (method, path) = (rows[i].method.to_lowercase(), rows[i].path.clone());
    let mut t = twin_full.clone();
    edit(&mut t["paths"][&path][&method]);
    diff(&rows, &t)
}

/// The same, editing the PATH ITEM of the chosen operation.
fn drift_twin_path(pick: fn(&[Row]) -> usize, edit: impl FnOnce(&mut Value)) -> Vec<Drift> {
    let twin_full = twin();
    let rows = exported(&twin_full);
    let i = pick(&rows);
    let path = rows[i].path.clone();
    let mut t = twin_full.clone();
    edit(&mut t["paths"][&path]);
    diff(&rows, &t)
}

#[test]
fn f10_the_status_rule_is_explicit_and_each_part_of_it_can_fail() {
    // 503 is listed by the twin on EVERY authenticated operation: an export without it is a drift.
    let d = drift_after(|rows| {
        let i = first_authenticated_read(rows);
        rows[i].statuses.retain(|s| *s != 503);
    });
    assert!(
        matches!(d.as_slice(), [Drift::Statuses { only_twin, only_exported, .. }] if only_twin == &[503] && only_exported.is_empty()),
        "{d:?}"
    );
    // 428 is the key status of a WRITE, listed explicitly there and not universal: a read that claims it.
    let d = drift_after(|rows| {
        let i = first_authenticated_read(rows);
        rows[i].statuses.push(428);
        rows[i].statuses.sort_unstable();
    });
    assert!(
        matches!(d.as_slice(), [Drift::Statuses { only_exported, only_twin, .. }] if only_exported == &[428] && only_twin.is_empty()),
        "{d:?}"
    );
    // The twin lists a chain status the chain cannot answer for this kind: 428 on a read.
    let d = drift_twin_op(first_authenticated_read, |op| {
        op["responses"]["428"] = json!({"description": "x"});
    });
    assert!(
        matches!(d.as_slice(), [Drift::Statuses { only_twin, .. }] if only_twin == &[428]),
        "{d:?}"
    );
    // 409 and 500 are covered by the twin's `default` (the shared Problem). Without it a write's 409 and
    // 500 are claimed by the export and documented nowhere; a read claims 500 only.
    let d = drift_twin_op(first_authenticated_write, |op| {
        op["responses"].as_object_mut().unwrap().remove("default");
    });
    assert!(
        matches!(d.as_slice(), [Drift::Statuses { only_exported, .. }] if only_exported == &[409, 500]),
        "{d:?}"
    );
    let d = drift_twin_op(first_authenticated_read, |op| {
        op["responses"].as_object_mut().unwrap().remove("default");
    });
    assert!(
        matches!(d.as_slice(), [Drift::Statuses { only_exported, .. }] if only_exported == &[500]),
        "{d:?}"
    );
    // A `default` that is not the shared Problem covers nothing.
    let d = drift_twin_op(first_authenticated_read, |op| {
        op["responses"]["default"] = json!({"description": "something else"});
    });
    assert!(
        matches!(d.as_slice(), [Drift::Statuses { only_exported, .. }] if only_exported == &[500]),
        "{d:?}"
    );
    // 429 is reserved: the twin lists it on a read, the chain cannot answer it there yet, no drift.
    let twin_full = twin();
    let rows = exported(&twin_full);
    let i = first_authenticated_read(&rows);
    let op = &twin_full["paths"][&rows[i].path][rows[i].method.to_lowercase()];
    assert!(
        op["responses"].get("429").is_some(),
        "the twin lists 429 on this read"
    );
    assert!(
        !rows[i].statuses.contains(&429),
        "the export does not claim 429 for a read"
    );
    assert_eq!(diff(&rows, &twin_full), vec![]);
    // An open route answers no problem status of the chain's: one that claims 401 is a drift. What the
    // twin lists on an open route (503 on /v2/health/ready) is that route's own handler, not compared.
    let d = drift_after(|rows| {
        let i = rows.iter().position(|r| r.open).expect("an open route");
        rows[i].statuses.push(401);
    });
    assert!(
        matches!(d.as_slice(), [Drift::Statuses { only_exported, .. }] if only_exported == &[401]),
        "{d:?}"
    );
}

#[test]
fn f12_the_named_status_sets_are_quoted_from_the_twins_own_text() {
    let t = twin();
    let rule = |prefix: &str| {
        t["x-rules"]
            .as_object()
            .expect("x-rules")
            .iter()
            .find(|(k, _)| k.starts_with(prefix))
            .unwrap_or_else(|| panic!("no x-rules entry {prefix}"))
            .1
            .to_string()
    };
    let (r9, r16) = (rule("R-9 "), rule("R-16 "));
    let problem = t["components"]["responses"]["Problem"]["description"].to_string();
    for c in cortex_http::table::UNIVERSAL_R9 {
        assert!(r9.contains(&c.to_string()), "R-9 does not mention {c}");
    }
    for c in cortex_http::table::UNIVERSAL_R16 {
        assert!(r16.contains(&c.to_string()), "R-16 does not mention {c}");
    }
    for c in cortex_http::table::DEFAULT_COVERS {
        assert!(
            problem.contains(&c.to_string()),
            "the Problem response does not mention {c}"
        );
    }
    // The key status is a write's own, listed explicitly: R-9 does not make it universal.
    assert!(
        !r9.contains("428"),
        "R-9 mentions 428, so it would be universal"
    );
    // 429 reserved is the one status the twin lists on every authenticated operation that a read cannot answer.
    assert_eq!(cortex_http::table::RESERVED, [429]);
    // The shared Problem response is what every operation's `default` points at, in the pinned twin.
    for (method, path, op) in operations(&t) {
        assert_eq!(
            op["responses"]["default"],
            json!({"$ref": "#/components/responses/Problem"}),
            "{method} {path}"
        );
    }
}

/// `Routes`' own cap for a route that never calls `.max_body()`, read from the library.
fn default_cap() -> u64 {
    Routes::new()
        .read(
            Method::GET,
            "/v2/probe",
            "self:read",
            TargetRule::None,
            || async { "x" },
        )
        .entries()
        .next()
        .expect("one route")
        .max_body
}

#[test]
fn f11_a_body_cap_that_differs_from_the_twins_is_a_drift_and_a_forgotten_one_too() {
    let twin_full = twin();
    let rows = exported(&twin_full);
    let i = first_authenticated_write(&rows);
    let (method, path) = (rows[i].method.to_lowercase(), rows[i].path.clone());
    let real = twin_full["paths"][&path][&method]["x-max-body"]
        .as_u64()
        .expect("every write in the twin carries x-max-body");
    assert_eq!(
        rows[i].max_body, real,
        "the synthetic table declares the twin's cap"
    );
    assert_ne!(
        default_cap(),
        real,
        "the twin's cap is not Routes' default, or this cell proves nothing"
    );

    // The cap is changed.
    let d = drift_after(|rows| rows[i].max_body = real + 1);
    assert!(
        matches!(d.as_slice(), [Drift::MaxBody { exported, twin, .. }] if *exported == real + 1 && *twin == real),
        "{d:?}"
    );
    // The cap is forgotten: the route keeps Routes' own default.
    let default = default_cap();
    let d = drift_after(|rows| rows[i].max_body = default);
    assert!(
        matches!(d.as_slice(), [Drift::MaxBody { exported, .. }] if *exported == default),
        "{d:?}"
    );

    // A twin cap that is not a non-negative whole number is its own drift.
    for bogus in [json!("big"), json!(-1), json!(1.5), Value::Null] {
        let mut t = twin_full.clone();
        t["paths"][&path][&method]["x-max-body"] = bogus.clone();
        let d = diff(&rows, &t);
        assert!(
            d.iter()
                .any(|x| matches!(x, Drift::TwinField { field, .. } if *field == "x-max-body")),
            "x-max-body = {bogus}: {d:?}"
        );
    }

    // A read the twin gives no cap: it makes no claim, so the export's cap is not compared.
    let d = drift_after(|rows| {
        let j = first_authenticated_read(rows);
        rows[j].max_body = 7;
    });
    assert_eq!(d, vec![], "the twin declares no cap for a plain read");
}

#[test]
fn f13_an_optional_key_header_is_not_a_required_key() {
    let twin_full = twin();
    let rows = exported(&twin_full);
    let writes = rows.iter().filter(|r| r.write).count();
    assert!(writes > 0);
    // The shared parameter says required:false, or says nothing: every write drifts, and nothing else does.
    for (what, change) in [
        ("required:false", Some(json!(false))),
        ("required absent", None),
    ] {
        let mut t = twin_full.clone();
        let param = &mut t["components"]["parameters"]["IdempotencyKey"];
        match change {
            Some(v) => param["required"] = v,
            None => {
                param.as_object_mut().unwrap().remove("required");
            }
        }
        let d = diff(&rows, &t);
        let hits = d
            .iter()
            .filter(|x| {
                matches!(
                    x,
                    Drift::IdempotencyKey {
                        exported: true,
                        twin: false,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(hits, writes, "{what}: every write must drift: {d:?}");
        assert_eq!(d.len(), writes, "{what}: and nothing else: {d:?}");
    }
    // A `required` that is not a boolean is its own drift on every write.
    let mut t = twin_full.clone();
    t["components"]["parameters"]["IdempotencyKey"]["required"] = json!("true");
    let d = diff(&rows, &t);
    let hits = d
        .iter()
        .filter(|x| matches!(x, Drift::TwinField { field, .. } if *field == "required"))
        .count();
    assert_eq!(hits, writes, "{d:?}");
    // The same three, on a DIRECT parameter of one write.
    let inline = |op: &mut Value, required: Option<Value>| {
        for p in op["parameters"].as_array_mut().expect("parameters") {
            if p.get("$ref")
                .and_then(Value::as_str)
                .is_some_and(|r| r.ends_with("/IdempotencyKey"))
            {
                let mut direct = json!({"name": "Idempotency-Key", "in": "header"});
                if let Some(v) = required.clone() {
                    direct["required"] = v;
                }
                *p = direct;
            }
        }
    };
    let d = drift_twin_op(first_authenticated_write, |op| {
        inline(op, Some(json!(false)))
    });
    assert!(
        matches!(
            d.as_slice(),
            [Drift::IdempotencyKey {
                exported: true,
                twin: false,
                ..
            }]
        ),
        "{d:?}"
    );
    let d = drift_twin_op(first_authenticated_write, |op| inline(op, None));
    assert!(
        matches!(
            d.as_slice(),
            [Drift::IdempotencyKey {
                exported: true,
                twin: false,
                ..
            }]
        ),
        "{d:?}"
    );
    // The positive control: a direct parameter that IS required is the key.
    let d = drift_twin_op(first_authenticated_write, |op| {
        inline(op, Some(json!(true)))
    });
    assert_eq!(d, vec![], "a direct required key header must pass");
}

#[test]
fn f14_what_the_comparison_cannot_read_is_a_drift_not_a_silent_pass() {
    // A path-item `parameters` (the pinned twin has none) is not interpreted: a drift, so a query added
    // there cannot make both sides agree by being ignored.
    let d = drift_twin_path(first_authenticated_read, |item| {
        item["parameters"] = json!([{"name": "q", "in": "query"}]);
    });
    assert!(
        d.iter().any(
            |x| matches!(x, Drift::TwinField { field, .. } if *field == "path-item parameters")
        ),
        "{d:?}"
    );
    // A parameter reference that resolves to nothing.
    let d = drift_twin_op(first_authenticated_read, |op| {
        op["parameters"] = json!([{"$ref": "#/components/parameters/Nope"}]);
    });
    assert!(
        d.iter()
            .any(|x| matches!(x, Drift::TwinField { field, .. } if *field == "parameters")),
        "{d:?}"
    );
    // An operation without an `operationId` is still an operation: the twin's missing one is reported ...
    let twin_full = twin();
    let rows = exported(&twin_full);
    let mut t = twin_full.clone();
    t["paths"]["/v2/not-yet-routed"] = json!({
        "post": {
            "x-class": "work", "x-scope": "records:write", "x-target": "None", "x-mutates": true,
            "x-piece": "records", "x-max-body": 16384,
            "responses": {"200": {"description": "x"}, "default": {"$ref": "#/components/responses/Problem"}}
        }
    });
    let d = diff(&rows, &t);
    assert!(
        matches!(d.as_slice(), [Drift::MissingOperation { method, path }] if method == "POST" && path == "/v2/not-yet-routed"),
        "{d:?}"
    );
    // ... and removing the `operationId` of an existing one does not hide it: it is still compared.
    let d = drift_twin_op(first_authenticated_read, |op| {
        op.as_object_mut().unwrap().remove("operationId");
    });
    assert_eq!(
        d,
        vec![],
        "an operation without operationId is compared like any other"
    );
}

#[test]
fn f15_the_owning_piece_is_compared_except_for_the_servers_open_routes() {
    // A row declared by another piece than the twin's x-piece.
    let twin_full = twin();
    let rows = exported(&twin_full);
    let i = first_authenticated_read(&rows);
    let real = twin_full["paths"][&rows[i].path][rows[i].method.to_lowercase()]["x-piece"]
        .as_str()
        .expect("x-piece")
        .to_owned();
    let d = drift_after(|rows| rows[i].piece = "wrong_piece".to_owned());
    assert!(
        matches!(d.as_slice(), [Drift::Piece { exported, twin, .. }] if exported == "wrong_piece" && *twin == real),
        "{d:?}"
    );
    // The open routes are the server's own (piece 11): the twin's `health` is not a piece to compare.
    let d = drift_after(|rows| {
        for r in rows.iter_mut().filter(|r| r.open) {
            r.piece = "anything".to_owned();
        }
    });
    assert_eq!(d, vec![], "an open route's piece is not compared");
    // But an authenticated operation whose twin says `health` is compared, and drifts.
    let d = drift_twin_op(first_authenticated_read, |op| {
        op["x-piece"] = json!("health")
    });
    assert!(
        matches!(d.as_slice(), [Drift::Piece { twin, .. }] if twin == "health"),
        "{d:?}"
    );
    // A twin x-piece that is missing or not a string is its own drift.
    for bogus in [json!(7), Value::Null] {
        let d = drift_twin_op(first_authenticated_read, |op| op["x-piece"] = bogus.clone());
        assert!(
            d.iter()
                .any(|x| matches!(x, Drift::TwinField { field, .. } if *field == "x-piece")),
            "x-piece = {bogus}: {d:?}"
        );
    }
    let d = drift_twin_op(first_authenticated_read, |op| {
        op.as_object_mut().unwrap().remove("x-piece");
    });
    assert!(
        d.iter()
            .any(|x| matches!(x, Drift::TwinField { field, .. } if *field == "x-piece")),
        "{d:?}"
    );
}

#[test]
fn f16_a_default_that_points_at_a_component_that_is_not_there_covers_nothing() {
    let twin_full = twin();
    let rows = exported(&twin_full);
    let mut t = twin_full.clone();
    t["components"]["responses"]
        .as_object_mut()
        .expect("components.responses")
        .remove("Problem");
    let d = diff(&rows, &t);
    let authenticated = rows.iter().filter(|r| !r.open).count();
    assert!(authenticated > 0);
    // Every authenticated operation claims 500 (and a write 409) and now nothing documents it.
    let uncovered = d
        .iter()
        .filter(|x| {
            matches!(x, Drift::Statuses { only_exported, only_twin, .. }
                if only_exported.contains(&500) && only_twin.is_empty())
        })
        .count();
    assert_eq!(uncovered, authenticated, "{d:?}");
    assert_eq!(d.len(), authenticated, "and nothing else drifts: {d:?}");
    assert!(
        d.iter().any(
            |x| matches!(x, Drift::Statuses { only_exported, .. } if only_exported == &[409, 500])
        ),
        "a write claims 409 as well"
    );
}

#[test]
fn f17_429_is_reserved_for_reads_only_a_write_without_it_drifts() {
    // The twin lists 429 on every authenticated operation. A READ cannot answer it yet (f10 pins that), but
    // a WRITE can: a lock held by another request is 429 `busy`. So an export that drops 429 from a write
    // is a drift, and the reserved exception must not hide it (vera, F-REWORK-REVIEW, Low).
    let d = drift_after(|rows| {
        let i = first_authenticated_write(rows);
        rows[i].statuses.retain(|s| *s != 429);
    });
    assert!(
        matches!(d.as_slice(), [Drift::Statuses { only_twin, only_exported, .. }] if only_twin == &[429] && only_exported.is_empty()),
        "{d:?}"
    );
    // The same edit on a read changes nothing (the export never claims 429 for a read), and a read that
    // CLAIMS 429 is not covered by the twin's reservation either way: it is listed, so no drift.
    let twin_full = twin();
    let rows = exported(&twin_full);
    let i = first_authenticated_read(&rows);
    assert!(!rows[i].statuses.contains(&429));
    assert_eq!(diff(&rows, &twin_full), vec![]);
}
