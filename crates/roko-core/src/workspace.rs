//! Canonical workspace layout abstraction.
//!
//! Provides a [`Workspace`] struct that validates a project root and exposes
//! typed access to the `.roko/` directory hierarchy. Use this as the public
//! boundary for workspace-local paths; lower-level filesystem crates may keep
//! narrower path catalogs for their own internals.
//!
//! ## Migration note
//!
//! `.roko/learn/` is the canonical directory for learning state.
//! `.roko/memory/` is retained only as a legacy read/migration surface.
//! Callers should prefer `learn_dir()` for all new writes and fall back to
//! `memory_dir()` only when reading pre-migration data.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// Represents a validated Roko workspace rooted at a project directory.
#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Open an existing workspace by validating that `.roko/` exists under `root`.
    ///
    /// Returns an error if the `.roko/` directory does not exist.
    ///
    /// Emits a `tracing::warn!` if `.roko/memory` exists but `.roko/learn` does
    /// not, indicating the workspace uses the legacy layout and should be
    /// migrated.
    pub fn open(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let roko_dir = root.join(".roko");
        if !roko_dir.is_dir() {
            bail!(
                "not a roko workspace: {} (missing .roko/ directory)",
                root.display()
            );
        }

        // Migration warning: .roko/memory exists but .roko/learn does not.
        let memory_dir = roko_dir.join("memory");
        let learn_dir = roko_dir.join("learn");
        if memory_dir.is_dir() && !learn_dir.is_dir() {
            tracing::warn!(
                workspace = %root.display(),
                "workspace has .roko/memory/ but no .roko/learn/ — \
                 .roko/learn/ is now the canonical learning directory; \
                 run `roko init` or create .roko/learn/ to migrate"
            );
        }

        Ok(Self { root })
    }

    /// Create a new workspace by ensuring the `.roko/` directory layout exists.
    ///
    /// Creates `.roko/` and its standard subdirectories if they do not already exist.
    pub fn create(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let roko_dir = root.join(".roko");

        let dirs = [
            &roko_dir,
            &roko_dir.join("state"),
            &roko_dir.join("plans"),
            &roko_dir.join("runtime"),
            &roko_dir.join("memory"),
            &roko_dir.join("runs"),
            &roko_dir.join("config"),
            &roko_dir.join("cache"),
            &roko_dir.join("learn"),
            &roko_dir.join("prd"),
            &roko_dir.join("research"),
        ];

        for dir in &dirs {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creating directory {}", dir.display()))?;
        }

        Ok(Self { root })
    }

    /// Open an existing workspace, or create one if `.roko/` does not exist.
    pub fn open_or_create(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref();
        if root.join(".roko").is_dir() {
            Self::open(root)
        } else {
            Self::create(root)
        }
    }

    // ─── Path accessors ─────────────────────────────────────────────────────

    /// Workspace root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `.roko/` directory.
    #[must_use]
    pub fn roko_dir(&self) -> PathBuf {
        self.root.join(".roko")
    }

    /// `.roko/state/` — executor snapshots and persistent state.
    #[must_use]
    pub fn state_dir(&self) -> PathBuf {
        self.root.join(".roko/state")
    }

    /// `.roko/plans/` — plan artifacts, task files, reviews.
    #[must_use]
    pub fn plans_dir(&self) -> PathBuf {
        self.root.join(".roko/plans")
    }

    /// `.roko/runtime/` — runtime artifacts (PIDs, sockets).
    #[must_use]
    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join(".roko/runtime")
    }

    /// `.roko/memory/` — legacy memory artifacts retained for migration.
    #[must_use]
    pub fn memory_dir(&self) -> PathBuf {
        self.root.join(".roko/memory")
    }

    /// `.roko/runs/` — per-run metrics, traces, snapshots.
    #[must_use]
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join(".roko/runs")
    }

    /// `.roko/config/` — workspace-local config presets and overlays.
    #[must_use]
    pub fn config_dir(&self) -> PathBuf {
        self.root.join(".roko/config")
    }

    /// `.roko/cache/` — local runtime/cache artifacts.
    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join(".roko/cache")
    }

    /// `.roko/learn/` — learning artifacts (router, experiments, thresholds).
    #[must_use]
    pub fn learn_dir(&self) -> PathBuf {
        self.root.join(".roko/learn")
    }

    /// `.roko/episodes.jsonl` — agent episode log.
    #[must_use]
    pub fn episodes_path(&self) -> PathBuf {
        self.root.join(".roko/episodes.jsonl")
    }

    /// `.roko/signals.jsonl` — signal log.
    #[must_use]
    pub fn signals_path(&self) -> PathBuf {
        self.root.join(".roko/signals.jsonl")
    }

    /// `.roko/roko.log` — main log file.
    #[must_use]
    pub fn log_path(&self) -> PathBuf {
        self.root.join(".roko/roko.log")
    }

    /// `roko.toml` — workspace configuration file.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.root.join("roko.toml")
    }

    /// `.roko/prd/` — PRD storage.
    #[must_use]
    pub fn prd_dir(&self) -> PathBuf {
        self.root.join(".roko/prd")
    }

    /// `.roko/research/` — research artifacts.
    #[must_use]
    pub fn research_dir(&self) -> PathBuf {
        self.root.join(".roko/research")
    }

    /// `.roko/state/executor.json` — executor snapshot for resume.
    #[must_use]
    pub fn executor_snapshot_path(&self) -> PathBuf {
        self.root.join(".roko/state/executor.json")
    }

    /// `.roko/learn/gate-thresholds.json` — adaptive gate thresholds.
    #[must_use]
    pub fn gate_thresholds_path(&self) -> PathBuf {
        self.root.join(".roko/learn/gate-thresholds.json")
    }

    /// `.roko/learn/cascade-router.json` — cascade router state.
    #[must_use]
    pub fn cascade_router_path(&self) -> PathBuf {
        self.root.join(".roko/learn/cascade-router.json")
    }

    /// `.roko/learn/efficiency.jsonl` — per-turn efficiency events.
    #[must_use]
    pub fn efficiency_log_path(&self) -> PathBuf {
        self.root.join(".roko/learn/efficiency.jsonl")
    }

    /// `.roko/events.jsonl` — append-only runner event log.
    #[must_use]
    pub fn events_jsonl_path(&self) -> PathBuf {
        self.root.join(".roko/events.jsonl")
    }

    /// `.roko/state/run-state.json` — runner-owned resume state.
    #[must_use]
    pub fn run_state_path(&self) -> PathBuf {
        self.root.join(".roko/state/run-state.json")
    }

    /// `.roko/state/task-trackers.json` — per-task tracker state.
    #[must_use]
    pub fn task_trackers_path(&self) -> PathBuf {
        self.root.join(".roko/state/task-trackers.json")
    }

    /// `.roko/learn/playbooks/` — learned playbook store.
    #[must_use]
    pub fn playbooks_dir(&self) -> PathBuf {
        self.root.join(".roko/learn/playbooks")
    }

    /// `.roko/learn/archive/` — learning archive directory.
    #[must_use]
    pub fn archive_dir(&self) -> PathBuf {
        self.root.join(".roko/learn/archive")
    }

    /// `.roko/mcp.json` — workspace-local MCP configuration file.
    #[must_use]
    pub fn mcp_config_path(&self) -> PathBuf {
        self.root.join(".roko/mcp.json")
    }

    /// `.roko/runner-stderr.log` — approval-TUI stderr redirect.
    #[must_use]
    pub fn runner_stderr_log(&self) -> PathBuf {
        self.root.join(".roko/runner-stderr.log")
    }

    /// `.roko/learn/episodes.jsonl` — episode log under learn directory.
    #[must_use]
    pub fn learn_episodes_path(&self) -> PathBuf {
        self.root.join(".roko/learn/episodes.jsonl")
    }

    /// `.roko/engrams.jsonl` — the main engram log.
    #[must_use]
    pub fn engrams_path(&self) -> PathBuf {
        self.root.join(".roko/engrams.jsonl")
    }

    /// `.roko/serve.pid` — PID file for the `roko dev` / `roko serve` process.
    #[must_use]
    pub fn serve_pid_file(&self) -> PathBuf {
        self.root.join(".roko/serve.pid")
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    #[test]
    fn open_fails_without_roko_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let err = Workspace::open(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("not a roko workspace"));
    }

    #[test]
    fn create_makes_directory_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws = Workspace::create(tmp.path()).expect("create workspace");

        assert!(ws.roko_dir().is_dir());
        assert!(ws.state_dir().is_dir());
        assert!(ws.plans_dir().is_dir());
        assert!(ws.runtime_dir().is_dir());
        assert!(ws.memory_dir().is_dir());
        assert!(ws.runs_dir().is_dir());
        assert!(ws.config_dir().is_dir());
        assert!(ws.cache_dir().is_dir());
        assert!(ws.learn_dir().is_dir());
        assert!(ws.prd_dir().is_dir());
        assert!(ws.research_dir().is_dir());
    }

    #[test]
    fn open_succeeds_after_create() {
        let tmp = tempfile::tempdir().expect("tempdir");
        Workspace::create(tmp.path()).expect("create");
        let ws = Workspace::open(tmp.path()).expect("open existing");
        assert_eq!(ws.root(), tmp.path());
    }

    #[test]
    fn open_or_create_creates_when_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws = Workspace::open_or_create(tmp.path()).expect("open_or_create");
        assert!(ws.roko_dir().is_dir());
    }

    #[test]
    fn open_or_create_opens_when_existing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join(".roko")).expect("mkdir");
        let ws = Workspace::open_or_create(tmp.path()).expect("open_or_create");
        assert_eq!(ws.root(), tmp.path());
    }

    #[test]
    fn path_accessors_return_expected_paths() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws = Workspace::create(tmp.path()).expect("create");

        assert_eq!(ws.episodes_path(), tmp.path().join(".roko/episodes.jsonl"));
        assert_eq!(ws.signals_path(), tmp.path().join(".roko/signals.jsonl"));
        assert_eq!(ws.log_path(), tmp.path().join(".roko/roko.log"));
        assert_eq!(ws.config_path(), tmp.path().join("roko.toml"));
        assert_eq!(
            ws.executor_snapshot_path(),
            tmp.path().join(".roko/state/executor.json")
        );
        assert_eq!(
            ws.gate_thresholds_path(),
            tmp.path().join(".roko/learn/gate-thresholds.json")
        );
        assert_eq!(
            ws.cascade_router_path(),
            tmp.path().join(".roko/learn/cascade-router.json")
        );
        assert_eq!(
            ws.efficiency_log_path(),
            tmp.path().join(".roko/learn/efficiency.jsonl")
        );
        // New accessors added in task 004.
        assert_eq!(
            ws.events_jsonl_path(),
            tmp.path().join(".roko/events.jsonl")
        );
        assert_eq!(
            ws.run_state_path(),
            tmp.path().join(".roko/state/run-state.json")
        );
        assert_eq!(
            ws.task_trackers_path(),
            tmp.path().join(".roko/state/task-trackers.json")
        );
        assert_eq!(ws.playbooks_dir(), tmp.path().join(".roko/learn/playbooks"));
        assert_eq!(ws.archive_dir(), tmp.path().join(".roko/learn/archive"));
        assert_eq!(ws.mcp_config_path(), tmp.path().join(".roko/mcp.json"));
        assert_eq!(
            ws.runner_stderr_log(),
            tmp.path().join(".roko/runner-stderr.log")
        );
        assert_eq!(
            ws.learn_episodes_path(),
            tmp.path().join(".roko/learn/episodes.jsonl")
        );
        assert_eq!(ws.engrams_path(), tmp.path().join(".roko/engrams.jsonl"));
        assert_eq!(ws.serve_pid_file(), tmp.path().join(".roko/serve.pid"));
    }

    #[test]
    fn open_warns_on_legacy_memory_without_learn() {
        // This test verifies the code path is exercised. The tracing::warn
        // is emitted but not captured here — it is verified by integration
        // tests or manual observation.
        let tmp = tempfile::tempdir().expect("tempdir");
        let roko = tmp.path().join(".roko");
        std::fs::create_dir_all(roko.join("memory")).expect("mkdir memory");
        // No .roko/learn/ directory — should trigger warning path.
        let ws = Workspace::open(tmp.path()).expect("open succeeds");
        assert_eq!(ws.root(), tmp.path());
    }
}
