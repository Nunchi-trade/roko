//! Workspace layout abstraction.
//!
//! Provides a [`Workspace`] struct that validates and provides typed access to
//! the `.roko/` directory hierarchy. This is the canonical way to discover paths
//! for state, plans, runtime, learning, and other workspace artifacts.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// Represents a validated Roko workspace rooted at a directory containing `.roko/`.
#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Open an existing workspace by validating that `.roko/` exists under `root`.
    ///
    /// Returns an error if the `.roko/` directory does not exist.
    pub fn open(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let roko_dir = root.join(".roko");
        if !roko_dir.is_dir() {
            bail!(
                "not a roko workspace: {} (missing .roko/ directory)",
                root.display()
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
}

#[cfg(test)]
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
    }
}
