//! The route table (S0 section 1.4): how a piece declares its routes, and what the chain hands its
//! handlers. Written by mira for finn's lane A (rows 851b9cdd, 1874971a); `cortex-http`
//! owns the chain that runs these routes (`mvp/socket/cortex-http-signatures.md` r2).
//!
//! - A **read** route is a free axum handler. It extracts [`Ctx`] (put in the request by the
//!   chain) and opens its own transaction with [`Ctx::begin`].
//! - A **write** route is TWO erased closures (delta 4; cox c025db40, kai 68cecbdb): `authorize`
//!   (step-0 422, the read: 404, then [`Ctx::authorize`]: 403) and `execute` (the 409 checks and the
//!   write). The CHAIN runs R-16/R-19's order: decode the body strictly, take the idempotency key,
//!   open the transaction, `authorize`, look up the key (the lookup takes the erased proof, so it
//!   cannot run before `authorize`), `execute`, store the reply with the effect, commit. `execute`
//!   cannot run without an [`Authorized`], and only [`Ctx::authorize`] makes one. A piece cannot
//!   forget or mis-bind the key, nor skip the target check.
//!
//! The write closures' bound is `for<'a> Fn(&'a mut ModuleTx, &'a Ctx, …) -> BoxFuture<'a, …>`
//! (the sqlx `transaction` pattern; the piece writes `|tx, ctx, body| Box::pin(async move { … })`).
//! r4's `AsyncFn` bound cannot be erased: stable Rust cannot prove its future `Send` (a probe on
//! 1.99.1: "future cannot be sent between threads safely", `CallRefFuture` is not `Send`).
use std::any::Any;
use std::borrow::Cow;
use std::sync::Arc;

use axum::extract::{FromRequestParts, RawPathParams};
use axum::http::request::Parts;
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter, on};
use cortex_db::{Actor, DbError, DbPool, ModuleTx, TxContext, TxOptions};
use cortex_types::{Id, ModuleId, ProblemType, ProjectId, RequestId, TokenId, WorkerId};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::piece::BoxFuture;
use crate::ports::{PortError, Principal, ScopePolicy, Target, TargetRule, WorkerKind};
use crate::strict::{StrictError, strict_body_from_slice, strict_from_query, strict_from_slice};

/// A route's default whole-body cap (C3): 1 MiB.
pub const DEFAULT_MAX_BODY: u64 = 1024 * 1024;

/// The state every route runs with. Only `cortex-http` builds it.
#[derive(Clone)]
pub struct AppState {
    pub pool: Arc<DbPool>,
}

/// What the chain hands a handler: who is calling, in which project, for which request. The
/// project is the TOKEN's (`Principal::project`), never a header, path or body field.
///
/// **One source of identity (delta 4b; vera's residual):** the principal, project, request id and,
/// once the chain has called [`Ctx::with_route`], the route's declared rule and the policy all live in
/// ONE immutable `Authority` behind an `Arc`. There is no public field: [`Ctx::principal`],
/// [`Ctx::project`], [`Ctx::request_id`], [`Ctx::actor`], [`Ctx::begin`] and the proofs
/// ([`Authorized`], [`Authorization`]) all read that one `Arc`, so a clone cannot diverge from what a
/// proof was made for. `Ctx::new` and `with_route` stay `pub` because the chain is another crate; the
/// bound on who calls them is an xtask fitness rule (only `cortex-http`'s product source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ctx {
    authority: Arc<Authority>,
    /// The matched path parameters (socket delta 2; S3 R-5): read through `path_id` and
    /// `path_param`, which answer 404 for an absent or malformed value.
    path: Vec<(String, String)>,
}

/// The chain's AUTHORITY for one request (delta 4 and 4b): the authenticated principal, the project
/// and request id, and (after [`Ctx::with_route`]) the route's declared rule and the `ScopePolicy`.
/// Immutable. A proof ([`Authorized`], [`Authorization`]) carries the `Arc`, and [`seal`] and [`open`]
/// require the very same one (`Arc::ptr_eq`) as the `Ctx` the chain handed to the handler. A
/// replacement made by `with_route` or `Ctx::new` is another `Arc`, so a proof minted from it is
/// refused whatever it holds; a plain clone of the `Ctx` (and `with_path`) keeps the same one.
struct Authority {
    principal: Principal,
    project: ProjectId,
    request_id: RequestId,
    route: Option<RouteDecl>,
}

struct RouteDecl {
    rule: TargetRule,
    policy: Arc<dyn ScopePolicy>,
}

impl PartialEq for RouteDecl {
    fn eq(&self, other: &Self) -> bool {
        self.rule == other.rule && Arc::ptr_eq(&self.policy, &other.policy)
    }
}

impl std::fmt::Debug for Authority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authority")
            .field("principal", &self.principal)
            .field("project", &self.project)
            .field("request_id", &self.request_id)
            .field("rule", &self.route.as_ref().map(|r| r.rule))
            .finish_non_exhaustive()
    }
}

impl PartialEq for Authority {
    fn eq(&self, other: &Self) -> bool {
        self.principal == other.principal
            && self.project == other.project
            && self.request_id == other.request_id
            && self.route == other.route
    }
}
impl Eq for Authority {}

impl Ctx {
    /// The chain's constructor: the project is copied from the principal, so the two cannot differ.
    /// The chain only: an xtask rule keeps it out of every other crate's product source.
    pub fn new(principal: Principal, request_id: RequestId) -> Ctx {
        let project = principal.project;
        Ctx {
            authority: Arc::new(Authority {
                principal,
                project,
                request_id,
                route: None,
            }),
            path: Vec::new(),
        }
    }

    /// The authenticated caller (the token's principal).
    pub fn principal(&self) -> &Principal {
        &self.authority.principal
    }

    /// The TOKEN's project.
    pub fn project(&self) -> ProjectId {
        self.authority.project
    }

    pub fn request_id(&self) -> RequestId {
        self.authority.request_id
    }

