//! `issue_as_host`: the library function piece 11's `cortex-server bootstrap` calls (cell TOK-32). It issues a
//! token for a worker on the HOST path: one `ModuleTx::begin_host` (role `cortex_install_admin`, actor
//! `HostOperator { uid }`), the same issue as the route, `issued_by = {"kind":"host"}`, one
//! `token.issued` (first run) or `token.reissued` (re-run) audit record, and the secret returned once to the caller. The file mechanics (a fresh 0600 file,
//! no symlink, never printed) are piece 11's.
//!
//! The project is the caller's to name: it is set on the host transaction so the identity reads run under that
//! project's row-level security, and the worker must be one of its workers (an unknown worker, or one of another
//! project, is an error and writes nothing).

use cortex_db::{DbError, DbPool, HostContext, ModuleTx, TxOptions};
use cortex_module::{AuditSink, IdentityApi};
use cortex_types::{Clock, Secret, TokenId, WorkerId};

use crate::entropy::Entropy;
use crate::issue::{BootstrapRun, IssueError, Issuing, issue_in_host_tx};

/// A token issued on the host path.
pub struct HostIssued {
    /// The whole token: shown to the caller once.
    pub token: Secret,
    pub token_id: TokenId,
    /// The new token's expiry, microseconds since the epoch.
    pub expires_at_micros: i64,
}

/// Why a host issue failed. Nothing was written.
#[derive(Debug)]
pub enum HostIssueError {
    /// The worker is unknown to the project.
    UnknownWorker,
    /// The worker is retired.
    Retired,
    /// The database or a port refused; the text is the error's own code text (never row data).
    Unavailable(String),
    /// The operating system could not supply randomness.
    NoRandomness,
}

/// Issues a token for `worker` of the project in `host.project` (which must be `Some`), in a host transaction of
/// its own, and COMMITS it before returning. `run` is the bootstrap intent (`token.issued` or `token.reissued`).
///
/// **Not for piece 11's bootstrap:** that must write the secret to its file BEFORE the commit and commit the project,
/// the lead, the token and the audit rows together, which needs the caller's own host transaction: it calls
/// [`issue_in_host_tx`] instead. This function is for a caller that wants nothing but a token.
#[allow(clippy::too_many_arguments)] // the ports, the pool, the host context, the worker and the intent: each is its own thing
pub async fn issue_as_host(
    pool: &DbPool,
    identity: &dyn IdentityApi,
    audit: &dyn AuditSink,
    clock: &dyn Clock,
    entropy: &dyn Entropy,
    host: HostContext,
    worker: WorkerId,
    run: BootstrapRun,
) -> Result<HostIssued, HostIssueError> {
    if host.project.is_none() {
        return Err(HostIssueError::UnknownWorker);
    }
    let mut tx = ModuleTx::begin_host(pool, host, TxOptions::default())
        .await
        .map_err(|e: DbError| HostIssueError::Unavailable(format!("begin_host: {e}")))?;
    let k = Issuing {
        identity,
        audit,
        clock,
        entropy,
    };
    let issued = match issue_in_host_tx(&mut tx, &k, worker, run).await {
        Ok(i) => i,
        Err(e) => {
            let _ = tx.rollback().await;
            return Err(match e {
                IssueError::NotFound => HostIssueError::UnknownWorker,
                IssueError::Retired => HostIssueError::Retired,
                IssueError::Entropy => HostIssueError::NoRandomness,
                IssueError::Port(e) => HostIssueError::Unavailable(format!("port: {e}")),
                IssueError::Db(e) => HostIssueError::Unavailable(format!("database: {e}")),
            });
        }
    };
    tx.commit()
        .await
        .map_err(|e| HostIssueError::Unavailable(format!("commit: {e}")))?;
    Ok(HostIssued {
        token: issued.token,
        token_id: issued.id,
        expires_at_micros: issued.expires_at_micros,
    })
}
