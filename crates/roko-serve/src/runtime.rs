//! Trait abstraction for CLI operations needed by the HTTP server.
//!
//! `roko-serve` depends on this trait rather than directly on `roko-cli`,
//! breaking the circular dependency. The CLI crate provides the concrete
//! implementation.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::bench::BenchConfigOverrides;

/// Token usage reported by an LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResultUsage {
    /// Number of input (prompt) tokens consumed.
    pub input_tokens: u64,
    /// Number of output (completion) tokens generated.
    pub output_tokens: u64,
}

/// Result of a single `run_once()` invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    /// Whether the overall run succeeded (all gates passed).
    pub success: bool,
    /// Final text output produced by the run, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
    /// Real token usage from the provider, when available.
    /// Gateway falls back to a character-based heuristic when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RunResultUsage>,
    /// Structured gate results collected during execution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate_results: Vec<RuntimeGateResult>,
}

/// Result of generating an implementation plan from a PRD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanGenerationResult {
    /// Root directory where plan artifacts were generated.
    pub plans_root: PathBuf,
    /// Specific plan directories or files that should be executed.
    pub plan_targets: Vec<PathBuf>,
    /// Plan-related files to attach to the job submission.
    pub artifacts: Vec<PathBuf>,
}

/// Structured gate result collected while executing a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeGateResult {
    /// Verify name.
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Human-readable gate detail.
    pub detail: String,
}

/// Result of executing an implementation plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanExecutionResult {
    /// Whether the execution completed successfully.
    pub success: bool,
    /// Text summary or stdout-like output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
    /// Structured gate results when the runtime can provide them.
    pub gate_results: Vec<RuntimeGateResult>,
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

/// Options for starting a SWE-bench run via the HTTP API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweBenchRunOptions {
    /// Optional path to a local JSONL dataset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_path: Option<PathBuf>,
    /// Agent mode (e.g. "gold", "empty", "command").
    #[serde(default = "default_agent_mode")]
    pub agent_mode: String,
    /// Maximum number of instances to run.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// Offset into the dataset.
    #[serde(default)]
    pub offset: usize,
    /// Whether to record learning episodes.
    #[serde(default)]
    pub record_learning: bool,
}

fn default_agent_mode() -> String {
    "gold".to_string()
}

fn default_batch_size() -> usize {
    10
}

/// Per-instance result from a SWE-bench run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct SweBenchInstanceResult {
    /// SWE-bench instance id.
    pub instance_id: String,
    /// Repository label.
    #[serde(default)]
    pub repo: String,
    /// Whether the patch was a valid unified diff.
    #[serde(default)]
    pub format_valid: bool,
    /// Whether `git apply --check` accepted the patch.
    #[serde(default)]
    pub apply_check: bool,
    /// Whether the test command passed.
    #[serde(default)]
    pub tests_passed: bool,
    /// Final proxy outcome.
    #[serde(default)]
    pub resolved: bool,
    /// Patch size in bytes.
    #[serde(default)]
    pub patch_bytes: usize,
    /// Wall-clock runtime in milliseconds.
    #[serde(default)]
    pub duration_ms: u64,
    /// Short failure reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

/// Result of a SWE-bench run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweBenchRunResult {
    /// Stable run id.
    pub run_id: String,
    /// Dataset label.
    #[serde(default)]
    pub dataset: String,
    /// Agent mode used.
    #[serde(default)]
    pub agent_mode: String,
    /// Number of instances evaluated.
    pub total: usize,
    /// Number of instances resolved.
    pub resolved: usize,
    /// Pass rate (resolved / total).
    pub pass_rate: f64,
    /// Per-instance results.
    #[serde(default)]
    pub instances: Vec<SweBenchInstanceResult>,
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
        Ok(RunResult {
            success: true,
            output_text: None,
            usage: None,
            gate_results: Vec::new(),
        })
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

    /// Run a single prompt with bench config overrides.
    ///
    /// The default ignores the overrides and delegates to `run_once`.
    /// The real CLI runtime applies model/backend overrides and can inspect
    /// the bench strategy hint carried in those overrides.
    async fn run_once_with_config(
        &self,
        workdir: &std::path::Path,
        prompt: &str,
        _overrides: &BenchConfigOverrides,
    ) -> anyhow::Result<RunResult> {
        self.run_once(workdir, prompt).await
    }

    /// Generate implementation plans from a PRD.
    ///
    /// Runtime implementations that know the real CLI internals should
    /// override this. The default is explicit so callers can fall back to a
    /// local synthetic plan without assuming every runtime supports PRD
    /// planning.
    async fn generate_plan_from_prd(
        &self,
        workdir: &std::path::Path,
        slug: &str,
        prd_path: &std::path::Path,
    ) -> anyhow::Result<PlanGenerationResult> {
        let _ = (workdir, slug, prd_path);
        anyhow::bail!("runtime does not support PRD plan generation")
    }

    /// Execute a plan target.
    ///
    /// The default delegates to `run_once()` with a plan-execution prompt so
    /// lightweight test runtimes and remote runtimes still have a functional
    /// path. The real CLI runtime overrides this with `PlanRunner`.
    async fn run_plan(
        &self,
        workdir: &std::path::Path,
        plan_target: &std::path::Path,
    ) -> anyhow::Result<PlanExecutionResult> {
        let prompt = format!(
            "Execute the implementation plan at {} in the current workspace. \
             Run the relevant gates and include changed files plus gate results in the response.",
            plan_target.display()
        );
        let result = self.run_once(workdir, &prompt).await?;
        Ok(PlanExecutionResult {
            success: result.success,
            output_text: result.output_text,
            gate_results: Vec::new(),
        })
    }

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

    /// Run a SWE-bench evaluation. Returns per-instance results.
    ///
    /// The default returns an error since not all runtimes support SWE-bench.
    async fn run_swe_bench(
        &self,
        _workdir: &std::path::Path,
        _options: SweBenchRunOptions,
    ) -> anyhow::Result<SweBenchRunResult> {
        anyhow::bail!("runtime does not support SWE-bench")
    }
}