    /// The chain, once per request: the route's DECLARED rule and the `ScopePolicy` (delta 4). The
    /// chain only: an xtask rule keeps it out of every other crate's product source.
    ///
    /// **Call it once per request** and pass clones of that `Ctx` on: it makes the request's
    /// authority, and a proof is valid only for the `Ctx` family of the call that made it. A second
    /// `with_route` (or a `Ctx::new`) on the same request is another authority.
    pub fn with_route(self, rule: TargetRule, policy: Arc<dyn ScopePolicy>) -> Ctx {
        Ctx {
            authority: Arc::new(Authority {
                principal: self.authority.principal.clone(),
                project: self.authority.project,
                request_id: self.authority.request_id,
                route: Some(RouteDecl { rule, policy }),
            }),
            path: self.path,
        }
    }

    /// Delta 4: the ONLY maker of an [`Authorized`]. Pure and synchronous: it takes no lock and
    /// opens nothing. It asks the policy about the route's DECLARED rule (there is no rule
    /// argument, so a handler cannot prove itself against a weaker one) and the row the handler
    /// read. `false` is 403 `target_not_allowed`. A `Ctx` without a route (one the chain did not
    /// make with [`Ctx::with_route`]) is 500 `internal_error`: never allowed.
    pub fn authorize<T>(&self, target: &Target, value: T) -> Result<Authorized<T>, Problem> {
        let route = self
            .authority
            .route
            .as_ref()
            .ok_or_else(Problem::not_through_chain)?;
        if !route
            .policy
            .target(&self.authority.principal, route.rule, target)
        {
            return Err(Problem::target_not_allowed());
        }
        Ok(Authorized {
            value,
            rule: route.rule,
            authority: Arc::clone(&self.authority),
        })
    }

    /// The matched path parameters, for the chain's write path (from axum's `RawPathParams`).
    pub fn with_path(mut self, params: impl IntoIterator<Item = (String, String)>) -> Ctx {
        self.path = params.into_iter().collect();
        self
    }

    /// A path id (S3 R-5): absent or malformed is 404 `not_found`, the same answer as an id of
    /// another project, so the caller cannot tell the cases apart.
    pub fn path_id<T: Id>(&self, name: &str) -> Result<T, Problem> {
        self.path_param(name)
            .and_then(|v| T::parse(v).map_err(|_| Problem::not_found()))
    }

    /// A non-id path segment; absent is 404 `not_found`.
    pub fn path_param(&self, name: &str) -> Result<&str, Problem> {
        self.path
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .ok_or_else(Problem::not_found)
    }

    /// The actor every write is attributed to.
    pub fn actor(&self) -> Actor {
        let p = &self.authority.principal;
        match p.kind {
            WorkerKind::Worker => Actor::Worker(p.worker),
            WorkerKind::Service => Actor::Service(p.worker),
        }
    }

    /// A `ModuleTx` under the piece's own role, with this project and actor (S1).
    pub async fn begin(&self, pool: &DbPool, piece: &ModuleId) -> Result<ModuleTx, DbError> {
        let ctx = TxContext {
            module: piece.clone(),
            project: Some(self.authority.project),
            actor: self.actor(),
            request_id: self.authority.request_id,
        };
        ModuleTx::begin(pool, ctx, TxOptions::default()).await
    }
}

/// A read handler extracts its `Ctx`. A request without one never passed the chain: 500, fail
/// closed (it means a route was mounted around `cortex-http`).
impl<S: Send + Sync> FromRequestParts<S> for Ctx {
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Ctx, Problem> {
        let ctx = parts.extensions.get::<Ctx>().cloned().ok_or(Problem {
            code: "internal_error",
            status: 500,
            title: "Internal error",
            detail: Cow::Borrowed("The request did not pass the chain."),
            fix: "Retry later; if it repeats, report the request ID.",
            scope: None,
        })?;
        if !ctx.path.is_empty() {
            return Ok(ctx);
        }
        // Socket delta 2 (R-5): the matched path parameters. A rejection (a value that is not
        // UTF-8 once percent-decoded) leaves them empty, so path_id answers 404, never 400.
        Ok(
            match RawPathParams::from_request_parts(parts, state).await {
                Ok(raw) => ctx.with_path(raw.iter().map(|(k, v)| (k.to_owned(), v.to_owned()))),
                Err(_) => ctx,
            },
        )
    }
}

/// A route's query parameters, strictly (S3 R-4): an unknown parameter or a bad value is 422
/// `invalid_request`, never axum's 400. `T` derives `Deserialize` as a plain struct of scalar parameters; an
/// unknown parameter is refused even without `#[serde(deny_unknown_fields)]` (Wave 1: the strict helper).
/// A query type has no use for a buffering serde attribute, which `xtask fitness` forbids (cox W1-21). Only a
/// route marked with `Routes::query` may take a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrictQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for StrictQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Problem> {
        // Wave 1: through `serde_ignored`, so an unknown parameter is refused even when `T` forgot
        // `#[serde(deny_unknown_fields)]`.
        strict_from_query::<T>(parts.uri.query().unwrap_or_default())
            .map(StrictQuery)
            .map_err(|_| {
                Problem::invalid_request(
                    422,
                    "The query does not match this operation's parameters.",
                )
            })
    }
}

/// A successful reply's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Created,
    NoContent,
}

impl Status {
    pub fn code(self) -> u16 {
        match self {
            Status::Ok => 200,
            Status::Created => 201,
            Status::NoContent => 204,
        }
    }
}

/// A write handler's reply. `issued: Some(id)` means the body carries a secret (a token issue):
/// the stored replay keeps only `{"marker":"secret_not_stored","token_id":…}` (C2).
#[derive(Debug, Clone, PartialEq)]
pub struct Reply<R> {
    pub status: Status,
    pub body: R,
    pub issued: Option<TokenId>,
}

impl<R> Reply<R> {
    pub fn ok(body: R) -> Reply<R> {
        Reply {
            status: Status::Ok,
            body,
            issued: None,
        }
    }
    pub fn created(body: R) -> Reply<R> {
        Reply {
            status: Status::Created,
            body,
            issued: None,
        }
    }
}

