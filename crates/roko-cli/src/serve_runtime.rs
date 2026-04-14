//! [`CliRuntime`] implementation backed by the real CLI internals.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use roko_core::config::schema::RokoConfig;
use roko_serve::runtime::{CliRuntime, DashboardInfo, RepoInfo, RunResult, SessionStatusInfo};

use crate::config::{Config, RepoRegistry};
use crate::run::run_once;
use crate::status::SessionStatus;
use crate::tui::DashboardScaffold;

/// Concrete runtime that delegates to the real CLI functions.
pub struct RokoCliRuntime {
    config: Config,
    repo_registry: RepoRegistry,
}

impl RokoCliRuntime {
    #[must_use]
    pub fn new(config: Config, repo_registry: RepoRegistry) -> Self {
        Self {
            config,
            repo_registry,
        }
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

    fn resolve_repo_workdir(&self, repo_full_name: &str) -> Option<PathBuf> {
        self.repo_registry
            .find_by_full_name(repo_full_name)
            .map(|entry| entry.root.clone())
    }

    fn repo_roko_config(&self, repo_name: &str) -> Option<RokoConfig> {
        self.repo_registry
            .get(repo_name)
            .and_then(|entry| entry.roko_config.clone())
    }

    fn list_repos(&self) -> Vec<RepoInfo> {
        self.repo_registry
            .repos()
            .iter()
            .map(|entry| RepoInfo {
                name: entry.config.name.clone(),
                path: entry.root.clone(),
                branch: entry.config.branch.clone(),
            })
            .collect()
    }
}
