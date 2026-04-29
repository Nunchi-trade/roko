//! Core types and storage helpers for the bench system.
//!
//! A bench run executes a suite of prompt-based tasks through `run_once()`,
//! collecting pass/fail results, timing, and token usage per task. Results
//! persist to `.roko/bench/` for comparison and pareto analysis.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Timestamp serialization helpers (Unix seconds <-> ISO 8601 string)
// ---------------------------------------------------------------------------

#[allow(clippy::trivially_copy_pass_by_ref)]
fn serialize_timestamp_iso<S: serde::Serializer>(ts: &u64, ser: S) -> Result<S::Ok, S::Error> {
    use std::time::{Duration, UNIX_EPOCH};
    let dt = UNIX_EPOCH + Duration::from_secs(*ts);
    let datetime: chrono::DateTime<chrono::Utc> = dt.into();
    ser.serialize_str(&datetime.to_rfc3339())
}

fn deserialize_timestamp_iso<'de, D: serde::Deserializer<'de>>(de: D) -> Result<u64, D::Error> {
    let s = String::deserialize(de)?;
    // Try ISO 8601 first, fall back to parsing as plain u64.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&s) {
        Ok(dt.timestamp() as u64)
    } else if let Ok(n) = s.parse::<u64>() {
        Ok(n)
    } else {
        Err(serde::de::Error::custom(format!(
            "expected ISO 8601 timestamp or integer, got: {s}"
        )))
    }
}

#[allow(clippy::ref_option)]
fn serialize_opt_timestamp_iso<S: serde::Serializer>(
    ts: &Option<u64>,
    ser: S,
) -> Result<S::Ok, S::Error> {
    match ts {
        Some(t) => serialize_timestamp_iso(t, ser),
        None => ser.serialize_none(),
    }
}

fn deserialize_opt_timestamp_iso<'de, D: serde::Deserializer<'de>>(
    de: D,
) -> Result<Option<u64>, D::Error> {
    let opt: Option<String> = Option::deserialize(de)?;
    match opt {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&s) {
                Ok(Some(dt.timestamp() as u64))
            } else if let Ok(n) = s.parse::<u64>() {
                Ok(Some(n))
            } else {
                Err(serde::de::Error::custom(format!(
                    "expected ISO 8601 timestamp or integer, got: {s}"
                )))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Suite + task definitions
// ---------------------------------------------------------------------------

/// A benchmark suite containing a list of tasks to run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchSuite {
    /// Unique suite identifier (derived from filename).
    pub id: String,
    /// Human-readable suite name.
    pub name: String,
    /// Optional description.
    #[serde(default)]
    pub description: String,
    /// Tasks in this suite.
    pub tasks: Vec<BenchTask>,
    /// Default timeout per task in seconds.
    #[serde(default = "default_task_timeout")]
    pub default_timeout_secs: u64,
    /// Estimated cost in USD (computed if not set).
    #[serde(default)]
    pub estimated_cost_usd: f64,
    /// Difficulty range [min, max] (computed if not set).
    #[serde(default)]
    pub difficulty_range: (u8, u8),
}

impl BenchSuite {
    /// Fill in computed fields (`estimated_cost_usd`, `difficulty_range`) from tasks.
    pub fn fill_computed(&mut self) {
        if self.estimated_cost_usd == 0.0 && !self.tasks.is_empty() {
            // Rough estimate: ~$0.01 per easy task, ~$0.05 per hard task.
            self.estimated_cost_usd = self.tasks.iter().map(|t| t.difficulty as f64 * 0.01).sum();
        }
        if self.difficulty_range == (0, 0) && !self.tasks.is_empty() {
            let min = self.tasks.iter().map(|t| t.difficulty).min().unwrap_or(1);
            let max = self.tasks.iter().map(|t| t.difficulty).max().unwrap_or(1);
            self.difficulty_range = (min, max);
        }
    }
}

/// A single benchmark task (prompt + expected-outcome metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchTask {
    /// Task identifier, unique within the suite.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// The prompt to send to `run_once()`.
    pub prompt: String,
    /// Optional expected substring in successful output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_output: Option<String>,
    /// Per-task timeout override in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Tags for filtering / categorization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Difficulty tier (1 = easy, 5 = hard).
    #[serde(default = "default_difficulty")]
    pub difficulty: u8,
}

fn default_task_timeout() -> u64 {
    300
}

fn default_difficulty() -> u8 {
    1
}

// ---------------------------------------------------------------------------
// Run configuration
// ---------------------------------------------------------------------------

/// Bench execution strategy requested by the UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchStrategy {
    /// Smallest, least-enriched execution path.
    #[default]
    Minimal,
    /// Add repo context before dispatch.
    ContextEnriched,
    /// Add neuro-derived context before dispatch.
    NeuroAugmented,
    /// Apply the full cascade of enrichments before dispatch.
    FullCascade,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_default_bench_strategy(strategy: &BenchStrategy) -> bool {
    matches!(strategy, BenchStrategy::Minimal)
}

/// Configuration overrides applied to each task in a bench run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchConfigOverrides {
    /// Force a specific model slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Force a specific agent backend.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "provider")]
    pub backend: Option<String>,
    /// Maximum tokens for the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Temperature override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Execution strategy requested for this bench run.
    #[serde(default, skip_serializing_if = "is_default_bench_strategy")]
    pub strategy: BenchStrategy,
}

/// How a bench run was triggered.
///
/// Serializes to strings matching the frontend `BenchRunKind` type:
/// `'single' | 'suite' | 'comparison' | 'regression'`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BenchRunKind {
    /// Triggered from the UI / API → serializes as "suite".
    #[serde(rename = "suite")]
    Manual,
    /// Scheduled / automated → serializes as "regression".
    #[serde(rename = "regression")]
    Scheduled,
    /// Comparison A/B test → serializes as "comparison".
    #[serde(rename = "comparison")]
    Comparison,
}

/// Configuration for a bench run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRunConfig {
    /// Suite to run.
    pub suite_id: String,
    /// How this run was triggered.
    #[serde(default = "default_run_kind")]
    pub kind: BenchRunKind,
    /// Config overrides for all tasks in this run.
    #[serde(default)]
    pub overrides: BenchConfigOverrides,
    /// Optional label for the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

fn default_run_kind() -> BenchRunKind {
    BenchRunKind::Manual
}

// ---------------------------------------------------------------------------
// Run results
// ---------------------------------------------------------------------------

