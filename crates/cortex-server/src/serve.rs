//! `cortex-server serve` (contract r0.4 sections 1 and 2; piece 11, mira): the router over the assembled registry, the listener and the
//! graceful stop. The assembly is [`crate::assembly`], the open health routes [`crate::health_routes`]. The database side of readiness and the
//! migration-ledger check (W1-38 B, W1-41) come in the next increment.
//!
//! Every refusal is ONE line of fixed text that names a module at most: no path, no value, no secret.

use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use cortex_config::{Config, Env};
use cortex_db::DbPool;
use cortex_db::migrate::platform_migrations;
use cortex_http::chain::{Chain, Limits, LogSink, router_with_open};
use cortex_http::request_id::RequestIds;
use cortex_module::route::AppState;
use cortex_module::{Registry, Routes};
use cortex_types::{Clock, SystemClock};
use tokio::net::TcpListener;

use crate::assembly::{
    Wiring, assemble_wired, compiled, db_config, duplicate_enabled, startup_line,
};
use crate::fault::Fault;
use crate::health_routes::{HealthState, routes as routes_open};
use crate::ledger::DbLedger;
use crate::probe::{Ledger, ServerProbe, Startup, startup_check};

/// A refused startup: one line, `serve refused: ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The refusal when the serving loop itself fails (an accept error): fixed text. An accept error cannot be provoked in a DB-free
/// test, so the wording is pinned on its own (`rt7`); the path that reaches it is `run`'s `map_err` and is covered by review only.
pub fn stopped_with_error() -> Refused {
    refused("the server stopped with an error")
}

pub(crate) fn refused(what: &str) -> Refused {
    Refused(format!("serve refused: {what}"))
}

/// The whole router: every enabled piece's routes behind the chain (its verifier and policy from the registry's ports), the three open
/// health routes beside them. A disabled piece has no routes, so its paths are 404. It takes the routes out of the registry, so the
/// registry cannot be shared afterwards; [`build_router_from`] is for the caller that already took them.
pub fn build_router(
    registry: &mut Registry,
    health: Arc<HealthState>,
    state: AppState,
) -> Result<Router, Refused> {
    let routes = registry.take_routes();
    build_router_from(registry, routes, health, state)
}

/// [`build_router`] with the routes already taken out of the registry (`Registry::take_routes`), so that the registry can then be
/// shared (the readiness probe asks it for each piece's health).
pub fn build_router_from(
    registry: &Registry,
    routes: Vec<(&'static str, Routes)>,
    health: Arc<HealthState>,
    state: AppState,
) -> Result<Router, Refused> {
    let ports = registry.ports();
    let verifier = ports
        .token_verifier()
        .ok_or_else(|| refused("no piece provides the token verifier"))?;
    let policy = ports
        .scope_policy()
        .ok_or_else(|| refused("no piece provides the scope policy"))?;
    let chain = Chain {
        verifier,
        policy,
        log: LogSink::stderr(),
        limits: Limits::default(),
    };
    router_with_open(
        chain,
        state,
        RequestIds {
            clock: Arc::new(SystemClock),
        },
        routes,
        routes_open(health),
    )
    .map_err(|_| refused("the router could not be built"))
}

/// Bind the configured address.
pub async fn bind(addr: SocketAddr) -> Result<TcpListener, Refused> {
    // The address is operator input and is not echoed.
    TcpListener::bind(addr)
        .await
        .map_err(|_| refused("could not listen on the configured address"))
}

/// Serve until `shutdown` completes, then stop accepting, let the requests in flight finish, and return.
pub async fn run(
    listener: TcpListener,
    router: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Refused> {
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|_| stopped_with_error())
}

/// Completes on SIGTERM or SIGINT. The handlers are installed when this is CALLED, not when it is first polled.
pub fn shutdown_signal() -> std::io::Result<impl Future<Output = ()> + Send + 'static> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    })
}

/// The startup warnings about `CORTEX_TEST_FAULT`, never echoing the value.
pub fn startup_warnings(var: Option<&str>) -> Vec<&'static str> {
    crate::fault::warning(var).into_iter().collect()
}

/// What `serve_with` needs besides the config and the registry.
pub struct ServeOptions {
    /// The test-only fault of this build (`Fault::None` in a release build).
    pub fault: Fault,
    /// SemVer machine version, as `--version` prints it.
    pub version: String,
    /// Where nix's `LAST_GOOD` file is mounted, if anywhere (OPEN for hue and piece 12).
    pub last_good: Option<PathBuf>,
    pub clock: Arc<dyn Clock>,
    /// The checks made on the assembled registry BEFORE anything else (before the pool): the real command runs [`production_checks`];
    /// fixture pieces, whose scopes are not in the real policy table, run [`no_checks`].
    pub checks: fn(&Registry) -> Result<(), Refused>,
    /// The pieces that need the pool once it exists (the real command: `mod-tokens`' verifier); [`Wiring::none`] for fixture pieces.
    pub wiring: Wiring,
}

/// The checks of a fixture run: none (see [`ServeOptions::checks`]).
pub fn no_checks(_registry: &Registry) -> Result<(), Refused> {
    Ok(())
}

