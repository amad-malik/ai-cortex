//! `Dumper` over the installed `<home>/bin/cortex-dump` (brief r1 section 3, Q-8). ONE program, by its absolute path,
//! with a fixed vector; the engine decides what each exit code means.

use super::name::container_name;
use super::run::{failed, last_line, run, run_failure, stderr_failure};
use crate::ports::{DumpLabel, DumpResult, Dumper, OneLine, PortError, Service};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

/// The fixed program name used in the failure lines.
const TOOL: &str = "cortex-dump";
/// The longest stdout line that is reported.
const STDOUT_LINE_MAX: usize = 4096;

#[derive(Debug, Clone)]
pub struct RealDumper {
    home: PathBuf,
    current_release_dir: PathBuf,
    grace: Duration,
}

impl RealDumper {
    /// The real one: the outer deadline is the caller's timeout plus 30 seconds.
    pub fn new(home: PathBuf, current_release_dir: PathBuf) -> RealDumper {
        RealDumper::with_grace(home, current_release_dir, Duration::from_secs(30))
    }

    /// The slack added to the caller's timeout for the outer deadline, for the cells.
    pub fn with_grace(home: PathBuf, current_release_dir: PathBuf, grace: Duration) -> RealDumper {
        RealDumper {
            home,
            current_release_dir,
            grace,
        }
    }
}

impl Dumper for RealDumper {
    fn dump(&self, label: &DumpLabel, timeout: Duration) -> Result<DumpResult, PortError> {
        if !self.home.is_absolute() {
            return Err(failed("the home directory is not an absolute path"));
        }
        let tool = self.home.join("bin").join(TOOL);
        // A regular file (not a link) that its owner can execute: nothing is started otherwise.
        let installed = std::fs::symlink_metadata(&tool)
            .map(|m| m.file_type().is_file() && m.permissions().mode() & 0o100 != 0)
            .unwrap_or(false);
        if !installed {
            return Err(failed("the dump tool is not installed"));
        }
        let ctr = container_name(&self.current_release_dir, Service::Db)
            .map_err(|_| failed("the current release has no valid project name"))?;
        let args: Vec<OsString> = vec![
            OsString::from("--target"),
            OsString::from(ctr),
            OsString::from("--out"),
            self.home.join("dumps").into_os_string(),
            OsString::from("--label"),
            OsString::from(label.as_str()),
            OsString::from("--timeout"),
            OsString::from(timeout.as_secs().to_string()),
        ];
        let o =
            run(tool.as_os_str(), &args, timeout + self.grace).map_err(|e| run_failure(e, TOOL))?;
        let Some(exit) = o.code else {
            // ended by a signal: no exit code to report
            return Err(stderr_failure(&o));
        };
        let last_stdout_line = if o.stdout_cut {
            None
        } else {
            last_line(&o.stdout).filter(|l| l.len() <= STDOUT_LINE_MAX)
        };
        Ok(DumpResult {
            exit,
            last_stdout_line,
            last_stderr_line: OneLine::last_line_of(&o.stderr),
        })
    }
}
