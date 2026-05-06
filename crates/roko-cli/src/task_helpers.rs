//! Task-level helpers extracted from `orchestrate.rs`.
//!
//! This module contains:
//! - Crate derivation from file paths (`crate_name_for_path`, `crate_root_for_path`)
//! - Full-crate source reading for context injection
//! - Tasks.toml validation and logging
//! - Task-definition conversions (to `TaskInput`, `Task`, CLI args)
//! - Task output persistence and truncation
//! - Review drift detection
//! - Symbol extraction, attestation, conductor signal building

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow};
use roko_core::attestation::{self, SigningKey};
use roko_core::{Body, ContentHash, Engram, Task, TaskStatus};

use crate::task_parser::{TaskValidationIssue, TasksFile};

// ── Crate helpers ─────────────────────────────────────────────────────

/// Derive a crate name from the task's modified files.
pub(crate) fn task_crate_name(task_def: Option<&crate::task_parser::TaskDef>) -> Option<String> {
    let mut seen = HashSet::new();
    task_def
        .into_iter()
        .flat_map(|task| task.files.iter())
        .filter_map(|file| crate_name_for_path(file))
        .find(|crate_name| seen.insert(crate_name.clone()))
}

/// Collect all distinct crate names from a task's modified files.
///
/// Used by gate dispatch to scope `cargo check` / `cargo clippy` to only
/// the crates a task touches, avoiding false failures from pre-existing
/// errors in unrelated crates.
pub(crate) fn task_target_crates(task_def: Option<&crate::task_parser::TaskDef>) -> Vec<String> {
    let mut seen = HashSet::new();
    task_def
        .into_iter()
        .flat_map(|task| task.files.iter())
        .filter_map(|file| crate_name_for_path(file))
        .filter(|name| seen.insert(name.clone()))
        .collect()
}

/// Best-effort crate key derivation from a repository-relative file path.
pub(crate) fn crate_name_for_path(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    match parts.as_slice() {
        [first, second, ..] if *first == "crates" || *first == "apps" => {
            Some((*second).to_string())
        }
        [first, second, ..] if matches!(*second, "src" | "tests" | "benches") => {
            Some((*first).to_string())
        }
        [first, ..] if matches!(*first, "src" | "tests" | "benches") => {
            Some("workspace".to_string())
        }
        [first, ..] if *first == "Cargo.toml" => Some("workspace".to_string()),
        _ => None,
    }
}

pub(crate) fn crate_root_for_path(path: &str) -> Option<PathBuf> {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    match parts.as_slice() {
        [first, second, ..] if *first == "crates" || *first == "apps" => {
            Some(PathBuf::from(first).join(second))
        }
        [first, ..] if matches!(*first, "src" | "tests" | "benches" | "examples") => {
            Some(PathBuf::new())
        }
        [first] if matches!(*first, "Cargo.toml" | "build.rs") => Some(PathBuf::new()),
        _ => None,
    }
}

pub(crate) fn collect_crate_source_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_crate_source_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }

    Ok(())
}

pub(crate) fn read_full_crate_source(crate_root: &Path) -> Result<String> {
    let mut files = Vec::new();

    for path in [crate_root.join("Cargo.toml"), crate_root.join("build.rs")] {
        if path.is_file() {
            files.push(path);
        }
    }
    for dir in ["src", "tests", "benches", "examples"] {
        collect_crate_source_files(&crate_root.join(dir), &mut files)?;
    }

    files.sort();
    files.dedup();

    let mut combined = String::new();
    for path in files {
        let contents =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let relative = path.strip_prefix(crate_root).unwrap_or(path.as_path());
        combined.push_str(&format!(
            "// FILE: {}\n{}\n\n",
            relative.display(),
            contents
        ));
    }

    Ok(combined)
}

// ── Task validation ───────────────────────────────────────────────────