/// Result of a single bench task execution.
///
/// Field names match the frontend `BenchTaskResult` type in `bench-types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchTaskResult {
    /// Task identifier.
    pub task_id: String,
    /// Task name.
    pub task_name: String,
    /// Task status: "pass", "fail", or "skipped".
    /// Frontend expects `status: TaskStatus` not a boolean.
    pub status: String,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Model that was actually used.
    #[serde(default = "default_model_string")]
    pub model: String,
    /// Input tokens consumed.
    #[serde(default, alias = "input_tokens")]
    pub tokens_in: u64,
    /// Output tokens generated.
    #[serde(default, alias = "output_tokens")]
    pub tokens_out: u64,
    /// Estimated cost in USD.
    #[serde(default)]
    pub cost_usd: f64,
    /// Gate verdicts for this task.
    #[serde(default)]
    pub gate_verdicts: Vec<serde_json::Value>,
    /// Number of retries used.
    #[serde(default)]
    pub retries_used: u32,
    /// Output text (truncated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    /// Error message if the task failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BenchTaskResult {
    /// Whether this task passed.
    pub fn passed(&self) -> bool {
        self.status == "pass"
    }
}

fn default_model_string() -> String {
    "unknown".to_string()
}

/// Aggregate summary for a completed bench run.
///
/// Field names match the frontend `BenchRunSummary` type in `bench-types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRunSummary {
    /// Total tasks in the suite.
    pub total_tasks: usize,
    /// Number of tasks that passed.
    pub passed: usize,
    /// Number of tasks that failed.
    pub failed: usize,
    /// Number of tasks skipped.
    #[serde(default)]
    pub skipped: usize,
    /// Pass rate as a fraction (0.0 - 1.0).
    pub pass_rate: f64,
    /// Total execution time in milliseconds.
    pub total_duration_ms: u64,
    /// Total estimated cost in USD.
    pub total_cost_usd: f64,
    /// Total tokens (input + output) across all tasks.
    pub total_tokens: u64,
    /// Cost per successful task in USD.
    #[serde(default)]
    pub cost_per_success_usd: f64,
    /// Average duration per task in milliseconds.
    #[serde(default)]
    pub avg_duration_ms: f64,
}

impl BenchRunSummary {
    /// Compute summary from task results.
    pub fn from_results(results: &[BenchTaskResult]) -> Self {
        let total_tasks = results.len();
        let passed = results.iter().filter(|r| r.passed()).count();
        let failed = total_tasks - passed;
        let pass_rate = if total_tasks > 0 {
            passed as f64 / total_tasks as f64
        } else {
            0.0
        };
        let total_duration_ms: u64 = results.iter().map(|r| r.duration_ms).sum();
        let total_cost_usd: f64 = results.iter().map(|r| r.cost_usd).sum();
        let total_in: u64 = results.iter().map(|r| r.tokens_in).sum();
        let total_out: u64 = results.iter().map(|r| r.tokens_out).sum();
        let total_tokens = total_in + total_out;
        let cost_per_success_usd = if passed > 0 {
            total_cost_usd / passed as f64
        } else {
            0.0
        };
        let avg_duration_ms = if total_tasks > 0 {
            total_duration_ms as f64 / total_tasks as f64
        } else {
            0.0
        };

        Self {
            total_tasks,
            passed,
            failed,
            skipped: 0,
            pass_rate,
            total_duration_ms,
            total_cost_usd,
            total_tokens,
            cost_per_success_usd,
            avg_duration_ms,
        }
    }
}

/// Status of a bench run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BenchRunStatus {
    /// Currently executing tasks.
    Running,
    /// All tasks completed.
    Completed,
    /// Run was cancelled.
    Cancelled,
    /// Run encountered a fatal error.
    Failed,
}

/// A complete bench run record.
///
/// Field names match the frontend `BenchRun` type in `bench-types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRun {
    /// Unique run identifier.
    pub id: String,
    /// Suite that was executed.
    pub suite_id: String,
    /// Suite name (denormalized for convenience).
    pub suite_name: String,
    /// How this run was triggered.
    pub kind: BenchRunKind,
    /// Config overrides applied to this run.
    /// Frontend expects `config: BenchRunConfig`.
    #[serde(rename = "config", alias = "overrides")]
    pub overrides: BenchConfigOverrides,
    /// Optional label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Run status.
    pub status: BenchRunStatus,
    /// When the run started — serialized as ISO 8601.
    #[serde(
        serialize_with = "serialize_timestamp_iso",
        deserialize_with = "deserialize_timestamp_iso"
    )]
    pub started_at: u64,
    /// When the run finished — serialized as ISO 8601.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_timestamp_iso",
        deserialize_with = "deserialize_opt_timestamp_iso"
    )]
    pub finished_at: Option<u64>,
    /// Per-task results.
    pub results: Vec<BenchTaskResult>,
    /// Aggregate summary (populated once complete).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<BenchRunSummary>,
    /// Index of the currently executing task (for progress).
    #[serde(default)]
    pub current_task_index: usize,
    /// Total tasks in the suite.
    pub total_tasks: usize,
}

/// Lightweight index entry for fast listing (internal storage format).
///
/// Note: This uses raw u64 timestamps for storage. The `list_bench_runs`
/// endpoint loads full `BenchRun` objects which serialize timestamps as ISO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRunIndexEntry {
    /// Run identifier.
    pub id: String,
    /// Suite identifier.
    pub suite_id: String,
    /// Suite name.
    pub suite_name: String,
    /// Run status.
    pub status: BenchRunStatus,
    /// When the run started (Unix seconds).
    pub started_at: u64,
    /// When the run finished (Unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// Optional label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Model used (from overrides or default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Pass rate (0.0 - 1.0), populated once complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_rate: Option<f64>,
    /// Total cost in USD, populated once complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
}

// ---------------------------------------------------------------------------
// Pareto frontier point
// ---------------------------------------------------------------------------

/// A point on the cost vs pass_rate pareto frontier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParetoPoint {
    pub run_id: String,
    pub suite_id: String,
    pub model: Option<String>,
    pub label: Option<String>,
    pub pass_rate: f64,
    pub total_cost_usd: f64,
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn bench_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("bench")
}

fn runs_dir(workdir: &Path) -> PathBuf {
    bench_dir(workdir).join("runs")
}

fn suites_dir(workdir: &Path) -> PathBuf {
    bench_dir(workdir).join("suites")
}

