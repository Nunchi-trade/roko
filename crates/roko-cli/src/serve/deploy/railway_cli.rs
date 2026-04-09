//! Railway CLI deploy backend.
//!
//! Implements [`DeployBackend`] by shelling out to the `railway` CLI tool.
//! Useful when the user has the Railway CLI installed and authenticated.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use tokio::process::Command;
use tracing::{debug, info};

use super::{DeployBackend, DeploySpec, Deployment, DeploymentStatus};

/// Deploy backend that shells out to the `railway` CLI.
pub struct RailwayCliBackend {
    project_id: Option<String>,
    environment_id: Option<String>,
}

impl RailwayCliBackend {
    /// Create a new Railway CLI backend with optional project/environment IDs.
    pub const fn new(project_id: Option<String>, environment_id: Option<String>) -> Self {
        Self {
            project_id,
            environment_id,
        }
    }

    /// Run a railway CLI command and return stdout.
    async fn railway_cmd(&self, args: &[&str]) -> Result<String> {
        let mut cmd = Command::new("railway");
        cmd.args(args);

        if let Some(ref pid) = self.project_id {
            cmd.env("RAILWAY_PROJECT_ID", pid);
        }
        if let Some(ref eid) = self.environment_id {
            cmd.env("RAILWAY_ENVIRONMENT_ID", eid);
        }

        debug!(args = ?args, "running railway CLI");

        let output = cmd.output().await.context("failed to run railway CLI")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("railway {} failed: {stderr}", args.join(" "));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

#[async_trait]
impl DeployBackend for RailwayCliBackend {
    async fn deploy(&self, spec: &DeploySpec) -> Result<Deployment> {
        // Create service
        let create_out = self
            .railway_cmd(&["service", "create", "--name", &spec.name, "--json"])
            .await
            .context("railway service create failed")?;

        let create_json: Value = serde_json::from_str(&create_out)
            .unwrap_or_else(|_| serde_json::json!({ "id": create_out.trim() }));

        let service_id = create_json["id"]
            .as_str()
            .unwrap_or_else(|| create_out.trim())
            .to_string();

        info!(%service_id, name = %spec.name, "created Railway service via CLI");

        // Set environment variables
        for (key, value) in &spec.env_vars {
            self.railway_cmd(&[
                "variables",
                "set",
                &format!("{key}={value}"),
                "--service",
                &service_id,
            ])
            .await
            .context(format!("failed to set variable {key}"))?;
        }

        // Deploy with image
        self.railway_cmd(&[
            "up",
            "--image",
            &spec.image,
            "--service",
            &service_id,
            "--detach",
        ])
        .await
        .context("railway up failed")?;

        Ok(Deployment {
            id: service_id,
            name: spec.name.clone(),
            status: DeploymentStatus::Creating,
            url: None,
            created_at: Utc::now(),
        })
    }

    async fn status(&self, deployment_id: &str) -> Result<DeploymentStatus> {
        let out = self
            .railway_cmd(&["status", "--service", deployment_id, "--json"])
            .await?;

        let json: Value = serde_json::from_str(&out)?;
        let status_str = json["status"].as_str().unwrap_or("UNKNOWN");

        match status_str {
            "SUCCESS" | "READY" | "RUNNING" => {
                let url = json["url"]
                    .as_str()
                    .map(|u| {
                        if u.starts_with("http") {
                            u.to_string()
                        } else {
                            format!("https://{u}")
                        }
                    })
                    .unwrap_or_default();
                Ok(DeploymentStatus::Ready { url })
            }
            "BUILDING" => Ok(DeploymentStatus::Building),
            "CRASHED" | "FAILED" => Ok(DeploymentStatus::Failed {
                reason: format!("Railway service {status_str}"),
            }),
            "REMOVED" => Ok(DeploymentStatus::TornDown),
            // DEPLOYING, INITIALIZING, and any unknown status
            _ => Ok(DeploymentStatus::Deploying),
        }
    }

    async fn teardown(&self, deployment_id: &str) -> Result<()> {
        self.railway_cmd(&["service", "delete", "--service", deployment_id, "--yes"])
            .await
            .context("railway service delete failed")?;
        info!(%deployment_id, "torn down Railway service via CLI");
        Ok(())
    }

    async fn logs(&self, deployment_id: &str, tail: usize) -> Result<Vec<String>> {
        let out = self
            .railway_cmd(&[
                "logs",
                "--service",
                deployment_id,
                "--lines",
                &tail.to_string(),
            ])
            .await?;
        Ok(out.lines().map(String::from).collect())
    }
}
