//! Wave 1 item B (mira; cox W1-35 B and kai d2a88cf7): the STRICT DECODERS through the REAL chain. DB-free.
//!
//! `cortex-module`'s `strict_from_slice`, `strict_body_from_slice` and `strict_from_query` are proved in their own crate against the
//! decoder alone (finn's `w1_s_*` and `w1_s0_strict2.rs`). What that cannot show is what a CLIENT sees: that the chain puts the decode
//! at the right step, maps its three refusals to the right status and Problem text, never echoes the request, and applies it to a
//! request type that FORGOT `#[serde(deny_unknown_fields)]`. These cells drive POST routes with plain request types (no
//! `deny_unknown_fields` anywhere in this file) and a GET route with a plain query type through `chain::router`.
//!
//! The pool never connects (port 9), as in `f_chain_statuses.rs`: a body that decodes goes on to the idempotency-key step and, with a
//! key, to `begin`, where the dead pool answers the chain's 503 `unavailable`. So "the body was accepted" is observable without a
//! database: 428 without a key, 503 with one. A refusal of the decoder is 400 or 422 BEFORE both.
//!
//! Statuses (S3 r2.7 R-3, finn's strict2): not well-formed JSON (whole input UTF-8, truncated, trailing characters, a lone `[`) is 400
//! `invalid_request` "The body is not valid JSON."; any other failure to build the type, an unknown field, a non-object body and an
//! EMPTY body (decoded as `null`) is 422 `invalid_request`; a query that does not match is 422. Where one of these cells and a cell of
//! finn's in `cortex-module` disagree, his decides and I change mine.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request};
use cortex_db::{DbConfig, DbEndpoint, DbPool, ModuleRegistry};
use cortex_http::chain::{Chain, Limits, LogSink, router};
use cortex_http::request_id::RequestIds;
use cortex_module::route::AppState;
use cortex_module::{
    AuthError, Authorized, Principal, Reply, Role, Routes, ScopePolicy, StrictQuery, Target,
    TargetRule, TokenVerifier, WorkerKind,
};
use cortex_types::{
    ModuleId, ProjectId, ScopeName, Secret, SystemClock, Timestamp, TokenId, WorkerId,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tower::ServiceExt;

const LENIENT: &str = "/v2/b/lenient";
const NESTED: &str = "/v2/b/nested";
const ENUM: &str = "/v2/b/enum";
const QUERY: &str = "/v2/b/query";
const PLAIN: &str = "/v2/b/plain";
const OPTIONAL: &str = "/v2/b/optional";

fn v7<T>(s: &str, parse: fn(&str) -> Result<T, cortex_types::IdError>) -> T {
    parse(s).expect("a UUIDv7")
}

struct Verifier;

#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
        if bearer.expose() != "ok" {
            return Err(AuthError::Unknown);
        }
        Ok(Principal {
            worker: v7("0193e6a0-0000-7000-8000-000000000301", WorkerId::parse),
            project: v7("0193e6a0-0000-7000-8000-00000000a0a0", ProjectId::parse),
            kind: WorkerKind::Worker,
            role: Role::Member(None),
            token: v7("0193e6a0-0000-7000-8000-0000000003aa", TokenId::parse),
            expires_at: Timestamp::from_unix_micros(4_102_444_799_000_000),
            renew_due: false,
        })
    }
}

struct Policy;

impl ScopePolicy for Policy {
    fn allows(&self, _p: &Principal, scope: &ScopeName) -> bool {
        matches!(scope.as_str(), "b:read" | "b:write")
    }
    fn target(&self, _p: &Principal, rule: TargetRule, _t: &Target) -> bool {
        matches!(rule, TargetRule::None)
    }
}

// ---- request types: PLAIN structs and a default enum. None of them has `deny_unknown_fields`: the decoder must refuse an unknown
// ---- field by itself (that is the whole point of the central helper).