fn index_path(workdir: &Path) -> PathBuf {
    bench_dir(workdir).join("index.jsonl")
}

fn run_path(workdir: &Path, run_id: &str) -> PathBuf {
    runs_dir(workdir).join(format!("bench_{run_id}.json"))
}

async fn write_text_atomically(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    let tmp_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}.tmp"))
        .unwrap_or_else(|| "tmp".to_string());
    let tmp = path.with_file_name(tmp_name);

    tokio::fs::write(&tmp, content).await?;
    if let Err(err) = tokio::fs::rename(&tmp, path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(err.into());
    }
    Ok(())
}

/// Save a bench run to disk.
pub async fn save_bench_run(workdir: &Path, run: &BenchRun) -> anyhow::Result<()> {
    let path = run_path(workdir, &run.id);
    let json = serde_json::to_string_pretty(run)?;
    write_text_atomically(&path, &json).await
}

/// Load a bench run from disk.
pub async fn load_bench_run(workdir: &Path, run_id: &str) -> anyhow::Result<Option<BenchRun>> {
    let path = run_path(workdir, run_id);
    if !path.exists() {
        return Ok(None);
    }
    let data = tokio::fs::read_to_string(&path).await?;
    let run: BenchRun = serde_json::from_str(&data)?;
    Ok(Some(run))
}

/// Delete a bench run from disk.
pub async fn delete_bench_run(workdir: &Path, run_id: &str) -> anyhow::Result<()> {
    let path = run_path(workdir, run_id);
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }
    Ok(())
}

/// Append an index entry to the JSONL index file.
pub async fn append_index_entry(workdir: &Path, entry: &BenchRunIndexEntry) -> anyhow::Result<()> {
    let dir = bench_dir(workdir);
    tokio::fs::create_dir_all(&dir).await?;
    let path = index_path(workdir);
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?
        .write_all(line.as_bytes())
        .await?;
    Ok(())
}

/// Update an existing index entry (rewrite the full file atomically).
pub async fn update_index_entry(workdir: &Path, entry: &BenchRunIndexEntry) -> anyhow::Result<()> {
    let path = index_path(workdir);
    let mut entries = load_index_entries(workdir).await;
    if let Some(existing) = entries.iter_mut().find(|e| e.id == entry.id) {
        *existing = entry.clone();
    } else {
        entries.push(entry.clone());
    }
    let mut content = String::new();
    for e in &entries {
        let line = serde_json::to_string(e)?;
        content.push_str(&line);
        content.push('\n');
    }
    write_text_atomically(&path, &content).await
}

/// Load all index entries from the JSONL file.
pub async fn load_index_entries(workdir: &Path) -> Vec<BenchRunIndexEntry> {
    let path = index_path(workdir);
    let data = match tokio::fs::read_to_string(&path).await {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    data.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Load all available suites from `.roko/bench/suites/`.
pub async fn load_suites(workdir: &Path) -> Vec<BenchSuite> {
    let dir = suites_dir(workdir);
    let mut suites = Vec::new();

    // Load from disk.
    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                if let Ok(data) = tokio::fs::read_to_string(&path).await {
                    if let Ok(mut suite) = toml::from_str::<BenchSuite>(&data) {
                        // Derive ID from filename if not set.
                        if suite.id.is_empty() {
                            suite.id = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("unknown")
                                .to_string();
                        }
                        suites.push(suite);
                    }
                }
            }
        }
    }

    // Keep the inlined learnable suite visible even before its TOML file has
    // been materialized on disk.
    if !suites
        .iter()
        .any(|suite| suite.id.as_str() == "learnable-rust")
    {
        suites.push(builtin_learnable_rust_suite());
    }

    // Fill computed fields for all suites.
    for suite in &mut suites {
        suite.fill_computed();
    }

    // Sort by id for stable ordering.
    suites.sort_by(|a, b| a.id.cmp(&b.id));
    suites
}

/// Load a single suite by ID.
pub async fn load_suite(workdir: &Path, suite_id: &str) -> Option<BenchSuite> {
    let suites = load_suites(workdir).await;
    suites.into_iter().find(|s| s.id == suite_id)
}

/// Save a suite to disk.
pub async fn save_suite(workdir: &Path, suite: &BenchSuite) -> anyhow::Result<()> {
    let dir = suites_dir(workdir);
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{}.toml", suite.id));
    let toml_str = toml::to_string_pretty(suite)?;
    tokio::fs::write(&path, toml_str).await?;
    Ok(())
}

/// Build the pareto frontier from all completed runs.
pub async fn compute_pareto_frontier(workdir: &Path) -> Vec<ParetoPoint> {
    let entries = load_index_entries(workdir).await;
    let mut points: Vec<ParetoPoint> = entries
        .into_iter()
        .filter(|e| {
            e.status == BenchRunStatus::Completed
                && e.pass_rate.is_some()
                && e.total_cost_usd.is_some()
        })
        .map(|e| ParetoPoint {
            run_id: e.id,
            suite_id: e.suite_id,
            model: e.model,
            label: e.label,
            pass_rate: e.pass_rate.unwrap_or(0.0),
            total_cost_usd: e.total_cost_usd.unwrap_or(0.0),
        })
        .collect();

    // Sort by cost ascending.
    points.sort_by(|a, b| a.total_cost_usd.total_cmp(&b.total_cost_usd));

    // Extract pareto frontier: keep points where pass_rate is strictly higher
    // than all cheaper points.
    let mut frontier = Vec::new();
    let mut best_pass_rate = -1.0_f64;
    for point in points {
        if point.pass_rate > best_pass_rate {
            best_pass_rate = point.pass_rate;
            frontier.push(point);
        }
    }
    frontier
}

/// List available model slugs from the roko config.
pub fn list_models_from_config(config: &roko_core::config::schema::RokoConfig) -> Vec<String> {
    let mut models = Vec::new();
    let default = &config.agent.default_model;
    if !default.is_empty() {
        models.push(default.clone());
    }
    for slug in config.models.keys() {
        if !models.contains(slug) {
            models.push(slug.clone());
        }
    }
    if models.is_empty() {
        models.push("claude-sonnet-4-6".to_string());
    }
    models
}

