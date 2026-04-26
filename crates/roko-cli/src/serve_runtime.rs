//! [`CliRuntime`] implementation backed by the real CLI internals.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use roko_core::config::schema::RokoConfig;
use roko_serve::runtime::{
    CliRuntime, DashboardInfo, PlanExecutionResult, PlanGenerationResult, RepoInfo, RunResult,
    RuntimeGateResult, SessionStatusInfo,
};

use crate::config::{Config, RepoRegistry};
use crate::orchestrate::PlanRunner;
use crate::prd;
use crate::run::run_once;
use crate::status::collect_session_status;
use crate::tui::DashboardScaffold;
use crate::workspace_paths;

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
            output_text: report.output_text,
            usage: None,
        })
    }

    async fn generate_plan_from_prd(
        &self,
        workdir: &Path,
        slug: &str,
        prd_path: &Path,
    ) -> anyhow::Result<PlanGenerationResult> {
        let plans_root = workspace_paths::plans_dir(workdir);
        let before = snapshot_plan_artifacts(&plans_root);
        let generated_root = prd::generate_plan_from_prd(slug, prd_path, false).await?;
        let after = snapshot_plan_artifacts(&generated_root);

        let mut plan_targets = changed_plan_targets(&generated_root, &before, &after);
        if plan_targets.is_empty() {
            plan_targets.push(generated_root.clone());
        }

        Ok(PlanGenerationResult {
            plans_root: generated_root,
            artifacts: plan_artifact_paths(&plan_targets),
            plan_targets,
        })
    }

    async fn run_plan(
        &self,
        workdir: &Path,
        plan_target: &Path,
    ) -> anyhow::Result<PlanExecutionResult> {
        let workdir = workdir.to_path_buf();
        let plan_target = plan_target.to_path_buf();
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || run_plan_on_local_runtime(workdir, plan_target, config))
            .await
            .map_err(|err| anyhow::anyhow!("plan execution worker failed: {err}"))?
    }

    fn session_status(&self, workdir: PathBuf) -> SessionStatusInfo {
        let ss = collect_session_status(&workdir);
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
            rendered: scaffold.render_overview_text(),
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

fn run_plan_on_local_runtime(
    workdir: PathBuf,
    plan_target: PathBuf,
    config: Config,
) -> anyhow::Result<PlanExecutionResult> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async move {
        let execution_root = prepare_plan_execution_root(&workdir, &plan_target)?;
        let metrics = Arc::new(roko_core::obs::MetricRegistry::new());
        roko_core::obs::register_standard_metrics(&metrics);
        let mut runner =
            PlanRunner::from_plans_dir(&execution_root, &workdir, config, metrics, false).await?;
        let report = runner.run(&execution_root).await?;
        let gate_results = report
            .plans
            .iter()
            .flat_map(|plan| {
                plan.gate_results
                    .iter()
                    .map(move |(gate, passed)| RuntimeGateResult {
                        gate: format!("{}:{gate}", plan.plan_id),
                        passed: *passed,
                        detail: if *passed {
                            "gate passed".to_string()
                        } else {
                            "gate failed".to_string()
                        },
                    })
            })
            .collect::<Vec<_>>();
        let output_text = Some(render_plan_execution_summary(&report));

        Ok(PlanExecutionResult {
            success: report.all_succeeded(),
            output_text,
            gate_results,
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlanArtifactSnapshot {
    modified: Option<SystemTime>,
    len: u64,
}

fn snapshot_plan_artifacts(root: &Path) -> BTreeMap<PathBuf, PlanArtifactSnapshot> {
    let mut out = BTreeMap::new();
    collect_plan_artifact_snapshots(root, root, &mut out);
    out
}

fn collect_plan_artifact_snapshots(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<PathBuf, PlanArtifactSnapshot>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_plan_artifact_snapshots(root, &path, out);
            continue;
        }
        if !meta.is_file() || !is_plan_artifact_file(&path) {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        out.insert(
            rel,
            PlanArtifactSnapshot {
                modified: meta.modified().ok(),
                len: meta.len(),
            },
        );
    }
}

fn is_plan_artifact_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "tasks.toml" || name == "plan.md" || path.extension().is_some_and(|ext| ext == "md")
}

fn changed_plan_targets(
    root: &Path,
    before: &BTreeMap<PathBuf, PlanArtifactSnapshot>,
    after: &BTreeMap<PathBuf, PlanArtifactSnapshot>,
) -> Vec<PathBuf> {
    let mut targets = BTreeSet::new();
    for (rel, snapshot) in after {
        let changed = before.get(rel).is_none_or(|old| old != snapshot);
        if !changed {
            continue;
        }
        if rel
            .file_name()
            .is_some_and(|name| name == "tasks.toml" || name == "plan.md")
        {
            if let Some(parent) = rel.parent() {
                targets.insert(root.join(parent));
            }
        } else {
            targets.insert(root.join(rel));
        }
    }
    targets.into_iter().collect()
}

fn plan_artifact_paths(targets: &[PathBuf]) -> Vec<PathBuf> {
    let mut artifacts = BTreeSet::new();
    for target in targets {
        if target.extension().is_some_and(|ext| ext == "md") {
            artifacts.insert(target.clone());
            continue;
        }
        artifacts.insert(target.join("plan.md"));
        artifacts.insert(target.join("tasks.toml"));
    }
    artifacts.into_iter().collect()
}

fn prepare_plan_execution_root(workdir: &Path, plan_target: &Path) -> anyhow::Result<PathBuf> {
    let absolute_target = if plan_target.is_absolute() {
        plan_target.to_path_buf()
    } else {
        workdir.join(plan_target)
    };

    if absolute_target.is_dir()
        && !absolute_target.join("plan.md").is_file()
        && !absolute_target.join("tasks.toml").is_file()
    {
        return Ok(absolute_target);
    }

    let plan_base = plan_target
        .file_stem()
        .or_else(|| plan_target.file_name())
        .and_then(|name| name.to_str())
        .map(sanitize_plan_base)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "job-plan".to_string());
    let run_root = workdir
        .join(".roko")
        .join("jobs")
        .join("plan-runs")
        .join(format!("{plan_base}-{}", unique_suffix()));
    std::fs::create_dir_all(&run_root)?;

    if absolute_target.is_dir() {
        copy_dir_recursive(&absolute_target, &run_root.join(&plan_base))?;
    } else if absolute_target.is_file() {
        let file_name = absolute_target.file_name().ok_or_else(|| {
            anyhow::anyhow!(
                "plan target has no file name: {}",
                absolute_target.display()
            )
        })?;
        std::fs::copy(&absolute_target, run_root.join(file_name))?;
    } else {
        anyhow::bail!("plan target does not exist: {}", absolute_target.display());
    }

    Ok(run_root)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let meta = entry.metadata()?;
        if meta.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if meta.is_file() {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn sanitize_plan_base(value: &str) -> String {
    let mut out = value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase();
    if out
        .chars()
        .next()
        .is_none_or(|ch| !ch.is_ascii_alphanumeric())
    {
        out.insert_str(0, "job-");
    }
    out
}

fn unique_suffix() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("{}-{millis}", std::process::id())
}

fn render_plan_execution_summary(report: &crate::orchestrate::OrchestrationReport) -> String {
    let mut lines = vec![format!(
        "plan execution {}: {} agent calls, {} gate runs",
        if report.all_succeeded() {
            "succeeded"
        } else {
            "failed"
        },
        report.total_agent_calls,
        report.total_gate_runs
    )];
    for plan in &report.plans {
        lines.push(format!(
            "{}: {} ({} agent calls, {} gate results)",
            plan.plan_id,
            if plan.succeeded {
                "succeeded"
            } else {
                "failed"
            },
            plan.agent_calls,
            plan.gate_results.len()
        ));
    }
    lines.join("\n")
}
