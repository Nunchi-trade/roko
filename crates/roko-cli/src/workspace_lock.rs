use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use fs2::FileExt;

/// Acquires an exclusive advisory lock on the workspace.
/// Returns a guard that releases the lock on drop.
/// Fails immediately if another process holds the lock.
pub fn acquire_workspace_lock(roko_dir: &Path) -> Result<WorkspaceLockGuard> {
    let lock_dir = roko_dir.join("runtime");
    fs::create_dir_all(&lock_dir)
        .with_context(|| format!("create lock dir: {}", lock_dir.display()))?;

    let lock_path = lock_dir.join("roko.lock");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&lock_path)
        .with_context(|| format!("open lock file: {}", lock_path.display()))?;

    match file.try_lock_exclusive() {
        Ok(()) => {
            // Write PID for diagnostics
            let mut f = &file;
            let _ = writeln!(f, "{}", std::process::id());
            Ok(WorkspaceLockGuard { file })
        }
        Err(_) => {
            // Read PID of holder for better error message
            let holder_pid = fs::read_to_string(&lock_path)
                .unwrap_or_default()
                .trim()
                .to_string();
            bail!(
                "Another roko process is running in this workspace (PID {}).\n  \
                 hint: wait for it to finish, or kill it with `kill {}`",
                holder_pid,
                holder_pid
            );
        }
    }
}

/// RAII guard that releases the file lock on drop.
pub struct WorkspaceLockGuard {
    file: File,
}

impl Drop for WorkspaceLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