pub(crate) fn log_tasks_validation_issue(
    plan_id: &str,
    plan_base: &str,
    tasks_path: &Path,
    issue: &TaskValidationIssue,
) {
    match issue {
        TaskValidationIssue::MissingRequiredField { task_id, field } => {
            tracing::error!(
                target: "plan_validation",
                plan_id = %plan_id,
                plan_base = %plan_base,
                tasks_path = %tasks_path.display(),
                issue = "missing_required_field",
                task_id = %task_id,
                field = field,
                "tasks.toml validation failed"
            );
        }
        TaskValidationIssue::UnknownDependency {
            task_id,
            dependency,
        } => {
            tracing::error!(
                target: "plan_validation",
                plan_id = %plan_id,
                plan_base = %plan_base,
                tasks_path = %tasks_path.display(),
                issue = "unknown_dependency",
                task_id = %task_id,
                dependency = %dependency,
                "tasks.toml validation failed"
            );
        }
        TaskValidationIssue::CircularDependency { cycle } => {
            tracing::error!(
                target: "plan_validation",
                plan_id = %plan_id,
                plan_base = %plan_base,
                tasks_path = %tasks_path.display(),
                issue = "circular_dependency",
                cycle = ?cycle,
                "tasks.toml validation failed"
            );
        }
        TaskValidationIssue::NoStartNode => {
            tracing::error!(
                target: "plan_validation",
                plan_id = %plan_id,
                plan_base = %plan_base,
                tasks_path = %tasks_path.display(),
                issue = "no_start_node",
                "tasks.toml validation failed"
            );
        }
    }
}

pub(crate) fn validate_tasks_file_for_execution(
    plan_id: &str,
    plan_base: &str,
    tasks_path: &Path,
    tasks_file: &TasksFile,
) -> Result<()> {
    let issues = tasks_file.validate_structure();
    if issues.is_empty() {
        return Ok(());
    }

    for issue in &issues {
        log_tasks_validation_issue(plan_id, plan_base, tasks_path, issue);
    }

    Err(anyhow!(
        "tasks.toml validation failed for {}",
        tasks_path.display()
    ))
}

// ── Review drift ──────────────────────────────────────────────────────

/// Summary of how tightly a review output stays anchored to the task spec.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewDriftReport {
    pub matched: usize,
    pub expected: usize,
    pub missing: Vec<String>,
}

impl ReviewDriftReport {
    pub fn coverage(&self) -> f64 {
        if self.expected == 0 {
            1.0
        } else {
            self.matched as f64 / self.expected as f64
        }
    }

    pub fn drifted(&self) -> bool {
        self.expected > 0 && self.coverage() < 0.35
    }
}

/// Render the task spec into a reviewable summary block.
pub(crate) fn task_spec_summary(tasks_file: &TasksFile) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "[meta]\nplan = {}\niteration = {}\ntotal = {}\ndone = {}\nstatus = {}\nmax_parallel = {}\nestimated_total_minutes = {}\n",
        tasks_file.meta.plan,
        tasks_file.meta.iteration,
        tasks_file.meta.total,
        tasks_file.meta.done,
        tasks_file.meta.status,
        tasks_file.meta.max_parallel,
        tasks_file.meta.estimated_total_minutes,
    ));

    for task in &tasks_file.tasks {
        out.push_str(&format!("\n### {} - {}\n", task.id, task.title));
        out.push_str(&format!("tier = {}\n", task.tier));
        if !task.files.is_empty() {
            out.push_str("files:\n");
            for file in &task.files {
                out.push_str(&format!("- {file}\n"));
            }
        }
        if !task.depends_on.is_empty() {
            out.push_str(&format!("depends_on = {}\n", task.depends_on.join(", ")));
        }
        if !task.depends_on_plan.is_empty() {
            out.push_str(&format!(
                "depends_on_plan = {}\n",
                task.depends_on_plan.join(", ")
            ));
        }
        if !task.acceptance.is_empty() {
            out.push_str("acceptance:\n");
            for item in &task.acceptance {
                out.push_str(&format!("- {item}\n"));
            }
        }
        if !task.verify.is_empty() {
            out.push_str("verify:\n");
            for step in &task.verify {
                out.push_str(&format!("- [{}] {}\n", step.phase, step.command));
            }
        }
    }

    out
}

