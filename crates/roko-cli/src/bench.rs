//! Native benchmark runners for `roko bench`.
//!
//! The first runner is a SWE-bench-style proxy harness. It is intentionally
//! local and deterministic by default so CI and developer machines can verify
//! the benchmark plumbing without Docker, Python, HuggingFace, or a live LLM.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use chrono::Utc;
use clap::ValueEnum;
use roko_core::{ConfigHash, TaskMetric};
use roko_learn::cfactor::CFactor;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, GateVerdict, Usage};
use roko_learn::runtime_feedback::{CompletedRunInput, LearningRuntime, refresh_cfactor_snapshot};
use roko_neuro::{KnowledgeEntry, KnowledgeKind, KnowledgeStore, KnowledgeTier};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Agent adapter used by the SWE-bench proxy runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SweAgentMode {
    /// Use the dataset gold patch. This validates harness plumbing, not model skill.
    Gold,
    /// Submit an empty patch. Useful as a negative control.
    Empty,
    /// Read patches from a SWE-bench predictions JSONL file.
    PredictionFile,
    /// Run a caller-provided command that receives instance JSON on stdin and prints a patch.
    Command,
}

impl SweAgentMode {
    fn label(self) -> &'static str {
        match self {
            Self::Gold => "gold",
            Self::Empty => "empty",
            Self::PredictionFile => "prediction-file",
            Self::Command => "command",
        }
    }
}

/// Options for `roko bench swe`.
#[derive(Debug, Clone)]
pub struct SweBenchOptions {
    /// Workspace root where `.roko/bench` and `.roko/learn` are written.
    pub workdir: PathBuf,
    /// Optional local JSONL dataset. If omitted, a built-in two-task smoke set is generated.
    pub dataset: Option<PathBuf>,
    /// Maximum number of instances to run.
    pub batch_size: usize,
    /// Offset into the dataset.
    pub offset: usize,
    /// Agent adapter.
    pub agent_mode: SweAgentMode,
    /// Optional predictions JSONL for [`SweAgentMode::PredictionFile`].
    pub predictions: Option<PathBuf>,
    /// Optional command for [`SweAgentMode::Command`].
    pub agent_command: Option<String>,
    /// Optional scores JSONL path.
    pub report: Option<PathBuf>,
    /// Optional SWE-bench-style predictions JSONL export path.
    pub export_predictions: Option<PathBuf>,
    /// Whether to write learning episodes and C-factor snapshots.
    pub record_learning: bool,
    /// Keep per-instance workdirs after the run.
    pub keep_workdirs: bool,
}

/// Aggregate benchmark report returned by [`run_swe_bench`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweBenchReport {
    /// Stable-ish run id used for artifacts.
    pub run_id: String,
    /// Arena name.
    pub arena: String,
    /// Dataset label or path.
    pub dataset: String,
    /// Agent adapter label.
    pub agent_mode: String,
    /// Number of instances evaluated.
    pub total: usize,
    /// Number of instances resolved by the proxy scorer.
    pub resolved: usize,
    /// `resolved / total`.
    pub pass_rate: f64,
    /// Number of patches that looked like unified diffs.
    pub format_valid: usize,
    /// Number of patches accepted by `git apply --check`.
    pub apply_check: usize,
    /// Number of instances whose test command passed after patch application.
    pub tests_passed: usize,
    /// Per-instance rows.
    pub instances: Vec<SweBenchInstanceResult>,
    /// Scores JSONL path.
    pub report_path: PathBuf,
    /// Detailed run JSON path.
    pub run_path: PathBuf,
    /// Optional predictions export path.
    pub predictions_path: Option<PathBuf>,
    /// C-factor before recording the batch, if available.
    pub cfactor_before: Option<CFactor>,
    /// C-factor after recording the batch, if learning was enabled.
    pub cfactor_after: Option<CFactor>,
}

