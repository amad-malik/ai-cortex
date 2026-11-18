//! The exact name of a service's container (R4): `<project>-<service>`, the project being the `name:` of the release's
//! `compose.yml`, read STRICTLY: exactly ONE line at column 0 that starts with `name:`, and it is `name: <value>` with the
//! value matching `^[a-z][a-z0-9-]{0,31}$`. The file is read up to 1 MiB; more is refused. The name is passed to
//! `podman` whole (no filter, no prefix, no glob).

use crate::ports::Service;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// `O_NONBLOCK`, so that opening a FIFO that has no writer returns at once instead of waiting for one (vera's UPREAL-V2:
/// the open came before every check and every deadline). The file is then asked what it is, on the DESCRIPTOR, so there
/// is no gap between the check and the use. `std` has no name for the flag and this crate takes no new dependency.
#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "riscv64"
    )
))]
const O_NONBLOCK: i32 = 0o4000;
#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const O_NONBLOCK: i32 = 0x4;

/// The most bytes of `compose.yml` that are read.
const MAX: u64 = 1 << 20;

/// Why a name could not be derived. Carries nothing of the operator's: no path, no text of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoProjectName;

fn valid(project: &str) -> bool {
    let b = project.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The project name of the release in `release_dir`.
pub fn project_name(release_dir: &Path) -> Result<String, NoProjectName> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NONBLOCK)
        .open(release_dir.join("compose.yml"))
        .map_err(|_| NoProjectName)?;
    if !file.metadata().map(|m| m.is_file()).unwrap_or(false) {
        return Err(NoProjectName);
    }
    let mut buf = Vec::new();
    file.take(MAX + 1)
        .read_to_end(&mut buf)
        .map_err(|_| NoProjectName)?;
    if buf.len() as u64 > MAX {
        return Err(NoProjectName);
    }
    let text = String::from_utf8(buf).map_err(|_| NoProjectName)?;
    let mut name_lines = text.split('\n').filter(|l| l.starts_with("name:"));
    let line = name_lines.next().ok_or(NoProjectName)?;
    if name_lines.next().is_some() {
        return Err(NoProjectName);
    }
    let value = line.strip_prefix("name: ").ok_or(NoProjectName)?;
    if valid(value) {
        Ok(value.to_string())
    } else {
        Err(NoProjectName)
    }
}

/// `<project>-<service>`.
pub fn container_name(release_dir: &Path, service: Service) -> Result<String, NoProjectName> {
    Ok(format!("{}-{}", project_name(release_dir)?, service.name()))
}