pub(crate) fn significant_terms(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "the",
        "and",
        "for",
        "with",
        "from",
        "into",
        "that",
        "this",
        "task",
        "plan",
        "should",
        "must",
        "have",
        "has",
        "are",
        "was",
        "were",
        "will",
        "would",
        "could",
        "can",
        "done",
        "make",
        "build",
        "update",
        "implement",
        "review",
        "please",
        "then",
        "than",
        "when",
    ];

    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '/') {
        let term = raw.trim().to_lowercase();
        if term.len() < 4 || STOP_WORDS.contains(&term.as_str()) {
            continue;
        }
        if seen.insert(term.clone()) {
            terms.push(term);
        }
    }
    terms
}

pub(crate) fn review_drift_report(
    tasks_file: &TasksFile,
    output: &str,
) -> Option<ReviewDriftReport> {
    let lower = output.to_lowercase();
    let mut expected = Vec::new();
    let mut seen = HashSet::new();

    let mut push_expected = |value: String| {
        let value = value.trim().to_lowercase();
        if value.is_empty() {
            return;
        }
        if seen.insert(value.clone()) {
            expected.push(value);
        }
    };

    for task in &tasks_file.tasks {
        push_expected(task.id.clone());
        push_expected(task.title.clone());

        for term in significant_terms(&task.title) {
            push_expected(term);
        }

        for file in &task.files {
            push_expected(file.clone());
            if let Some(name) = std::path::Path::new(file)
                .file_name()
                .and_then(|n| n.to_str())
            {
                push_expected(name.to_string());
            }
        }

        for verify in &task.verify {
            push_expected(verify.phase.clone());
        }

        for acceptance in &task.acceptance {
            push_expected(acceptance.clone());
            for term in significant_terms(acceptance) {
                push_expected(term);
            }
        }

        for anti_pattern in task
            .context
            .as_ref()
            .map(|ctx| ctx.anti_patterns.iter())
            .into_iter()
            .flatten()
        {
            push_expected(anti_pattern.clone());
            for term in significant_terms(anti_pattern) {
                push_expected(term);
            }
        }
    }

    if expected.is_empty() {
        return None;
    }

    let mut matched = 0usize;
    let mut missing = Vec::new();
    for anchor in &expected {
        if lower.contains(anchor) {
            matched += 1;
        } else {
            missing.push(anchor.clone());
        }
    }

    Some(ReviewDriftReport {
        matched,
        expected: expected.len(),
        missing,
    })
}

/// Parse a review verdict from agent output text.
///
/// Looks for `verdict = "approve"` / `verdict = "revise"` patterns,
/// falls back to keyword matching. Returns `true` for approve.
pub(crate) fn parse_review_verdict(output: &str) -> bool {
    let lower = output.to_lowercase();
    // Structured verdict
    if lower.contains("verdict = \"approve\"") || lower.contains("verdict: approve") {
        return true;
    }
    if lower.contains("verdict = \"revise\"")
        || lower.contains("verdict: revise")
        || lower.contains("verdict = \"reject\"")
        || lower.contains("verdict: reject")
    {
        return false;
    }
    // Keyword fallback
    if lower.contains("approved") || lower.contains("lgtm") || lower.contains("looks good") {
        return true;
    }
    if lower.contains("revise") || lower.contains("reject") || lower.contains("rework") {
        return false;
    }
    // Default: approve (don't block pipeline on ambiguous output)
    true
}

// ── Task conversions ──────────────────────────────────────────────────

