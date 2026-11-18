//! The `migrate` decision (contract r0.3 section 1, row `migrate`): a ONE-SHOT run before `serve`, the only place the owner role is
//! used. It REQUIRES the `owner_*` keys, applies `platform` first and then every enabled piece in the REGISTRY's migration order, passed by the caller (forward only, each set
//! in its own transactions, a current schema is a no-op), and stops at the first failure, naming the module and the migration
//! version. Nothing secret is printed: no SQL, no password, no file path.
//!
//! The database side is a trait so the decision is testable without a database; the real implementation (the owner connection and
//! `cortex_db::migrate::apply` over each enabled piece's `MigrationSet`) plugs in with the piece registry.
//!
//! NOT here yet (no data to decide on): the D95 rule that a `contract` migration is refused unless it belongs to the release being
//! installed. A `Migration` carries no expand/contract kind today; the rule lands with the first contract step.

use async_trait::async_trait;
use cortex_config::Config;
use cortex_types::ModuleId;

use crate::bootstrap::Outcome;
use crate::cli::{EXIT_FAILURE, EXIT_OK};

/// One migration step that failed: its version. The module is the one being applied.
#[derive(Debug, PartialEq, Eq)]
pub struct StepFailed(pub u32);

/// The database side: applies ONE module's migration set under the owner role, forward only, and says how many steps it applied
/// (0 when the schema is already current).
#[async_trait]
pub trait Migrator: Send {
    async fn apply(&mut self, module: &str) -> Result<u32, StepFailed>;
}

fn fail(line: String) -> Outcome {
    Outcome {
        code: EXIT_FAILURE,
        stderr: Some(line),
    }
}

/// The caller passes the registry's migration order (`platform` first, then every enabled piece, dependencies before dependents); the
/// config's own order is NOT used, because cortex-config accepts any order and duplicates (hale N-2). It must be exactly `platform`
/// followed by the enabled pieces, each once: anything else is a caller bug and is refused before anything runs.
fn order_matches(cfg: &Config, order: &[ModuleId]) -> bool {
    use std::collections::BTreeSet;
    let Some((first, rest)) = order.split_first() else {
        return false;
    };
    let rest_set: BTreeSet<&str> = rest.iter().map(ModuleId::as_str).collect();
    let enabled: BTreeSet<&str> = cfg.modules.enabled.iter().map(ModuleId::as_str).collect();
    first.as_str() == "platform"
        && rest_set.len() == rest.len()
        && !rest_set.contains("platform")
        && rest_set == enabled
}

/// Decide and act. Nothing is printed here; `main` writes `stderr` and exits with `code`.
pub async fn run(cfg: &Config, order: &[ModuleId], m: &mut dyn Migrator) -> Outcome {
    let Some(db) = &cfg.database else {
        return fail("migrate refused: the config has no [database] table".to_owned());
    };
    if let Err(e) = db.owner_for_migrate() {
        // `ConfigError::Missing` names the KEY (`missing: database.owner_user`), never a value.
        return fail(format!("migrate refused: {e}"));
    }
    if !order_matches(cfg, order) {
        return fail(
            "migrate refused: the module order does not match the enabled pieces".to_owned(),
        );
    }
    for module in order.iter().map(ModuleId::as_str) {
        if let Err(StepFailed(version)) = m.apply(module).await {
            return fail(format!(
                "migrate failed: module {module}, migration {version}"
            ));
        }
    }
    Outcome {
        code: EXIT_OK,
        stderr: None,
    }
}
