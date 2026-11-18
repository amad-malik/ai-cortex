//! The host bootstrap decision (contract r0.3, row `bootstrap`; ruling W1-REISSUE / W1-3 / W1-18).
//!
//! The first run creates the project and its lead and mints the lead's first token; a re-run for the same project and lead
//! re-issues the lead's token; a different lead for an existing project is a conflict. With `--require-existing` the command
//! is a RE-RUN ONLY: it creates NOTHING. It refuses with exit [`crate::cli::EXIT_REFUSED`] and ONE fixed stderr line when the
//! project does not exist, or the lead does not exist, or the lead is not the project's lead; no token file, no row.
//!
//! The store is a trait so the decision is testable without a database; the real implementation (sophi's
//! `bootstrap_project` and finn's host transaction and token mint) plugs in later.

use async_trait::async_trait;
use cortex_types::Secret;

use crate::cli::{BootstrapArgs, EXIT_FAILURE, EXIT_OK, EXIT_REFUSED};
use crate::tokenfile;

/// The ONE stderr line of a `--require-existing` refusal: fixed text, no name and no id in it.
pub const REFUSED_LINE: &str = "bootstrap refused: --require-existing, and the project, the lead, or their pairing does not exist";

/// What the store knows about a project and a lead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// Neither the project nor the lead exists.
    Nothing,
    /// The project exists, the lead does not.
    ProjectOnly,
    /// The lead exists (as a worker of this project), but the project's lead is somebody else.
    LeadNotProjectLead,
    /// The project exists and this worker is its lead.
    ProjectAndLead,
}

#[derive(Debug, PartialEq, Eq)]
pub struct StoreError(pub String);

/// The database side of the bootstrap. Every method runs in the host transaction the caller opened.
#[async_trait]
pub trait BootstrapStore: Send {
    /// Read only: what exists for this project and lead.
    async fn find(&mut self, project: &str, lead: &str) -> Result<Found, StoreError>;
    /// Create the project and the lead (the first run) or find them (a re-run); mint the lead's token; return it.
    /// A different lead for an existing project is `Err(StoreError("conflict"))`.
    async fn create_or_reissue(&mut self, project: &str, lead: &str) -> Result<Secret, StoreError>;
    /// Commit the host transaction (project, lead, token and the audit rows of W1-18 together). Called ONLY after the token file
    /// was written.
    async fn commit(&mut self) -> Result<(), StoreError>;
    /// Roll the host transaction back (any failure before the commit).
    async fn rollback(&mut self);
}

/// What the process must do: write `stderr` (if any) and exit with `code`; the token file is written by [`run`] itself.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub code: i32,
    pub stderr: Option<String>,
}

/// Decide and act. Nothing is printed here; the token is never returned to the caller, only written to the token file.
///
/// ORDER (each step is proved by `tests/bootstrap.rs`): read what exists; with `--require-existing` refuse unless the project and
/// its lead exist, BEFORE anything is created; check the token file is ABSENT before minting (minting re-issues the lead's token and
/// supersedes the old one, so it must never happen when the new one cannot be delivered); mint; write the file; commit. Any failure
/// before the commit rolls back; a failed commit is terminal (no rollback, no retry) and removes the file, so no dead token is left behind. Messages are constant text:
/// no project name, no lead name and no token ever appears in an [`Outcome`].
pub async fn run<S: BootstrapStore>(store: &mut S, args: &BootstrapArgs) -> Outcome {
    let found = match store.find(&args.project, &args.lead).await {
        Ok(f) => f,
        Err(_) => return fail(store, "bootstrap failed: could not read the project state").await,
    };
    if args.require_existing && found != Found::ProjectAndLead {
        store.rollback().await;
        return Outcome {
            code: EXIT_REFUSED,
            stderr: Some(REFUSED_LINE.to_owned()),
        };
    }
    // Only "not found" means the destination is free. An existing file or symlink (even a dangling one) is refused, and so is any other
    // error (a parent that is not a directory, no permission): the check could not be made, so nothing is minted (vera's Low).
    match std::fs::symlink_metadata(&args.token_file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => return fail(store, "bootstrap failed: the token file already exists").await,
        Err(_) => return fail(store, "bootstrap failed: could not check the token file").await,
    }
    let token = match store.create_or_reissue(&args.project, &args.lead).await {
        Ok(t) => t,
        Err(_) => {
            return fail(
                store,
                "bootstrap failed: could not create the project or re-issue the token",
            )
            .await;
        }
    };
    let written = tokenfile::write_fresh(&args.token_file, token.expose().as_bytes());
    if written.is_err() {
        return fail(store, "bootstrap failed: could not write the token file").await;
    }
    if store.commit().await.is_err() {
        // A failed commit is TERMINAL: its outcome is unknown, so no rollback is attempted and nothing is retried. The token file is
        // removed because its token may not exist; if that removal fails, the operator is told, because the file may then hold a token
        // that IS valid (the commit's acknowledgement can be lost after the token was committed). A re-run reissues the lead's token
        // and writes a fresh file.
        let line = match std::fs::remove_file(&args.token_file) {
            Ok(()) => {
                "bootstrap failed: could not commit (the outcome is unknown); run bootstrap again to reissue the token"
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                "bootstrap failed: could not commit (the outcome is unknown); run bootstrap again to reissue the token"
            }
            Err(_) => {
                "bootstrap failed: could not commit (the outcome is unknown), and the token file could not be removed; it may hold a valid token: remove it by hand, then run bootstrap again"
            }
        };
        return Outcome {
            code: EXIT_FAILURE,
            stderr: Some(line.to_owned()),
        };
    }
    Outcome {
        code: EXIT_OK,
        stderr: None,
    }
}

async fn fail<S: BootstrapStore>(store: &mut S, line: &str) -> Outcome {
    store.rollback().await;
    Outcome {
        code: EXIT_FAILURE,
        stderr: Some(line.to_owned()),
    }
}