/// Estimate cost in USD from token counts and model slug.
///
/// Uses approximate per-1K-token pricing. Falls back to Sonnet pricing
/// when the model is unknown.
pub fn estimate_cost_usd(model: Option<&str>, input_tokens: u64, output_tokens: u64) -> f64 {
    let (input_rate, output_rate) = match model.unwrap_or("") {
        m if m.contains("haiku") => (0.00025, 0.00125),
        m if m.contains("sonnet") => (0.003, 0.015),
        m if m.contains("opus") => (0.015, 0.075),
        m if m.contains("gpt-4o-mini") => (0.00015, 0.0006),
        m if m.contains("gpt-4o") => (0.005, 0.015),
        m if m.contains("o3-mini") => (0.0011, 0.0044),
        m if m.contains("gemini") => (0.00125, 0.01),
        m if m.contains("llama") || m.contains("cerebras") => (0.0001, 0.0001),
        _ => (0.003, 0.015),
    };
    (input_tokens as f64 * input_rate / 1000.0) + (output_tokens as f64 * output_rate / 1000.0)
}

use tokio::io::AsyncWriteExt;

// ---------------------------------------------------------------------------
// Built-in suite definitions
// ---------------------------------------------------------------------------

pub fn builtin_learnable_rust_suite() -> BenchSuite {
    BenchSuite {
        id: "learnable-rust".to_string(),
        name: "Learnable Rust".to_string(),
        description: "Five short Rust tasks tuned for the Llama 3.1 8B boundary. Each task rewards a reusable playbook: search the scaffold, make a minimal edit, and finish with cargo test/check.".to_string(),
        tasks: vec![
            BenchTask {
                id: "grep-fix-todo".to_string(),
                name: "Grep and Fix TODO".to_string(),
                prompt: r"Work in the pre-initialized Cargo project at the bench workdir. Use `grep` to find the first `TODO:` in `src/lib.rs`, inspect the surrounding code with `read_file`, and replace only that first TODO with a real implementation using `edit_file`. If you need to confirm the project layout, use `glob` and `grep` first. Keep the edit minimal and do not rewrite unrelated code. Finish by running `cargo test` from the project root with `bash`; keep iterating until the final output contains `test result: ok`.".to_string(),
                expected_output: Some("test result: ok".to_string()),
                timeout_secs: None,
                tags: vec![
                    "learnable".to_string(),
                    "rust".to_string(),
                    "grep".to_string(),
                    "tests".to_string(),
                ],
                difficulty: 1,
            },
            BenchTask {
                id: "extract-helper-function".to_string(),
                name: "Extract Helper Function".to_string(),
                prompt: r"Use `glob` to inspect the project layout and `grep` to find duplicated logic in `src/lib.rs`. The scaffold intentionally repeats one small block twice. Extract that repeated logic into a private helper function in `src/lib.rs`, update both call sites with `edit_file`, and keep the public behavior unchanged. Read the edited file back with `read_file` if needed before saving. Finish with `bash` running `cargo check`; do not stop until the output contains `Finished`.".to_string(),
                expected_output: Some("Finished".to_string()),
                timeout_secs: None,
                tags: vec![
                    "learnable".to_string(),
                    "rust".to_string(),
                    "refactor".to_string(),
                    "check".to_string(),
                ],
                difficulty: 2,
            },
            BenchTask {
                id: "fix-broken-import".to_string(),
                name: "Fix Broken Import".to_string(),
                prompt: r"Use `grep` on `src/lib.rs` to find the broken `use` import that prevents compilation. Then use `glob` and `grep` to locate where the referenced type is actually defined elsewhere in the repo, confirm the definition with `read_file`, and fix only the import path in `src/lib.rs` with `edit_file`. Do not move or rename the type definition. Verify the minimal fix with `bash` and `cargo check`; the final output should contain `Finished`.".to_string(),
                expected_output: Some("Finished".to_string()),
                timeout_secs: None,
                tags: vec![
                    "learnable".to_string(),
                    "rust".to_string(),
                    "imports".to_string(),
                    "check".to_string(),
                ],
                difficulty: 2,
            },
            BenchTask {
                id: "generic-wrap-result".to_string(),
                name: "Implement Generic Wrapper with Tests".to_string(),
                prompt: r"In the scaffolded `src/lib.rs`, implement the generic `wrap_result<T>` stub and add tests that cover success and error handling. Use `read_file` to inspect the stub, `grep` to find the existing test module, and `write_file` only if you need a new test file; otherwise `edit_file` is enough. Keep the helper generic and do not special-case a single concrete type. Finish by running `cargo test` with `bash` until it passes; the last command must produce output containing `test result: ok`.".to_string(),
                expected_output: Some("test result: ok".to_string()),
                timeout_secs: None,
                tags: vec![
                    "learnable".to_string(),
                    "rust".to_string(),
                    "generics".to_string(),
                    "tests".to_string(),
                ],
                difficulty: 3,
            },
            BenchTask {
                id: "countup-iterator".to_string(),
                name: "Implement Custom Iterator".to_string(),
                prompt: r"In `src/lib.rs`, implement `Iterator` for the existing `CountUp` struct. Use `read_file` and `grep` to inspect the scaffold and any tests, then use `edit_file` to add the `next()` logic and `write_file` if you need to add or expand unit tests. The iterator should count upward by one per call, stop cleanly according to the scaffolded end condition, and match the behavior already implied by the tests. Finish with `bash` running `cargo test` until the output contains `test result: ok`.".to_string(),
                expected_output: Some("test result: ok".to_string()),
                timeout_secs: None,
                tags: vec![
                    "learnable".to_string(),
                    "rust".to_string(),
                    "iterators".to_string(),
                    "tests".to_string(),
                ],
                difficulty: 3,
            },
        ],
        default_timeout_secs: 300,
        estimated_cost_usd: 0.0,
        difficulty_range: (0, 0),
    }
}

// ---------------------------------------------------------------------------
// Built-in suite TOML content (embedded)
// ---------------------------------------------------------------------------

pub const SMOKE_SUITE_TOML: &str = r#"id = "smoke"
name = "Smoke Test"
description = "Quick validation that the agent pipeline works end-to-end"
default_timeout_secs = 120

[[tasks]]
id = "hello"
name = "Hello World"
prompt = "Create a file called hello.txt containing 'Hello, World!'"
expected_output = "hello"
difficulty = 1
tags = ["basic", "file-io"]

[[tasks]]
id = "simple-fn"
name = "Simple Function"
prompt = "Write a Rust function `fn add(a: i32, b: i32) -> i32` that returns the sum of two integers. Put it in src/lib.rs."
expected_output = "add"
difficulty = 1
tags = ["basic", "rust"]