impl SweBenchReport {
    /// Human-readable one-screen summary.
    #[must_use]
    pub fn render_text(&self) -> String {
        let before = self
            .cfactor_before
            .as_ref()
            .map(|cf| format!("{:.3}", cf.overall))
            .unwrap_or_else(|| "n/a".to_string());
        let after = self
            .cfactor_after
            .as_ref()
            .map(|cf| format!("{:.3}", cf.overall))
            .unwrap_or_else(|| "n/a".to_string());
        let delta = match (&self.cfactor_before, &self.cfactor_after) {
            (Some(before), Some(after)) => format!(" ({:+.3})", after.overall - before.overall),
            _ => String::new(),
        };

        format!(
            "\
SWE-bench proxy run: {run_id}
dataset: {dataset}
agent: {agent}
resolved: {resolved}/{total} ({pass_rate:.1}%)
format_valid: {format_valid}/{total}
apply_check: {apply_check}/{total}
tests_passed: {tests_passed}/{total}
c-factor: {before} -> {after}{delta}
report: {report}
details: {details}",
            run_id = self.run_id,
            dataset = self.dataset,
            agent = self.agent_mode,
            resolved = self.resolved,
            total = self.total,
            pass_rate = self.pass_rate * 100.0,
            format_valid = self.format_valid,
            apply_check = self.apply_check,
            tests_passed = self.tests_passed,
            report = self.report_path.display(),
            details = self.run_path.display(),
        )
    }
}

/// Per-instance proxy score row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweBenchInstanceResult {
    /// SWE-bench instance id.
    pub instance_id: String,
    /// Repository label.
    pub repo: String,
    /// Whether the patch looked like a unified diff.
    pub format_valid: bool,
    /// Whether `git apply --check` accepted the patch.
    pub apply_check: bool,
    /// Whether patch application succeeded.
    pub apply: bool,
    /// Whether the task test command passed after patch application.
    pub tests_passed: bool,
    /// Final proxy outcome.
    pub resolved: bool,
    /// Patch size in bytes.
    pub patch_bytes: usize,
    /// Wall-clock runtime in milliseconds.
    pub duration_ms: u64,
    /// Short failure reason.
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SweBenchInstance {
    instance_id: String,
    #[serde(default)]
    repo: String,
    #[serde(default)]
    repo_path: Option<PathBuf>,
    #[serde(default)]
    base_commit: String,
    #[serde(default)]
    problem_statement: String,
    #[serde(default)]
    patch: String,
    #[serde(default, alias = "test_command")]
    test_cmd: String,
}

#[derive(Debug, Deserialize)]
struct PredictionRow {
    instance_id: String,
    #[serde(default, alias = "patch")]
    model_patch: String,
}

