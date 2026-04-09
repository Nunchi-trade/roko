//! Worktree manager — wraps `git worktree add/remove/list/prune` so the
//! orchestrator can give each live plan its own isolated working
//! directory (parity §15).
//!
//! The manager shells out to the `git` binary via [`tokio::process::Command`]
//! and tracks active worktrees in an in-memory map guarded by a
//! [`parking_lot::Mutex`]. All git operations are fallible and surface as
//! [`WorktreeError`].
//!
//! ## Shipped features
//!
//! - §15.1–§15.2 Create / remove worktrees
//! - §15.3 Ephemeral branch naming ([`format_branch_name`])
//! - §15.4 Extended config: `max_live` budget, `idle_ttl`
//! - §15.5 Health checks ([`WorktreeHealth`])
//! - §15.6 Budget enforcement + idle reclamation
//! - §15.7 Stale lock detection ([`WorktreeManager::clear_stale_locks`])
//! - §15.8 Force-remove (via [`WorktreeManager::remove`])
//! - §15.9 Prune stale git metadata ([`WorktreeManager::prune`])

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::process::Command;

/// Locks older than this are considered stale (§15.7).
const STALE_LOCK_SECS: u64 = 60;

/// Configuration handed to [`WorktreeManager::new`].
#[derive(Debug, Clone)]
pub struct WorktreeConfig {
    /// Absolute path to the main repository checkout. `git worktree`
    /// commands are executed with this as their working directory.
    pub repo_root: PathBuf,
    /// Branch used as the starting point when no explicit HEAD is
    /// provided to `git worktree add`. Mirrors Mori's behaviour of
    /// branching from `main` / the plan base branch.
    pub base_branch: String,
    /// Directory under which new worktrees are materialised. Each
    /// worktree lives at `<worktrees_root>/<id>`.
    pub worktrees_root: PathBuf,
    /// Maximum simultaneously live worktrees. `None` = unlimited (§15.6).
    pub max_live: Option<usize>,
    /// After this duration without a [`WorktreeManager::touch`] call,
    /// a worktree becomes a candidate for [`WorktreeManager::reclaim_idle`]
    /// (§15.4).
    pub idle_ttl: Duration,
}

/// A live worktree tracked by the manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeHandle {
    /// Stable caller-assigned identifier (typically the plan id).
    pub id: String,
    /// Absolute path to the worktree's checkout directory.
    pub path: PathBuf,
    /// Branch checked out in the worktree.
    pub branch: String,
    /// Unix epoch milliseconds at which the handle was created.
    pub created_at_ms: i64,
    /// Unix epoch milliseconds of last activity. Updated by
    /// [`WorktreeManager::touch`].
    pub last_active_ms: i64,
}

/// Health of a tracked worktree (§15.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeHealth {
    /// Path exists and the expected branch is checked out.
    Ok,
    /// Worktree directory is missing from disk.
    Missing,
    /// A stale `.git/index.lock` was found (> 60 seconds old).
    StaleLock,
    /// HEAD is not on the expected branch (detached or switched).
    Detached,
}

