//! Trait abstraction for CLI operations needed by the HTTP server.
//!
//! `roko-serve` depends on this trait rather than directly on `roko-cli`,
//! breaking the circular dependency. The CLI crate provides the concrete
//! implementation.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Result of a single `run_once()` invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    /// Whether the overall run succeeded (all gates passed).
    pub success: bool,
}

/// Summary info for a configured repository, used to give agents
/// cross-repo context during dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    /// Human-readable repo name (matches the `[[repos]]` entry name).
    pub name: String,
    /// Filesystem path to the repository root.
    pub path: PathBuf,
    /// Branch tracked for this repo.
    pub branch: String,
}

/// Snapshot of session status (mirrors `roko_cli::SessionStatus` fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatusInfo {
    /// Active session identifier when a daemon-backed session exists.
    pub session_id: Option<String>,
    /// Repository working directory used to resolve local `.roko/` state.
    pub workdir: PathBuf,
    /// Whether the background daemon is currently running.
    pub daemon_running: bool,
    /// Number of known signals, if available from the runtime implementation.
    pub signal_count: Option<usize>,
    /// Number of recorded episodes, if available from the runtime implementation.
    pub episode_count: Option<usize>,
    /// Whether the latest episode passed, if the runtime can determine it.
    pub last_episode_passed: Option<bool>,
}

/// Opaque dashboard payload rendered by the CLI dashboard scaffold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardInfo {
    /// Pre-rendered textual dashboard output.
    pub rendered: String,
}

/// No-op runtime used in tests.
#[cfg(test)]
pub struct NoOpRuntime;

#[cfg(test)]
#[async_trait]
impl CliRuntime for NoOpRuntime {
    async fn run_once(
        &self,
        _workdir: &std::path::Path,
        _prompt: &str,
    ) -> anyhow::Result<RunResult> {
        Ok(RunResult { success: true })
    }

    fn session_status(&self, workdir: PathBuf) -> SessionStatusInfo {
        SessionStatusInfo {
            session_id: None,
            workdir,
            daemon_running: false,
            signal_count: None,
            episode_count: None,
            last_episode_passed: None,
        }
    }

    fn dashboard_scaffold(&self, _workdir: &std::path::Path) -> DashboardInfo {
        DashboardInfo {
            rendered: String::new(),
        }
    }
}

/// Trait that roko-serve calls for operations that live in roko-cli.
///
/// The HTTP server holds an `Arc<dyn CliRuntime>` and delegates to it
/// whenever a handler needs to invoke the CLI's universal loop, query
/// session status, or render the dashboard scaffold.
#[async_trait]
pub trait CliRuntime: Send + Sync + 'static {
    /// Run a single prompt through the universal loop.
    async fn run_once(&self, workdir: &std::path::Path, prompt: &str) -> anyhow::Result<RunResult>;

    /// Return current session status for the given workdir.
    fn session_status(&self, workdir: PathBuf) -> SessionStatusInfo;

    /// Return a dashboard scaffold rendering.
    fn dashboard_scaffold(&self, workdir: &std::path::Path) -> DashboardInfo;

    /// Resolve the working directory for a repo identified by its full name
    /// (e.g. from a webhook `repository.full_name`). Returns `None` when the
    /// repo is not configured.
    fn resolve_repo_workdir(&self, repo_full_name: &str) -> Option<PathBuf> {
        let _ = repo_full_name;
        None
    }

    /// Return the merged `RokoConfig` for a named repo, applying per-repo
    /// overrides on top of the global config. Returns `None` when the repo
    /// is not configured.
    fn repo_roko_config(&self, _repo_name: &str) -> Option<roko_core::config::schema::RokoConfig> {
        None
    }

    /// Return a list of all configured repositories. Used to inject
    /// cross-repo context into agent system prompts during dispatch.
    fn list_repos(&self) -> Vec<RepoInfo> {
        Vec::new()
    }
}