/// Convert a `TaskDef` (from the CLI's task_parser) into a `TaskInput`
/// (from roko-compose's `context_provider`). This bridges the two crate
/// boundaries without creating a dependency.
pub(crate) fn task_def_to_input(td: &crate::task_parser::TaskDef) -> roko_compose::TaskInput {
    let (read_files, symbols, anti_patterns, prior_failures) = match &td.context {
        Some(ctx) => (
            ctx.read_files
                .iter()
                .map(|rf| roko_compose::ReadFileSpec {
                    path: rf.path.clone(),
                    lines: rf.lines.clone(),
                    why: rf.why.clone(),
                })
                .collect(),
            ctx.symbols.clone(),
            ctx.anti_patterns.clone(),
            ctx.prior_failures.clone(),
        ),
        None => (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    };

    roko_compose::TaskInput {
        id: td.id.clone(),
        title: td.title.clone(),
        description: td.description.clone(),
        tier: td.tier.clone(),
        files: td.files.clone(),
        read_files,
        symbols,
        anti_patterns,
        prior_failures,
        verify_commands: td
            .verify
            .iter()
            .map(|v| roko_compose::VerifySpec {
                phase: v.phase.clone(),
                command: v.command.clone(),
                fail_msg: v.fail_msg.clone(),
            })
            .collect(),
        acceptance: td.acceptance.clone(),
        depends_on: td.depends_on.clone(),
        max_loc: td.max_loc,
    }
}

pub(crate) fn task_def_to_dag_task(task: &crate::task_parser::TaskDef, completed: bool) -> Task {
    let mut dag_task = Task::new(task.id.clone(), task.title.clone());
    dag_task.status = if completed {
        TaskStatus::Done
    } else {
        TaskStatus::Pending
    };
    dag_task.files = task.files.clone();
    dag_task.role = task.role.clone();
    dag_task.acceptance = task.acceptance.clone();
    dag_task.depends_on = task.depends_on.clone();
    dag_task
}

/// Convert declared task context files into Claude CLI `--read` args.
pub(crate) fn task_read_cli_args(task_def: &crate::task_parser::TaskDef) -> Vec<String> {
    task_def
        .context
        .as_ref()
        .map(|ctx| {
            ctx.read_files
                .iter()
                .flat_map(|rf| ["--read".to_string(), rf.path.clone()])
                .collect()
        })
        .unwrap_or_default()
}

// ── File and output helpers ───────────────────────────────────────────

pub(crate) fn file_contains_public_api(path: &str, content: &str) -> bool {
    let normalized = path.replace('\\', "/");
    if normalized.ends_with("/src/lib.rs") || normalized.ends_with("/src/mod.rs") {
        return true;
    }

    content.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub struct ")
            || trimmed.starts_with("pub enum ")
            || trimmed.starts_with("pub trait ")
            || trimmed.starts_with("pub type ")
            || trimmed.starts_with("pub use ")
            || trimmed.starts_with("pub mod ")
    })
}

pub(crate) fn truncate_doc_snippet(content: &str, max_chars: usize) -> String {
    let mut chars = content.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_none() {
        content.to_string()
    } else {
        format!("{truncated}\n\n[... truncated]")
    }
}

/// Load prior task outputs from `.roko/task-outputs/{task_id}.txt`.
///
/// When a task completes successfully, we persist a summary of its output
/// so that downstream tasks can reference it. If no outputs exist on disk,
/// returns an empty vec.
pub(crate) fn load_prior_task_outputs(
    workdir: &Path,
    depends_on: &[String],
) -> Vec<roko_compose::PriorTaskOutput> {
    let output_dir = workdir.join(".roko").join("task-outputs");
    let mut outputs = Vec::new();

    for dep_id in depends_on {
        let output_path = output_dir.join(format!("{dep_id}.txt"));
        if let Ok(summary) = std::fs::read_to_string(&output_path) {
            if !summary.trim().is_empty() {
                outputs.push(roko_compose::PriorTaskOutput {
                    task_id: dep_id.clone(),
                    summary,
                });
            }
        }
    }

    outputs
}

/// Maximum output size stored in task outputs and episode context (32 KB).
pub(crate) const MAX_OUTPUT_BYTES: usize = 32_768;
/// Number of output lines to include in task failure logs.
pub(crate) const TASK_FAILURE_OUTPUT_TAIL_LINES: usize = 20;

/// Truncate an agent output string, keeping the last N lines if it exceeds
/// `MAX_OUTPUT_BYTES` and prepending a truncation header.
pub(crate) fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_OUTPUT_BYTES {
        return output.to_string();
    }
    // Keep the tail -- the most recent output is usually most relevant.
    let tail = &output[output.len() - MAX_OUTPUT_BYTES..];
    // Find the first newline to avoid a partial first line.
    let start = tail.find('\n').map_or(0, |i| i + 1);
    format!(
        "[truncated: original {} bytes, showing last {} bytes]\n{}",
        output.len(),
        MAX_OUTPUT_BYTES,
        &tail[start..]
    )
}

/// Return the last `line_count` lines from `output`, preserving order.
pub(crate) fn tail_output_lines(output: &str, line_count: usize) -> String {
    if output.is_empty() || line_count == 0 {
        return String::new();
    }

    let mut lines: Vec<&str> = output.lines().rev().take(line_count).collect();
    lines.reverse();
    lines.join("\n")
}

