//! The query (S3 r2.6 `boot.get` parameters): `budget` (bytes, maximum 262144, default 12288, no
//! schema minimum) and `format` (`md` default, or `json`). Decoding is strict (R-4, R-6): an
//! unknown parameter, a repeated one or a value of the wrong type or enum is 422 `invalid_request`
//! from [`cortex_module::StrictQuery`]. The ranges are the handler's (R-3).

use cortex_module::Problem;
use serde::Deserialize;

use crate::render::Needed;

/// The default budget (S2 §7.2: 12 KiB).
pub const DEFAULT_BUDGET: u32 = 12_288;
/// The smallest budget boot accepts (S2 §7.2: 4 KiB).
pub const MIN_BUDGET: u32 = 4_096;
/// The largest budget (S3 `maximum`: 256 KiB).
pub const MAX_BUDGET: u32 = 262_144;

/// The output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Md,
    Json,
}

/// The query as decoded. `deny_unknown_fields`: `?worker=…` is 422, never honoured.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootQuery {
    /// Signed: S3 types it as an integer with no schema minimum, so a negative value decodes and
    /// the range check answers it (sam's BOOT12-02).
    pub budget: Option<i64>,
    pub format: Option<Format>,
}

/// The checked arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Args {
    pub budget: u32,
    pub format: Format,
}

/// The range checks, before any read: over the maximum is 422 `invalid_request`; below the
/// minimum is 422 `boot_budget_too_small` naming the minimum as `needed`.
pub fn check(q: &BootQuery) -> Result<Args, Problem> {
    let budget = match q.budget {
        None => DEFAULT_BUDGET,
        Some(b) if b > i64::from(MAX_BUDGET) => {
            return Err(Problem::invalid_request(
                422,
                "budget is over the maximum of 262144 bytes.",
            ));
        }
        // Below the minimum, zero and negative values included, names the minimum.
        Some(b) if b < i64::from(MIN_BUDGET) => {
            return Err(Needed {
                needed: MIN_BUDGET as usize,
                budget: b,
            }
            .problem());
        }
        // In range: 4096..=262144 fits in u32.
        Some(b) => b as u32,
    };
    Ok(Args {
        budget,
        format: q.format.unwrap_or(Format::Md),
    })
}