/// Errors returned by [`WorktreeManager`].
///
/// Every git invocation that exits non-zero surfaces its stderr
/// verbatim via [`WorktreeError::GitFailed`] so callers can log the
/// underlying failure without re-running the command.
#[derive(Debug, Error)]
pub enum WorktreeError {
    /// `git` returned a non-zero exit status. `stderr` is captured
    /// verbatim (lossily decoded if it was not valid UTF-8).
    #[error("git command failed: {stderr}")]
    GitFailed {
        /// Captured stderr from the failing git invocation.
        stderr: String,
    },
    /// Caller asked for a worktree that the manager doesn't track.
    #[error("worktree not found: {0}")]
    NotFound(String),
    /// Caller tried to create a worktree whose id is already live.
    #[error("worktree already exists: {0}")]
    AlreadyExists(String),
    /// Supplied identifier failed validation (empty, path separator, …).
    #[error("invalid worktree id: {0}")]
    InvalidId(String),
    /// Creating a new worktree would exceed [`WorktreeConfig::max_live`]
    /// (§15.6).
    #[error("max live worktrees reached ({max})")]
    BudgetExhausted {
        /// The configured cap.
        max: usize,
    },
    /// Local filesystem error while preparing or removing a worktree.
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Derive the canonical branch name for a plan (§15.3).
///
/// Convention: `roko/plan/<plan_id>`. This keeps all roko-managed
/// branches under a single ref namespace, making cleanup and
/// enumeration straightforward.
#[must_use]
pub fn format_branch_name(plan_id: &str) -> String {
    format!("roko/plan/{plan_id}")
}

/// Manages the lifecycle of per-plan git worktrees.
///
/// Clones of [`WorktreeManager`] share the same internal registry — the
/// handle is cheap to clone and safe to move across tasks.
#[derive(Clone)]
pub struct WorktreeManager {
    config: Arc<WorktreeConfig>,
    active: Arc<Mutex<HashMap<String, WorktreeHandle>>>,
}

impl WorktreeManager {
    /// Construct a new manager. The caller is responsible for making
    /// sure `config.repo_root` is a real git repository — validation is
    /// lazy: the first git command that runs inside the worktree root
    /// will surface any misconfiguration as [`WorktreeError::GitFailed`].
    #[must_use]
    pub fn new(config: WorktreeConfig) -> Self {
        Self {
            config: Arc::new(config),
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Compute the path the manager would use for `id`. This is a pure
    /// function of the config and does not touch the filesystem.
    #[must_use]
    pub fn path_for(&self, id: &str) -> PathBuf {
        self.config.worktrees_root.join(id)
    }

    /// Create a worktree for `id` checked out onto `branch`.
    ///
    /// `branch` is created from [`WorktreeConfig::base_branch`] if it
    /// does not already exist. Rejects duplicate ids, invalid id
    /// strings (empty, containing `/`, `\`, NUL, or leading `.`), and
    /// requests that would exceed [`WorktreeConfig::max_live`].
    pub async fn create(&self, id: &str, branch: &str) -> Result<WorktreeHandle, WorktreeError> {
        validate_id(id)?;

        // Reserve the slot up-front so racing callers conflict cleanly.
        {
            let guard = self.active.lock();
            if guard.contains_key(id) {
                return Err(WorktreeError::AlreadyExists(id.to_string()));
            }
            // §15.6 — enforce budget before touching git.
            if let Some(max) = self.config.max_live {
                if guard.len() >= max {
                    return Err(WorktreeError::BudgetExhausted { max });
                }
            }
        }

        let path = self.path_for(id);

        // Make sure `worktrees_root` exists before git tries to write.
        tokio::fs::create_dir_all(&self.config.worktrees_root).await?;

        let path_str = path.to_string_lossy().into_owned();

        // `git worktree add -B <branch> <path> <base>` creates or resets
        // the branch to match the base, then checks it out into a new
        // worktree. Using `-B` (vs `-b`) keeps the call idempotent at
        // the git level if a stray branch is lying around.
        let output = Command::new("git")
            .current_dir(&self.config.repo_root)
            .args([
                "worktree",
                "add",
                "-B",
                branch,
                &path_str,
                &self.config.base_branch,
            ])
            .output()
            .await?;

        if !output.status.success() {
            return Err(WorktreeError::GitFailed {
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let now_ms = chrono::Utc::now().timestamp_millis();
        let handle = WorktreeHandle {
            id: id.to_string(),
            path,
            branch: branch.to_string(),
            created_at_ms: now_ms,
            last_active_ms: now_ms,
        };

        let conflict = {
            let mut guard = self.active.lock();
            // Double-check in case a concurrent caller inserted first.
            if guard.contains_key(id) {
                true
            } else {
                guard.insert(id.to_string(), handle.clone());
                false
            }
        };

        if conflict {
            // Best-effort rollback of the git side; ignore failures
            // since the real winner owns the worktree now.
            let _ = self.git_remove(&handle.path).await;
            return Err(WorktreeError::AlreadyExists(id.to_string()));
        }

        Ok(handle)
    }

    /// Convenience: create a worktree using the canonical branch naming
    /// convention (`roko/plan/<plan_id>`). §15.3
    pub async fn create_for_plan(&self, plan_id: &str) -> Result<WorktreeHandle, WorktreeError> {
        let branch = format_branch_name(plan_id);
        self.create(plan_id, &branch).await
    }

    /// Get a tracked worktree handle by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<WorktreeHandle> {
        self.active.lock().get(id).cloned()
    }

    /// Ensure a plan worktree exists and return its handle.
    ///
    /// If the worktree is already tracked, this touches and returns it.
    /// Otherwise a new canonical plan worktree is created.
    pub async fn ensure_for_plan(&self, plan_id: &str) -> Result<WorktreeHandle, WorktreeError> {
        if let Some(existing) = self.get(plan_id) {
            self.touch(plan_id);
            return self
                .get(plan_id)
                .ok_or(WorktreeError::NotFound(existing.id));
        }
        self.create_for_plan(plan_id).await
    }

    /// Return the active worktree path for `plan_id` if tracked.
    #[must_use]
    pub fn plan_path(&self, plan_id: &str) -> Option<PathBuf> {
        self.get(plan_id).map(|h| h.path)
    }

    /// Remove the worktree tracked under `id`. Errors if `id` isn't
    /// tracked. The underlying git directory is removed via
    /// `git worktree remove --force` so uncommitted files are cleaned up
    /// along with the metadata.
    pub async fn remove(&self, id: &str) -> Result<(), WorktreeError> {
        let handle = {
            let mut guard = self.active.lock();
            guard
                .remove(id)
                .ok_or_else(|| WorktreeError::NotFound(id.to_string()))?
        };

        if let Err(err) = self.git_remove(&handle.path).await {
            // Restore the registry entry so callers can retry.
            self.active.lock().insert(id.to_string(), handle);
            return Err(err);
        }

        Ok(())
    }

    /// Snapshot of every worktree currently tracked by the manager.
    /// Does **not** consult `git worktree list` — it reports the
    /// in-memory registry only.
    pub fn list(&self) -> Result<Vec<WorktreeHandle>, WorktreeError> {
        let mut out: Vec<WorktreeHandle> = {
            let guard = self.active.lock();
            guard.values().cloned().collect()
        };
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Number of active worktrees currently tracked in memory.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active.lock().len()
    }

    /// Bump the last-active timestamp for `id` (§15.4).
    ///
    /// Called by the orchestrator when agents write to a worktree so
    /// idle-time reclamation skips actively-used worktrees.
    pub fn touch(&self, id: &str) {
        let mut guard = self.active.lock();
        if let Some(handle) = guard.get_mut(id) {
            handle.last_active_ms = chrono::Utc::now().timestamp_millis();
        }
    }

    /// Probe the health of the worktree tracked under `id` (§15.5).
    ///
    /// Returns [`WorktreeHealth::Ok`] when the directory exists and the
    /// expected branch is checked out; other variants describe the
    /// specific failure mode.
    pub async fn check_health(&self, id: &str) -> Result<WorktreeHealth, WorktreeError> {
        let handle = {
            let guard = self.active.lock();
            guard
                .get(id)
                .cloned()
                .ok_or_else(|| WorktreeError::NotFound(id.to_string()))?
        };

        if !handle.path.exists() {
            return Ok(WorktreeHealth::Missing);
        }

        // Check for a stale index.lock in the worktree's gitdir.
        if let Some(gitdir) = read_gitdir(&handle.path) {
            let lock = gitdir.join("index.lock");
            if lock.exists() && is_stale_lock(&lock) {
                return Ok(WorktreeHealth::StaleLock);
            }
        }

        // Verify the expected branch is checked out.
        let output = Command::new("git")
            .current_dir(&handle.path)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .await?;

        if output.status.success() {
            let current = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if current != handle.branch {
                return Ok(WorktreeHealth::Detached);
            }
        }

        Ok(WorktreeHealth::Ok)
    }

    /// Evict worktrees whose `last_active_ms` is older than
    /// [`WorktreeConfig::idle_ttl`], oldest first. Returns the ids
    /// that were successfully reclaimed (§15.6).
    pub async fn reclaim_idle(&self) -> Result<Vec<String>, WorktreeError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let ttl_ms = i64::try_from(self.config.idle_ttl.as_millis()).unwrap_or(i64::MAX);

        let stale_ids: Vec<String> = {
            let mut candidates: Vec<_> = self
                .active
                .lock()
                .values()
                .filter(|h| (now_ms - h.last_active_ms) > ttl_ms)
                .map(|h| (h.id.clone(), h.last_active_ms))
                .collect();
            // Evict oldest first.
            candidates.sort_by_key(|(_, ts)| *ts);
            candidates.into_iter().map(|(id, _)| id).collect()
        };

        let mut removed = Vec::new();
        for id in stale_ids {
            if self.remove(&id).await.is_ok() {
                removed.push(id);
            }
        }

        Ok(removed)
    }

    /// Remove all currently tracked worktrees.
    ///
    /// Returns the ids that were successfully removed.
    pub async fn remove_all(&self) -> Result<Vec<String>, WorktreeError> {
        let ids: Vec<String> = self.active.lock().keys().cloned().collect();
        let mut removed = Vec::new();
        for id in ids {
            if self.remove(&id).await.is_ok() {
                removed.push(id);
            }
        }
        removed.sort();
        Ok(removed)
    }

    /// Remove stale `.git/index.lock` files (older than 60 seconds)
    /// across the main repo and all git-tracked worktrees (§15.7).
    pub fn clear_stale_locks(&self) -> Result<Vec<PathBuf>, WorktreeError> {
        let mut cleared = Vec::new();

        // Main repo lock.
        let main_lock = self.config.repo_root.join(".git").join("index.lock");
        if main_lock.exists()
            && is_stale_lock(&main_lock)
            && std::fs::remove_file(&main_lock).is_ok()
        {
            cleared.push(main_lock);
        }

        // Per-worktree locks stored under .git/worktrees/<name>/index.lock.
        let wt_meta_dir = self.config.repo_root.join(".git").join("worktrees");
        if wt_meta_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&wt_meta_dir) {
                for entry in entries.flatten() {
                    let lock = entry.path().join("index.lock");
                    if lock.exists() && is_stale_lock(&lock) && std::fs::remove_file(&lock).is_ok()
                    {
                        cleared.push(lock);
                    }
                }
            }
        }

        Ok(cleared)
    }

    /// Run `git worktree prune` to clean up stale git worktree metadata
    /// that no longer corresponds to on-disk directories (§15.9).
    pub async fn prune(&self) -> Result<String, WorktreeError> {
        let output = Command::new("git")
            .current_dir(&self.config.repo_root)
            .args(["worktree", "prune"])
            .output()
            .await?;

        if !output.status.success() {
            return Err(WorktreeError::GitFailed {
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    async fn git_remove(&self, path: &Path) -> Result<(), WorktreeError> {
        let path_str = path.to_string_lossy().into_owned();
        let output = Command::new("git")
            .current_dir(&self.config.repo_root)
            .args(["worktree", "remove", "--force", &path_str])
            .output()
            .await?;

        if !output.status.success() {
            return Err(WorktreeError::GitFailed {
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(())
    }
}

fn validate_id(id: &str) -> Result<(), WorktreeError> {
    if id.is_empty() {
        return Err(WorktreeError::InvalidId("id is empty".to_string()));
    }
    if id.starts_with('.') || id.starts_with('-') {
        return Err(WorktreeError::InvalidId(format!(
            "id `{id}` may not start with `.` or `-`"
        )));
    }
    if id.contains("..") {
        return Err(WorktreeError::InvalidId(format!(
            "id `{id}` may not contain `..`"
        )));
    }
    // Characters that would break either a filesystem path or a git ref:
    // `/`, `\`, NUL, whitespace, and the git-ref-forbidden set
    // (`~`, `^`, `:`, `?`, `*`, `[`, ASCII control). Also reject `@{`.
    for ch in id.chars() {
        let bad = matches!(
            ch,
            '/' | '\\' | '\0' | '~' | '^' | ':' | '?' | '*' | '[' | '@'
        ) || ch.is_whitespace()
            || ch.is_control();
        if bad {
            return Err(WorktreeError::InvalidId(format!(
                "id `{id}` contains forbidden character `{ch}`"
            )));
        }
    }
    Ok(())
}

/// Read the `gitdir:` pointer from a worktree's `.git` file.
///
/// In a worktree `.git` is a regular file (not a directory) containing
/// a single line like `gitdir: /repo/.git/worktrees/<name>`. Returns
/// `None` if the file is missing, is a directory, or is unparseable.
fn read_gitdir(worktree_path: &Path) -> Option<PathBuf> {
    let git_file = worktree_path.join(".git");
    if git_file.is_dir() {
        // Main repo — the gitdir is the .git directory itself.
        return Some(git_file);
    }
    let content = std::fs::read_to_string(&git_file).ok()?;
    let path_str = content.trim().strip_prefix("gitdir: ")?;
    let p = PathBuf::from(path_str);
    if p.is_absolute() {
        Some(p)
    } else {
        Some(worktree_path.join(p))
    }
}

/// A lock file is considered stale when its mtime is ≥ [`STALE_LOCK_SECS`]
/// seconds in the past.
fn is_stale_lock(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    std::time::SystemTime::now()
        .duration_since(modified)
        .map(|age| age.as_secs() >= STALE_LOCK_SECS)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    //! Tests below each spin up a throwaway git repo in a
    //! [`tempfile::TempDir`]. Every test that needs `git` first checks
    //! the binary via [`git_available`]; if git isn't on `$PATH`, the
    //! test returns early so `cargo test` still succeeds on machines
    //! without git (for instance minimal CI images).

    #![allow(clippy::unwrap_used)]

    use super::{
        WorktreeConfig, WorktreeError, WorktreeHealth, WorktreeManager, format_branch_name,
        validate_id,
    };
    use std::path::Path;
    use std::process::Command as StdCommand;
    use std::time::Duration;
    use tempfile::TempDir;

    fn git_available() -> bool {
        StdCommand::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn init_repo(dir: &Path) {
        // Keep the default branch name deterministic across git
        // versions and user configs.
        let status = StdCommand::new("git")
            .current_dir(dir)
            .args(["init", "-b", "main"])
            .status()
            .unwrap();
        assert!(status.success(), "git init failed");

        // Scoped identity so the test's commit is reproducible without
        // touching the user's global git config.
        for (k, v) in [
            ("user.email", "roko@example.test"),
            ("user.name", "Roko Test"),
            ("commit.gpgsign", "false"),
        ] {
            let ok = StdCommand::new("git")
                .current_dir(dir)
                .args(["config", k, v])
                .status()
                .unwrap()
                .success();
            assert!(ok, "git config {k} failed");
        }

        let ok = StdCommand::new("git")
            .current_dir(dir)
            .args(["commit", "--allow-empty", "-m", "init"])
            .status()
            .unwrap()
            .success();
        assert!(ok, "git commit --allow-empty failed");
    }

    fn make_manager() -> Option<(TempDir, WorktreeManager)> {
        if !git_available() {
            eprintln!("skipping: git not on PATH");
            return None;
        }
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_root).unwrap();
        init_repo(&repo_root);
        let worktrees_root = tmp.path().join("worktrees");
        let mgr = WorktreeManager::new(WorktreeConfig {
            repo_root,
            base_branch: "main".to_string(),
            worktrees_root,
            max_live: None,
            idle_ttl: Duration::from_secs(3600),
        });
        Some((tmp, mgr))
    }

    fn make_manager_with_budget(max_live: usize) -> Option<(TempDir, WorktreeManager)> {
        if !git_available() {
            eprintln!("skipping: git not on PATH");
            return None;
        }
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_root).unwrap();
        init_repo(&repo_root);
        let worktrees_root = tmp.path().join("worktrees");
        let mgr = WorktreeManager::new(WorktreeConfig {
            repo_root,
            base_branch: "main".to_string(),
            worktrees_root,
            max_live: Some(max_live),
            idle_ttl: Duration::from_secs(3600),
        });
        Some((tmp, mgr))
    }

    // ── Existing tests (§15.1–§15.2, §15.8) ──

    #[tokio::test]
    async fn create_worktree_materialises_directory() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        let handle = mgr.create("01-alpha", "feature/alpha").await.unwrap();
        assert_eq!(handle.id, "01-alpha");
        assert_eq!(handle.branch, "feature/alpha");
        assert!(handle.path.exists(), "worktree dir should exist");
        assert!(handle.path.join(".git").exists(), ".git file expected");
        assert!(handle.created_at_ms > 0);
    }

    #[tokio::test]
    async fn list_returns_every_active_handle_sorted() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        mgr.create("02-bravo", "feature/bravo").await.unwrap();
        mgr.create("01-alpha", "feature/alpha").await.unwrap();
        let listed = mgr.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "01-alpha");
        assert_eq!(listed[1].id, "02-bravo");
    }

    #[tokio::test]
    async fn remove_drops_handle_and_worktree() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        let handle = mgr.create("03-charlie", "feature/charlie").await.unwrap();
        assert!(handle.path.exists());
        mgr.remove("03-charlie").await.unwrap();
        assert!(mgr.list().unwrap().is_empty());
        assert!(
            !handle.path.exists(),
            "git worktree remove --force should delete the dir"
        );
    }

    #[tokio::test]
    async fn create_remove_roundtrip_allows_reuse() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        mgr.create("04-delta", "feature/delta").await.unwrap();
        mgr.remove("04-delta").await.unwrap();
        // After removal we must be able to use the id again.
        let h2 = mgr.create("04-delta", "feature/delta").await.unwrap();
        assert_eq!(h2.id, "04-delta");
        assert_eq!(mgr.list().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn remove_nonexistent_is_not_found() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        let err = mgr.remove("nope").await.unwrap_err();
        assert!(matches!(err, WorktreeError::NotFound(ref id) if id == "nope"));
    }

    #[tokio::test]
    async fn duplicate_id_is_rejected() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        mgr.create("05-echo", "feature/echo").await.unwrap();
        let err = mgr.create("05-echo", "feature/echo-2").await.unwrap_err();
        assert!(matches!(err, WorktreeError::AlreadyExists(ref id) if id == "05-echo"));
        // Original handle is still there, in registry and on disk.
        assert_eq!(mgr.list().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn path_for_does_not_touch_disk() {
        let tmp = TempDir::new().unwrap();
        let mgr = WorktreeManager::new(WorktreeConfig {
            repo_root: tmp.path().join("repo"),
            base_branch: "main".to_string(),
            worktrees_root: tmp.path().join("worktrees"),
            max_live: None,
            idle_ttl: Duration::from_secs(3600),
        });
        let p = mgr.path_for("06-foxtrot");
        assert_eq!(p, tmp.path().join("worktrees").join("06-foxtrot"));
        assert!(!p.exists(), "path_for should be pure");
        // No git calls were made, so nothing was created.
        assert!(mgr.list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn invalid_ids_are_rejected_before_git_runs() {
        let tmp = TempDir::new().unwrap();
        let mgr = WorktreeManager::new(WorktreeConfig {
            repo_root: tmp.path().join("repo"),
            base_branch: "main".to_string(),
            worktrees_root: tmp.path().join("worktrees"),
            max_live: None,
            idle_ttl: Duration::from_secs(3600),
        });
        // Neither repo nor worktrees_root exists — but we never reach
        // git because validate_id rejects these inputs first.
        for bad in [
            "",
            ".hidden",
            "-starts-with-dash",
            "has/slash",
            "has\\back",
            "white space",
            "has..dots",
            "has~tilde",
            "has^caret",
            "has:colon",
            "has?q",
            "has*star",
            "has[bracket",
            "has@at",
            "has\ttab",
        ] {
            let err = mgr.create(bad, "feature/x").await.unwrap_err();
            assert!(
                matches!(err, WorktreeError::InvalidId(_)),
                "expected InvalidId for `{bad}`, got {err:?}"
            );
        }
    }

    #[test]
    fn validate_id_accepts_reasonable_ids() {
        for good in [
            "01-alpha",
            "plan_42",
            "08a-some-thing",
            "nested.branch-name",
        ] {
            validate_id(good).unwrap();
        }
    }

    // ── New tests (§15.3–§15.7, §15.9) ──

    #[test]
    fn format_branch_name_uses_convention() {
        assert_eq!(format_branch_name("01-alpha"), "roko/plan/01-alpha");
        assert_eq!(format_branch_name("fix_typo"), "roko/plan/fix_typo");
    }

    #[tokio::test]
    async fn create_for_plan_uses_canonical_branch() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        let handle = mgr.create_for_plan("07-golf").await.unwrap();
        assert_eq!(handle.branch, "roko/plan/07-golf");
        assert_eq!(handle.id, "07-golf");
        assert!(handle.path.exists());
    }

    #[tokio::test]
    async fn get_and_plan_path_return_tracked_handle() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        let handle = mgr.create_for_plan("07-golf-2").await.unwrap();
        let fetched = mgr.get("07-golf-2").unwrap();
        assert_eq!(fetched.id, "07-golf-2");
        assert_eq!(mgr.plan_path("07-golf-2"), Some(handle.path));
    }

    #[tokio::test]
    async fn ensure_for_plan_reuses_existing_worktree() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        let first = mgr.create_for_plan("07-golf-3").await.unwrap();
        let before_count = mgr.active_count();
        let ensured = mgr.ensure_for_plan("07-golf-3").await.unwrap();
        assert_eq!(before_count, mgr.active_count());
        assert_eq!(ensured.id, first.id);
        assert_eq!(ensured.path, first.path);
        assert_eq!(ensured.branch, first.branch);
    }

    #[tokio::test]
    async fn remove_all_clears_every_tracked_worktree() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        mgr.create_for_plan("07-golf-4").await.unwrap();
        mgr.create_for_plan("07-golf-5").await.unwrap();

        let removed = mgr.remove_all().await.unwrap();
        assert_eq!(
            removed,
            vec!["07-golf-4".to_string(), "07-golf-5".to_string()]
        );
        assert_eq!(mgr.active_count(), 0);
    }

    #[tokio::test]
    async fn budget_exhausted_when_max_live_reached() {
        let Some((_tmp, mgr)) = make_manager_with_budget(1) else {
            return;
        };
        mgr.create("a", "feature/a").await.unwrap();
        let err = mgr.create("b", "feature/b").await.unwrap_err();
        assert!(matches!(err, WorktreeError::BudgetExhausted { max: 1 }));
        // Original still intact.
        assert_eq!(mgr.list().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn touch_updates_last_active() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        let h = mgr.create("08-hotel", "feature/hotel").await.unwrap();
        let before = h.last_active_ms;
        tokio::time::sleep(Duration::from_millis(50)).await;
        mgr.touch("08-hotel");
        let listed = mgr.list().unwrap();
        let after = listed[0].last_active_ms;
        assert!(after > before, "touch should bump last_active_ms");
    }

    #[tokio::test]
    async fn check_health_ok_for_live_worktree() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        mgr.create("09-india", "feature/india").await.unwrap();
        let health = mgr.check_health("09-india").await.unwrap();
        assert_eq!(health, WorktreeHealth::Ok);
    }

    #[tokio::test]
    async fn check_health_missing_when_dir_deleted() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        let h = mgr.create("10-juliet", "feature/juliet").await.unwrap();
        std::fs::remove_dir_all(&h.path).unwrap();
        let health = mgr.check_health("10-juliet").await.unwrap();
        assert_eq!(health, WorktreeHealth::Missing);
    }

    #[tokio::test]
    async fn reclaim_idle_evicts_stale_worktrees() {
        if !git_available() {
            return;
        }
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_root).unwrap();
        init_repo(&repo_root);
        // idle_ttl = 0 so every worktree is immediately reclaimable.
        let mgr = WorktreeManager::new(WorktreeConfig {
            repo_root,
            base_branch: "main".to_string(),
            worktrees_root: tmp.path().join("worktrees"),
            max_live: None,
            idle_ttl: Duration::ZERO,
        });
        mgr.create("11-kilo", "feature/kilo").await.unwrap();
        mgr.create("12-lima", "feature/lima").await.unwrap();
        // Small sleep so timestamps are in the past.
        tokio::time::sleep(Duration::from_millis(5)).await;
        let evicted = mgr.reclaim_idle().await.unwrap();
        assert_eq!(evicted.len(), 2);
        assert!(mgr.list().unwrap().is_empty());
    }