[[tasks]]
id = "fix-syntax"
name = "Fix Syntax Error"
prompt = "The file src/lib.rs contains: `fn broken( -> i32 { 42 }`. Fix the syntax error so it compiles."
difficulty = 1
tags = ["basic", "fix"]

[[tasks]]
id = "read-file"
name = "Read and Summarize"
prompt = "Read the file Cargo.toml and tell me the package name and version."
difficulty = 1
tags = ["basic", "comprehension"]

[[tasks]]
id = "multi-step"
name = "Multi-Step Task"
prompt = "Create a Rust module src/math.rs with functions add, subtract, and multiply for i32 values. Then add `mod math;` to src/lib.rs."
difficulty = 2
tags = ["basic", "multi-step"]
"#;

pub const CODEGEN_SUITE_TOML: &str = r#"id = "codegen"
name = "Code Generation"
description = "Tests the agent's ability to generate correct Rust code"
default_timeout_secs = 300

[[tasks]]
id = "fizzbuzz"
name = "FizzBuzz"
prompt = "Implement fizzbuzz for numbers 1-100 in src/main.rs. Print 'Fizz' for multiples of 3, 'Buzz' for multiples of 5, 'FizzBuzz' for both."
difficulty = 1
tags = ["codegen", "basic"]

[[tasks]]
id = "linked-list"
name = "Linked List"
prompt = "Implement a singly linked list in Rust with push_front, pop_front, and len methods in src/lib.rs."
difficulty = 2
tags = ["codegen", "data-structures"]

[[tasks]]
id = "binary-search"
name = "Binary Search"
prompt = "Implement binary search for a sorted slice in src/lib.rs: `fn binary_search(haystack: &[i32], needle: i32) -> Option<usize>`"
difficulty = 2
tags = ["codegen", "algorithms"]

[[tasks]]
id = "error-handling"
name = "Error Handling"
prompt = "Create a config parser in src/config.rs that reads a TOML file and returns a typed Config struct with proper error handling using thiserror."
difficulty = 3
tags = ["codegen", "error-handling"]

[[tasks]]
id = "trait-impl"
name = "Trait Implementation"
prompt = "Define a trait `Drawable` with a `draw(&self) -> String` method. Implement it for Circle and Rectangle structs in src/shapes.rs."
difficulty = 2
tags = ["codegen", "traits"]

[[tasks]]
id = "iterator"
name = "Custom Iterator"
prompt = "Implement a custom iterator `FibIter` that yields Fibonacci numbers. It should implement Iterator<Item = u64> in src/lib.rs."
difficulty = 2
tags = ["codegen", "iterators"]

[[tasks]]
id = "builder-pattern"
name = "Builder Pattern"
prompt = "Implement a builder pattern for a `HttpRequest` struct with method, url, headers, and body fields in src/http.rs."
difficulty = 3
tags = ["codegen", "patterns"]

[[tasks]]
id = "async-fn"
name = "Async Function"
prompt = "Write an async function that reads a file, processes each line, and returns a Vec<String> of non-empty trimmed lines in src/lib.rs."
difficulty = 2
tags = ["codegen", "async"]

[[tasks]]
id = "generic-cache"
name = "Generic Cache"
prompt = "Implement a generic LRU cache `LruCache<K, V>` with get, put, and len methods using a HashMap and VecDeque in src/cache.rs."
difficulty = 3
tags = ["codegen", "generics"]

[[tasks]]
id = "macro-rules"
name = "Macro Rules"
prompt = "Write a macro `vec_of_strings!` that takes string literals and returns a Vec<String>. Put it in src/macros.rs."
difficulty = 3
tags = ["codegen", "macros"]

[[tasks]]
id = "cli-parser"
name = "CLI Parser"
prompt = "Create a simple CLI argument parser in src/cli.rs that handles --name, --count, and --verbose flags without external dependencies."
difficulty = 3
tags = ["codegen", "cli"]

[[tasks]]
id = "state-machine"
name = "State Machine"
prompt = "Implement a type-safe state machine for a traffic light (Red -> Green -> Yellow -> Red) using Rust's type system in src/fsm.rs."
difficulty = 4
tags = ["codegen", "type-system"]

[[tasks]]
id = "concurrent-counter"
name = "Concurrent Counter"
prompt = "Implement a thread-safe counter that can be incremented from multiple threads using Arc and AtomicU64 in src/counter.rs."
difficulty = 2
tags = ["codegen", "concurrency"]

[[tasks]]
id = "json-parser"
name = "JSON Value Parser"
prompt = "Implement a minimal JSON value parser (strings, numbers, booleans, null, arrays, objects) from scratch in src/json.rs."
difficulty = 4
tags = ["codegen", "parsing"]

[[tasks]]
id = "refactor-extract"
name = "Extract Method Refactor"
prompt = "The function `process_data` in src/lib.rs is 50+ lines long. Refactor it by extracting logical sections into well-named helper functions."
difficulty = 3
tags = ["codegen", "refactor"]

[[tasks]]
id = "fibonacci"
name = "Fibonacci"
prompt = "Implement a function `fn fib(n: u64) -> u64` that returns the nth Fibonacci number using iteration (not recursion) in src/lib.rs. Add tests for fib(0)=0, fib(1)=1, fib(10)=55."
difficulty = 1
tags = ["codegen", "basic", "math"]

[[tasks]]
id = "string-reverse"
name = "String Reverse"
prompt = "Implement `fn reverse_string(s: &str) -> String` that reverses a UTF-8 string by grapheme clusters (not just bytes) in src/lib.rs. Add tests including an emoji string."
difficulty = 1
tags = ["codegen", "basic", "strings"]

[[tasks]]
id = "palindrome"
name = "Palindrome Check"
prompt = "Implement `fn is_palindrome(s: &str) -> bool` that checks if a string is a palindrome ignoring case and non-alphanumeric characters in src/lib.rs. Add tests."
difficulty = 1
tags = ["codegen", "basic", "strings"]

[[tasks]]
id = "array-rotate"
name = "Array Rotate"
prompt = "Implement `fn rotate_left<T: Clone>(slice: &mut [T], k: usize)` that rotates elements left by k positions in-place in src/lib.rs. Handle k > len. Add tests."
difficulty = 1
tags = ["codegen", "basic", "arrays"]

