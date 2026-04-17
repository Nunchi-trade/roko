//! Pluggable deploy backends for creating worker instances.
//!
//! The [`DeployBackend`] trait abstracts over different deployment targets
//! (Railway API, Railway CLI, manual Dockerfile bundles). The [`create_backend`]
//! factory reads config to pick the right implementation.

pub mod manual;
pub mod railway_api;
pub mod railway_cli;

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Specification for deploying a worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploySpec {
    /// Human-readable service name.
    pub name: String,
    /// Docker image to deploy.
    pub image: String,
    /// Environment variables to set on the service.
    pub env_vars: HashMap<String, String>,
    /// Target region (optional).
    pub region: Option<String>,
}

/// Runtime configuration for cloud execution workers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudExecutionConfig {
    /// Workspace directory used for cloning and working copies.
    #[serde(default = "default_cloud_workspace_dir")]
    pub workspace_dir: PathBuf,
    /// GitHub token used for cloning and pushing.
    #[serde(default)]
    pub github_token: String,
    /// Maximum number of tasks to run in parallel.
    #[serde(default = "default_cloud_max_parallel")]
    pub max_parallel: usize,
    /// Cost budget in cents.
    #[serde(default = "default_cloud_cost_budget_cents")]
    pub cost_budget_cents: u64,
    /// Maximum allowed execution time in seconds.
    #[serde(default = "default_cloud_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_cloud_workspace_dir() -> PathBuf {
    PathBuf::from("/tmp/roko-workspace")
}

const fn default_cloud_max_parallel() -> usize {
    2
}

const fn default_cloud_cost_budget_cents() -> u64 {
    5_000
}

const fn default_cloud_timeout_secs() -> u64 {
    3_600
}

impl Default for CloudExecutionConfig {
    fn default() -> Self {
        Self {
            workspace_dir: default_cloud_workspace_dir(),
            github_token: String::new(),
            max_parallel: default_cloud_max_parallel(),
            cost_budget_cents: default_cloud_cost_budget_cents(),
            timeout_secs: default_cloud_timeout_secs(),
        }
    }
}

/// A live or completed deployment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    /// Unique deployment ID.
    pub id: String,
    /// Service name.
    pub name: String,
    /// Current status.
    pub status: DeploymentStatus,
    /// Public URL (available when Ready).
    pub url: Option<String>,
    /// When the deployment was created.
    pub created_at: DateTime<Utc>,
}

/// Status of a deployment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DeploymentStatus {
    /// Service is being created.
    Creating,
    /// Image is being built.
    Building,
    /// Service is deploying.
    Deploying,
    /// Service is live and reachable.
    Ready {
        /// Public URL where the service is accessible.
        url: String,
    },
    /// Deployment failed.
    Failed {
        /// Human-readable failure reason.
        reason: String,
    },
    /// Service has been torn down.
    TornDown,
}

impl DeploymentStatus {
    /// Returns true if this is a terminal state.
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Ready { .. } | Self::Failed { .. } | Self::TornDown
        )
    }
}

/// Abstraction over deployment targets.
#[async_trait]
pub trait DeployBackend: Send + Sync {
    /// Create and start a new deployment.
    async fn deploy(&self, spec: &DeploySpec) -> Result<Deployment>;

    /// Check the current status of a deployment.
    async fn status(&self, deployment_id: &str) -> Result<DeploymentStatus>;

    /// Tear down (delete) a deployment.
    async fn teardown(&self, deployment_id: &str) -> Result<()>;

    /// Fetch recent logs from a deployment.
    async fn logs(&self, deployment_id: &str, tail: usize) -> Result<Vec<String>>;
}

/// Factory function: create a deploy backend from config.
///
/// # Errors
///
/// Returns an error if `backend_name` is unknown, or if the `railway-api`
/// backend is selected without the required API token.
pub fn create_backend(
    backend_name: &str,
    api_token: Option<&str>,
    project_id: Option<&str>,
    environment_id: Option<&str>,
) -> Result<Box<dyn DeployBackend>> {
    match backend_name {
        "railway-api" => {
            let token = api_token
                .ok_or_else(|| anyhow::anyhow!("railway-api backend requires railway_api_token"))?;
            Ok(Box::new(railway_api::RailwayApiBackend::new(
                token.to_string(),
                project_id.map(str::to_owned),
                environment_id.map(str::to_owned),
            )))
        }
        "railway-cli" => Ok(Box::new(railway_cli::RailwayCliBackend::new(
            project_id.map(String::from),
            environment_id.map(String::from),
        ))),
        "manual" | "" => Ok(Box::new(manual::ManualBackend::default())),
        other => anyhow::bail!("unknown deploy backend: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_execution_config_defaults_match_spec() {
        let cfg = CloudExecutionConfig::default();
        assert_eq!(cfg.workspace_dir, PathBuf::from("/tmp/roko-workspace"));
        assert!(cfg.github_token.is_empty());
        assert_eq!(cfg.max_parallel, 2);
        assert_eq!(cfg.cost_budget_cents, 5_000);
        assert_eq!(cfg.timeout_secs, 3_600);
    }
}
