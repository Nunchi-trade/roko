//! [`CliRuntime`] implementation backed by the real CLI internals.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use roko_serve::runtime::{CliRuntime, DashboardInfo, RunResult, SessionStatusInfo};

use crate::config::Config;
use crate::run::run_once;
use crate::status::SessionStatus;
use crate::tui::DashboardScaffold;

/// Concrete runtime that delegates to the real CLI functions.
pub struct RokoCliRuntime {
    config: Config,
}

impl RokoCliRuntime {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn into_arc(self) -> Arc<dyn CliRuntime> {
        Arc::new(self)
    }
}

#[async_trait]
impl CliRuntime for RokoCliRuntime {
    async fn run_once(&self, workdir: &Path, prompt: &str) -> anyhow::Result<RunResult> {
        let report = run_once(workdir, &self.config, prompt).await?;
        Ok(RunResult {
            success: report.overall_success(),
        })
    }

    fn session_status(&self, workdir: PathBuf) -> SessionStatusInfo {
        let ss = SessionStatus::offline(workdir);
        SessionStatusInfo {
            session_id: ss.session_id,
            workdir: ss.workdir,
            daemon_running: ss.daemon_running,
            signal_count: ss.signal_count,
            episode_count: ss.episode_count,
            last_episode_passed: ss.last_episode_passed,
        }
    }

    fn dashboard_scaffold(&self, workdir: &Path) -> DashboardInfo {
        let scaffold = DashboardScaffold::new_in(workdir);
        DashboardInfo {
            rendered: format!("{scaffold:?}"),
        }
    }
}
