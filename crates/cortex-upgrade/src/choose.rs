//! Choosing the release to install (design §4, step 3). Pure: no network, no files.

use crate::manifest::{Entry, Manifest, Release};
use crate::version::Version;
use std::cmp::Ordering;

/// What is running now (from `current/stack.env` and the stack).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub version: Version,
    pub db_image: String,
}

/// `--check` and a plain `upgrade` ask for the newest; `--to V` asks for exactly V.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    Newest,
    Exactly(Version),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision<'a> {
    /// Nothing newer (or the asked-for version is the installed one): exit 0.
    UpToDate,
    Upgrade(&'a Release),
}

/// Each variant is a refusal before any change (exit 2); none carries manifest text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChooseError {
    #[error("refused: that is a downgrade; a downgrade is only done with --rollback")]
    Downgrade,
    #[error("refused: the installed version is below the release's min_upgrade_from")]
    BelowMinUpgradeFrom,
    #[error("refused: the release's db_image differs from the running one")]
    DbImageDiffers,
    #[error("refused: the manifest has no usable release with that version")]
    NotInManifest,
}

/// Precedence order (build metadata never takes part; versions here have none anyway).
fn order(a: &Version, b: &Version) -> Ordering {
    a.cmp_precedence(b)
}

/// Step 3 of the state machine. `Newest` is the highest usable release above the installed one;
/// it is refused, not replaced by an older one, when it cannot be installed from here (the
/// operator must be told that a step is needed). `Exactly(v)`: below the installed version is a
/// downgrade (only `--rollback` does that), equal is nothing to do.
pub fn choose<'a>(
    installed: &Installed,
    manifest: &'a Manifest,
    wanted: &Wanted,
) -> Result<Decision<'a>, ChooseError> {
    let mut usable = manifest.entries.iter().filter_map(|e| match e {
        Entry::Usable(r) => Some(&**r),
        Entry::Skipped(_) => None,
    });
    let release = match wanted {
        Wanted::Exactly(to) => match order(to, &installed.version) {
            Ordering::Less => return Err(ChooseError::Downgrade),
            Ordering::Equal => return Ok(Decision::UpToDate),
            Ordering::Greater => usable
                .find(|r| order(&r.version, to) == Ordering::Equal)
                .ok_or(ChooseError::NotInManifest)?,
        },
        Wanted::Newest => {
            let newest = usable
                .filter(|r| order(&r.version, &installed.version) == Ordering::Greater)
                .max_by(|a, b| order(&a.version, &b.version));
            match newest {
                Some(r) => r,
                None => return Ok(Decision::UpToDate),
            }
        }
    };
    if let Some(min) = &release.cortex.min_upgrade_from
        && order(&installed.version, min) == Ordering::Less
    {
        return Err(ChooseError::BelowMinUpgradeFrom);
    }
    if release.cortex.db_image != installed.db_image {
        return Err(ChooseError::DbImageDiffers);
    }
    Ok(Decision::Upgrade(release))
}