[[tasks]]
id = "hash-map"
name = "Hash Map from Scratch"
prompt = "Implement a basic open-addressing hash map with insert, get, and remove for String keys and i64 values in src/hashmap.rs. Do not use std::collections::HashMap."
difficulty = 2
tags = ["codegen", "data-structures"]

[[tasks]]
id = "stack-impl"
name = "Generic Stack"
prompt = "Implement a generic `Stack<T>` backed by a Vec with push, pop, peek, is_empty, and len methods in src/stack.rs. Add tests covering edge cases."
difficulty = 2
tags = ["codegen", "data-structures", "generics"]

[[tasks]]
id = "tree-traversal"
name = "Binary Tree Traversal"
prompt = "Define a binary tree `enum Tree<T> { Leaf, Node(T, Box<Tree<T>>, Box<Tree<T>>)}` in src/tree.rs. Implement inorder, preorder, and postorder traversal methods returning Vec<&T>."
difficulty = 2
tags = ["codegen", "data-structures", "trees"]

[[tasks]]
id = "simple-regex"
name = "Simple Regex Matcher"
prompt = "Implement a minimal regex matcher supporting '.', '*', and literal characters: `fn matches(pattern: &str, text: &str) -> bool` in src/regex.rs. Add tests."
difficulty = 2
tags = ["codegen", "algorithms", "strings"]

[[tasks]]
id = "graph-bfs"
name = "Graph BFS"
prompt = "Implement a graph as an adjacency list in src/graph.rs with `add_edge` and `bfs(start: usize) -> Vec<usize>` that returns nodes in BFS order. Add tests with a 6-node graph."
difficulty = 3
tags = ["codegen", "algorithms", "graphs"]

[[tasks]]
id = "middleware-chain"
name = "Middleware Chain"
prompt = "Implement a middleware chain pattern in src/middleware.rs: each middleware is `Fn(&mut Request, &dyn Fn(&mut Request) -> Response) -> Response`. Build a chain runner that composes N middlewares. Add tests."
difficulty = 3
tags = ["codegen", "patterns", "composition"]

[[tasks]]
id = "rate-limiter"
name = "Token Bucket Rate Limiter"
prompt = "Implement a token-bucket rate limiter in src/ratelimit.rs with `new(rate: f64, capacity: u64)`, `try_acquire() -> bool`, and `try_acquire_n(n: u64) -> bool`. Use std::time::Instant. Add tests."
difficulty = 3
tags = ["codegen", "concurrency", "algorithms"]

[[tasks]]
id = "event-emitter"
name = "Event Emitter"
prompt = "Implement a typed event emitter in src/events.rs: `on<F: Fn(&str)>(&mut self, event: &str, handler: F)`, `emit(&self, event: &str, data: &str)`, and `off(&mut self, event: &str)`. Store handlers in a HashMap. Add tests."
difficulty = 3
tags = ["codegen", "patterns", "events"]

[[tasks]]
id = "protocol-parser"
name = "Protocol Parser"
prompt = "Implement a parser for a simple line-based protocol in src/protocol.rs: messages are 'CMD key value\\n'. Parse into enum Command { Get(String), Set(String, String), Del(String), Quit }. Handle malformed input with errors. Add tests."
difficulty = 4
tags = ["codegen", "parsing", "protocols"]

[[tasks]]
id = "crdt-merge"
name = "CRDT G-Counter Merge"
prompt = "Implement a G-Counter CRDT in src/crdt.rs: each node has an ID, can increment locally, and merge with remote state using element-wise max. Implement `increment`, `value`, and `merge`. Add tests with 3 nodes."
difficulty = 4
tags = ["codegen", "distributed", "crdt"]

[[tasks]]
id = "type-checker"
name = "Simple Type Checker"
prompt = "Implement a type checker for a tiny expression language in src/typecheck.rs: expressions are Int literals, Bool literals, Add(e1,e2), If(cond,then,else). Infer types and report errors. Add tests."
difficulty = 4
tags = ["codegen", "type-system", "compilers"]

[[tasks]]
id = "async-runtime-stub"
name = "Async Runtime Stub"
prompt = "Implement a minimal single-threaded async executor in src/executor.rs: a `block_on` function that polls a future to completion using a simple waker. Support spawning tasks into a VecDeque and draining them. Add tests with async blocks."
difficulty = 5
tags = ["codegen", "async", "runtime"]
"#;

pub const INTEGRATION_SUITE_TOML: &str = r#"id = "integration"
name = "Integration"
description = "Multi-file and cross-module tasks that test the agent's ability to navigate, modify, and connect code across boundaries"
default_timeout_secs = 600

[[tasks]]
id = "add-module-reexport"
name = "Add Module Re-export"
prompt = "The crate has src/internal.rs with a pub struct Config. Add a re-export in src/lib.rs so users can access it as `crate::Config`. Run cargo check to verify."
difficulty = 3
tags = ["integration", "modules"]

[[tasks]]
id = "wire-error-type"
name = "Wire Error Type Across Modules"
prompt = "src/errors.rs defines AppError. src/db.rs and src/api.rs each define their own error types. Add From impls so both module errors convert into AppError. Update callers to use ?. Run cargo check."
difficulty = 3
tags = ["integration", "errors", "cross-module"]

[[tasks]]
id = "add-integration-test"
name = "Add Integration Test"
prompt = "Create tests/integration_test.rs that imports the public API from src/lib.rs and tests the full create-read-update-delete cycle using the existing Storage trait. Run cargo test."
difficulty = 3
tags = ["integration", "testing"]

[[tasks]]
id = "extract-shared-types"
name = "Extract Shared Types"
prompt = "src/server.rs and src/client.rs both define identical Request and Response structs. Extract them into src/types.rs, update both files to import from there, and ensure cargo check passes."
difficulty = 3
tags = ["integration", "refactor", "cross-module"]

[[tasks]]
id = "api-endpoint-wiring"
name = "Wire New API Endpoint"
prompt = "src/handlers.rs has a `health_check` handler function. Wire it into the router in src/routes.rs at GET /health, add it to the OpenAPI spec in src/openapi.rs, and add a test. Run cargo check."
difficulty = 4
tags = ["integration", "api", "wiring"]

[[tasks]]
id = "feature-flag-module"
name = "Feature Flag Conditional Compilation"
prompt = "Add a Cargo feature 'metrics' that conditionally compiles src/metrics.rs. Update Cargo.toml, add #[cfg(feature)] guards in src/lib.rs, and ensure the crate builds with and without the feature."
difficulty = 4
tags = ["integration", "features", "conditional"]