/// Run the native SWE-bench-style proxy harness.
///
/// This is not official SWE-bench scoring. It is a fast local proxy that
/// verifies patch format, `git apply --check`, patch application, and an
/// instance-specific test command.
pub async fn run_swe_bench(options: SweBenchOptions) -> Result<SweBenchReport> {
    if options.batch_size == 0 {
        bail!("--batch-size must be greater than zero");
    }
    if options.agent_mode == SweAgentMode::PredictionFile && options.predictions.is_none() {
        bail!("--predictions is required when --agent-mode=prediction-file");
    }
    if options.agent_mode == SweAgentMode::Command && options.agent_command.is_none() {
        bail!("--agent-command is required when --agent-mode=command");
    }

    fs::create_dir_all(options.workdir.join(".roko").join("bench"))?;
    let run_id = run_id();
    let dataset_label = options
        .dataset
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "builtin:swe-smoke".to_string());
    let instances = load_instances(&options)?;
    let selected: Vec<SweBenchInstance> = instances
        .into_iter()
        .skip(options.offset)
        .take(options.batch_size)
        .collect();
    if selected.is_empty() {
        bail!(
            "no instances selected for offset={} batch_size={}",
            options.offset,
            options.batch_size
        );
    }

    let predictions = load_predictions(options.predictions.as_deref())?;
    let run_root = options
        .workdir
        .join(".roko")
        .join("bench")
        .join("workdirs")
        .join(&run_id);
    fs::create_dir_all(&run_root)?;

    let learn_root = options.workdir.join(".roko").join("learn");
    let cfactor_before = if options.record_learning && learn_root.join("episodes.jsonl").exists() {
        refresh_cfactor_snapshot(&learn_root).await.ok()
    } else {
        None
    };
    let runtime = if options.record_learning {
        Some(LearningRuntime::open_under(&learn_root).await?)
    } else {
        None
    };

    let mut rows = Vec::new();
    let mut prediction_exports = Vec::new();
    for instance in selected {
        let instance_start = Instant::now();
        let instance_workdir = run_root.join(sanitize_id(&instance.instance_id));
        copy_dir(
            instance
                .repo_path
                .as_deref()
                .ok_or_else(|| anyhow!("instance {} missing repo_path", instance.instance_id))?,
            &instance_workdir,
        )
        .with_context(|| format!("prepare workdir for {}", instance.instance_id))?;
        init_git_repo(&instance_workdir).with_context(|| {
            format!("initialize isolated git repo for {}", instance.instance_id)
        })?;

        let patch = produce_patch(&options, &instance, &predictions)?;
        let format_ok = format_valid(&patch);
        let (apply_check_ok, apply_check_error) = if format_ok {
            git_apply(&instance_workdir, &patch, true)?
        } else {
            (false, Some("format invalid".to_string()))
        };
        let (apply_ok, apply_error) = if apply_check_ok {
            git_apply(&instance_workdir, &patch, false)?
        } else {
            (false, apply_check_error.clone())
        };
        let (tests_ok, test_error) = if apply_ok && !instance.test_cmd.trim().is_empty() {
            run_shell(&instance_workdir, &instance.test_cmd)?
        } else if apply_ok {
            (true, None)
        } else {
            (false, apply_error.clone())
        };
        let resolved = format_ok && apply_check_ok && apply_ok && tests_ok;
        let duration_ms = elapsed_ms(instance_start);
        let failure_reason = if resolved {
            None
        } else {
            test_error.or(apply_error).or(apply_check_error)
        };

        let row = SweBenchInstanceResult {
            instance_id: instance.instance_id.clone(),
            repo: instance.repo.clone(),
            format_valid: format_ok,
            apply_check: apply_check_ok,
            apply: apply_ok,
            tests_passed: tests_ok,
            resolved,
            patch_bytes: patch.len(),
            duration_ms,
            failure_reason,
        };

        if let Some(runtime) = &runtime {
            record_learning(runtime, &options, &run_id, &instance, &row, &patch).await?;
        }

        prediction_exports.push(json!({
            "instance_id": instance.instance_id,
            "model_patch": patch,
            "model_name_or_path": format!("roko-bench/{}", options.agent_mode.label()),
        }));
        rows.push(row);
    }

    let cfactor_after = if options.record_learning {
        Some(refresh_cfactor_snapshot(&learn_root).await?)
    } else {
        None
    };

    let report_path = options.report.clone().unwrap_or_else(|| {
        options
            .workdir
            .join(".roko")
            .join("bench")
            .join("scores.jsonl")
    });
    let run_path = options
        .workdir
        .join(".roko")
        .join("bench")
        .join("runs")
        .join(format!("{run_id}.json"));
    let predictions_path = options.export_predictions.clone();

    let total = rows.len();
    let resolved = rows.iter().filter(|row| row.resolved).count();
    let report = SweBenchReport {
        run_id,
        arena: "swe-bench-proxy".to_string(),
        dataset: dataset_label,
        agent_mode: options.agent_mode.label().to_string(),
        total,
        resolved,
        pass_rate: resolved as f64 / total as f64,
        format_valid: rows.iter().filter(|row| row.format_valid).count(),
        apply_check: rows.iter().filter(|row| row.apply_check).count(),
        tests_passed: rows.iter().filter(|row| row.tests_passed).count(),
        instances: rows,
        report_path,
        run_path,
        predictions_path,
        cfactor_before,
        cfactor_after,
    };

    write_report_artifacts(&report, &prediction_exports)?;
    if options.record_learning {
        write_neuro_benchmark_insights(&options, &report)?;
    }
    if !options.keep_workdirs {
        let _ = fs::remove_dir_all(&run_root);
    }

    Ok(report)
}

fn load_instances(options: &SweBenchOptions) -> Result<Vec<SweBenchInstance>> {
    match options.dataset.as_deref() {
        Some(path) => load_jsonl_instances(path),
        None => create_builtin_smoke_dataset(&options.workdir),
    }
}