/// A piece's problem code, registered once (C4; CONTRACTS r4). The registry mints `code` at
/// startup; a duplicate, or a clash with a platform code, stops startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProblemSpec {
    pub code: &'static str,
    pub status: u16,
    pub title: &'static str,
    /// The refusal's fix text; the chain puts it in `Problem.fix` (F1-24).
    pub fix: &'static str,
}

/// A refusal, without its request id: `cortex-http` renders it as problem+json with the id, in
/// the contract's field order. As a response it carries itself in the response's extensions and
/// no body yet, so the chain renders every refusal the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub code: &'static str,
    pub status: u16,
    pub title: &'static str,
    /// A fixed text, or one naming a field, a scope or an id. Never a request value or row data.
    pub detail: Cow<'static, str>,
    pub fix: &'static str,
    /// `install` or `project`, only on an admission 503.
    pub scope: Option<&'static str>,
}

impl Problem {
    /// A piece's own problem, from its registered spec.
    pub fn new(spec: &ProblemSpec, detail: impl Into<Cow<'static, str>>) -> Problem {
        Problem {
            code: spec.code,
            status: spec.status,
            title: spec.title,
            detail: detail.into(),
            fix: spec.fix,
            scope: None,
        }
    }

    /// 404 `not_found`: another project's object looks exactly like a missing one.
    pub fn not_found() -> Problem {
        Problem {
            code: ProblemType::NOT_FOUND.code(),
            status: 404,
            title: "Not found",
            detail: Cow::Borrowed("Nothing was found at this address in your project."),
            fix: "Check the address and the ID.",
            scope: None,
        }
    }

    /// 400 or 422 `invalid_request`, with a fixed detail.
    pub fn invalid_request(status: u16, detail: &'static str) -> Problem {
        Problem {
            code: ProblemType::INVALID_REQUEST.code(),
            status,
            title: "Invalid request",
            detail: Cow::Borrowed(detail),
            fix: "Correct the request; the detail says what is wrong.",
            scope: None,
        }
    }

    /// 403 `target_not_allowed`, for a handler's `ScopePolicy::target` refusal (C7).
    pub fn target_not_allowed() -> Problem {
        Problem {
            code: ProblemType::TARGET_NOT_ALLOWED.code(),
            status: 403,
            title: "Target not allowed",
            detail: Cow::Borrowed("Your role may not act on this target."),
            fix: "Ask your project lead, or act on a target your role covers.",
            scope: None,
        }
    }
}

/// The chain's four codes, minted by cortex-http (mira's signatures r2 section 3, Q3).
const INTERNAL_ERROR: &str = "internal_error";
const UNAVAILABLE: &str = "unavailable";
const BUSY: &str = "busy";
const CONFLICT: &str = "conflict";

impl Problem {
    fn chain(
        code: &'static str,
        status: u16,
        title: &'static str,
        detail: &'static str,
        fix: &'static str,
    ) -> Problem {
        Problem {
            code,
            status,
            title,
            detail: Cow::Borrowed(detail),
            fix,
            scope: None,
        }
    }
}

impl Problem {
    /// 500 `internal_error`: the request did not pass the chain (a `Ctx` without a route, or a proof
    /// that is not this request's). Fixed text, no request value.
    fn not_through_chain() -> Problem {
        Problem::chain(
            INTERNAL_ERROR,
            500,
            "Internal error",
            "The request did not pass the chain.",
            "Retry later; if it repeats, report the request ID.",
        )
    }
}

/// 422: the erased body is not the type `decode` made (a chain bug, refused closed).
fn body_mismatch() -> Problem {
    Problem::invalid_request(422, "The body does not match this operation's request.")
}

/// A database error, by kind only (mira's signatures r2 section 3): no message, no constraint
/// name. The same mapping as cortex-http's `from_db` (84ed1f6), so a read handler that opens its
/// own transaction refuses exactly as the chain does; one home for it is S0 (finn, 5 Nov).
impl From<DbError> for Problem {
    fn from(e: DbError) -> Problem {
        match e {
            // 429, not 409 (cox's ruling 45e33f74 Q3, after the Part A freeze): a lock held by
            // another request is "retry later"; the chain adds Retry-After.
            DbError::Busy => Problem::chain(
                BUSY,
                429,
                "Busy",
                "The request met a lock held by another request.",
                "Retry with the same Idempotency-Key.",
            ),
            DbError::StatementTimeout | DbError::SessionEnded | DbError::Unavailable => {
                Problem::chain(
                    UNAVAILABLE,
                    503,
                    "Unavailable",
                    "A service this request needs is unavailable or did not answer in time.",
                    "Retry later with the same Idempotency-Key.",
                )
            }
            DbError::Constraint { .. } => Problem::chain(
                CONFLICT,
                409,
                "Conflict",
                "The request conflicts with the current state.",
                "Read the current state, then retry.",
            ),
            DbError::PermissionDenied
            | DbError::DelegationFailed
            | DbError::LockOrder { .. }
            | DbError::UnknownModule
            | DbError::Isolation
            | DbError::Other(_) => Problem::chain(
                INTERNAL_ERROR,
                500,
                "Internal error",
                "The server could not complete the request.",
                "Retry later; if it repeats, report the request ID.",
            ),
        }
    }
}

/// A port's error (mira's mapping table): `NotFound` 404, `Invalid` 422, `Forbidden` 403
/// `target_not_allowed`, `Conflict` 409, `Unavailable` 503, `Db` as a database error. The port's
/// `&'static str` reason is not sent: it is a code for logs, not for the caller.
impl From<PortError> for Problem {
    fn from(e: PortError) -> Problem {
        match e {
            PortError::NotFound => Problem::not_found(),
            PortError::Invalid(_) => {
                Problem::invalid_request(422, "The request does not fit this operation's rules.")
            }
            PortError::Forbidden => Problem::target_not_allowed(),
            PortError::Conflict(_) => Problem::from(DbError::Constraint {
                name: String::new(),
            }),
            PortError::Unavailable => Problem::from(DbError::Unavailable),
            PortError::Db(d) => Problem::from(d),
        }
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut r = status.into_response();
        r.extensions_mut().insert(self);
        r
    }
}