    #[tokio::test]
    async fn clear_stale_locks_removes_old_lock_files() {
        let Some((tmp, mgr)) = make_manager() else {
            return;
        };
        mgr.create("13-mike", "feature/mike").await.unwrap();

        // Plant a stale lock in the git worktrees metadata dir.
        let repo_root = tmp.path().join("repo");
        let lock_dir = repo_root.join(".git").join("worktrees").join("13-mike");
        assert!(
            lock_dir.exists(),
            "git should have created worktree metadata"
        );
        let lock_path = lock_dir.join("index.lock");
        std::fs::write(&lock_path, "").unwrap();

        // Backdate the lock to epoch so it appears stale (> 5 min).
        std::fs::File::options()
            .write(true)
            .open(&lock_path)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_accessed(std::time::SystemTime::UNIX_EPOCH)
                    .set_modified(std::time::SystemTime::UNIX_EPOCH),
            )
            .unwrap();

        let cleared = mgr.clear_stale_locks().unwrap();
        assert!(cleared.contains(&lock_path), "stale lock should be cleared");
        assert!(!lock_path.exists(), "lock file should be deleted");
    }

    #[tokio::test]
    async fn prune_runs_without_error() {
        let Some((_tmp, mgr)) = make_manager() else {
            return;
        };
        let result = mgr.prune().await;
        assert!(result.is_ok());
    }
}