/// Everything after the registry is assembled: the pool, the migration-ledger check, the router, the listener and the stop.
///
/// The order, and what each step refuses (every refusal is one `serve refused: ...` line):
///
/// 0. the options' checks on the assembled registry (the real command: policy's startup check);
/// 1. the pool configuration (`[database]`, the app password file);
/// 2. the pool (the database must answer);
/// 3. the ledger check: a changed checksum, a malformed set or a registry without `platform` STOP here, before anything listens; an
///    unapplied step or a database ahead of this build starts the server NOT READY (one `line`);
/// 4. the router;
/// 5. the listener (`bound` is told the address);
/// 6. serve until `shutdown`.
pub async fn serve_with(
    cfg: &Config,
    mut registry: Registry,
    compiled: Vec<&'static str>,
    opts: ServeOptions,
    shutdown: impl Future<Output = ()> + Send + 'static,
    line: impl Fn(&str) + Send,
    bound: impl FnOnce(SocketAddr) + Send,
) -> Result<(), Refused> {
    // The checks on the assembled registry come first: the real command's refuse a scope the policy does not know before anything else.
    (opts.checks)(&registry)?;
    let db = db_config(cfg, &registry, opts.clock.clone()).map_err(|e| refused(&e.to_string()))?;
    let pool = Arc::new(
        DbPool::connect(&db)
            .await
            .map_err(|_| refused("the database could not be reached"))?,
    );
    // Hand the pool to the pieces that need it before any request can arrive (the verifier fails closed until it is bound).
    opts.wiring.bind_pool(&pool);
    // `platform` first, then the enabled pieces in migration order: the order `Registry::db_modules` gives the pool.
    let mut sets = vec![platform_migrations()];
    sets.extend(registry.pieces().map(|p| p.migrations()));
    let ledger = Arc::new(DbLedger::new(pool.clone(), sets));
    if let Startup::NotReady(reason) = startup_check(&ledger.read().await)? {
        line(&format!("serve: starting NOT READY: {reason}"));
    }
    // The routes come out first; then the registry is shared with the probe, which asks each piece for its health.
    let routes = registry.take_routes();
    let registry = Arc::new(registry);
    let probe = Arc::new(ServerProbe::new(
        ledger,
        registry.clone(),
        compiled,
        opts.clock.clone(),
    ));
    let health = Arc::new(HealthState {
        version: opts.version,
        probe,
        last_good: opts.last_good,
        fault: opts.fault,
        clock: opts.clock,
    });
    let router = build_router_from(&registry, routes, health, AppState { pool })?;
    let listener = bind(cfg.server.listen).await?;
    bound(
        listener
            .local_addr()
            .map_err(|_| refused("could not read the listening address"))?,
    );
    run(listener, router, shutdown).await
}

/// Where the server container's dedicated read-only STATUS directory is mounted (vera ruled the dump mount, option C; hue builds it): the
/// file nix's piece 12 writes. A missing file is `backup: none`; a symlink or a non-regular file is unusable, as in `health.rs`.
pub const LAST_GOOD_PATH: &str = "/var/lib/cortex-dumps/LAST_GOOD";

/// The options the real command runs with: the build's version, the fault `CORTEX_TEST_FAULT` names (if this build has it), the system
/// clock, and the LAST_GOOD file in the dump status directory.
pub fn default_options(test_fault: Option<&str>) -> ServeOptions {
    ServeOptions {
        fault: crate::fault::fault(test_fault),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        last_good: Some(PathBuf::from(LAST_GOOD_PATH)),
        clock: Arc::new(SystemClock),
        checks: production_checks,
        wiring: Wiring::none(),
    }
}

/// The checks the PRODUCTION path makes on an assembled registry before it serves (`serve_with` from tests, with fixture pieces,
/// does not): policy's startup check (S3 `x-scopes-rule`). One `serve refused:` line, scope names only. Without the `mod-policy` feature
/// the policy is not compiled in and there is nothing to check (that build cannot serve anyway: no piece provides the scope policy).
pub fn production_checks(registry: &Registry) -> Result<(), Refused> {
    #[cfg(feature = "mod-policy")]
    crate::assembly::check_policy(registry).map_err(|line| refused(&line))?;
    #[cfg(not(feature = "mod-policy"))]
    let _ = registry;
    Ok(())
}

/// `cortex-server serve --config PATH`: load the config, print the startup warnings, refuse a module enabled twice, assemble the compiled
/// pieces, install the signal handlers, and `serve_with`. `test_fault` is the value of `CORTEX_TEST_FAULT`, if set.
pub async fn command(config: &Path, test_fault: Option<&str>) -> Result<(), Refused> {
    let cfg = cortex_config::load(config, &Env::from_process()).map_err(|e| {
        refused(&format!(
            "configuration: {}",
            e.to_string().replace(['\n', '\r'], " ")
        ))
    })?;
    for warning in startup_warnings(test_fault) {
        eprintln!("serve: {warning}");
    }
    if let Some(twice) = duplicate_enabled(&cfg.modules.enabled) {
        return Err(refused(&format!(
            "module {} is enabled twice",
            twice.as_str()
        )));
    }
    let compiled_ids: Vec<&'static str> = compiled().iter().map(|p| p.manifest().id).collect();
    let (registry, wiring) = assemble_wired(&cfg).map_err(|errs| refused(&startup_line(&errs)))?;
    let shutdown =
        shutdown_signal().map_err(|_| refused("could not install the signal handlers"))?;
    serve_with(
        &cfg,
        registry,
        compiled_ids,
        ServeOptions {
            wiring,
            ..default_options(test_fault)
        },
        shutdown,
        |l| eprintln!("{l}"),
        |_| {},
    )
    .await
}

/// The process entry for `serve`: a runtime, `command`, the refusal on stderr, an exit code (0 on a clean stop).
pub fn run_command(config: &Path) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => {
            eprintln!("{}", refused("could not start the runtime"));
            return crate::cli::EXIT_FAILURE;
        }
    };
    let fault = std::env::var("CORTEX_TEST_FAULT").ok();
    match runtime.block_on(command(config, fault.as_deref())) {
        Ok(()) => crate::cli::EXIT_OK,
        Err(refusal) => {
            eprintln!("{refusal}");
            crate::cli::EXIT_FAILURE
        }
    }
}
