//! `Psql` over `podman exec` (brief r1 section 3). ONE `podman exec -i --user postgres <db ctr> psql ...` per call; the
//! count query is a constant. The engine judges the text.

use super::name::container_name;
use super::run::run;
use crate::ports::{NoAnswer, Psql, Service};
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

/// The count of `pg_hba` rules that are `trust` or have an error (design 4a). A constant: nothing is interpolated.
pub const TRUST_RULES_SQL: &str =
    "select count(*) from pg_hba_file_rules where auth_method = 'trust' or error is not null";

/// The most bytes an answer may have (a count has a few); more is no answer.
const ANSWER_MAX: usize = 64;

#[derive(Debug, Clone)]
pub struct RealPsql {
    podman: OsString,
    deadline: Duration,
    current_release_dir: PathBuf,
}

impl RealPsql {
    /// The real one: `podman` from PATH, 30 seconds, the database container of the CURRENT release.
    pub fn new(current_release_dir: PathBuf) -> RealPsql {
        RealPsql::with(
            OsString::from("podman"),
            Duration::from_secs(30),
            current_release_dir,
        )
    }

    /// The program and the deadline, for the cells.
    pub fn with(
        podman: impl Into<OsString>,
        deadline: Duration,
        current_release_dir: PathBuf,
    ) -> RealPsql {
        RealPsql {
            podman: podman.into(),
            deadline,
            current_release_dir,
        }
    }
}

impl Psql for RealPsql {
    fn trust_rule_count(&self) -> Result<String, NoAnswer> {
        let ctr = container_name(&self.current_release_dir, Service::Db).map_err(|_| NoAnswer)?;
        let args: Vec<OsString> = [
            "exec",
            "-i",
            "--user",
            "postgres",
            ctr.as_str(),
            "psql",
            "-U",
            "postgres",
            "-d",
            "cortex",
            "-tAc",
            TRUST_RULES_SQL,
        ]
        .iter()
        .map(OsString::from)
        .collect();
        let o = run(&self.podman, &args, self.deadline).map_err(|_| NoAnswer)?;
        if o.code != Some(0) {
            return Err(NoAnswer);
        }
        // The engine judges the text, so the text must be ALL of the answer: an answer that was cut, or that is longer than
        // the most a count can be, is no answer. Shortening it would let a poisoned tail read as a clean `0`.
        if o.stdout_cut {
            return Err(NoAnswer);
        }
        let text = String::from_utf8_lossy(&o.stdout);
        let trimmed = text.trim();
        if trimmed.len() > ANSWER_MAX {
            return Err(NoAnswer);
        }
        Ok(trimmed.to_string())
    }
}
