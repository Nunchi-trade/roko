//! Cloud execution helpers for `code-implementer`.
//!
//! This module owns the ephemeral clone / branch / commit / push / PR flow
//! used by the deployed code-implementer worker.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use roko_agent::mcp::{McpClient, StdioTransport};
use roko_core::obs::MetricRegistry;
use serde::Deserialize;
use serde_json::json;

use crate::config::Config;
use crate::serve::deploy::CloudExecutionConfig;
use crate::PlanRunner;

/// Cloud execution parameters for a single plan run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudExecution {
    /// GitHub repository owner.
    pub owner: String,
    /// GitHub repository name.
    pub repo: String,
    /// GitHub token used for clone/push and MCP auth.
    pub github_token: String,
    /// Plan slug used for branch naming.
    pub plan_slug: String,
    /// Base branch to target when opening the implementation PR.
    pub base_branch: String,
    /// Human-readable implementation PR title.
    pub pr_title: String,
    /// Human-readable implementation PR body.
    pub pr_body: String,
    /// Command used to spawn the GitHub MCP server.
    pub github_mcp_command: String,
    /// Extra args passed to the GitHub MCP server command.
    pub github_mcp_args: Vec<String>,
}

impl CloudExecution {
    /// Branch name used for implementation work.
    #[must_use]
    pub fn branch_name(&self) -> String {
        format!("impl/{}", self.plan_slug)
    }

    /// Build the clone URL with embedded token authentication.
    #[must_use]
    pub fn clone_url(&self) -> String {
        format!(
            "https://x-access-token:{}@github.com/{}/{}.git",
            self.github_token, self.owner, self.repo
        )
    }

    /// Build the HTTPS push URL with embedded token authentication.
    #[must_use]
    pub fn push_url(&self) -> String {
        self.clone_url()
    }
}

/// Parse cloud execution params from a template invocation.
#[derive(Debug, Deserialize)]
pub struct CloudExecutionParams {
    /// Repository owner.
    pub github_owner: String,
    /// Repository name.
    pub github_repo: String,
    /// GitHub token.
    pub github_token: String,
    /// Plan slug / branch slug.
    pub plan_slug: String,
    /// Optional base branch. Defaults to `main`.
    #[serde(default)]
    pub base_branch: Option<String>,
    /// Optional PR title override.
    #[serde(default)]
    pub pr_title: Option<String>,
    /// Optional PR body override.
    #[serde(default)]
    pub pr_body: Option<String>,
    /// Optional plan directory relative to the cloned workspace.
    #[serde(default)]
    pub plan_dir: Option<PathBuf>,
    /// Optional workspace root directory. Defaults to `/tmp/roko-workspace`.
    #[serde(default)]
    pub workspace_dir: Option<PathBuf>,
    /// Optional GitHub MCP command override.
    #[serde(default)]
    pub github_mcp_command: Option<String>,
    /// Optional GitHub MCP args override.
    #[serde(default)]
    pub github_mcp_args: Option<Vec<String>>,
}

impl CloudExecutionParams {
    /// Parse execution parameters from a flat string map.
    pub fn from_map(params: &HashMap<String, String>) -> Result<Self> {
        let (github_owner, github_repo) = match (
            params.get("github_owner").cloned(),
            params.get("github_repo").cloned(),
        ) {
            (Some(owner), Some(repo)) => (owner, repo),
            _ => parse_owner_repo(
                params
                    .get("repo_url")
                    .ok_or_else(|| anyhow!("missing required params: github_owner/github_repo or repo_url"))?,
            )?,
        };

        Ok(Self {
            github_owner,
            github_repo,
            github_token: params
                .get("github_token")
                .cloned()
                .or_else(|| std::env::var("GITHUB_TOKEN").ok())
                .ok_or_else(|| anyhow!("missing required param: github_token"))?,
            plan_slug: params
                .get("plan_slug")
                .cloned()
                .ok_or_else(|| anyhow!("missing required param: plan_slug"))?,
            base_branch: params.get("base_branch").cloned(),
            pr_title: params.get("pr_title").cloned(),
            pr_body: params.get("pr_body").cloned(),
            plan_dir: params.get("plan_dir").cloned().map(PathBuf::from),
            workspace_dir: params
                .get("workspace_dir")
                .cloned()
                .map(PathBuf::from),
            github_mcp_command: params.get("github_mcp_command").cloned(),
            github_mcp_args: params
                .get("github_mcp_args")
                .map(|args| {
                    args.split_whitespace()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                }),
        })
    }