/// Proof that THIS request's target check passed (delta 4). It has **no public constructor and
/// private fields**: only [`Ctx::authorize`] makes one. It carries the chain's authority for the
/// request (see `Authority`) and the value `authorize` decided on (ids, not state to be trusted);
/// `execute` re-reads whatever it relies on under its own locks. `Debug` shows no value.
pub struct Authorized<T> {
    value: T,
    rule: TargetRule,
    authority: Arc<Authority>,
}

impl<T> Authorized<T> {
    pub fn value(&self) -> &T {
        &self.value
    }
    pub fn into_value(self) -> T {
        self.value
    }
}

impl<T> std::fmt::Debug for Authorized<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authorized")
            .field("rule", &self.rule)
            .finish_non_exhaustive()
    }
}

/// An [`Authorized<T>`] with `T` erased: what the chain holds between `authorize` and `execute`.
/// The replay lookup takes it by reference, so a lookup cannot run before `authorize`: **no public
/// constructor, private fields**. `Send + Sync`, so `&Authorization` crosses an `.await`.
pub struct Authorization {
    inner: Box<dyn Any + Send + Sync>,
    rule: TargetRule,
    authority: Arc<Authority>,
}

impl Authorization {
    /// The rule it was checked against: the route's declared one.
    pub fn rule(&self) -> TargetRule {
        self.rule
    }
    /// The authenticated worker the chain made the authority for.
    pub fn worker(&self) -> WorkerId {
        self.authority.principal.worker
    }
    pub fn project(&self) -> ProjectId {
        self.authority.project
    }
    /// Whether it was made by THIS request's authority: the very `Arc` that [`Ctx::with_route`]
    /// made for the `Ctx` (or a clone of it). Equal ids are not enough.
    pub fn belongs_to(&self, ctx: &Ctx) -> bool {
        Arc::ptr_eq(&ctx.authority, &self.authority)
    }
}

impl std::fmt::Debug for Authorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authorization")
            .field("rule", &self.rule)
            .finish_non_exhaustive()
    }
}

/// Erase an `Authorized<T>`, refusing a proof that another authority made (a replacement route, a
/// forged `Ctx`, a proof stashed from another request: 500).
pub(crate) fn seal<T: Send + Sync + 'static>(
    ctx: &Ctx,
    a: Authorized<T>,
) -> Result<Authorization, Problem> {
    if !Arc::ptr_eq(&ctx.authority, &a.authority) {
        return Err(Problem::not_through_chain());
    }
    Ok(Authorization {
        inner: Box::new(a.value),
        rule: a.rule,
        authority: a.authority,
    })
}

/// The inverse of [`seal`]: refuses another authority's proof, and a proof made for another `T`.
pub(crate) fn open<T: Send + Sync + 'static>(
    ctx: &Ctx,
    a: Authorization,
) -> Result<Authorized<T>, Problem> {
    if !a.belongs_to(ctx) {
        return Err(Problem::not_through_chain());
    }
    let value = *a
        .inner
        .downcast::<T>()
        .map_err(|_| Problem::not_through_chain())?;
    Ok(Authorized {
        value,
        rule: a.rule,
        authority: a.authority,
    })
}

/// A decoded body, type-erased between `decode`, `authorize` and `execute`. `Sync`, because
/// `authorize` borrows it across an await.
pub type Decoded = Box<dyn Any + Send + Sync>;
/// Step 8: strict decoding of the raw body. An empty body decodes as JSON `null`.
pub type DecodeFn = Arc<dyn Fn(&[u8]) -> Result<Decoded, Problem> + Send + Sync>;
/// The first phase, in the chain's transaction: the piece's `authorize` (step-0 422, the read: 404,
/// `Ctx::authorize`: 403), its proof erased and checked to be this request's (delta 4).
pub type AuthorizeFn = Arc<
    dyn for<'a> Fn(
            &'a mut ModuleTx,
            &'a Ctx,
            &'a Decoded,
        ) -> BoxFuture<'a, Result<Authorization, Problem>>
        + Send
        + Sync,
>;
/// The second phase: the piece's `execute` (the 409 checks and the write), which consumes the proof;
/// its reply as JSON (finn's condition a).
pub type ExecuteFn = Arc<
    dyn for<'a> Fn(
            &'a mut ModuleTx,
            &'a Ctx,
            Authorization,
            Decoded,
        ) -> BoxFuture<'a, Result<Reply<serde_json::Value>, Problem>>
        + Send
        + Sync,
>;

/// Pin the closures to the higher-ranked signatures, so the compiler infers the borrowed lifetimes
/// from the bound rather than from the closure's body.
fn erase_authorize<F>(f: F) -> AuthorizeFn
where
    F: for<'a> Fn(
            &'a mut ModuleTx,
            &'a Ctx,
            &'a Decoded,
        ) -> BoxFuture<'a, Result<Authorization, Problem>>
        + Send
        + Sync
        + 'static,
{
    Arc::new(f)
}

fn erase_execute<F>(f: F) -> ExecuteFn
where
    F: for<'a> Fn(
            &'a mut ModuleTx,
            &'a Ctx,
            Authorization,
            Decoded,
        ) -> BoxFuture<'a, Result<Reply<serde_json::Value>, Problem>>
        + Send
        + Sync
        + 'static,
{
    Arc::new(f)
}

/// What a route is, for the chain.
pub enum Handler {
    /// Boxed: a `MethodRouter` is much larger than the write closures.
    Read(Box<MethodRouter<AppState>>),
    Write {
        decode: DecodeFn,
        authorize: AuthorizeFn,
        execute: ExecuteFn,
    },
}

