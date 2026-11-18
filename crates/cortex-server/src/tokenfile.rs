//! The fresh token file (contract r0.3, row `bootstrap`): written FRESH with mode 0600, never through a symlink or an
//! existing file, the token never printed or logged; on any failure no partial file is left.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Create `path` (it must not exist, not even as a dangling symlink), mode 0600, write `secret`, and sync it.
/// On any error after the file was created the file is removed.
///
/// `create_new` is `O_CREAT | O_EXCL`: the kernel refuses an existing file and ALSO a symlink at the path (dangling or not), so
/// no symlink is ever followed and nothing is overwritten. The mode is set at creation (0600, narrowed further by a stricter umask),
/// so the secret is never in a more readable file, not even for an instant. An `io::Error` carries the OS reason only, never the secret.
/// The same, with the write-and-sync step supplied by the caller. The real step is [`write_fresh`]'s; this seam exists so the
/// "failure AFTER the file was created" path can be exercised by a test (vera's Low): whatever `finish` returns as an error, the
/// file is removed and the error is returned unchanged.
pub fn write_fresh_with(
    path: &Path,
    secret: &[u8],
    finish: impl FnOnce(&mut std::fs::File, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    let written = finish(&mut f, secret);
    if written.is_err() {
        drop(f);
        let _ = fs::remove_file(path);
    }
    written
}

pub fn write_fresh(path: &Path, secret: &[u8]) -> io::Result<()> {
    write_fresh_with(path, secret, |f, secret| {
        f.write_all(secret)?;
        f.sync_all()
    })
}