#[derive(Deserialize)]
struct Lenient {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    n: Option<u32>,
}

#[derive(Deserialize)]
struct Inner {
    #[allow(dead_code)]
    x: u32,
}

#[derive(Deserialize)]
struct Nested {
    #[allow(dead_code)]
    inner: Inner,
}

#[derive(Deserialize)]
enum Kind {
    Ops,
    Dev,
}

#[derive(Deserialize)]
struct WithEnum {
    #[allow(dead_code)]
    kind: Kind,
}

/// Every field optional: `{}` is a good body for it, so an EMPTY body that were read as `{}` would be accepted (it must be `null`: 422).
#[derive(Deserialize)]
struct AllOptional {
    #[allow(dead_code)]
    a: Option<u32>,
}

#[derive(Deserialize)]
struct Lq {
    limit: Option<u32>,
}

fn write<B>(routes: Routes, path: &'static str) -> Routes
where
    B: serde::de::DeserializeOwned + Send + Sync + 'static,
{
    routes.write(
        Method::POST,
        path,
        "b:write",
        TargetRule::None,
        |_tx, ctx, _b: &B| Box::pin(async move { ctx.authorize(&Target::default(), ()) }),
        |_tx, _ctx, _a: Authorized<()>, _b: B| Box::pin(async move { Ok(Reply::ok(json!({}))) }),
    )
}

fn routes() -> Routes {
    let r = Routes::new().read(Method::GET, PLAIN, "b:read", TargetRule::None, || async {
        "ok"
    });
    let r = write::<Lenient>(r, LENIENT);
    let r = write::<Nested>(r, NESTED);
    let r = write::<WithEnum>(r, ENUM);
    let r = write::<AllOptional>(r, OPTIONAL);
    r.read(
        Method::GET,
        QUERY,
        "b:read",
        TargetRule::None,
        |StrictQuery(q): StrictQuery<Lq>| async move { q.limit.unwrap_or(0).to_string() },
    )
    .query()
}

fn dead_state() -> AppState {
    let cfg = DbConfig {
        endpoint: DbEndpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 9,
        },
        database: "none".into(),
        app_user: "none".into(),
        app_password: Secret::new("none"),
        max_connections: 1,
        statement_cache: false,
        registry: ModuleRegistry::new([ModuleId::parse("b_probe").expect("a module id")]),
        clock: Arc::new(SystemClock),
    };
    AppState {
        pool: Arc::new(DbPool::connect_lazy(&cfg)),
    }
}

/// The router, and the lines its log sink was given.
fn app() -> (Router, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    let chain = Chain {
        verifier: Arc::new(Verifier),
        policy: Arc::new(Policy),
        log: LogSink::new(move |l| sink.lock().expect("log").push(l.to_owned())),
        limits: Limits {
            max_in_flight: 128,
            request_timeout: Duration::from_secs(5),
        },
    };
    let router = router(
        chain,
        dead_state(),
        RequestIds {
            clock: Arc::new(SystemClock),
        },
        vec![("b_probe", routes())],
    )
    .expect("the router builds");
    (router, lines)
}

/// What came back.
struct Got {
    status: u16,
    body: Vec<u8>,
}

