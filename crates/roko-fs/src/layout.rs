//! `.roko/` directory layout definition and path helpers.
//!
//! The canonical layout under a project root is:
//!
//! ```text
//! .roko/
//!   runtime/        # pid files, sockets, locks
//!   memory/         # episodes, playbook, skills
//!   plans/          # enrichment artifacts per plan
//!     {plan_id}/
//!   runs/           # per-run metrics, traces, snapshots
//!     {run_id}/
//!   state/          # orchestrator snapshots, event logs, session state
//!   config/         # config.toml, presets
//!   cache/          # cargo-target, context-pack-cache
//! ```
//!
//! [`RokoLayout`] exposes typed path helpers so call-sites never hard-code
//! the directory structure. [`LayoutVersion`] tracks on-disk format
//! migrations.

use std::path::{Path, PathBuf};

/// On-disk format version for the `.roko/` directory.
///
/// When the directory layout changes in a backwards-incompatible way, a
/// new variant is added here, and the migration code maps old versions to
/// the current one. The version is persisted in `.roko/VERSION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayoutVersion {
    /// Initial layout: `runtime/`, `memory/`, `plans/`, `runs/`,
    /// `state/`, `config/`, `cache/`.
    V1 = 1,
}

impl LayoutVersion {
    /// The most recent version. New directories are initialized to this.
    pub const CURRENT: Self = Self::V1;

    /// Parse from the numeric value stored in `.roko/VERSION`.
    ///
    /// Returns `None` for unrecognized values.
    #[must_use]
    pub const fn from_u32(n: u32) -> Option<Self> {
        match n {
            1 => Some(Self::V1),
            _ => None,
        }
    }

    /// The numeric value written to `.roko/VERSION`.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }
}

/// Path helpers for the `.roko/` directory tree.
///
/// All methods are pure path arithmetic — no I/O is performed. Call
/// [`RokoLayout::ensure_dirs`] to create the directory structure on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RokoLayout {
    /// Root of the `.roko/` directory (e.g. `/project/.roko`).
    root: PathBuf,
}

impl RokoLayout {
    /// Construct a layout rooted at `root` (the `.roko/` directory itself).
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Construct a layout for the given project directory.
    ///
    /// Equivalent to `RokoLayout::new(project_root.join(".roko"))`.
    #[must_use]
    pub fn for_project(project_root: impl AsRef<Path>) -> Self {
        Self::new(project_root.as_ref().join(".roko"))
    }

    /// Construct a layout for a named repo under the project's `.roko/repos/{name}/`.
    ///
    /// Repo-specific signals, episodes, state, etc. are stored in their own
    /// subdirectory so they don't intermingle with the global data.
    #[must_use]
    pub fn for_repo(project_root: impl AsRef<Path>, repo_name: &str) -> Self {
        Self::new(
            project_root
                .as_ref()
                .join(".roko")
                .join("repos")
                .join(repo_name),
        )
    }

    // ── root accessors ────────────────────────────────────────────────────

    /// The `.roko/` root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path to the VERSION file: `.roko/VERSION`.
    #[must_use]
    pub fn version_file(&self) -> PathBuf {
        self.root.join("VERSION")
    }

    // ── top-level subdirectories ──────────────────────────────────────────

