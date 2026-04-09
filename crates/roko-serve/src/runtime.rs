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

/// Snapshot of session status (mirrors `roko_cli::SessionStatus` fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatusInfo {
    pub session_id: Option<String>,
    pub workdir: PathBuf,
    pub daemon_running: bool,
    pub signal_count: Option<usize>,
    pub episode_count: Option<usize>,
    pub last_episode_passed: Option<bool>,
}

/// Opaque dashboard payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardInfo {
    pub rendered: String,
}

/// Trait that roko-serve calls for operations that live in roko-cli.
///
/// The HTTP server holds an `Arc<dyn CliRuntime>` and delegates to it
/// whenever a handler needs to invoke the CLI's universal loop, query
/// session status, or render the dashboard scaffold.
#[async_trait]
pub trait CliRuntime: Send + Sync + 'static {
    /// Run a single prompt through the universal loop.
    async fn run_once(
        &self,
        workdir: &std::path::Path,
        prompt: &str,
    ) -> anyhow::Result<RunResult>;

    /// Return current session status for the given workdir.
    fn session_status(&self, workdir: PathBuf) -> SessionStatusInfo;

    /// Return a dashboard scaffold rendering.
    fn dashboard_scaffold(&self, workdir: &std::path::Path) -> DashboardInfo;
}
