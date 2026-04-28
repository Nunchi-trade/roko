//! Core types and storage helpers for the bench system.
//!
//! A bench run executes a suite of prompt-based tasks through `run_once()`,
//! collecting pass/fail results, timing, and token usage per task. Results
//! persist to `.roko/bench/` for comparison and pareto analysis.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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

/// Configuration overrides applied to each task in a bench run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchConfigOverrides {
    /// Force a specific model slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Force a specific agent backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// Maximum tokens for the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Temperature override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

/// How a bench run was triggered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchRunKind {
    /// Triggered from the UI / API.
    Manual,
    /// Scheduled / automated.
    Scheduled,
    /// Comparison A/B test.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchTaskResult {
    /// Task identifier.
    pub task_id: String,
    /// Task name.
    pub task_name: String,
    /// Whether the task passed (run_once succeeded + optional output match).
    pub passed: bool,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Model that was actually used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_used: Option<String>,
    /// Input tokens consumed.
    #[serde(default)]
    pub input_tokens: u64,
    /// Output tokens generated.
    #[serde(default)]
    pub output_tokens: u64,
    /// Estimated cost in USD.
    #[serde(default)]
    pub cost_usd: f64,
    /// Output text (truncated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    /// Error message if the task failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Aggregate summary for a completed bench run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchRunSummary {
    /// Total tasks in the suite.
    pub total_tasks: usize,
    /// Number of tasks that passed.
    pub passed: usize,
    /// Number of tasks that failed.
    pub failed: usize,
    /// Pass rate as a fraction (0.0 - 1.0).
    pub pass_rate: f64,
    /// Total execution time in milliseconds.
    pub total_duration_ms: u64,
    /// Total estimated cost in USD.
    pub total_cost_usd: f64,
    /// Total input tokens across all tasks.
    pub total_input_tokens: u64,
    /// Total output tokens across all tasks.
    pub total_output_tokens: u64,
}

impl BenchRunSummary {
    /// Compute summary from task results.
    pub fn from_results(results: &[BenchTaskResult]) -> Self {
        let total_tasks = results.len();
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = total_tasks - passed;
        let pass_rate = if total_tasks > 0 {
            passed as f64 / total_tasks as f64
        } else {
            0.0
        };
        let total_duration_ms = results.iter().map(|r| r.duration_ms).sum();
        let total_cost_usd = results.iter().map(|r| r.cost_usd).sum();
        let total_input_tokens = results.iter().map(|r| r.input_tokens).sum();
        let total_output_tokens = results.iter().map(|r| r.output_tokens).sum();

        Self {
            total_tasks,
            passed,
            failed,
            pass_rate,
            total_duration_ms,
            total_cost_usd,
            total_input_tokens,
            total_output_tokens,
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
    pub overrides: BenchConfigOverrides,
    /// Optional label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Run status.
    pub status: BenchRunStatus,
    /// When the run started (Unix timestamp seconds).
    pub started_at: u64,
    /// When the run finished (Unix timestamp seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

/// Lightweight index entry for fast listing.
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
    /// When the run started.
    pub started_at: u64,
    /// When the run finished.
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

/// Save a bench run to disk.
pub async fn save_bench_run(workdir: &Path, run: &BenchRun) -> anyhow::Result<()> {
    let dir = runs_dir(workdir);
    tokio::fs::create_dir_all(&dir).await?;
    let path = run_path(workdir, &run.id);
    let json = serde_json::to_string_pretty(run)?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, json).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
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

/// Update an existing index entry (rewrite the full file).
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
    tokio::fs::write(&path, content).await?;
    Ok(())
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

use tokio::io::AsyncWriteExt;

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
"#;

/// Ensure built-in suites exist on disk. Writes them only if not already present.
pub async fn ensure_builtin_suites(workdir: &Path) {
    let dir = suites_dir(workdir);
    let _ = tokio::fs::create_dir_all(&dir).await;

    for (filename, content) in [
        ("smoke.toml", SMOKE_SUITE_TOML),
        ("codegen.toml", CODEGEN_SUITE_TOML),
    ] {
        let path = dir.join(filename);
        if !path.exists() {
            let _ = tokio::fs::write(&path, content).await;
        }
    }
}