/// One declared route.
pub struct Entry {
    pub method: Method,
    pub path: &'static str,
    pub scope: &'static str,
    pub target: TargetRule,
    pub max_body: u64,
    /// Whether the route takes query parameters (S3 R-4; socket delta 2). DEFAULT-DENY: false
    /// unless `Routes::query` follows the route, and the chain then refuses any query string as
    /// 422 `invalid_request`. A route with `true` reads it with `StrictQuery<T>`.
    pub query: bool,
    pub handler: Handler,
}

/// The read-only view the registry walks for `UnknownScope` and `RouteTwice` (finn's condition b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteInfo<'a> {
    pub method: &'a Method,
    pub path: &'static str,
    pub scope: &'static str,
    pub target: TargetRule,
    pub write: bool,
    pub max_body: u64,
    /// See `Entry::query`.
    pub query: bool,
}

/// A piece's routes. The only way to declare one; `cortex-http` mounts them behind the chain.
#[derive(Default)]
pub struct Routes {
    entries: Vec<Entry>,
}

impl Routes {
    pub fn new() -> Routes {
        Routes::default()
    }

    /// A read route: a free axum handler that extracts `Ctx` and opens its own transaction.
    pub fn read<H, T>(
        mut self,
        method: Method,
        path: &'static str,
        scope: &'static str,
        target: TargetRule,
        h: H,
    ) -> Routes
    where
        H: axum::handler::Handler<T, AppState>,
        T: 'static,
    {
        // A method axum cannot route is a declaration bug: fail at startup, never mount it as GET.
        let filter = MethodFilter::try_from(method.clone())
            .expect("a route's method is a standard HTTP method");
        self.entries.push(Entry {
            method,
            path,
            scope,
            target,
            max_body: DEFAULT_MAX_BODY,
            query: false,
            handler: Handler::Read(Box::new(on(filter, h))),
        });
        self
    }