impl Got {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    fn problem(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("a problem body: {}", self.text()))
    }
    fn code(&self) -> String {
        self.problem()["type"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
}

async fn send(app: &Router, method: Method, uri: &str, key: Option<&str>, body: &[u8]) -> Got {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", "Bearer ok");
    if let Some(k) = key {
        b = b.header("idempotency-key", k);
    }
    let resp = app
        .clone()
        .oneshot(b.body(Body::from(body.to_vec())).expect("a request"))
        .await
        .expect("a response");
    let status = resp.status().as_u16();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("a body")
        .to_vec();
    Got { status, body }
}

/// A POST with a fresh idempotency key.
async fn post(app: &Router, path: &str, body: &[u8]) -> Got {
    send(app, Method::POST, path, Some("b-key-0001"), body).await
}

/// The body was ACCEPTED by the decoder: with a key it reached `begin`, where the dead pool answers 503 `unavailable`.
async fn assert_accepted(app: &Router, path: &str, body: &[u8], what: &str) {
    let got = post(app, path, body).await;
    assert_eq!(
        (got.status, got.code()),
        (503, "unavailable".to_owned()),
        "{what}: a good body goes on to `begin`: {}",
        got.text()
    );
}

async fn assert_refused(app: &Router, path: &str, body: &[u8], status: u16, what: &str) {
    let got = post(app, path, body).await;
    assert_eq!(
        (got.status, got.code()),
        (status, "invalid_request".to_owned()),
        "{what}: {}",
        got.text()
    );
}

// ---- (1) an unknown field is 422 on a request type that did not forbid it ------------------------------------------------------

#[tokio::test]
async fn b1_an_unknown_field_is_422_although_the_type_never_forbade_it() {
    let (app, _) = app();
    assert_accepted(&app, LENIENT, br#"{"name":"a"}"#, "control").await;
    assert_accepted(
        &app,
        LENIENT,
        br#"{"name":"a","n":3}"#,
        "control with the optional field",
    )
    .await;
    for (body, what) in [
        (
            &br#"{"name":"a","bogus":1}"#[..],
            "an unknown field, a number",
        ),
        (&br#"{"bogus":1,"name":"a"}"#[..], "the unknown field first"),
        (
            &br#"{"name":"a","bogus":{"deep":[1,2,{"x":null}]}}"#[..],
            "an unknown field holding a structure",
        ),
        (
            &br#"{"name":"a","NAME":"b"}"#[..],
            "a field that differs only in case",
        ),
    ] {
        assert_refused(&app, LENIENT, body, 422, what).await;
    }
    // The unknown field's VALUE is still read as JSON (rule 1: syntax first): a malformed value is 400, not 422.
    assert_refused(
        &app,
        LENIENT,
        br#"{"name":"a","bogus":[1,}"#,
        400,
        "an unknown field with a malformed value",
    )
    .await;
}

// ---- (2) an unknown field INSIDE a nested object -------------------------------------------------------------------------------

#[tokio::test]
async fn b2_an_unknown_field_inside_a_nested_object_is_422() {
    let (app, _) = app();
    assert_accepted(&app, NESTED, br#"{"inner":{"x":1}}"#, "control").await;
    assert_refused(
        &app,
        NESTED,
        br#"{"inner":{"x":1,"bogus":2}}"#,
        422,
        "unknown inside the nested object",
    )
    .await;
    assert_refused(
        &app,
        NESTED,
        br#"{"inner":{"bogus":2,"x":1}}"#,
        422,
        "unknown first inside",
    )
    .await;
    assert_refused(
        &app,
        NESTED,
        br#"{"inner":{"x":1},"bogus":2}"#,
        422,
        "unknown beside the nested object",
    )
    .await;
    assert_refused(
        &app,
        NESTED,
        br#"{"inner":{"x":"one"}}"#,
        422,
        "a wrong type inside the nested object",
    )
    .await;
    assert_refused(
        &app,
        NESTED,
        br#"{"inner":{}}"#,
        422,
        "a missing field inside",
    )
    .await;
    assert_refused(
        &app,
        NESTED,
        br#"{"inner":null}"#,
        422,
        "the nested object given null",
    )
    .await;
}

// ---- (3) syntax is 400, with the fixed text ------------------------------------------------------------------------------------

#[tokio::test]
async fn b3_a_body_that_is_not_json_is_400_with_the_fixed_text() {
    let (app, _) = app();
    let cases: Vec<(&[u8], &str)> = vec![
        (br#"{"name":"a""#, "truncated"),
        (br#"{"name":"a"} x"#, "trailing characters"),
        (br#"{"name":"a"}{"name":"b"}"#, "two values"),
        (b"hello", "plain text"),
        (br#"{'name':'a'}"#, "single quotes"),
        (br#"{"name":"a",}"#, "a trailing comma"),
        (b"[", "a lone open bracket"),
        (b"[1,", "a truncated array"),
        (b"{", "a lone open brace"),
        (b"   ", "white space only"),
        (b"{\"name\":\"\xff\"}", "invalid UTF-8 inside a string"),
        (
            b"{\"name\":\"a\",\"bogus\":\"\xc3\x28\"}",
            "invalid UTF-8 inside an UNKNOWN field's string",
        ),
    ];
    for (body, what) in cases {
        let got = post(&app, LENIENT, body).await;
        assert_eq!(
            (got.status, got.code()),
            (400, "invalid_request".to_owned()),
            "{what}: {}",
            got.text()
        );
        assert_eq!(
            got.problem()["detail"],
            "The body is not valid JSON.",
            "{what}"
        );
    }
}

// ---- (4) the empty body --------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn b4_an_empty_body_is_422_and_a_blank_one_is_400() {
    let (app, _) = app();
    // An empty body decodes as `null`, which is not an object: a shape error.
    assert_refused(
        &app,
        LENIENT,
        b"",
        422,
        "an empty body on a route that takes an object",
    )
    .await;
    assert_refused(&app, NESTED, b"", 422, "an empty body, another type").await;
    assert_refused(&app, LENIENT, b"  \n\t ", 400, "white space only").await;
    // A type whose every field is optional accepts `{}` and still refuses an EMPTY body: the empty body is `null`, not an empty object.
    assert_accepted(
        &app,
        OPTIONAL,
        b"{}",
        "an empty object on the all-optional type",
    )
    .await;
    assert_accepted(
        &app,
        OPTIONAL,
        br#"{"a":1}"#,
        "a field on the all-optional type",
    )
    .await;
    assert_refused(
        &app,
        OPTIONAL,
        b"",
        422,
        "an empty body on the all-optional type",
    )
    .await;
    assert_refused(
        &app,
        OPTIONAL,
        b"null",
        422,
        "null on the all-optional type",
    )
    .await;
    assert_refused(
        &app,
        OPTIONAL,
        br#"{"a":1,"bogus":2}"#,
        422,
        "an unknown field on the all-optional type",
    )
    .await;
}

// ---- (5) shape: wrong type, missing, repeated, and a body that is not an object -----------------------------------------------

#[tokio::test]
async fn b5_a_wrong_type_a_missing_or_repeated_field_and_a_non_object_body_are_422() {
    let (app, _) = app();
    let cases: Vec<(&[u8], &str)> = vec![
        (br#"{"name":7}"#, "a number for a string"),
        (br#"{"name":null}"#, "null for a required string"),
        (br#"{"name":["a"]}"#, "an array for a string"),
        (br#"{"name":"a","n":"3"}"#, "a string for a number"),
        (br#"{"name":"a","n":-1}"#, "a negative number for a u32"),
        (br#"{"name":"a","n":1.5}"#, "a fraction for a u32"),
        (br#"{"name":"a","n":99999999999}"#, "a number past u32"),
        (br#"{}"#, "a missing required field"),
        (br#"{"n":1}"#, "a missing required field, another present"),
        (br#"{"name":"a","name":"b"}"#, "a repeated field"),
        (br#"{"name":"a","n":1,"n":2}"#, "a repeated optional field"),
        // A request body is an OBJECT (S3; strict2 rule 3). These are valid JSON, so 422 and not 400.
        (b"[]", "an empty array"),
        (
            br#"["Ops"]"#,
            "the sequence form of the struct, one element",
        ),
        (
            br#"["a",3]"#,
            "the sequence form of the struct, both fields",
        ),
        (br#""text""#, "a string"),
        (b"7", "a number"),
        (b"null", "null"),
        (b"true", "a boolean"),
        (b"  [1] ", "an array behind white space"),
    ];
    for (body, what) in cases {
        assert_refused(&app, LENIENT, body, 422, what).await;
    }
    // And the same valid-JSON-but-not-an-object bodies are 422 on the other types too.
    for body in [&b"[]"[..], &br#"["Ops"]"#[..], &b"7"[..]] {
        assert_refused(
            &app,
            NESTED,
            body,
            422,
            "a non-object body on the nested type",
        )
        .await;
        assert_refused(&app, ENUM, body, 422, "a non-object body on the enum type").await;
    }
}

// ---- (5b) an enum field ----------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn b5b_an_enum_field_given_anything_but_one_of_its_names_is_422_never_400() {
    let (app, _) = app();
    assert_accepted(&app, ENUM, br#"{"kind":"Ops"}"#, "control Ops").await;
    assert_accepted(&app, ENUM, br#"{"kind":"Dev"}"#, "control Dev").await;
    for (body, what) in [
        (&br#"{"kind":null}"#[..], "null"),
        (&br#"{"kind":7}"#[..], "a number"),
        (&br#"{"kind":true}"#[..], "a boolean"),
        (&br#"{"kind":["x"]}"#[..], "an array"),
        (&br#"{"kind":{"k":1}}"#[..], "an object"),
        (&br#"{"kind":"Nope"}"#[..], "a name the enum does not have"),
        (&br#"{"kind":"ops"}"#[..], "the wrong case"),
        (&br#"{"kind":""}"#[..], "an empty string"),
    ] {
        assert_refused(&app, ENUM, body, 422, what).await;
    }
    assert_refused(
        &app,
        ENUM,
        br#"{"kind":"#,
        400,
        "an enum field cut short is syntax",
    )
    .await;
}

// ---- (6) the decode comes BEFORE the idempotency key ----------------------------------------------------------------------------

#[tokio::test]
async fn b6_the_body_is_decoded_before_the_idempotency_key_is_looked_at() {
    let (app, _) = app();
    // A GOOD body with no key reaches the key step: 428.
    let good = send(&app, Method::POST, LENIENT, None, br#"{"name":"a"}"#).await;
    assert_eq!(
        (good.status, good.code()),
        (428, "idempotency_key_required".to_owned()),
        "{}",
        good.text()
    );
    // A BAD body with no key, and with a bad key, is the decoder's refusal: the key was never looked at.
    let too_long = "k".repeat(129);
    for (key, what) in [
        (None, "no key"),
        (Some("x"), "a key that is too short"),
        (Some(too_long.as_str()), "a key that is too long"),
    ] {
        let shape = send(&app, Method::POST, LENIENT, key, br#"{"name":7}"#).await;
        assert_eq!(
            (shape.status, shape.code()),
            (422, "invalid_request".to_owned()),
            "shape, {what}: {}",
            shape.text()
        );
        let syntax = send(&app, Method::POST, LENIENT, key, b"{").await;
        assert_eq!(
            (syntax.status, syntax.code()),
            (400, "invalid_request".to_owned()),
            "syntax, {what}: {}",
            syntax.text()
        );
        let unknown = send(
            &app,
            Method::POST,
            LENIENT,
            key,
            br#"{"name":"a","bogus":1}"#,
        )
        .await;
        assert_eq!(
            (unknown.status, unknown.code()),
            (422, "invalid_request".to_owned()),
            "unknown, {what}: {}",
            unknown.text()
        );
    }
}

// ---- (7) nothing of the request is echoed: not in the response, not in the log ------------------------------------------------

#[tokio::test]
async fn b7_a_refusal_echoes_nothing_the_client_sent_in_the_response_or_the_log() {
    let (app, lines) = app();
    // Each body carries the canary where its refusal happens: a value, a field name, a nested name, an enum value, the malformed part.
    let bodies: Vec<(&str, Vec<u8>)> = vec![
        (
            LENIENT,
            br#"{"name":"canary-value-zzq","canary-unknown-zzq":1}"#.to_vec(),
        ),
        (LENIENT, br#"{"name":{"canary-shape-zzq":1}}"#.to_vec()),
        (
            LENIENT,
            br#"{"name":"canary-dup-zzq","name":"canary-dup2-zzq"}"#.to_vec(),
        ),
        (LENIENT, br#"{"name":"canary-syntax-zzq""#.to_vec()),
        (LENIENT, br#"["canary-seq-zzq"]"#.to_vec()),
        (LENIENT, b"canary-text-zzq".to_vec()),
        (
            NESTED,
            br#"{"inner":{"x":1,"canary-nested-zzq":2}}"#.to_vec(),
        ),
        (ENUM, br#"{"kind":"canary-enum-zzq"}"#.to_vec()),
        (ENUM, br#"{"kind":["canary-enum-seq-zzq"]}"#.to_vec()),
    ];
    for (path, body) in &bodies {
        let got = post(&app, path, body).await;
        assert!(
            got.status == 400 || got.status == 422,
            "{path} {}: {}",
            String::from_utf8_lossy(body),
            got.text()
        );
        assert!(
            !got.text().contains("zzq"),
            "the response echoes the request ({path}): {}",
            got.text()
        );
    }
    // The query as well.
    for q in [
        "limit=canary-limit-zzq",
        "limit=1&canary-param-zzq=2",
        "canary-only-zzq=1",
        "limit=1&limit=canary-dup-zzq",
    ] {
        let got = send(&app, Method::GET, &format!("{QUERY}?{q}"), None, b"").await;
        assert_eq!(got.status, 422, "{q}: {}", got.text());
        assert!(
            !got.text().contains("zzq"),
            "the response echoes the query ({q}): {}",
            got.text()
        );
    }
    let log = lines.lock().expect("log").join("\n");
    assert!(
        !log.is_empty(),
        "the log sink was never given a line, so this check would be vacuous"
    );
    assert!(!log.contains("zzq"), "the log echoes the request:\n{log}");
}

// ---- (8) the same, for the query -----------------------------------------------------------------------------------------------

#[tokio::test]
async fn b8_a_query_type_that_never_forbade_unknown_parameters_still_refuses_them() {
    let (app, _) = app();
    let ok = |q: &str| format!("{QUERY}{q}");
    let g = send(&app, Method::GET, &ok("?limit=7"), None, b"").await;
    assert_eq!((g.status, g.text()), (200, "7".to_owned()), "control");
    let g = send(&app, Method::GET, &ok(""), None, b"").await;
    assert_eq!(
        (g.status, g.text()),
        (200, "0".to_owned()),
        "no query at all"
    );
    for (q, what) in [
        ("?limit=1&bogus=2", "an unknown parameter after a good one"),
        ("?bogus=2&limit=1", "an unknown parameter first"),
        ("?bogus=2", "only an unknown parameter"),
        ("?limit=abc", "a value of the wrong type"),
        ("?limit=-1", "a negative number for a u32"),
        ("?limit=1.5", "a fraction for a u32"),
        ("?limit=1&limit=2", "a repeated parameter"),
        ("?Limit=1", "a parameter that differs only in case"),
    ] {
        let g = send(&app, Method::GET, &ok(q), None, b"").await;
        assert_eq!(
            (g.status, g.code()),
            (422, "invalid_request".to_owned()),
            "{what}: {}",
            g.text()
        );
        assert_eq!(
            g.problem()["detail"],
            "The query does not match this operation's parameters.",
            "{what}"
        );
    }
    // A route NOT marked as taking a query refuses any query (the chain's own rule, beside the decoder's).
    let g = send(&app, Method::GET, &format!("{PLAIN}?x=1"), None, b"").await;
    assert_eq!(
        (g.status, g.code()),
        (422, "invalid_request".to_owned()),
        "{}",
        g.text()
    );
}
