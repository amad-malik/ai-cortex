//! Wave 0 S0 delta 4 (cox c025db40; kai 68cecbdb): the two-phase write route as the registry and the
//! chain see it. DB-free: the closures need a transaction, so what is pinned here is the SHAPE
//! (what a piece writes, what `into_entries` hands the chain) and the registry's view; the closures
//! themselves run in `plug-identity`'s DB test (the one DB run).
//!
//! A piece writes `authorize` (step-0 422, then the read: 404, then `ctx.authorize`: 403) and `execute`
//! (the 409 checks and the write). `execute` cannot run without an `Authorized<T>`, and only
//! `Ctx::authorize` makes one.
//!
//! RED: `Routes::write` takes one closure (compile error E0061); `Handler::Write` has no `authorize`
//! or `execute` (E0026); `write_open` is `todo!()`.
use axum::http::Method;
use cortex_module::route::Handler;
use cortex_module::{Authorized, Reply, Routes, TargetRule};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Touch {
    n: i64,
}

struct Which {
    n: i64,
}

fn two_phase() -> Routes {
    Routes::new()
        .write(
            Method::POST,
            "/v2/probe/{id}/touch",
            "probe:write",
            TargetRule::TokenTarget,
            |_tx, ctx, b: &Touch| {
                Box::pin(async move { ctx.authorize(&Default::default(), Which { n: b.n }) })
            },
            |_tx, _ctx, auth: Authorized<Which>, b: Touch| {
                Box::pin(async move { Ok(Reply::created(auth.value().n + b.n)) })
            },
        )
        .max_body(1024)
        .write_open(
            Method::POST,
            "/v2/probe/open",
            "probe:write",
            |_tx, _ctx, b: Touch| Box::pin(async move { Ok(Reply::ok(b.n)) }),
        )
}

#[test]
fn d4_r_1_the_registry_lists_both_routes_as_writes_with_their_declared_rules() {
    let r = two_phase();
    let v: Vec<_> = r.entries().collect();
    assert_eq!(v.len(), 2);
    assert_eq!(
        (
            v[0].path,
            v[0].write,
            v[0].target,
            v[0].max_body,
            v[0].query
        ),
        (
            "/v2/probe/{id}/touch",
            true,
            TargetRule::TokenTarget,
            1024,
            false
        )
    );
    assert_eq!((v[0].method, v[0].scope), (&Method::POST, "probe:write"));
    // `write_open` has no rule parameter: its rule is `None` by construction.
    assert_eq!(
        (v[1].path, v[1].write, v[1].target),
        ("/v2/probe/open", true, TargetRule::None)
    );
}

#[test]
fn d4_r_2_the_chain_gets_decode_authorize_and_execute_for_every_write() {
    let mut seen = 0;
    for e in two_phase().into_entries() {
        match e.handler {
            Handler::Write {
                decode,
                authorize,
                execute,
            } => {
                seen += 1;
                // The closures exist and decode is the strict one (400 syntax, 422 shape).
                let _ = (&authorize, &execute);
                assert!(decode(br#"{"n":1}"#).is_ok());
                assert_eq!(
                    decode(br#"{"n":1,"x":2}"#)
                        .err()
                        .map(|p| (p.status, p.code)),
                    Some((422, "invalid_request"))
                );
                assert_eq!(decode(b"{").err().map(|p| p.status), Some(400));
            }
            Handler::Read(_) => panic!("both routes are writes"),
        }
    }
    assert_eq!(seen, 2);
}

#[test]
fn d4_r_3_a_write_open_route_still_takes_max_body_and_stays_query_free() {
    let r = Routes::new()
        .write_open(
            Method::POST,
            "/v2/probe/open",
            "probe:write",
            |_tx, _ctx, b: Touch| Box::pin(async move { Ok(Reply::ok(b.n)) }),
        )
        .max_body(10);
    let v: Vec<_> = r.entries().collect();
    assert_eq!((v[0].max_body, v[0].query), (10, false));
}