    /// A write route in two phases (delta 4; S3 R-16 and R-19): the CHAIN owns the transaction (C1).
    ///
    /// - `authorize` is the first phase: step-0 422 (from the body), then the read of the target
    ///   (404), then [`Ctx::authorize`] (403). It reads only: **no advisory lock and no row lock**
    ///   (the chain asserts the lock ledger is empty afterwards). It returns the [`Authorized`] value.
    /// - `execute` is the second phase: the 409 checks and the write. It takes the [`Authorized`],
    ///   so it cannot run without one, and re-reads whatever it relies on under its own locks.
    ///
    /// The route's `target` rule is the one `Ctx::authorize` checks (the chain puts it in the `Ctx`).
    /// An unknown field in the body is 422 for every `B` that is a plain struct or a default (externally
    /// tagged) enum (Wave 1: the strict helper; `deny_unknown_fields` is no longer needed). It does NOT hold
    /// for a type with a buffering serde attribute (`flatten`, `untagged`, `tag = ..`, `content = ..`): the
    /// helper cannot see inside a buffered payload, so `xtask fitness` forbids those attributes in product
    /// source (cox W1-21; `strict.rs` has the reasons).
    #[allow(clippy::too_many_arguments)]
    pub fn write<A, E, B, T, R>(
        mut self,
        method: Method,
        path: &'static str,
        scope: &'static str,
        target: TargetRule,
        authorize: A,
        execute: E,
    ) -> Routes
    where
        A: for<'a> Fn(
                &'a mut ModuleTx,
                &'a Ctx,
                &'a B,
            ) -> BoxFuture<'a, Result<Authorized<T>, Problem>>
            + Send
            + Sync
            + 'static,
        E: for<'a> Fn(
                &'a mut ModuleTx,
                &'a Ctx,
                Authorized<T>,
                B,
            ) -> BoxFuture<'a, Result<Reply<R>, Problem>>
            + Send
            + Sync
            + 'static,
        B: DeserializeOwned + Send + Sync + 'static,
        T: Send + Sync + 'static,
        R: Serialize + Send + 'static,
    {
        let decode: DecodeFn = Arc::new(|raw: &[u8]| {
            // Wave 1: through `serde_ignored`, so an unknown field is 422 even when `B` forgot
            // `#[serde(deny_unknown_fields)]`; strict2 (W1-35 B): not JSON is 400, any other failure is 422, and a
            // non-empty body must be an object. An EMPTY body decodes as `null` (unchanged).
            let decoded = if raw.is_empty() {
                strict_from_slice::<B>(b"null")
            } else {
                strict_body_from_slice::<B>(raw)
            };
            match decoded {
                Ok(b) => Ok(Box::new(b) as Decoded),
                Err(StrictError::Syntax) => {
                    Err(Problem::invalid_request(400, "The body is not valid JSON."))
                }
                Err(_) => Err(body_mismatch()),
            }
        });
        let authorize = Arc::new(authorize);
        let authorize = erase_authorize(move |tx, ctx, decoded| {
            let authorize = Arc::clone(&authorize);
            Box::pin(async move {
                // `decode` above made this value; anything else is a chain bug, refused closed.
                let body = decoded.downcast_ref::<B>().ok_or_else(body_mismatch)?;
                let authorized = authorize(tx, ctx, body).await?;
                seal(ctx, authorized)
            })
        });
        let execute = Arc::new(execute);
        let execute = erase_execute(move |tx, ctx, authorization, decoded| {
            let execute = Arc::clone(&execute);
            Box::pin(async move {
                let authorized = open::<T>(ctx, authorization)?;
                let body = *decoded.downcast::<B>().map_err(|_| body_mismatch())?;
                let reply = execute(tx, ctx, authorized, body).await?;
                let value = serde_json::to_value(&reply.body).map_err(|_| Problem {
                    code: "internal_error",
                    status: 500,
                    title: "Internal error",
                    detail: Cow::Borrowed("The reply could not be encoded."),
                    fix: "Retry later; if it repeats, report the request ID.",
                    scope: None,
                })?;
                Ok(Reply {
                    status: reply.status,
                    body: value,
                    issued: reply.issued,
                })
            })
        });
        self.entries.push(Entry {
            method,
            path,
            scope,
            target,
            max_body: DEFAULT_MAX_BODY,
            query: false,
            handler: Handler::Write {
                decode,
                authorize,
                execute,
            },
        });
        self
    }

    /// A write route with rule `None` that reads nothing before the write (delta 4). There is no
    /// rule parameter, so it cannot declare another rule. Its `authorize` is
    /// `ctx.authorize(&Target::default(), ())`; the piece's closure is `(tx, ctx, body)`.
    pub fn write_open<E, B, R>(
        self,
        method: Method,
        path: &'static str,
        scope: &'static str,
        execute: E,
    ) -> Routes
    where
        E: for<'a> Fn(&'a mut ModuleTx, &'a Ctx, B) -> BoxFuture<'a, Result<Reply<R>, Problem>>
            + Send
            + Sync
            + 'static,
        B: DeserializeOwned + Send + Sync + 'static,
        R: Serialize + Send + 'static,
    {
        self.write(
            method,
            path,
            scope,
            TargetRule::None,
            |_tx, ctx, _b: &B| Box::pin(async move { ctx.authorize(&Target::default(), ()) }),
            move |tx, ctx, _proof: Authorized<()>, b: B| execute(tx, ctx, b),
        )
    }

    /// The whole-body cap of the route added last (C3); 1 MiB by default.
    pub fn max_body(mut self, bytes: u64) -> Routes {
        if let Some(e) = self.entries.last_mut() {
            e.max_body = bytes;
        }
        self
    }

    /// The route added last takes query parameters (S3 R-4), read with `StrictQuery<T>`. Every
    /// other route refuses a query string. A write route cannot take one: its handler gets no
    /// query, and the registry refuses it at startup (`QueryOnWrite`).
    pub fn query(mut self) -> Routes {
        if let Some(e) = self.entries.last_mut() {
            e.query = true;
        }
        self
    }

    /// The registry's read-only view (finn's condition b).
    pub fn entries(&self) -> impl Iterator<Item = RouteInfo<'_>> {
        self.entries.iter().map(|e| RouteInfo {
            method: &e.method,
            path: e.path,
            scope: e.scope,
            target: e.target,
            write: matches!(e.handler, Handler::Write { .. }),
            max_body: e.max_body,
            query: e.query,
        })
    }

    /// For `cortex-http` only: the routes to mount behind the chain.
    pub fn into_entries(self) -> Vec<Entry> {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Touch {
        n: i64,
    }

    /// How a piece writes its routes: a read is a free axum handler; a write returns a boxed
    /// future (the closure's types are inferred from `write`'s bound).
    fn probe_routes() -> Routes {
        Routes::new()
            .read(
                Method::GET,
                "/v2/probe/{id}",
                "probe:read",
                TargetRule::None,
                |ctx: Ctx| async move { ctx.request_id().to_string() },
            )
            .write(
                Method::POST,
                "/v2/probe/{id}/touch",
                "probe:write",
                TargetRule::None,
                |_tx, ctx, b: &Touch| {
                    Box::pin(async move { ctx.authorize(&Target::default(), b.n) })
                },
                |_tx, _ctx, a: Authorized<i64>, _b: Touch| {
                    Box::pin(async move { Ok(Reply::created(*a.value())) })
                },
            )
            .max_body(1024)
    }

    fn decode_of(r: Routes, path: &str) -> DecodeFn {
        r.into_entries()
            .into_iter()
            .find_map(|e| match e.handler {
                Handler::Write { decode, .. } if e.path == path => Some(decode),
                _ => None,
            })
            .expect("a write route")
    }

    #[test]
    fn the_registry_view_lists_every_route_with_its_kind_and_cap() {
        let r = probe_routes();
        let v: Vec<_> = r.entries().collect();
        assert_eq!(v.len(), 2);
        assert_eq!(
            (v[0].path, v[0].write, v[0].max_body),
            ("/v2/probe/{id}", false, DEFAULT_MAX_BODY)
        );
        assert_eq!(
            (v[1].path, v[1].write, v[1].max_body),
            ("/v2/probe/{id}/touch", true, 1024)
        );
        assert_eq!((v[1].method, v[1].scope), (&Method::POST, "probe:write"));
    }

    #[test]
    fn h17_strict_decoding_400_for_syntax_422_for_shape() {
        let d = decode_of(probe_routes(), "/v2/probe/{id}/touch");
        assert!(d(br#"{"n":1}"#).is_ok());
        assert_eq!(
            d(br#"{"n":1,"x":2}"#).err().map(|p| (p.status, p.code)),
            Some((422, "invalid_request")),
            "unknown field"
        );
        assert_eq!(
            d(br#"{"n":"1"}"#).err().map(|p| p.status),
            Some(422),
            "wrong type"
        );
        assert_eq!(
            d(b"").err().map(|p| p.status),
            Some(422),
            "empty is null, not a Touch"
        );
        assert_eq!(
            d(b"{").err().map(|p| (p.status, p.code)),
            Some((400, "invalid_request")),
            "syntax"
        );
    }

    #[test]
    fn a_problem_response_carries_itself_for_the_chain_to_render() {
        let r = Problem::not_found().into_response();
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert_eq!(r.extensions().get::<Problem>(), Some(&Problem::not_found()));
    }

    // ---- delta 4: the erased proof (DB-free; the closures that use these need a transaction) ----

    use crate::ports::{Role, ScopePolicy};
    use cortex_types::{ScopeName, SystemClock, TeamId, Timestamp};

    struct Allow;
    impl ScopePolicy for Allow {
        fn allows(&self, _: &Principal, _: &ScopeName) -> bool {
            true
        }
        fn target(&self, _: &Principal, _: TargetRule, _: &Target) -> bool {
            true
        }
    }

    fn ctx_for(p: &Principal, request: RequestId, rule: TargetRule) -> Ctx {
        Ctx::new(p.clone(), request).with_route(rule, Arc::new(Allow))
    }

    fn some_principal() -> Principal {
        let c = SystemClock;
        Principal {
            worker: WorkerId::new(&c),
            project: ProjectId::new(&c),
            kind: WorkerKind::Worker,
            role: Role::Member(Some(TeamId::new(&c))),
            token: TokenId::new(&c),
            expires_at: Timestamp::from_unix_micros(0),
            renew_due: false,
        }
    }

    #[test]
    fn d4_u1_a_proof_sealed_for_this_request_belongs_to_it() {
        let p = some_principal();
        let ctx = ctx_for(&p, RequestId::new(&SystemClock), TargetRule::TokenTarget);
        let a = ctx.authorize(&Target::default(), 5_u8).expect("allowed");
        let sealed = seal(&ctx, a).expect("this request's proof");
        assert!(sealed.belongs_to(&ctx));
        assert_eq!(
            (sealed.rule(), sealed.worker(), sealed.project()),
            (TargetRule::TokenTarget, p.worker, p.project)
        );
        let back = open::<u8>(&ctx, sealed).expect("the same T, the same request");
        assert_eq!(back.into_value(), 5);
    }

    #[test]
    fn d4_u2_a_proof_from_another_request_is_refused_with_500() {
        let p = some_principal();
        let first = ctx_for(&p, RequestId::new(&SystemClock), TargetRule::None);
        let second = ctx_for(&p, RequestId::new(&SystemClock), TargetRule::None);
        // Same worker, same project, another request id: a stashed proof.
        let stale = first.authorize(&Target::default(), ()).expect("allowed");
        let err = seal(&second, stale).expect_err("refused");
        assert_eq!((err.status, err.code), (500, "internal_error"));
        // Another worker in the same project, and another project: refused the same way.
        let sealed =
            seal(&first, first.authorize(&Target::default(), ()).expect("ok")).expect("ok");
        assert!(!sealed.belongs_to(&second), "another request");
        let mut other = some_principal();
        other.project = p.project;
        let other_worker = ctx_for(&other, first.request_id(), TargetRule::None);
        assert!(!sealed.belongs_to(&other_worker), "another worker");
        let mut elsewhere = p.clone();
        elsewhere.project = ProjectId::new(&SystemClock);
        let other_project = ctx_for(&elsewhere, first.request_id(), TargetRule::None);
        assert!(!sealed.belongs_to(&other_project), "another project");
        assert_eq!(
            open::<()>(&second, sealed).err().map(|e| e.status),
            Some(500),
            "unsealing under another request is refused"
        );
    }

    #[test]
    fn d4_u3_a_proof_made_for_one_value_type_does_not_open_as_another() {
        let p = some_principal();
        let ctx = ctx_for(&p, RequestId::new(&SystemClock), TargetRule::SelfWorker);
        let sealed = seal(&ctx, ctx.authorize(&Target::default(), 1_u32).expect("ok")).expect("ok");
        let err = open::<String>(&ctx, sealed).expect_err("wrong T");
        assert_eq!((err.status, err.code), (500, "internal_error"));
    }

    // ---- delta 4 rework, D4-V1 (vera): a proof is bound to the chain's ORIGINAL authority ----
    //
    // A handler holds `&Ctx`, can clone it, and `with_route` is public (the chain needs it). So the
    // rule, the policy and the authenticated principal that made a proof must be the very ones the
    // chain put in the `Ctx` it handed over: not merely the same three ids.

    use std::sync::Mutex;

    struct Deny;
    impl ScopePolicy for Deny {
        fn allows(&self, _: &Principal, _: &ScopeName) -> bool {
            false
        }
        fn target(&self, _: &Principal, _: TargetRule, _: &Target) -> bool {
            false
        }
    }

    /// Allows, and remembers the principal and rule it was asked about.
    #[derive(Default)]
    struct Asked(Mutex<Vec<(Principal, TargetRule)>>);
    impl ScopePolicy for Asked {
        fn allows(&self, _: &Principal, _: &ScopeName) -> bool {
            true
        }
        fn target(&self, p: &Principal, rule: TargetRule, _: &Target) -> bool {
            self.0.lock().expect("lock").push((p.clone(), rule));
            true
        }
    }

    fn denying_ctx(p: &Principal, rule: TargetRule) -> Ctx {
        Ctx::new(p.clone(), RequestId::new(&SystemClock)).with_route(rule, Arc::new(Deny))
    }

    #[test]
    fn d4_u5_a_replacement_rule_and_policy_mint_a_proof_that_is_refused() {
        // The chain's authority denies. A handler replaces BOTH rule and policy with a weaker pair.
        let p = some_principal();
        let ctx = denying_ctx(&p, TargetRule::TokenTarget);
        let weaker = ctx.clone().with_route(TargetRule::None, Arc::new(Allow));
        let forged = weaker
            .authorize(&Target::default(), ())
            .expect("the replacement itself allows");
        let err = seal(&ctx, forged).expect_err("not the chain's authority");
        assert_eq!((err.status, err.code), (500, "internal_error"));
        // The original authority still decides for the original ctx.
        assert_eq!(
            ctx.authorize(&Target::default(), ())
                .expect_err("denied")
                .status,
            403
        );
    }

    #[test]
    fn d4_u6_a_permissive_policy_under_the_same_rule_is_refused_too() {
        // Comparing the rule alone would let this through (vera): same rule, other policy.
        let p = some_principal();
        let ctx = denying_ctx(&p, TargetRule::RetireTarget);
        let same_rule = ctx
            .clone()
            .with_route(TargetRule::RetireTarget, Arc::new(Allow));
        let forged = same_rule
            .authorize(&Target::default(), 1_u8)
            .expect("the replacement allows");
        assert_eq!(seal(&ctx, forged).expect_err("other policy").status, 500);
        let sealed = seal(
            &same_rule,
            same_rule.authorize(&Target::default(), 1_u8).expect("ok"),
        )
        .expect("a proof belongs to the Ctx whose authority made it");
        assert!(
            !sealed.belongs_to(&ctx),
            "and it is not the original ctx's proof"
        );
        assert_eq!(open::<u8>(&ctx, sealed).expect_err("refused").status, 500);
    }

    #[test]
    fn d4_u7_the_policy_sees_the_authenticated_principal_and_a_forged_role_is_another_authority() {
        let p = some_principal();
        let asked = Arc::new(Asked::default());
        let ctx = Ctx::new(p.clone(), RequestId::new(&SystemClock))
            .with_route(TargetRule::SelfWorker, asked.clone());
        // (c1) a clone whose principal is altered: IMPOSSIBLE since delta 4b (no public field; see the
        // trybuild fixtures d4b_assign_*_fail). What a handler can still do is the forgery below.
        let a = ctx.authorize(&Target::default(), ()).expect("allowed");
        let seen = asked.0.lock().expect("lock").clone();
        assert_eq!(
            seen,
            [(p.clone(), TargetRule::SelfWorker)],
            "the policy is asked about the principal the chain authenticated"
        );
        // The proof is the original authority's, so it seals against the original ctx.
        assert!(seal(&ctx, a).is_ok());
        // (c2) a Ctx forged around the SAME policy, rule, request id and worker, with another role:
        let mut forged_principal = p.clone();
        forged_principal.role = Role::ProjectLead;
        let forged = Ctx::new(forged_principal, ctx.request_id())
            .with_route(TargetRule::SelfWorker, asked.clone());
        let minted = forged.authorize(&Target::default(), ()).expect("allowed");
        assert_eq!(
            seal(&ctx, minted).expect_err("another authority").status,
            500,
            "same ids, same rule, same policy, other principal: refused"
        );
    }

    #[test]
    fn d4_u9_a_ctx_rebuilt_with_the_same_ids_rule_and_policy_is_still_another_authority() {
        // Cox's check (e): `Ctx::new` with the same request id, worker and project (here the very
        // same principal), the same rule and the same policy instance. Nothing differs but the
        // origin, and a handler must not be able to make one. Only the chain's own Ctx is accepted.
        let p = some_principal();
        let policy: Arc<dyn ScopePolicy> = Arc::new(Allow);
        let ctx = Ctx::new(p.clone(), RequestId::new(&SystemClock))
            .with_route(TargetRule::TokenTarget, Arc::clone(&policy));
        let rebuilt = Ctx::new(p, ctx.request_id()).with_route(TargetRule::TokenTarget, policy);
        let minted = rebuilt
            .authorize(&Target::default(), ())
            .expect("the rebuilt ctx allows");
        assert_eq!(
            seal(&ctx, minted).expect_err("not the chain's Ctx").status,
            500
        );
    }

    #[test]
    fn d4_u8_a_plain_clone_of_the_ctx_keeps_the_authority() {
        // The chain clones the Ctx into request extensions and adds path parameters; a handler may
        // clone it too. None of that changes whose authority it is.
        let p = some_principal();
        let ctx = ctx_for(&p, RequestId::new(&SystemClock), TargetRule::Lineage);
        let cloned = ctx.clone();
        let with_path = ctx.clone().with_path([("id".to_owned(), "x".to_owned())]);
        for c in [&cloned, &with_path] {
            let a = c.authorize(&Target::default(), 3_u8).expect("allowed");
            let sealed = seal(&ctx, a).expect("the same authority");
            assert!(sealed.belongs_to(&ctx) && sealed.belongs_to(c));
            assert_eq!(open::<u8>(&ctx, sealed).expect("same T").into_value(), 3);
        }
    }

    // ---- delta 4b (vera's sharpened residual): ONE source of identity, no public fields ----

    fn service_principal() -> Principal {
        let mut p = some_principal();
        p.kind = WorkerKind::Service;
        p
    }

    #[test]
    fn d4b_u1_the_accessors_return_what_the_ctx_was_made_from_and_survive_clone_path_and_route() {
        let p = some_principal();
        let rid = RequestId::new(&SystemClock);
        let plain = Ctx::new(p.clone(), rid);
        let routed = plain
            .clone()
            .with_path([("id".to_owned(), "x".to_owned())])
            .with_route(TargetRule::Lineage, Arc::new(Allow));
        for c in [&plain, &plain.clone(), &routed, &routed.clone()] {
            assert_eq!(c.principal(), &p);
            assert_eq!(c.project(), p.project, "the TOKEN's project");
            assert_eq!(c.request_id(), rid);
        }
    }

    #[test]
    fn d4b_u2_actor_names_the_same_worker_the_accessors_and_the_proof_do() {
        for (p, expect_service) in [(some_principal(), false), (service_principal(), true)] {
            let ctx = ctx_for(&p, RequestId::new(&SystemClock), TargetRule::SelfWorker);
            let clone = ctx.clone();
            for c in [&ctx, &clone] {
                let actor = c.actor();
                let (is_service, worker) = match actor {
                    Actor::Worker(w) => (false, w),
                    Actor::Service(w) => (true, w),
                    other => panic!("a token's actor is a worker or a service: {other:?}"),
                };
                assert_eq!(is_service, expect_service);
                assert_eq!(worker, c.principal().worker);
            }
            let sealed =
                seal(&ctx, clone.authorize(&Target::default(), ()).expect("ok")).expect("same");
            assert_eq!(
                (sealed.worker(), sealed.project()),
                (ctx.principal().worker, ctx.project()),
                "the proof names the identity the Ctx reports"
            );
        }
    }

    #[test]
    fn d4_u4_neither_proof_prints_its_value() {
        let p = some_principal();
        let ctx = ctx_for(&p, RequestId::new(&SystemClock), TargetRule::None);
        let secret = "ctx2_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.padding-so-it-looks-like-a-token";
        let a = ctx
            .authorize(&Target::default(), secret.to_owned())
            .expect("ok");
        assert!(!format!("{a:?}").contains("ctx2_"));
        let sealed = seal(&ctx, a).expect("ok");
        assert!(!format!("{sealed:?}").contains("ctx2_"));
    }
}
