//! Single-writer lock so the GUI, the CLI and the autosync timer never interleave.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use crate::{Error, sys};

/// An exclusive, non-blocking `flock` on `$XDG_RUNTIME_DIR/ak820.lock`.
///
/// The lock is released when the value is dropped (or the process exits).
pub struct DeviceLock {
    path: PathBuf,
    _file: File,
}

impl DeviceLock {
    /// Take the lock, or fail with [`Error::Locked`] if another process holds it.
    pub fn acquire() -> Result<Self, Error> {
        let path = lock_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        if !sys::try_lock_exclusive(file.as_raw_fd())? {
            return Err(Error::Locked(path.display().to_string()));
        }
        Ok(DeviceLock { path, _file: file })
    }

    /// Path of the lock file, for messages.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn lock_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    dir.join("ak820.lock")
}