[[tasks]]
id = "trait-across-crates"
name = "Cross-Crate Trait Implementation"
prompt = "crate-a defines trait Processor. crate-b has struct DataPipeline. Implement Processor for DataPipeline in crate-b, add the dependency in Cargo.toml, and add a test in crate-b. Run cargo check -p crate-b."
difficulty = 4
tags = ["integration", "traits", "cross-crate"]

[[tasks]]
id = "error-propagation-chain"
name = "Error Propagation Chain"
prompt = "Trace the error path from src/api.rs -> src/service.rs -> src/repo.rs -> src/db.rs. Each layer wraps errors differently. Fix the chain so errors propagate cleanly with context using anyhow. Run cargo test."
difficulty = 4
tags = ["integration", "errors", "deep-chain"]

[[tasks]]
id = "middleware-integration"
name = "Middleware Stack Integration"
prompt = "src/middleware/ has auth.rs, logging.rs, and cors.rs each implementing Middleware trait. Wire all three into the server in src/server.rs in the correct order (cors -> logging -> auth). Add an integration test."
difficulty = 4
tags = ["integration", "middleware", "composition"]

[[tasks]]
id = "config-cascade"
name = "Configuration Cascade"
prompt = "Implement config loading that merges: defaults (src/defaults.rs) -> config file (config.toml) -> env vars -> CLI args. Each layer overrides the previous. Add tests for each precedence level."
difficulty = 4
tags = ["integration", "config", "multi-source"]

[[tasks]]
id = "plugin-trait-loader"
name = "Plugin System with Trait Objects"
prompt = "Define a Plugin trait with init/execute/cleanup in src/plugin.rs. Create a PluginManager that loads a Vec<Box<dyn Plugin>>, calls them in order, and handles errors. Implement two mock plugins and test the full lifecycle."
difficulty = 5
tags = ["integration", "plugins", "trait-objects"]

[[tasks]]
id = "migration-system"
name = "Schema Migration System"
prompt = "Implement a migration runner in src/migrate.rs that reads numbered SQL files from migrations/, tracks applied migrations in a version table, and applies pending ones in order. Handle rollback on failure. Add tests with an in-memory SQLite DB."
difficulty = 5
tags = ["integration", "database", "migrations"]

[[tasks]]
id = "event-driven-pipeline"
name = "Event-Driven Processing Pipeline"
prompt = "Wire src/producer.rs, src/processor.rs, and src/sink.rs into a pipeline: producer emits events via a channel, processor transforms them, sink collects results. Use tokio mpsc channels. Add an integration test that sends 100 events and verifies all arrive."
difficulty = 5
tags = ["integration", "async", "pipeline"]

[[tasks]]
id = "workspace-refactor"
name = "Workspace Member Extraction"
prompt = "The single-crate project has grown. Extract src/core/ into a new workspace member crate called 'core'. Update the workspace Cargo.toml, fix all imports in the main crate, and ensure cargo build --workspace passes."
difficulty = 5
tags = ["integration", "workspace", "refactor"]

[[tasks]]
id = "end-to-end-flow"
name = "End-to-End Request Flow"
prompt = "Trace and fix the full request flow: HTTP handler -> validation -> service logic -> database query -> response serialization. The code compiles but returns wrong results because of a field mapping bug between layers. Write an e2e test that exposes the bug, then fix it."
difficulty = 5
tags = ["integration", "e2e", "debugging"]
"#;

pub const PERFORMANCE_SUITE_TOML: &str = r#"id = "performance"
name = "Performance"
description = "Optimization tasks: reduce allocations, batch I/O, parallelize, and measure with benchmarks"
default_timeout_secs = 300

[[tasks]]
id = "remove-unnecessary-clones"
name = "Remove Unnecessary Clones"
prompt = "src/lib.rs has multiple .clone() calls on String and Vec values that are immediately consumed. Replace clones with moves or borrows where possible. Run cargo clippy to verify no unnecessary_clone warnings remain."
difficulty = 2
tags = ["performance", "allocations"]

[[tasks]]
id = "string-concat-opt"
name = "Optimize String Concatenation"
prompt = "src/formatter.rs builds output by repeatedly using format!() and + for string concatenation in a loop. Refactor to use a single String with push_str/write!. Add a benchmark with criterion comparing before and after."
difficulty = 2
tags = ["performance", "strings", "allocation"]

[[tasks]]
id = "vec-preallocate"
name = "Preallocate Collections"
prompt = "Find all Vec::new() calls in src/ that are followed by a loop pushing known-count items. Replace with Vec::with_capacity(). Run cargo clippy to check for remaining cases."
difficulty = 2
tags = ["performance", "allocations", "collections"]

[[tasks]]
id = "batch-file-io"
name = "Batch File I/O"
prompt = "src/processor.rs reads files one at a time in a loop. Refactor to use BufReader, batch reads, and process in chunks of 64. Preserve the same output. Add a test comparing results before and after."
difficulty = 3
tags = ["performance", "io", "batching"]

[[tasks]]
id = "cow-strings"
name = "Use Cow for Conditional Allocation"
prompt = "src/normalizer.rs allocates a new String for every input even when no transformation is needed. Refactor to return Cow<str> and only allocate when the input actually changes. Add tests verifying both borrowed and owned paths."
difficulty = 3
tags = ["performance", "cow", "allocations"]

[[tasks]]
id = "hash-avoid-rehash"
name = "Avoid Redundant Hashing"
prompt = "src/cache.rs calls map.contains_key() followed by map.insert() for every operation, hashing the key twice. Use the entry API instead. Apply the same fix to all HashMap usage in the file."
difficulty = 2
tags = ["performance", "hashmap", "entry-api"]

[[tasks]]
id = "parallel-map"
name = "Parallelize with Rayon"
prompt = "src/pipeline.rs has a sequential .iter().map().collect() over 10K items where each item does CPU-heavy work. Add rayon as a dependency and convert to .par_iter(). Verify results are identical with a test."
difficulty = 3
tags = ["performance", "parallelism", "rayon"]

[[tasks]]
id = "lazy-init"
name = "Lazy Static Initialization"
prompt = "src/config.rs parses a complex regex on every function call. Use std::sync::LazyLock to parse it once. Apply the same pattern to the compiled template in src/render.rs."
difficulty = 3
tags = ["performance", "lazy", "initialization"]