fn load_jsonl_instances(path: &Path) -> Result<Vec<SweBenchInstance>> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("read dataset JSONL {}", path.display()))?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut instances = Vec::new();
    for (idx, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut instance: SweBenchInstance = serde_json::from_str(line)
            .with_context(|| format!("parse dataset line {}", idx + 1))?;
        if let Some(repo_path) = instance.repo_path.as_mut()
            && repo_path.is_relative()
        {
            *repo_path = base.join(&repo_path);
        }
        instances.push(instance);
    }
    Ok(instances)
}

fn create_builtin_smoke_dataset(workdir: &Path) -> Result<Vec<SweBenchInstance>> {
    let root = workdir
        .join(".roko")
        .join("bench")
        .join("fixtures")
        .join("swe-smoke");
    fs::create_dir_all(&root)?;

    let calc_repo = root.join("calc-add");
    reset_fixture_dir(&calc_repo)?;
    fs::write(
        calc_repo.join("calc.py"),
        "def add(a, b):\n    return a - b\n",
    )?;
    fs::write(
        calc_repo.join("test_calc.py"),
        "import unittest\nfrom calc import add\n\nclass CalcTest(unittest.TestCase):\n    def test_adds(self):\n        self.assertEqual(add(2, 3), 5)\n\nif __name__ == '__main__':\n    unittest.main()\n",
    )?;

    let slug_repo = root.join("slugify");
    reset_fixture_dir(&slug_repo)?;
    fs::write(
        slug_repo.join("slug.py"),
        "def slugify(value):\n    return value.lower()\n",
    )?;
    fs::write(
        slug_repo.join("test_slug.py"),
        "import unittest\nfrom slug import slugify\n\nclass SlugTest(unittest.TestCase):\n    def test_spaces(self):\n        self.assertEqual(slugify(' Hello World '), 'hello-world')\n\nif __name__ == '__main__':\n    unittest.main()\n",
    )?;

    Ok(vec![
        SweBenchInstance {
            instance_id: "roko-smoke__calc-add".to_string(),
            repo: "builtin/calc-add".to_string(),
            repo_path: Some(calc_repo),
            base_commit: String::new(),
            problem_statement: "The add helper subtracts instead of adding.".to_string(),
            patch: "\
diff --git a/calc.py b/calc.py
--- a/calc.py
+++ b/calc.py
@@ -1,2 +1,2 @@
 def add(a, b):
-    return a - b
+    return a + b
"
            .to_string(),
            test_cmd: "python3 -m unittest test_calc.py".to_string(),
        },
        SweBenchInstance {
            instance_id: "roko-smoke__slugify".to_string(),
            repo: "builtin/slugify".to_string(),
            repo_path: Some(slug_repo),
            base_commit: String::new(),
            problem_statement: "Slugify should trim whitespace and replace spaces with hyphens."
                .to_string(),
            patch: "\
diff --git a/slug.py b/slug.py
--- a/slug.py
+++ b/slug.py
@@ -1,2 +1,2 @@
 def slugify(value):
-    return value.lower()
+    return value.strip().lower().replace(\" \", \"-\")
"
            .to_string(),
            test_cmd: "python3 -m unittest test_slug.py".to_string(),
        },
    ])
}

fn reset_fixture_dir(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn load_predictions(path: Option<&Path>) -> Result<HashMap<String, String>> {
    let Some(path) = path else {
        return Ok(HashMap::new());
    };
    let contents = fs::read_to_string(path)
        .with_context(|| format!("read predictions JSONL {}", path.display()))?;
    let mut predictions = HashMap::new();
    for (idx, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: PredictionRow = serde_json::from_str(line)
            .with_context(|| format!("parse predictions line {}", idx + 1))?;
        predictions.insert(row.instance_id, row.model_patch);
    }
    Ok(predictions)
}

fn produce_patch(
    options: &SweBenchOptions,
    instance: &SweBenchInstance,
    predictions: &HashMap<String, String>,
) -> Result<String> {
    match options.agent_mode {
        SweAgentMode::Gold => Ok(instance.patch.clone()),
        SweAgentMode::Empty => Ok(String::new()),
        SweAgentMode::PredictionFile => predictions
            .get(&instance.instance_id)
            .cloned()
            .ok_or_else(|| anyhow!("prediction missing for {}", instance.instance_id)),
        SweAgentMode::Command => {
            let command = options
                .agent_command
                .as_deref()
                .ok_or_else(|| anyhow!("missing agent command"))?;
            let payload = serde_json::to_string(instance)?;
            run_agent_command(command, &payload)
        }
    }
}

fn run_agent_command(command: &str, stdin_json: &str) -> Result<String> {
    let mut child = Command::new("sh")
        .arg("-lc")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn agent command `{command}`"))?;
    {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("agent command stdin unavailable"))?;
        let mut stdin = stdin;
        stdin.write_all(stdin_json.as_bytes())?;
        stdin.flush()?;
        drop(stdin);
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "agent command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn format_valid(patch: &str) -> bool {
    patch.lines().any(|line| line.starts_with("diff --git a/"))
        && patch.lines().any(|line| line.starts_with("@@ -"))
}

fn git_apply(workdir: &Path, patch: &str, check_only: bool) -> Result<(bool, Option<String>)> {
    let patch_path = workdir.join(".roko-bench.patch");
    fs::write(&patch_path, patch).context("write temporary patch file")?;

    let mut cmd = Command::new("git");
    cmd.arg("apply");
    if check_only {
        cmd.arg("--check");
    }
    cmd.arg(".roko-bench.patch");
    cmd.current_dir(workdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = cmd.output().context("run git apply")?;
    let _ = fs::remove_file(&patch_path);
    if output.status.success() {
        Ok((true, None))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Ok((false, Some(first_line(&stderr))))
    }
}

fn run_shell(workdir: &Path, command: &str) -> Result<(bool, Option<String>)> {
    let output = Command::new("sh")
        .arg("-lc")
        .arg(command)
        .current_dir(workdir)
        .output()
        .with_context(|| format!("run test command `{command}`"))?;
    if output.status.success() {
        Ok((true, None))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        Ok((false, Some(first_line(&combined))))
    }
}

async fn record_learning(
    runtime: &LearningRuntime,
    options: &SweBenchOptions,
    run_id: &str,
    instance: &SweBenchInstance,
    row: &SweBenchInstanceResult,
    patch: &str,
) -> Result<()> {
    let task_id = format!("{run_id}/{}", instance.instance_id);
    let mut episode = Episode::new(
        format!("swe-bench-{}", options.agent_mode.label()),
        task_id.clone(),
    );
    episode.kind = "arena_task".to_string();
    episode.episode_id = format!("bench-swe-{run_id}-{}", instance.instance_id);
    episode.agent_template = format!("swe-bench/{}", options.agent_mode.label());
    episode.model = format!("roko-bench/{}", options.agent_mode.label());
    episode.backend = "roko-bench".to_string();
    episode.trigger_kind = "bench:swe".to_string();
    episode.completed_at = Utc::now();
    episode.duration_secs = row.duration_ms as f64 / 1000.0;
    episode.success = row.resolved;
    episode.turns = 1;
    episode.tokens_used =
        estimate_tokens(&instance.problem_statement).saturating_add(estimate_tokens(patch));
    episode.usage = Usage {
        input_tokens: estimate_tokens(&instance.problem_statement),
        output_tokens: estimate_tokens(patch),
        cost_usd: 0.0,
        cost_usd_without_cache: 0.0,
        wall_ms: row.duration_ms,
        ..Usage::default()
    };
    episode.gate_verdicts = vec![
        GateVerdict::new("bench:format", row.format_valid),
        GateVerdict::new("bench:git_apply_check", row.apply_check),
        GateVerdict::new("bench:test", row.tests_passed),
    ];
    episode.failure_reason = row.failure_reason.clone();
    episode
        .extra
        .insert("arena".to_string(), json!("swe-bench-proxy"));
    episode.extra.insert(
        "instance_id".to_string(),
        json!(instance.instance_id.clone()),
    );
    episode
        .extra
        .insert("repo".to_string(), json!(instance.repo.clone()));
    episode
        .extra
        .insert("patch_bytes".to_string(), json!(row.patch_bytes));

    let mut metric = TaskMetric {
        timestamp: Utc::now().to_rfc3339(),
        run_id: "roko-bench".to_string(),
        config_hash: ConfigHash("roko-bench".to_string()),
        plan_id: "swe-bench-proxy".to_string(),
        task_id: task_id.clone(),
        iteration: 1,
        role: "BenchAgent".to_string(),
        backend: "roko-bench".to_string(),
        model: format!("roko-bench/{}", options.agent_mode.label()),
        complexity_band: "standard".to_string(),
        gate: "swe-proxy".to_string(),
        gate_passed: row.resolved,
        wall_time_ms: row.duration_ms,
        input_tokens: episode.usage.input_tokens,
        output_tokens: episode.usage.output_tokens,
        cached_tokens: 0,
        cost_usd: 0.0,
        sections_included: 1,
        sections_dropped: 0,
        context_tokens: episode.usage.input_tokens,
        cache_hit_rate: 0.0,
    };
    if metric.timestamp.is_empty() {
        metric.timestamp = Utc::now().to_rfc3339();
    }

    runtime
        .record_completed_run(CompletedRunInput::from_episode(episode).with_task_metric(metric))
        .await?;

    let mut event = AgentEfficiencyEvent::default_event();
    event.agent_id = format!("swe-bench-{}", options.agent_mode.label());
    event.role = "BenchAgent".to_string();
    event.backend = "roko-bench".to_string();
    event.model = format!("roko-bench/{}", options.agent_mode.label());
    event.plan_id = "swe-bench-proxy".to_string();
    event.task_id = task_id;
    event.input_tokens = estimate_tokens(&instance.problem_statement);
    event.output_tokens = estimate_tokens(patch);
    event.total_prompt_tokens = event.input_tokens;
    event.system_prompt_tokens = 0;
    event.wall_time_ms = row.duration_ms;
    event.duration_ms = row.duration_ms;
    event.gate_passed = row.resolved;
    event.outcome = if row.resolved { "resolved" } else { "failed" }.to_string();
    if let Some(reason) = &row.failure_reason {
        event.gate_errors.push(reason.clone());
    }
    event.model_used = event.model.clone();
    event.timestamp = Utc::now().to_rfc3339();
    runtime.append_efficiency_event(&event).await?;

    Ok(())
}

fn write_report_artifacts(report: &SweBenchReport, predictions: &[Value]) -> Result<()> {
    if let Some(parent) = report.report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = report.run_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let summary = json!({
        "run_id": report.run_id,
        "arena": report.arena,
        "dataset": report.dataset,
        "agent_mode": report.agent_mode,
        "total": report.total,
        "resolved": report.resolved,
        "pass_rate": report.pass_rate,
        "format_valid": report.format_valid,
        "apply_check": report.apply_check,
        "tests_passed": report.tests_passed,
        "cfactor_before": report.cfactor_before.as_ref().map(|cf| cf.overall),
        "cfactor_after": report.cfactor_after.as_ref().map(|cf| cf.overall),
        "run_path": report.run_path,
    });
    let mut scores = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&report.report_path)?;
    writeln!(scores, "{}", serde_json::to_string(&summary)?)?;

    fs::write(&report.run_path, serde_json::to_string_pretty(report)?)?;

    if let Some(path) = &report.predictions_path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = File::create(path)?;
        for prediction in predictions {
            writeln!(f, "{}", serde_json::to_string(prediction)?)?;
        }
    }

    Ok(())
}

fn write_neuro_benchmark_insights(
    options: &SweBenchOptions,
    report: &SweBenchReport,
) -> Result<()> {
    let store = KnowledgeStore::for_workdir(&options.workdir);
    let source_episodes = report
        .instances
        .iter()
        .map(|row| format!("bench-swe-{}:{}", report.run_id, row.instance_id))
        .collect::<Vec<_>>();
    let command_label = options
        .agent_command
        .as_deref()
        .unwrap_or_else(|| options.agent_mode.label());
    let tags = vec![
        "benchmark".to_string(),
        "swe-bench-proxy".to_string(),
        options.agent_mode.label().to_string(),
    ];

    let mut insight = KnowledgeEntry {
        id: format!("bench:{}:summary", report.run_id),
        kind: KnowledgeKind::Insight,
        source: Some("roko-bench".to_string()),
        content: format!(
            "{} resolved {}/{} tasks ({:.1}%) in {} using {}.",
            report.arena,
            report.resolved,
            report.total,
            report.pass_rate * 100.0,
            report.dataset,
            command_label,
        ),
        confidence: 0.65 + (report.pass_rate * 0.3),
        confidence_weight: if report.pass_rate > 0.0 { 1.0 } else { -0.4 },
        source_episodes: source_episodes.clone(),
        tags: tags.clone(),
        source_model: Some(command_label.to_string()),
        model_generality: 0.35,
        created_at: Utc::now(),
        half_life_days: KnowledgeKind::Insight.default_half_life_days(),
        tier: if report.pass_rate >= 1.0 {
            KnowledgeTier::Working
        } else {
            KnowledgeTier::Transient
        },
        confirmation_count: u32::try_from(report.resolved).unwrap_or(u32::MAX),
        distinct_contexts: report
            .instances
            .iter()
            .map(|row| row.instance_id.clone())
            .collect(),
        ..KnowledgeEntry::default()
    };
    insight
        .tags
        .extend(["pass-rate".to_string(), "scoring".to_string()]);
    store.add(insight)?;

    let kind = if report.pass_rate >= 1.0 {
        KnowledgeKind::Heuristic
    } else {
        KnowledgeKind::AntiKnowledge
    };
    let content = if report.pass_rate >= 1.0 {
        format!(
            "For tiny code-repair tasks, {} is a viable local-agent strategy when patches are verified with git apply and task tests.",
            command_label
        )
    } else {
        format!(
            "Do not trust {} for this benchmark shape without additional context or a stronger model; it resolved {}/{} tasks.",
            command_label, report.resolved, report.total
        )
    };
    store.add(KnowledgeEntry {
        id: format!("bench:{}:{}", report.run_id, kind.as_str()),
        kind,
        source: Some("roko-bench".to_string()),
        content,
        confidence: if report.pass_rate >= 1.0 { 0.86 } else { 0.78 },
        confidence_weight: if report.pass_rate >= 1.0 { 1.0 } else { -1.0 },
        source_episodes,
        tags,
        source_model: Some(command_label.to_string()),
        model_generality: 0.35,
        created_at: Utc::now(),
        half_life_days: kind.default_half_life_days(),
        tier: if report.pass_rate >= 1.0 {
            KnowledgeTier::Consolidated
        } else {
            KnowledgeTier::Working
        },
        confirmation_count: u32::try_from(report.resolved).unwrap_or(u32::MAX),
        distinct_contexts: report
            .instances
            .iter()
            .map(|row| row.instance_id.clone())
            .collect(),
        ..KnowledgeEntry::default()
    })?;

    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        fs::remove_dir_all(dst)?;
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn init_git_repo(workdir: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(workdir)
        .output()
        .context("run git init")?;
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("command failed")
        .chars()
        .take(240)
        .collect()
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64 / 4).max(1)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn run_id() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("swe-{}-{:09}", duration.as_secs(), duration.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;
    use tempfile::TempDir;

    #[test]
    fn unified_diff_format_detection_rejects_empty_patch() {
        assert!(!format_valid(""));
        assert!(format_valid(
            "diff --git a/a.py b/a.py\n@@ -1 +1 @@\n-a\n+b\n"
        ));
    }

    #[tokio::test]
    async fn built_in_smoke_gold_agent_resolves() {
        let tmp = TempDir::new().unwrap();
        let report = run_swe_bench(SweBenchOptions {
            workdir: tmp.path().to_path_buf(),
            dataset: None,
            batch_size: 2,
            offset: 0,
            agent_mode: SweAgentMode::Gold,
            predictions: None,
            agent_command: None,
            report: None,
            export_predictions: Some(tmp.path().join("predictions.jsonl")),
            record_learning: true,
            keep_workdirs: false,
        })
        .await
        .unwrap();

        assert_eq!(report.total, 2);
        assert_eq!(report.resolved, 2);
        assert_eq!(report.format_valid, 2);
        assert_eq!(report.apply_check, 2);
        assert_eq!(report.tests_passed, 2);
        assert!(report.report_path.exists());
        assert!(report.run_path.exists());
        assert!(tmp.path().join(".roko/learn/episodes.jsonl").exists());
        assert!(tmp.path().join(".roko/learn/c-factor.jsonl").exists());

        let mut predictions = String::new();
        File::open(tmp.path().join("predictions.jsonl"))
            .unwrap()
            .read_to_string(&mut predictions)
            .unwrap();
        assert_eq!(predictions.lines().count(), 2);
    }
}