    /// Convert parsed params into a concrete execution configuration.
    #[must_use]
    pub fn into_execution(&self) -> CloudExecution {
        let branch = format!("impl/{}", self.plan_slug);
        CloudExecution {
            owner: self.github_owner.clone(),
            repo: self.github_repo.clone(),
            github_token: self.github_token.clone(),
            plan_slug: self.plan_slug.clone(),
            base_branch: self.base_branch.clone().unwrap_or_else(|| "main".to_string()),
            pr_title: self
                .pr_title
                .clone()
                .unwrap_or_else(|| format!("Implement {}", self.plan_slug)),
            pr_body: self.pr_body.clone().unwrap_or_else(|| {
                format!(
                    "Automated implementation for plan `{}`.\n\nBranch: `{}`",
                    self.plan_slug, branch
                )
            }),
            github_mcp_command: self
                .github_mcp_command
                .clone()
                .unwrap_or_else(|| "roko-mcp-github".to_string()),
            github_mcp_args: self.github_mcp_args.clone().unwrap_or_default(),
        }
    }

    /// Plan directory relative to the cloned workspace.
    #[must_use]
    pub fn plan_dir(&self) -> PathBuf {
        self.plan_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("plans/{}", self.plan_slug)))
    }

    /// Resolve the workspace root using the configured default when absent.
    #[must_use]
    pub fn workspace_dir(&self) -> PathBuf {
        self.workspace_dir
            .clone()
            .unwrap_or_else(|| CloudExecutionConfig::default().workspace_dir)
    }
}

fn parse_owner_repo(repo_url: &str) -> Result<(String, String)> {
    let trimmed = repo_url.trim().trim_end_matches(".git");
    if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        let mut parts = rest.splitn(2, '/');
        let owner = parts.next().unwrap_or_default();
        let repo = parts.next().unwrap_or_default();
        if !owner.is_empty() && !repo.is_empty() {
            return Ok((owner.to_string(), repo.to_string()));
        }
    }
    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        let mut parts = rest.splitn(2, '/');
        let owner = parts.next().unwrap_or_default();
        let repo = parts.next().unwrap_or_default();
        if !owner.is_empty() && !repo.is_empty() {
            return Ok((owner.to_string(), repo.to_string()));
        }
    }
    bail!("unable to parse GitHub owner/repo from repo_url: {repo_url}");
}

/// Clone the repository into the target workspace.
pub async fn git_clone(workspace: &Path, execution: &CloudExecution) -> Result<()> {
    let url = execution.clone_url();
    if let Some(parent) = workspace.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }

    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", &url])
        .arg(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .context("spawn git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git clone failed: {}", stderr.replace(&execution.github_token, "***"));
    }

    Ok(())
}

/// Create and switch to the implementation branch.
pub async fn git_checkout_new_branch(workspace: &Path, branch: &str) -> Result<()> {
    let output = tokio::process::Command::new("git")
        .args(["checkout", "-b", branch])
        .current_dir(workspace)
        .output()
        .await
        .context("spawn git checkout -b")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git checkout -b {branch} failed: {stderr}");
    }

    Ok(())
}

/// Stage and commit the current workspace state.
pub async fn git_commit(workspace: &Path, message: &str) -> Result<()> {
    let add_output = tokio::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(workspace)
        .output()
        .await
        .context("spawn git add -A")?;

    if !add_output.status.success() {
        let stderr = String::from_utf8_lossy(&add_output.stderr);
        bail!("git add -A failed: {stderr}");
    }

    let diff_output = tokio::process::Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(workspace)
        .output()
        .await
        .context("spawn git diff --cached")?;

    if diff_output.status.success() {
        bail!("nothing to commit (working tree clean)");
    }

    let output = tokio::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(workspace)
        .env("GIT_AUTHOR_NAME", "roko")
        .env("GIT_AUTHOR_EMAIL", "roko@nunchi.dev")
        .env("GIT_COMMITTER_NAME", "roko")
        .env("GIT_COMMITTER_EMAIL", "roko@nunchi.dev")
        .output()
        .await
        .context("spawn git commit")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git commit failed: {stderr}");
    }

    Ok(())
}

/// Push the implementation branch to origin.
pub async fn git_push(workspace: &Path, branch: &str, execution: &CloudExecution) -> Result<()> {
    let output = tokio::process::Command::new("git")
        .args(["push", "origin", branch])
        .current_dir(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .context("spawn git push")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git push failed: {}", stderr.replace(&execution.github_token, "***"));
    }

    Ok(())
}