/// Pull likely Rust symbol names out of task text for skill extraction.
pub(crate) fn extract_task_symbols(text: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    let mut seen = HashSet::new();

    for raw in text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == ':')) {
        if raw.is_empty() {
            continue;
        }

        for candidate in raw.split("::") {
            let candidate =
                candidate.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_');
            if candidate.len() < 3 {
                continue;
            }
            let has_underscore = candidate.contains('_');
            let has_upper = candidate.chars().any(|ch| ch.is_ascii_uppercase());
            if !has_underscore && !has_upper {
                continue;
            }

            let candidate = candidate.to_string();
            if seen.insert(candidate.clone()) {
                symbols.push(candidate);
            }
        }
    }

    symbols
}

/// Add task failure context to an error chain.
pub(crate) fn with_task_failure_context(
    error: anyhow::Error,
    task_id: &str,
    phase: &str,
    gate: &str,
    output_tail: Option<&str>,
) -> anyhow::Error {
    let error = error
        .context(format!("task_id={task_id}"))
        .context(format!("phase={phase}"))
        .context(format!("gate={gate}"));

    match output_tail {
        Some(tail) if !tail.trim().is_empty() => error.context(format!(
            "agent_output_tail_last_{}_lines:\n{}",
            TASK_FAILURE_OUTPUT_TAIL_LINES, tail
        )),
        _ => error.context(format!(
            "agent_output_tail_last_{}_lines: <unavailable>",
            TASK_FAILURE_OUTPUT_TAIL_LINES
        )),
    }
}

/// Persist a task's output summary so downstream tasks can reference it.
pub(crate) fn save_task_output(
    workdir: &Path,
    task_id: &str,
    output: &str,
    hub: Option<&crate::state_hub::StateHubSender>,
) {
    let output_dir = workdir.join(".roko").join("task-outputs");
    let _ = std::fs::create_dir_all(&output_dir);
    let output_path = output_dir.join(format!("{task_id}.txt"));
    let summary = truncate_output(output);
    let _ = std::fs::write(output_path, &summary);

    if let Some(hub) = hub {
        let lines: Vec<String> = summary.lines().map(String::from).collect();
        hub.publish(roko_core::DashboardEvent::TaskOutputAppended {
            task_id: task_id.to_string(),
            lines,
        });
    }
}

// ── Attestation and signal helpers ────────────────────────────────────

pub(crate) fn attestation_signing_key_from_env() -> Option<SigningKey> {
    let seed = std::env::var("ROKO_ATTEST_SIGNING_KEY_HEX").ok()?;
    let seed = seed.trim().trim_start_matches("0x");
    let hash = ContentHash::from_hex(seed)?;
    Some(SigningKey::from_bytes(&hash.0))
}

pub(crate) fn maybe_attest_engram(mut signal: Engram) -> Engram {
    if signal.attestation.is_none()
        && let Some(key) = attestation_signing_key_from_env()
    {
        signal.attestation = Some(attestation::sign(&signal, &key));
    }
    signal
}

pub(crate) fn conductor_signal_from_output(output: &Engram) -> Option<Engram> {
    let body = match &output.body {
        Body::Text(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            Body::text(trimmed)
        }
        Body::Json(value) => Body::Json(value.clone()),
        Body::Bytes(bytes) => {
            if bytes.is_empty() {
                return None;
            }
            Body::Bytes(bytes.clone())
        }
        Body::Empty => return None,
    };

    let mut builder = Engram::builder(output.kind.clone())
        .body(body)
        .provenance(output.provenance.clone())
        .lineage(
            output
                .lineage
                .iter()
                .copied()
                .chain(std::iter::once(output.id)),
        );
    for (key, value) in &output.tags {
        builder = builder.tag(key.clone(), value.clone());
    }
    if let Some(attestation) = output.attestation.clone() {
        builder = builder.attestation(attestation);
    }
    if let Some(emotional_tag) = output.emotional_tag.clone() {
        builder = builder.emotional_tag(emotional_tag);
    }
    Some(maybe_attest_engram(builder.build()))
}