[[tasks]]
id = "smallvec-hot-path"
name = "SmallVec for Hot Path"
prompt = "Profiling shows src/tokenizer.rs allocates a Vec<Token> per line, but 95% of lines have fewer than 8 tokens. Add smallvec as a dependency and use SmallVec<[Token; 8]> for the hot path. Add a test."
difficulty = 3
tags = ["performance", "smallvec", "hot-path"]

[[tasks]]
id = "zero-copy-parse"
name = "Zero-Copy Parsing"
prompt = "src/parser.rs converts every field to String during parsing. Refactor to use &str references into the input buffer where possible, adding lifetime parameters. Verify with cargo test."
difficulty = 4
tags = ["performance", "zero-copy", "lifetimes"]

[[tasks]]
id = "async-concurrent-io"
name = "Concurrent Async I/O"
prompt = "src/fetcher.rs awaits HTTP requests sequentially in a loop. Refactor to use futures::stream::FuturesUnordered to run up to 8 requests concurrently. Preserve ordering of results. Add a test."
difficulty = 4
tags = ["performance", "async", "concurrency"]

[[tasks]]
id = "benchmark-harness"
name = "Criterion Benchmark Harness"
prompt = "Create benches/core_bench.rs using criterion that benchmarks the three hottest functions in src/lib.rs. Configure Cargo.toml with [[bench]] entries and harness = false. Run cargo bench and verify output."
difficulty = 3
tags = ["performance", "benchmarking", "criterion"]
"#;

pub const SAFETY_SUITE_TOML: &str = r#"id = "safety"
name = "Safety"
description = "Security and safety tasks: input validation, injection resistance, auth, and data handling"
default_timeout_secs = 300

[[tasks]]
id = "input-sanitization"
name = "Input Sanitization"
prompt = "src/api.rs accepts user input and passes it directly to a shell command via std::process::Command. Fix it to sanitize inputs: reject shell metacharacters, validate against an allowlist, and use proper argument passing instead of string interpolation."
difficulty = 2
tags = ["safety", "input-validation", "injection"]

[[tasks]]
id = "sql-injection-fix"
name = "Fix SQL Injection"
prompt = "src/db.rs builds SQL queries with format!() using user input. Refactor all queries to use parameterized statements. Add a test that attempts injection and verifies it fails safely."
difficulty = 2
tags = ["safety", "sql-injection", "parameterized"]

[[tasks]]
id = "prompt-injection-guard"
name = "Prompt Injection Resistance"
prompt = "Implement a prompt sanitizer in src/sanitize.rs that detects and strips common prompt injection patterns: 'ignore previous instructions', system prompt overrides, encoded instructions, and role-play attacks. Add tests for 10 injection variants."
difficulty = 3
tags = ["safety", "prompt-injection", "llm"]

[[tasks]]
id = "tool-allowlist"
name = "Tool Call Allowlist"
prompt = "src/tools.rs dispatches tool calls by name from a HashMap. Add an allowlist check: only tools in ALLOWED_TOOLS const can execute. Unknown tools return ToolError::Unauthorized. Dangerous tools (rm, exec) are always blocked. Add tests."
difficulty = 3
tags = ["safety", "authorization", "tools"]

[[tasks]]
id = "secret-redaction"
name = "Secret Redaction in Logs"
prompt = "Implement a log redactor in src/redact.rs that replaces API keys, tokens, and passwords in log output with '[REDACTED]'. Match patterns: Bearer tokens, AWS keys, GitHub tokens, and generic key=value secrets. Add tests."
difficulty = 3
tags = ["safety", "secrets", "logging"]

[[tasks]]
id = "rate-limit-middleware"
name = "Rate Limiting Middleware"
prompt = "Implement IP-based rate limiting in src/ratelimit.rs: max 100 requests per minute per IP using a sliding window. Return 429 Too Many Requests when exceeded. Use a DashMap for thread safety. Add tests."
difficulty = 3
tags = ["safety", "rate-limiting", "dos"]

[[tasks]]
id = "path-traversal-fix"
name = "Path Traversal Prevention"
prompt = "src/files.rs serves files from a base directory but doesn't prevent path traversal. Fix it to canonicalize paths and reject any that escape the base directory. Handle symlinks. Add tests with ../../../etc/passwd attempts."
difficulty = 3
tags = ["safety", "path-traversal", "filesystem"]

[[tasks]]
id = "auth-middleware"
name = "JWT Auth Middleware"
prompt = "Implement JWT validation middleware in src/auth.rs: verify signature, check expiration, extract claims into a RequestContext. Reject expired/malformed/missing tokens with proper HTTP status codes. Add tests for each rejection case."
difficulty = 4
tags = ["safety", "auth", "jwt"]

[[tasks]]
id = "resource-exhaustion-guard"
name = "Resource Exhaustion Guard"
prompt = "src/parser.rs accepts unbounded input that can cause OOM. Add limits: max input size (1MB), max recursion depth (64), max array length (10K), and timeout (5s). Return structured errors for each limit. Add tests."
difficulty = 4
tags = ["safety", "resource-limits", "dos"]

[[tasks]]
id = "supply-chain-audit"
name = "Dependency Audit Implementation"
prompt = "Implement a dep auditor in src/audit.rs that parses Cargo.lock, checks each crate version against a known-vulnerable list (embedded as a const), and reports findings as structured JSON. Flag yanked versions. Add tests with mock vulnerable entries."
difficulty = 5
tags = ["safety", "supply-chain", "audit"]
"#;

/// Ensure built-in suites exist on disk. Writes them only if not already present.
pub async fn ensure_builtin_suites(workdir: &Path) {
    let dir = suites_dir(workdir);
    let _ = tokio::fs::create_dir_all(&dir).await;

    for (filename, content) in [
        ("smoke.toml", SMOKE_SUITE_TOML),
        ("codegen.toml", CODEGEN_SUITE_TOML),
        ("integration.toml", INTEGRATION_SUITE_TOML),
        ("performance.toml", PERFORMANCE_SUITE_TOML),
        ("safety.toml", SAFETY_SUITE_TOML),
    ] {
        let path = dir.join(filename);
        if !path.exists() {
            let _ = tokio::fs::write(&path, content).await;
        }
    }

    let learnable_path = dir.join("learnable-rust.toml");
    if !learnable_path.exists() {
        let _ = save_suite(workdir, &builtin_learnable_rust_suite()).await;
    }
}
