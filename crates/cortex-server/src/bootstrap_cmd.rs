//! `cortex-server bootstrap` (contract r0.5 sections 5 and 6; piece 11, cox): creates the project and its lead, or re-issues the lead's
//! token, through the REAL store ([`crate::store::DbStore`]), the REAL minter ([`crate::minter::TokenMinter`], mod-tokens) and the ASSEMBLED
//! registry's identity and audit ports (W1-37). The decision and the order of effects (find, check the token file is absent, mint, write the
//! file, commit; roll back on any failure) are [`crate::bootstrap::run`]; this module only builds its parts and hands them over.
//!
//! The pool is the `cortex_app` pool of [`crate::assembly::db_config`] (the app password file through the strict reader); `bootstrap` never
//! reads the owner password. Every refusal is ONE line of fixed text, `bootstrap refused: ...` or `bootstrap failed: ...`: no secret,
//! no path, no database text.

use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

use cortex_config::{Config, Env};
use cortex_db::DbPool;
use cortex_module::Registry;
use cortex_types::SystemClock;
use mod_tokens::OsEntropy;

use crate::assembly::{assemble, db_config, duplicate_enabled, startup_line};
use crate::bootstrap::Outcome;
use crate::bootstrap::run;
use crate::cli::{BootstrapArgs, EXIT_FAILURE};
use crate::minter::TokenMinter;
use crate::store::DbStore;

fn refused(what: &str) -> Outcome {
    Outcome {
        code: EXIT_FAILURE,
        stderr: Some(format!("bootstrap refused: {what}")),
    }
}

/// The operator's uid: the owner of `/proc/self` (no `unsafe`, the workspace forbids it). Recorded as the actor of the host transaction.
fn process_uid() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// Run the bootstrap over an ASSEMBLED registry (identity and audit providers must be present).
pub async fn bootstrap_with(cfg: &Config, registry: &Registry, args: &BootstrapArgs) -> Outcome {
    let (Some(identity), Some(audit)) = (registry.ports().identity(), registry.ports().audit())
    else {
        return refused("the identity and audit pieces are not both assembled");
    };
    let Some(uid) = process_uid() else {
        return refused("could not read the process uid");
    };
    let clock = Arc::new(SystemClock);
    let db = match db_config(cfg, registry, clock) {
        Ok(c) => c,
        Err(e) => return refused(&e.to_string()),
    };
    let pool = match DbPool::connect(&db).await {
        Ok(p) => p,
        Err(_) => {
            return Outcome {
                code: EXIT_FAILURE,
                stderr: Some("bootstrap failed: could not connect to the database".to_owned()),
            };
        }
    };
    let minter = TokenMinter::new(&*identity, &*audit, &SystemClock, &OsEntropy);
    let mut store = DbStore::new(&pool, &*identity, &*audit, &minter, &SystemClock, uid);
    run(&mut store, args).await
}

/// The command: the config, the assembly, then [`bootstrap_with`].
pub async fn command(args: &BootstrapArgs) -> Outcome {
    let cfg = match cortex_config::load(&args.config, &Env::from_process()) {
        Ok(c) => c,
        Err(e) => {
            return refused(&format!(
                "configuration: {}",
                e.to_string().replace(['\n', '\r'], " ")
            ));
        }
    };
    if let Some(twice) = duplicate_enabled(&cfg.modules.enabled) {
        return refused(&format!("module {} is enabled twice", twice.as_str()));
    }
    let registry = match assemble(&cfg) {
        Ok(r) => r,
        Err(errs) => return refused(&startup_line(&errs)),
    };
    bootstrap_with(&cfg, &registry, args).await
}

/// The process entry: a runtime, [`command`], the line on stderr, the exit code.
pub fn run_command(args: &BootstrapArgs) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => {
            eprintln!("bootstrap refused: could not start the runtime");
            return EXIT_FAILURE;
        }
    };
    let outcome = runtime.block_on(command(args));
    if let Some(line) = &outcome.stderr {
        eprintln!("{line}");
    }
    outcome.code
}