    /// `.roko/runtime/` — pid files, sockets, locks.
    #[must_use]
    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }

    /// `.roko/memory/` — episodes, playbook, skills.
    #[must_use]
    pub fn memory_dir(&self) -> PathBuf {
        self.root.join("memory")
    }

    /// `.roko/plans/` — enrichment artifacts per plan.
    #[must_use]
    pub fn plans_dir(&self) -> PathBuf {
        self.root.join("plans")
    }

    /// `.roko/runs/` — per-run metrics, traces, snapshots.
    #[must_use]
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    /// `.roko/state/` — orchestrator snapshots, event logs, session state.
    #[must_use]
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    /// `.roko/config/` — config.toml, presets.
    #[must_use]
    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    /// `.roko/cache/` — cargo-target, context-pack-cache.
    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    // ── per-entity paths ─────────────────────────────────────────────────

    /// `.roko/plans/{plan_id}/` — enrichment artifacts for one plan.
    #[must_use]
    pub fn plan_dir(&self, plan_id: &str) -> PathBuf {
        self.plans_dir().join(plan_id)
    }

    /// `.roko/runs/{run_id}/` — data directory for one run.
    #[must_use]
    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.runs_dir().join(run_id)
    }

    /// `.roko/runs/{run_id}/metrics.jsonl` — metrics log for one run.
    #[must_use]
    pub fn run_metrics(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("metrics.jsonl")
    }

    /// `.roko/runs/{run_id}/traces/` — traces directory for one run.
    #[must_use]
    pub fn run_traces_dir(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("traces")
    }

    /// `.roko/memory/episodes.jsonl` — the main episodes log.
    #[must_use]
    pub fn episodes_path(&self) -> PathBuf {
        self.memory_dir().join("episodes.jsonl")
    }

    /// `.roko/memory/playbook.toml` — the active playbook.
    #[must_use]
    pub fn playbook_path(&self) -> PathBuf {
        self.memory_dir().join("playbook.toml")
    }

    /// `.roko/memory/skills/` — learned skills directory.
    #[must_use]
    pub fn skills_dir(&self) -> PathBuf {
        self.memory_dir().join("skills")
    }

    /// `.roko/config/config.toml` — main configuration file.
    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    /// `.roko/cache/cargo-target/` — shared cargo target directory.
    #[must_use]
    pub fn cargo_target_dir(&self) -> PathBuf {
        self.cache_dir().join("cargo-target")
    }

    /// `.roko/cache/context-pack-cache/` — cached context packs.
    #[must_use]
    pub fn context_pack_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("context-pack-cache")
    }

    /// `.roko/state/executor.json` — executor snapshot for crash recovery.
    #[must_use]
    pub fn executor_snapshot(&self) -> PathBuf {
        self.state_dir().join("executor.json")
    }

    /// `.roko/state/events.json` — event log snapshot for crash recovery.
    #[must_use]
    pub fn event_log_snapshot(&self) -> PathBuf {
        self.state_dir().join("events.json")
    }

    /// `.roko/state/sessions/` — per-session directories.
    #[must_use]
    pub fn sessions_dir(&self) -> PathBuf {
        self.state_dir().join("sessions")
    }

    /// `.roko/state/sessions/{session_id}/` — one session's state.
    #[must_use]
    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_dir().join(session_id)
    }

    /// `.roko/runtime/roko.pid` — the PID file for the running process.
    #[must_use]
    pub fn pid_file(&self) -> PathBuf {
        self.runtime_dir().join("roko.pid")
    }

    /// `.roko/runtime/roko.lock` — the advisory lock file.
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.runtime_dir().join("roko.lock")
    }

    // ── I/O helpers ──────────────────────────────────────────────────────

    /// All top-level subdirectories that should exist for a working layout.
    #[must_use]
    pub fn top_level_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.runtime_dir(),
            self.memory_dir(),
            self.plans_dir(),
            self.runs_dir(),
            self.state_dir(),
            self.config_dir(),
            self.cache_dir(),
        ]
    }

    /// Create all top-level subdirectories and write the VERSION file.
    ///
    /// Idempotent: re-running on an existing layout is a no-op.
    ///
    /// # Errors
    ///
    /// Returns an error if any directory cannot be created or the VERSION
    /// file cannot be written.
    pub async fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in &self.top_level_dirs() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let version_path = self.version_file();
        if !version_path.exists() {
            tokio::fs::write(&version_path, LayoutVersion::CURRENT.as_u32().to_string()).await?;
        }
        Ok(())
    }

    /// Read the layout version from the VERSION file.
    ///
    /// Returns `None` if the file does not exist or contains an
    /// unrecognized version number.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure (other than not-found).
    pub async fn read_version(&self) -> std::io::Result<Option<LayoutVersion>> {
        let path = self.version_file();
        match tokio::fs::read_to_string(&path).await {
            Ok(contents) => {
                let parsed = contents.trim().parse::<u32>().ok();
                Ok(parsed.and_then(LayoutVersion::from_u32))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn layout_root_is_what_was_passed() {
        let layout = RokoLayout::new("/tmp/.roko");
        assert_eq!(layout.root(), Path::new("/tmp/.roko"));
    }

    #[test]
    fn for_project_appends_dot_roko() {
        let layout = RokoLayout::for_project("/home/user/project");
        assert_eq!(layout.root(), Path::new("/home/user/project/.roko"));
    }

    #[test]
    fn top_level_dirs_returns_seven() {
        let layout = RokoLayout::new("/tmp/.roko");
        let dirs = layout.top_level_dirs();
        assert_eq!(dirs.len(), 7);
        assert!(dirs.contains(&PathBuf::from("/tmp/.roko/runtime")));
        assert!(dirs.contains(&PathBuf::from("/tmp/.roko/memory")));
        assert!(dirs.contains(&PathBuf::from("/tmp/.roko/plans")));
        assert!(dirs.contains(&PathBuf::from("/tmp/.roko/runs")));
        assert!(dirs.contains(&PathBuf::from("/tmp/.roko/state")));
        assert!(dirs.contains(&PathBuf::from("/tmp/.roko/config")));
        assert!(dirs.contains(&PathBuf::from("/tmp/.roko/cache")));
    }

    #[test]
    fn plan_dir_is_under_plans() {
        let layout = RokoLayout::new("/p/.roko");
        assert_eq!(
            layout.plan_dir("plan-42"),
            PathBuf::from("/p/.roko/plans/plan-42")
        );
    }

    #[test]
    fn run_dir_and_children() {
        let layout = RokoLayout::new("/p/.roko");
        assert_eq!(layout.run_dir("r1"), PathBuf::from("/p/.roko/runs/r1"));
        assert_eq!(
            layout.run_metrics("r1"),
            PathBuf::from("/p/.roko/runs/r1/metrics.jsonl")
        );
        assert_eq!(
            layout.run_traces_dir("r1"),
            PathBuf::from("/p/.roko/runs/r1/traces")
        );
    }

    #[test]
    fn memory_paths() {
        let layout = RokoLayout::new("/x/.roko");
        assert_eq!(
            layout.episodes_path(),
            PathBuf::from("/x/.roko/memory/episodes.jsonl")
        );
        assert_eq!(
            layout.playbook_path(),
            PathBuf::from("/x/.roko/memory/playbook.toml")
        );
        assert_eq!(layout.skills_dir(), PathBuf::from("/x/.roko/memory/skills"));
    }

    #[test]
    fn config_and_cache_paths() {
        let layout = RokoLayout::new("/c/.roko");
        assert_eq!(
            layout.config_file(),
            PathBuf::from("/c/.roko/config/config.toml")
        );
        assert_eq!(
            layout.cargo_target_dir(),
            PathBuf::from("/c/.roko/cache/cargo-target")
        );
        assert_eq!(
            layout.context_pack_cache_dir(),
            PathBuf::from("/c/.roko/cache/context-pack-cache")
        );
    }

    #[test]
    fn runtime_paths() {
        let layout = RokoLayout::new("/r/.roko");
        assert_eq!(
            layout.pid_file(),
            PathBuf::from("/r/.roko/runtime/roko.pid")
        );
        assert_eq!(
            layout.lock_file(),
            PathBuf::from("/r/.roko/runtime/roko.lock")
        );
    }

    #[test]
    fn version_file_path() {
        let layout = RokoLayout::new("/v/.roko");
        assert_eq!(layout.version_file(), PathBuf::from("/v/.roko/VERSION"));
    }

    #[tokio::test]
    async fn ensure_dirs_creates_all_directories() {
        let tmp = TempDir::new().expect("tempdir");
        let layout = RokoLayout::for_project(tmp.path());
        layout.ensure_dirs().await.expect("ensure_dirs");

        for dir in &layout.top_level_dirs() {
            assert!(dir.is_dir(), "directory should exist: {dir:?}");
        }
        assert!(layout.version_file().exists(), "VERSION file should exist");
    }

    #[tokio::test]
    async fn ensure_dirs_is_idempotent() {
        let tmp = TempDir::new().expect("tempdir");
        let layout = RokoLayout::for_project(tmp.path());
        layout.ensure_dirs().await.expect("first call");
        layout.ensure_dirs().await.expect("second call");

        let version = layout.read_version().await.expect("read version");
        assert_eq!(version, Some(LayoutVersion::CURRENT));
    }

    #[tokio::test]
    async fn read_version_returns_none_for_missing() {
        let tmp = TempDir::new().expect("tempdir");
        let layout = RokoLayout::for_project(tmp.path());
        let version = layout.read_version().await.expect("read version");
        assert_eq!(version, None);
    }

    #[tokio::test]
    async fn read_version_round_trips() {
        let tmp = TempDir::new().expect("tempdir");
        let layout = RokoLayout::for_project(tmp.path());
        layout.ensure_dirs().await.expect("ensure dirs");

        let version = layout.read_version().await.expect("read version");
        assert_eq!(version, Some(LayoutVersion::V1));
    }

    #[test]
    fn layout_version_from_u32() {
        assert_eq!(LayoutVersion::from_u32(1), Some(LayoutVersion::V1));
        assert_eq!(LayoutVersion::from_u32(0), None);
        assert_eq!(LayoutVersion::from_u32(999), None);
    }

    #[test]
    fn layout_version_as_u32() {
        assert_eq!(LayoutVersion::V1.as_u32(), 1);
    }

    #[test]
    fn layout_version_current_is_v1() {
        assert_eq!(LayoutVersion::CURRENT, LayoutVersion::V1);
    }
}