/// Remove the workspace directory after execution.
pub async fn git_cleanup(workspace: &Path) -> Result<()> {
    match tokio::fs::remove_dir_all(workspace).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("remove {}", workspace.display())),
    }
}

/// Call the GitHub MCP `github.create_pr` tool.
pub async fn github_create_pr(workspace: &Path, execution: &CloudExecution) -> Result<String> {
    let transport = StdioTransport::spawn(
        &execution.github_mcp_command,
        &execution.github_mcp_args,
    )
    .map_err(|err| anyhow!("spawn GitHub MCP server: {err}"))?;
    let client = McpClient::new(transport);
    client
        .initialize()
        .await
        .map_err(|err| anyhow!("initialize GitHub MCP server: {err}"))?;

    let response = client
        .call_tool(
            "github.create_pr",
            json!({
                "owner": execution.owner,
                "repo": execution.repo,
                "title": execution.pr_title,
                "body": execution.pr_body,
                "head": execution.branch_name(),
                "base": execution.base_branch,
            }),
        )
        .await
        .map_err(|err| anyhow!("github.create_pr call failed: {err}"))?;

    let text = response
        .content
        .iter()
        .find_map(|content| content.text.clone())
        .ok_or_else(|| anyhow!("github.create_pr returned no text content"))?;

    tokio::fs::write(workspace.join(".roko").join("implementation-pr.json"), &text)
        .await
        .ok();

    Ok(text)
}

/// Execute the cloud code-implementer flow end-to-end.
pub async fn run_code_implementer_cloud(
    params: &HashMap<String, String>,
) -> Result<super::handler::TaskResult> {
    let started = std::time::Instant::now();
    let request = CloudExecutionParams::from_map(params)?;
    let execution = request.into_execution();
    let workspace_root = request.workspace_dir();
    let workspace = workspace_root.join(&execution.repo);
    let plan_dir = workspace.join(request.plan_dir());

    if workspace.exists() {
        git_cleanup(&workspace).await.ok();
    }

    let result = async {
        git_clone(&workspace, &execution).await?;
        git_checkout_new_branch(&workspace, &execution.branch_name()).await?;

        let config_path = workspace.join("roko.toml");
        let config = Config::from_file(&config_path).unwrap_or_else(|_| Config::default());
        let metrics = Arc::new(MetricRegistry::new());
        let mut runner =
            PlanRunner::from_plans_dir(&plan_dir, &workspace, config, metrics, false).await?;
        runner.enable_cloud_execution(execution.clone());

        let report = runner.run_task_plans(&plan_dir).await?;
        let success = report.all_succeeded();
        let gate_verdicts = report
            .plans
            .first()
            .map(|plan| plan.gate_results.clone())
            .unwrap_or_default();

        if success {
            github_create_pr(&workspace, &execution).await?;
        }

        Ok(super::handler::TaskResult {
            success,
            episode_id: None,
            gate_verdicts,
            error: None,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }
    .await;

    git_cleanup(&workspace).await.ok();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_execution_branch_name_is_prefixed() {
        let execution = CloudExecution {
            owner: "nunchi".into(),
            repo: "roko".into(),
            github_token: "token".into(),
            plan_slug: "p07-autofix".into(),
            base_branch: "main".into(),
            pr_title: "title".into(),
            pr_body: "body".into(),
            github_mcp_command: "roko-mcp-github".into(),
            github_mcp_args: Vec::new(),
        };
        assert_eq!(execution.branch_name(), "impl/p07-autofix");
        assert!(execution.clone_url().contains("x-access-token:token@github.com/nunchi/roko.git"));
    }

    #[test]
    fn params_default_plan_dir() {
        let params = CloudExecutionParams {
            github_owner: "nunchi".into(),
            github_repo: "roko".into(),
            github_token: "token".into(),
            plan_slug: "slug".into(),
            base_branch: None,
            pr_title: None,
            pr_body: None,
            plan_dir: None,
            github_mcp_command: None,
            github_mcp_args: None,
        };
        assert_eq!(params.plan_dir(), PathBuf::from("plans/slug"));
    }

    #[test]
    fn parse_https_repo_url() {
        let (owner, repo) = parse_owner_repo("https://github.com/nunchi/roko.git").unwrap();
        assert_eq!(owner, "nunchi");
        assert_eq!(repo, "roko");
    }
}
