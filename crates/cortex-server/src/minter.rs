//! The real `Minter` (contract r0.5 section 6, W1-37): finn's `mod_tokens::issue_in_host_tx` run INSIDE the caller's host transaction,
//! so the lead's token row, its audit row, the project, the lead and the `project.bootstrapped` row commit or roll back together.
//!
//! The token comes back as `cortex_types::Secret` (zeroised on drop); this type holds no secret itself. A first run is
//! `BootstrapRun::First` (audit `token.issued`), a re-run `BootstrapRun::Rerun` (audit `token.reissued`, the lead's earlier token is
//! superseded in the same transaction). Any failure of the issue is the one `MintError`: the store rolls the transaction back, so a
//! failing mint on a re-run leaves the earlier token valid.

use async_trait::async_trait;
use cortex_db::ModuleTx;
use cortex_module::{AuditSink, IdentityApi};
use cortex_types::{Clock, Secret, WorkerId};
use mod_tokens::{BootstrapRun, Entropy, Issuing, issue_in_host_tx};

use crate::store::{MintError, Minter};

pub struct TokenMinter<'a> {
    identity: &'a dyn IdentityApi,
    audit: &'a dyn AuditSink,
    clock: &'a dyn Clock,
    entropy: &'a dyn Entropy,
}

impl<'a> TokenMinter<'a> {
    pub fn new(
        identity: &'a dyn IdentityApi,
        audit: &'a dyn AuditSink,
        clock: &'a dyn Clock,
        entropy: &'a dyn Entropy,
    ) -> Self {
        TokenMinter {
            identity,
            audit,
            clock,
            entropy,
        }
    }
}

#[async_trait]
impl Minter for TokenMinter<'_> {
    async fn mint(
        &self,
        tx: &mut ModuleTx,
        lead: WorkerId,
        reissue: bool,
    ) -> Result<Secret, MintError> {
        let k = Issuing {
            identity: self.identity,
            audit: self.audit,
            clock: self.clock,
            entropy: self.entropy,
        };
        let run = if reissue {
            BootstrapRun::Rerun
        } else {
            BootstrapRun::First
        };
        issue_in_host_tx(tx, &k, lead, run)
            .await
            .map(|issued| issued.token)
            .map_err(|_| MintError)
    }
}
