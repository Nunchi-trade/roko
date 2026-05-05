//! Parse `tasks.toml` into structured task definitions with tiers, operating
//! frequencies, context, dependencies, model hints, and per-task verification
//! pipelines.
//!
//! This is the bridge between the plan generator output and the orchestrator.
//! It reads the extended task format (with `tier`, `model_hint`, `context`,
//! and `verify` sections) and produces [`TaskDef`]s that the executor uses
//! to select models, assemble context, and verify results.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context as _, Result};
use roko_core::{OperatingFrequency, TaskDomain};
use roko_gate::AcceptanceContract;
use roko_orchestrator::{ReplanStrategy, detect_cycle_nodes};
use roko_std::denied_tools_for_role;
use serde::{Deserialize, Deserializer, Serialize};

/// Parsed `[meta]` section of tasks.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMeta {
    pub plan: String,
    #[serde(default)]
    pub iteration: u32,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub done: u32,
    #[serde(default)]
    pub status: String,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: u32,
    #[serde(default)]
    pub estimated_total_minutes: u32,
    /// When `true`, skip the enrichment pipeline and transition directly to
    /// implementing.  Useful for pre-authored plans where tasks.toml already
    /// contains complete definitions.
    #[serde(default)]
    pub skip_enrichment: bool,
}

fn default_max_parallel() -> u32 {
    1
}

/// A single task definition.
#[derive(Debug, Clone, Serialize)]
pub struct TaskDef {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub role: Option<String>,
    pub status: String,
    /// Task complexity tier: mechanical, focused, integrative, architectural.
    pub tier: String,
    /// Optional operating frequency override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<OperatingFrequency>,
    /// Suggested model for this task.
    pub model_hint: Option<String>,
    /// Explicit replan strategy override for this task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replan_strategy: Option<ReplanStrategy>,
    /// Maximum lines of change.
    pub max_loc: Option<u32>,
    /// Files this task modifies.
    pub files: Vec<String>,
    /// Tool names this task is allowed to use.
    pub allowed_tools: Option<Vec<String>>,
    /// Tool names this task is forbidden to use.
    pub denied_tools: Option<Vec<String>>,
    /// MCP server names this task needs.
    pub mcp_servers: Option<Vec<String>>,
    /// Task IDs this task depends on.
    pub depends_on: Vec<String>,
    /// Plan IDs this task depends on before dispatching.
    pub depends_on_plan: Vec<String>,
    /// Subtasks created when this task is decomposed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_into: Option<Vec<String>>,
    /// Surgical context specification.
    pub context: Option<TaskContext>,
    /// Verification pipeline.
    pub verify: Vec<VerifyStep>,
    /// Per-task timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum retry attempts for this task.
    pub max_retries: u32,
    /// Free-form acceptance criteria (legacy format, strings).
    pub acceptance: Vec<String>,
    /// Typed done-gate contract for self-hosting tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_contract: Option<AcceptanceContract>,
    /// Work domain — controls gate selection and git policy.
    pub domain: Option<TaskDomain>,
    /// Definition order index (0-based) from the TOML array. Used for
    /// tie-breaking in DAG resolution so tasks without dependency constraints
    /// execute in the order they were authored, not alphabetically.
    #[serde(default)]
    pub sequence: usize,
}

impl TaskDef {
    /// Resolve the effective domain for this task: explicit > config default > None.
    #[must_use]
    pub fn effective_domain(&self, config_default: Option<&TaskDomain>) -> Option<TaskDomain> {
        self.domain.clone().or_else(|| config_default.cloned())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskDefSerde {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default = "default_tier")]
    pub tier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<OperatingFrequency>,
    #[serde(default)]
    pub model_hint: Option<String>,
    #[serde(default)]
    pub replan_strategy: Option<ReplanStrategy>,
    #[serde(default)]
    pub max_loc: Option<u32>,
    #[serde(default, alias = "write_files")]
    pub files: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub denied_tools: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_servers: Option<Vec<String>>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub depends_on_plan: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_into: Option<Vec<String>>,
    #[serde(default)]
    pub context: Option<TaskContext>,
    #[serde(default)]
    pub verify: Vec<VerifyStep>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default)]
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub acceptance_contract: Option<AcceptanceContract>,
    #[serde(default)]
    pub domain: Option<TaskDomain>,
}

impl From<TaskDefSerde> for TaskDef {
    fn from(raw: TaskDefSerde) -> Self {
        let mut task = Self {
            id: raw.id,
            title: raw.title,
            description: raw.description,
            role: raw.role,
            status: raw.status,
            tier: raw.tier,
            frequency: raw.frequency,
            model_hint: raw.model_hint,
            replan_strategy: raw.replan_strategy,
            max_loc: raw.max_loc,
            files: raw.files,
            allowed_tools: raw.allowed_tools,
            denied_tools: raw.denied_tools,
            mcp_servers: raw.mcp_servers,
            depends_on: raw.depends_on,
            depends_on_plan: raw.depends_on_plan,
            split_into: raw.split_into,
            context: raw.context,
            verify: raw.verify,
            timeout_secs: raw.timeout_secs,
            max_retries: raw.max_retries,
            acceptance: raw.acceptance,
            acceptance_contract: raw.acceptance_contract,
            domain: raw.domain,
            sequence: 0, // stamped by TasksFile::parse_str after deserialization
        };
        task.apply_role_tool_defaults();
        task
    }
}

/// Structural validation issue detected in a `tasks.toml` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskValidationIssue {
    /// A required field was missing or empty.
    MissingRequiredField {
        /// Task identifier being validated.
        task_id: String,
        /// Missing field name.
        field: &'static str,
    },
    /// A dependency points at a task or plan that does not exist.
    UnknownDependency {
        /// Task identifier being validated.
        task_id: String,
        /// Missing dependency identifier.
        dependency: String,
    },
    /// One or more tasks participate in a dependency cycle.
    CircularDependency {
        /// Task identifiers involved in the cycle.
        cycle: Vec<String>,
    },
    /// No task starts without dependencies.
    NoStartNode,
}

/// Non-blocking quality warning detected in a `tasks.toml` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskQualityWarning {
    /// Task description is likely too broad for surgical execution.
    LongDescription {
        /// Task identifier being checked.
        task_id: String,
        /// Number of words in the description.
        word_count: usize,
    },
    /// Task does not declare any files to read for context.
    MissingReadFiles {
        /// Task identifier being checked.
        task_id: String,
    },
    /// Task does not declare any executable verification commands.
    MissingVerify {
        /// Task identifier being checked.
        task_id: String,
    },
    /// Plan contains more tasks than we expect in one batch.
    TooManyTasks {
        /// Total task count in the plan.
        task_count: usize,
    },
}

/// A task that is missing one or more modern `tasks.toml` fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModernFieldIssue {
    /// Task identifier being checked.
    pub task_id: String,
    /// Modern fields that are absent or empty.
    pub missing_fields: Vec<&'static str>,
}

impl std::fmt::Display for TaskValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRequiredField { task_id, field } => {
                write!(f, "{task_id}: missing required field `{field}`")
            }
            Self::UnknownDependency {
                task_id,
                dependency,
            } => {
                write!(f, "{task_id}: depends on unknown dependency `{dependency}`")
            }
            Self::CircularDependency { cycle } => {
                write!(f, "circular dependency detected: {}", cycle.join(" -> "))
            }
            Self::NoStartNode => write!(f, "no task without dependencies found"),
        }
    }
}

impl std::fmt::Display for TaskQualityWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LongDescription {
                task_id,
                word_count,
            } => write!(
                f,
                "{task_id}: description is {word_count} words (>500); likely too coarse for ≤50 LOC changes"
            ),
            Self::MissingReadFiles { task_id } => {
                write!(
                    f,
                    "{task_id}: missing context.read_files; agent won't have file context"
                )
            }
            Self::MissingVerify { task_id } => {
                write!(
                    f,
                    "{task_id}: missing verify steps; no way to check completion"
                )
            }
            Self::TooManyTasks { task_count } => {
                write!(
                    f,
                    "plan has {task_count} tasks (>20); consider splitting it"
                )
            }
        }
    }
}

impl std::fmt::Display for ModernFieldIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: missing modern fields: {}",
            self.task_id,
            self.missing_fields.join(", ")
        )
    }
}

impl<'de> Deserialize<'de> for TaskDef {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        TaskDefSerde::deserialize(deserializer).map(Into::into)
    }
}

fn default_status() -> String {
    "ready".into()
}

fn default_tier() -> String {
    "focused".into()
}

fn default_timeout_secs() -> u64 {
    600
}

fn default_max_retries() -> u32 {
    3
}

fn is_valid_plan_dependency_reference(dependency: &str, current_plan: &str) -> bool {
    let dependency = dependency.trim();
    if dependency.is_empty() || dependency == current_plan {
        return false;
    }
    if dependency.starts_with('.') || dependency.starts_with('-') || dependency.contains("..") {
        return false;
    }

    dependency.chars().all(|ch| {
        !matches!(
            ch,
            '/' | '\\' | '\0' | '~' | '^' | ':' | '?' | '*' | '[' | '@'
        ) && !ch.is_whitespace()
            && !ch.is_control()
    })
}

fn infer_operating_frequency(description: Option<&str>) -> OperatingFrequency {
    let Some(description) = description else {
        return OperatingFrequency::Theta;
    };

    let description = description.to_ascii_lowercase();
    if description.contains("fix")
        || description.contains("quick")
        || description.contains("typo")
        || description.contains("rename")
    {
        OperatingFrequency::Gamma
    } else if description.contains("plan")
        || description.contains("design")
        || description.contains("architect")
        || description.contains("consolidate")
        || description.contains("dream")
    {
        OperatingFrequency::Delta
    } else {
        OperatingFrequency::Theta
    }
}

impl TaskDef {
    /// Whether this task may benefit from pre-dispatch search context enrichment.
    ///
    /// Returns `true` for complex tiers (`architectural`, `integrative`) that
    /// typically benefit from real-world examples and best-practice documentation
    /// found via the Perplexity Sonar search API.
    #[must_use]
    pub fn needs_external_context(&self) -> bool {
        matches!(self.tier.as_str(), "architectural" | "integrative")
    }

    /// Map the task to an operating frequency.
    ///
    /// If the task declares a frequency, use it. Otherwise infer it from the
    /// description:
    /// - contains `fix`, `quick`, `typo`, or `rename` → `Gamma`
    /// - contains `plan`, `design`, `architect`, `consolidate`, or `dream` → `Delta`
    /// - otherwise → `Theta`
    #[must_use]
    pub fn operating_frequency(&self) -> OperatingFrequency {
        self.frequency
            .unwrap_or_else(|| infer_operating_frequency(self.description.as_deref()))
    }

    /// Get the model to use for this task, falling back through:
    /// 1. model_hint from task definition (aliases like "sonnet" are normalized)
    /// 2. tier_models from config (if provided)
    /// 3. built-in tier defaults
    /// 4. provided fallback
    pub fn effective_model(
        &self,
        fallback: &str,
        tier_models: Option<&std::collections::HashMap<String, String>>,
    ) -> String {
        if let Some(ref hint) = self.model_hint {
            return normalize_model_alias(hint).to_owned();
        }
        // Check config tier_models first
        if let Some(models) = tier_models {
            if let Some(model) = models.get(&self.tier) {
                return model.clone();
            }
        }
        // Built-in defaults
        match self.tier.as_str() {
            "mechanical" => "claude-haiku-4-5".into(),
            "focused" => "claude-sonnet-4-6".into(),
            "integrative" => "claude-sonnet-4-6".into(),
            "architectural" => "claude-opus-4-6".into(),
            _ => fallback.into(),
        }
    }

    /// Whether this task is ready to execute (status = "ready" and all deps done).
    pub fn is_ready(&self, completed: &[String]) -> bool {
        self.status == "ready" && self.depends_on.iter().all(|dep| completed.contains(dep))
    }

    /// Whether this task is ready to execute, including cross-plan dependencies.
    pub fn is_ready_with_plan_deps(
        &self,
        completed_tasks: &[String],
        completed_plans: &[String],
    ) -> bool {
        self.is_ready(completed_tasks)
            && self
                .depends_on_plan
                .iter()
                .all(|dep| completed_plans.contains(dep))
    }

    /// Build the agent prompt from task title + surgical context.
    pub fn build_prompt(&self, plan_id: &str, workdir: &Path) -> String {
        let mut prompt = String::new();
        prompt.push_str(&format!("# Task: {}\n\n", self.title));
        prompt.push_str(&format!("Plan: {plan_id}\nTask ID: {}\n", self.id));

        if let Some(max) = self.max_loc {
            prompt.push_str(&format!("Maximum lines of change: {max}\n"));
        }

        // Inject PRD excerpt when available so agents see the high-level
        // requirements without having to locate the PRD file themselves.
        let prd_base = workdir.join(".roko").join("prd");
        let prd_candidates = [
            prd_base.join("published").join(format!("{plan_id}.md")),
            prd_base.join("draft").join(format!("{plan_id}.md")),
        ];
        for prd_path in &prd_candidates {
            if prd_path.exists() {
                if let Ok(content) = std::fs::read_to_string(prd_path) {
                    const PRD_BUILD_PROMPT_LIMIT: usize = 2_000;
                    let excerpt = if content.len() > PRD_BUILD_PROMPT_LIMIT {
                        let mut s = content
                            .chars()
                            .take(PRD_BUILD_PROMPT_LIMIT)
                            .collect::<String>();
                        s.push_str("\n[truncated]");
                        s
                    } else {
                        content
                    };
                    prompt.push_str("\n## PRD Requirements\n");
                    prompt.push_str(&excerpt);
                    prompt.push('\n');
                }
                break;
            }
        }

        if !self.files.is_empty() {
            prompt.push_str("\n## Files to modify\n");
            for f in &self.files {
                prompt.push_str(&format!("- `{f}`\n"));
            }
        }

        // Surgical context
        if let Some(ref ctx) = self.context {
            prompt.push_str("\n## Context (read these BEFORE making changes)\n");
            for rf in &ctx.read_files {
                prompt.push_str(&format!("\n### `{}`", rf.path));
                if let Some(ref lines) = rf.lines {
                    prompt.push_str(&format!(" (lines {lines})"));
                }
                prompt.push_str(&format!("\nWhy: {}\n", rf.why));
                // Try to inline the file content
                let full_path = workdir.join(&rf.path);
                if full_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&full_path) {
                        let lines_to_show = if let Some(ref range) = rf.lines {
                            extract_line_range(&content, range)
                        } else {
                            // Show first 100 lines max
                            content.lines().take(100).collect::<Vec<_>>().join("\n")
                        };
                        prompt.push_str(&format!("```\n{lines_to_show}\n```\n"));
                    }
                }
            }
            if !ctx.symbols.is_empty() {
                prompt.push_str("\n## Key symbols\n");
                for sym in &ctx.symbols {
                    prompt.push_str(&format!("- `{sym}`\n"));
                }
            }
            if !ctx.anti_patterns.is_empty() {
                prompt.push_str("\n## ⛔ Do NOT\n");
                for ap in &ctx.anti_patterns {
                    prompt.push_str(&format!("- {ap}\n"));
                }
            }
        }

        // Verification info for the agent
        if !self.verify.is_empty() {
            prompt.push_str("\n## Verification (these commands must pass after your changes)\n");
            for v in &self.verify {
                prompt.push_str(&format!(
                    "- `{}` — {}\n",
                    v.command,
                    v.fail_msg.as_deref().unwrap_or("must succeed")
                ));
            }
        } else if !self.acceptance.is_empty() {
            prompt.push_str("\n## Acceptance criteria\n");
            for a in &self.acceptance {
                prompt.push_str(&format!("- {a}\n"));
            }
        }

        prompt
    }

    /// Build a fix prompt for retry after verification failure.
    /// Appends failure context to the original task prompt.
    pub fn build_fix_prompt(
        &self,
        original_prompt: &str,
        failing_phase: &str,
        error_output: &str,
    ) -> String {
        let truncated = if error_output.len() > 4000 {
            &error_output[..4000]
        } else {
            error_output
        };

        format!(
            "{}\n\n---\n\n## ⚠️ Verification Failed\n\n\
            Phase: {}\n\n\
            Error output:\n```\n{}\n```\n\n\
            Fix the issue and ensure all verification steps pass.",
            original_prompt, failing_phase, truncated
        )
    }

    /// Apply role-specific tool defaults after TOML parsing.
    ///
    /// Explicit task settings take precedence over role defaults.
    fn apply_role_tool_defaults(&mut self) {
        if self.denied_tools.is_some() {
            return;
        }

        if let Some(denied) = self.role.as_deref().and_then(denied_tools_for_role) {
            self.denied_tools = Some(denied.iter().map(|tool| (*tool).to_string()).collect());
        }
    }
}

/// Normalize a model alias to its full model identifier.
///
/// Short aliases like `"haiku"`, `"sonnet"`, `"opus"` are accepted in
/// `model_hint` fields for convenience, but the executor and API require the
/// full `claude-*` identifiers. This function maps the known short forms and
/// passes everything else through unchanged.
///
/// This is called at execution time inside [`TaskDef::effective_model`] so
/// that plans written with short aliases work correctly without needing to be
/// regenerated.
pub fn normalize_model_alias(hint: &str) -> &str {
    match hint.trim() {
        "haiku" => "claude-haiku-4-5",
        "sonnet" => "claude-sonnet-4-6",
        "opus" => "claude-opus-4-6",
        other => other,
    }
}

/// Surgical context for a task.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskContext {
    /// Files to read (with optional line ranges).
    #[serde(default)]
    pub read_files: Vec<ReadFile>,
    /// Key symbols (types, functions) the agent should know about.
    #[serde(default)]
    pub symbols: Vec<String>,
    /// What NOT to do.
    #[serde(default)]
    pub anti_patterns: Vec<String>,
    /// Context from prior failed attempts.
    #[serde(default)]
    pub prior_failures: Vec<String>,
}

/// A file to read as context, with optional line range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFile {
    pub path: String,
    #[serde(default)]
    pub lines: Option<String>,
    #[serde(default = "default_why")]
    pub why: String,
}

fn default_why() -> String {
    "context".into()
}

/// One step in the per-task verification pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyStep {
    /// Phase: structural, compile, test, integration.
    #[serde(default)]
    pub phase: String,
    /// Shell command to run. Exit 0 = pass.
    pub command: String,
    /// Message to show on failure.
    #[serde(default)]
    pub fail_msg: Option<String>,
    /// Timeout in milliseconds.
    #[serde(default = "default_verify_timeout")]
    pub timeout_ms: u64,
}

fn default_verify_timeout() -> u64 {
    60_000
}

/// The full parsed tasks.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksFile {
    pub meta: TaskMeta,
    #[serde(rename = "task")]
    pub tasks: Vec<TaskDef>,
}

impl TasksFile {
    /// Parse a tasks.toml file.
    pub fn parse(path: &Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Self::parse_str(&content).with_context(|| format!("parse tasks.toml at {}", path.display()))
    }

    /// Parse a `tasks.toml` payload from a string slice.
    pub fn parse_str(content: &str) -> Result<Self> {
        let mut parsed: Self = toml::from_str(content).context("parse tasks.toml")?;
        // Stamp definition order so DAG sort preserves author intent.
        for (i, task) in parsed.tasks.iter_mut().enumerate() {
            task.sequence = i;
        }
        Ok(parsed)
    }

    /// Parse a `tasks.toml` payload returned inline by an agent.
    pub fn parse_agent_output(content: &str) -> Result<Self> {
        let payload = extract_toml_payload(content);
        Self::parse_str(&payload).context("parse tasks.toml from agent output")
    }

    /// Get all tasks that are ready to execute (deps satisfied).
    pub fn ready_tasks(&self, completed: &[String]) -> Vec<&TaskDef> {
        self.tasks
            .iter()
            .filter(|t| t.is_ready(completed))
            .collect()
    }

    /// Get tasks grouped by dependency level (for parallel execution).
    /// Level 0 = no deps, Level 1 = depends only on level 0, etc.
    pub fn parallel_groups(&self) -> Vec<Vec<&TaskDef>> {
        let mut levels: Vec<Vec<&TaskDef>> = Vec::new();
        let mut assigned: HashMap<String, usize> = HashMap::new();

        // Keep assigning levels until all tasks are placed
        loop {
            let mut new_level = Vec::new();
            for task in &self.tasks {
                if assigned.contains_key(&task.id) {
                    continue;
                }
                // All deps must already be assigned
                let deps_satisfied = task.depends_on.iter().all(|d| assigned.contains_key(d));
                if deps_satisfied {
                    new_level.push(task);
                }
            }
            if new_level.is_empty() {
                break;
            }
            let level = levels.len();
            for task in &new_level {
                assigned.insert(task.id.clone(), level);
            }
            levels.push(new_level);
        }

        levels
    }

    /// Update dependency references when a new plan's tasks are added.
    /// If any task in THIS file references tasks from `other_plan`,
    /// and those tasks have been renumbered/renamed, update the refs.
    pub fn update_cross_refs(&mut self, old_id: &str, new_id: &str) {
        for task in &mut self.tasks {
            for dep in &mut task.depends_on {
                if dep == old_id {
                    *dep = new_id.to_string();
                }
            }
        }
    }

    /// Validate that all tasks have the full metadata required for execution (§11).
    ///
    /// Returns a list of validation issues (empty = valid).
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        for task in &self.tasks {
            let tid = &task.id;
            if task.tier.is_empty() || task.tier == "unknown" {
                issues.push(format!("{tid}: missing or unknown tier"));
            }
            if task.verify.is_empty() {
                issues.push(format!("{tid}: missing verify steps"));
            }
            if task
                .context
                .as_ref()
                .is_none_or(|c| c.read_files.is_empty())
            {
                issues.push(format!("{tid}: missing context.read_files"));
            }
        }
        issues.extend(
            self.validate_structure()
                .into_iter()
                .map(|issue| issue.to_string()),
        );
        issues
    }

    /// Non-blocking quality heuristics for generated plans.
    pub fn quality_warnings(&self) -> Vec<TaskQualityWarning> {
        let mut warnings = Vec::new();

        if self.tasks.len() > 20 {
            warnings.push(TaskQualityWarning::TooManyTasks {
                task_count: self.tasks.len(),
            });
        }

        for task in &self.tasks {
            if let Some(description) = task.description.as_deref() {
                let word_count = description.split_whitespace().count();
                if word_count > 500 {
                    warnings.push(TaskQualityWarning::LongDescription {
                        task_id: task.id.clone(),
                        word_count,
                    });
                }
            }

            if task
                .context
                .as_ref()
                .is_none_or(|c| c.read_files.is_empty())
            {
                warnings.push(TaskQualityWarning::MissingReadFiles {
                    task_id: task.id.clone(),
                });
            }

            if task.verify.is_empty() {
                warnings.push(TaskQualityWarning::MissingVerify {
                    task_id: task.id.clone(),
                });
            }
        }

        warnings
    }

    /// Validate task definitions against the field schema.
    ///
    /// Checks that role, tier, and status values are from the known set,
    /// and that role-specific required fields are present.
    /// Returns a list of issues (empty = valid).
    pub fn validate_against_schema(&self) -> Vec<String> {
        const VALID_ROLES: &[&str] = &[
            "implementer",
            "researcher",
            "strategist",
            "architect",
            "reviewer",
            "quick-reviewer",
            "scribe",
        ];
        const VALID_TIERS: &[&str] = &["mechanical", "focused", "integrative", "architectural"];
        const VALID_STATUSES: &[&str] =
            &["pending", "ready", "active", "done", "blocked", "skipped"];
        // Role -> required fields
        const IMPLEMENTER_REQUIRED: &[&str] = &["verify", "files"];

        let mut issues = Vec::new();

        for (index, task) in self.tasks.iter().enumerate() {
            let tid = task.id.trim();
            let task_label = if tid.is_empty() {
                format!("task #{}", index + 1)
            } else {
                tid.to_string()
            };
            let role = task.role.as_deref().unwrap_or("implementer");

            if tid.is_empty() {
                issues.push(format!("{task_label}: missing 'id'"));
            }
            if task.title.trim().is_empty() {
                issues.push(format!("{task_label}: missing 'title'"));
            }

            // Check role is valid.
            if !VALID_ROLES.contains(&role) {
                issues.push(format!(
                    "{task_label}: unknown role '{role}' (valid: {})",
                    VALID_ROLES.join(", ")
                ));
            }

            // Check required fields for implementer role.
            if role == "implementer" {
                for &field in IMPLEMENTER_REQUIRED {
                    let missing = match field {
                        "verify" => task.verify.is_empty(),
                        "files" => task.files.is_empty(),
                        _ => false,
                    };
                    if missing {
                        issues.push(format!(
                            "{task_label}: missing '{field}' (required for role '{role}')"
                        ));
                    }
                }
            }

            // Check tier is valid.
            if !task.tier.is_empty()
                && task.tier != "unknown"
                && !VALID_TIERS.contains(&task.tier.as_str())
            {
                issues.push(format!(
                    "{task_label}: unknown tier '{}' (valid: {})",
                    task.tier,
                    VALID_TIERS.join(", ")
                ));
            }

            // Check status is valid.
            if !task.status.is_empty() && !VALID_STATUSES.contains(&task.status.as_str()) {
                issues.push(format!(
                    "{task_label}: unknown status '{}' (valid: {})",
                    task.status,
                    VALID_STATUSES.join(", ")
                ));
            }

            // Check numeric bounds.
            if task.timeout_secs == 0 {
                issues.push(format!("{task_label}: timeout_secs must be > 0"));
            }
            if task.max_loc.is_some_and(|m| m == 0) {
                issues.push(format!("{task_label}: max_loc must be > 0"));
            }
        }

        issues
    }

    /// Validate that the raw `tasks.toml` still carries the modern task fields.
    pub fn validate_modern_fields(path: &Path) -> Result<Vec<ModernFieldIssue>> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        validate_modern_fields_content(&content)
            .with_context(|| format!("parse modern field keys at {}", path.display()))
    }

    /// Validate the task graph structure required for execution.
    pub fn validate_structure(&self) -> Vec<TaskValidationIssue> {
        let mut issues = Vec::new();
        let task_ids: HashSet<&str> = self.tasks.iter().map(|task| task.id.as_str()).collect();
        let current_plan = self.meta.plan.trim();

        for task in &self.tasks {
            let tid = task.id.trim();
            if tid.is_empty() {
                issues.push(TaskValidationIssue::MissingRequiredField {
                    task_id: task.id.clone(),
                    field: "id",
                });
            }
            if task.title.trim().is_empty() {
                issues.push(TaskValidationIssue::MissingRequiredField {
                    task_id: task.id.clone(),
                    field: "title",
                });
            }
            if task
                .description
                .as_ref()
                .is_none_or(|description| description.trim().is_empty())
            {
                issues.push(TaskValidationIssue::MissingRequiredField {
                    task_id: task.id.clone(),
                    field: "description",
                });
            }

            for dependency in &task.depends_on {
                if !task_ids.contains(dependency.as_str()) {
                    issues.push(TaskValidationIssue::UnknownDependency {
                        task_id: task.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }

            for dependency in &task.depends_on_plan {
                if !is_valid_plan_dependency_reference(dependency, current_plan) {
                    issues.push(TaskValidationIssue::UnknownDependency {
                        task_id: task.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }

        if !self.tasks.iter().any(|task| task.depends_on.is_empty()) {
            issues.push(TaskValidationIssue::NoStartNode);
        }

        let mut valid_deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for task in &self.tasks {
            valid_deps.insert(
                task.id.clone(),
                task.depends_on
                    .iter()
                    .filter(|dep| task_ids.contains(dep.as_str()))
                    .cloned()
                    .collect(),
            );
        }

        let cycles = detect_cycle_nodes(&valid_deps);
        if !cycles.is_empty() {
            issues.push(TaskValidationIssue::CircularDependency { cycle: cycles });
        }

        issues
    }
}

fn validate_modern_fields_content(content: &str) -> Result<Vec<ModernFieldIssue>> {
    let raw: toml::Value = toml::from_str(content).context("parse tasks.toml")?;
    let Some(tasks) = raw.get("task").and_then(toml::Value::as_array) else {
        return Ok(vec![ModernFieldIssue {
            task_id: "tasks.toml".into(),
            missing_fields: vec!["task"],
        }]);
    };

    let mut issues = Vec::new();
    for (index, task) in tasks.iter().enumerate() {
        let task_table = task.as_table();
        let task_id = task_table
            .and_then(|table| table.get("id"))
            .and_then(toml::Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(|id| id.to_string())
            .unwrap_or_else(|| format!("task #{}", index + 1));

        let Some(table) = task_table else {
            issues.push(ModernFieldIssue {
                task_id,
                missing_fields: vec!["tier", "model_hint", "read_files", "verify", "depends_on"],
            });
            continue;
        };

        let mut missing_fields = Vec::new();

        let tier_missing = table
            .get("tier")
            .and_then(toml::Value::as_str)
            .is_none_or(|tier| tier.trim().is_empty());
        if tier_missing {
            missing_fields.push("tier");
        }

        let model_hint_missing = table
            .get("model_hint")
            .and_then(toml::Value::as_str)
            .is_none_or(|hint| hint.trim().is_empty());
        if model_hint_missing {
            missing_fields.push("model_hint");
        }

        let read_files_missing = table
            .get("context")
            .and_then(toml::Value::as_table)
            .and_then(|context| context.get("read_files"))
            .is_none();
        if read_files_missing {
            missing_fields.push("read_files");
        }

        if !table.contains_key("verify") {
            missing_fields.push("verify");
        }

        if !table.contains_key("depends_on") {
            missing_fields.push("depends_on");
        }

        if !missing_fields.is_empty() {
            issues.push(ModernFieldIssue {
                task_id,
                missing_fields,
            });
        }
    }

    Ok(issues)
}

/// Extract lines from content given a range like "40-80" or "10-".
fn extract_line_range(content: &str, range: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let parts: Vec<&str> = range.split('-').collect();
    let start = parts
        .first()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1)
        .saturating_sub(1);
    let end = parts
        .get(1)
        .and_then(|s| {
            if s.is_empty() {
                None
            } else {
                s.parse::<usize>().ok()
            }
        })
        .unwrap_or(lines.len())
        .min(lines.len());
    lines[start..end].join("\n")
}

fn extract_toml_payload(content: &str) -> String {
    let trimmed = content.trim();
    let Some(open_start) = trimmed.find("```") else {
        return trimmed.to_string();
    };
    let after_open = &trimmed[open_start + 3..];
    let Some(open_end) = after_open.find('\n') else {
        return trimmed.to_string();
    };
    let body = &after_open[open_end + 1..];
    if let Some(close_start) = body.rfind("```") {
        body[..close_start].trim().to_string()
    } else {
        body.trim().to_string()
    }
}

/// Deterministic TOML repair pipeline.
pub fn repair_toml(raw: &str) -> String {
    let t0 = std::time::Instant::now();
    let mut s = extract_toml_payload(raw);

    // Strip trailing prose after last ]]
    if let Some(pos) = s.rfind("]]") {
        let line_end = s[pos..].find('\n').map(|i| pos + i + 1).unwrap_or(pos + 2);
        let trailing = s[line_end..].trim();
        if !trailing.is_empty() && !trailing.starts_with('[') && !trailing.starts_with('#') {
            s.truncate(line_end);
        }
    }

    s = split_merged_fields(&s);
    s = close_unclosed_strings(&s);

    let elapsed_us = t0.elapsed().as_micros();
    if s != extract_toml_payload(raw) {
        tracing::info!(elapsed_us, "repair_toml: applied deterministic fixes");
    }
    s
}

fn split_merged_fields(s: &str) -> String {
    let field_boundaries = [
        "max_loc",
        "timeout_secs",
        "max_retries",
        "model_hint",
        "allowed_tools",
        "denied_tools",
        "depends_on",
        "status",
        "tier",
        "role",
        "verify",
        "files",
        "description",
        "title",
        "id",
    ];
    let mut result = s.to_string();
    for field in &field_boundaries {
        let pattern = format!("{field} = ");
        let merged = format!("\"{pattern}");
        let split = format!("\"\n{pattern}");
        result = result.replace(&merged, &split);
    }
    result
}

fn close_unclosed_strings(s: &str) -> String {
    s.lines()
        .map(|line| {
            if line.trim_start().starts_with('#') {
                return line.to_string();
            }
            let quote_count = line.chars().filter(|&c| c == '"').count();
            if quote_count % 2 != 0 {
                format!("{line}\"")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_tasks_toml() {
        let toml = r#"
[meta]
plan = "test"
total = 2

[[task]]
id = "T1"
title = "Do thing"
status = "ready"
depends_on = []

[[task]]
id = "T2"
title = "Do other thing"
status = "ready"
depends_on = ["T1"]
"#;
        let parsed: TasksFile = toml::from_str(toml).unwrap();
        assert_eq!(parsed.meta.plan, "test");
        assert_eq!(parsed.tasks.len(), 2);
        assert_eq!(parsed.tasks[1].depends_on, vec!["T1"]);
        assert_eq!(parsed.tasks[0].max_retries, 3);
    }

    #[test]
    fn parse_agent_output_strips_fences() {
        let output = r#"
Here is the plan:
```toml
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "Split work"
status = "ready"
depends_on = []
```
"#;
        let parsed = TasksFile::parse_agent_output(output).unwrap();
        assert_eq!(parsed.meta.plan, "test");
        assert_eq!(parsed.tasks.len(), 1);
        assert_eq!(parsed.tasks[0].id, "T1");
    }

    #[test]
    fn parse_full_task_with_context() {
        let toml = r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "Wire module"
role = "researcher"
tier = "focused"
frequency = "delta"
model_hint = "claude-sonnet-4-6"
replan_strategy = "decompose"
max_loc = 50
max_retries = 5
write_files = ["src/main.rs"]
allowed_tools = ["read_file", "grep"]
mcp_servers = ["filesystem", "git"]
depends_on = []

[task.context]
read_files = [
    { path = "src/lib.rs", lines = "40-80", why = "API surface" },
]
symbols = ["SystemPromptBuilder::new"]
anti_patterns = ["Do NOT create new files"]

[[task.verify]]
phase = "structural"
command = "grep -q SystemPromptBuilder src/main.rs"
fail_msg = "Import not found"

[[task.verify]]
phase = "compile"
command = "cargo check -p roko-cli"
"#;
        let parsed: TasksFile = toml::from_str(toml).unwrap();
        let task = &parsed.tasks[0];
        assert_eq!(task.tier, "focused");
        assert_eq!(task.frequency, Some(OperatingFrequency::Delta));
        assert_eq!(task.model_hint.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(task.replan_strategy, Some(ReplanStrategy::Decompose));
        assert_eq!(task.max_loc, Some(50));
        assert_eq!(task.files, vec!["src/main.rs"]);
        assert_eq!(
            task.allowed_tools,
            Some(vec!["read_file".into(), "grep".into()])
        );
        assert_eq!(
            task.mcp_servers,
            Some(vec!["filesystem".into(), "git".into()])
        );
        assert_eq!(task.max_retries, 5);
        let expected_denied: Vec<String> = denied_tools_for_role("researcher")
            .unwrap()
            .iter()
            .map(|tool| (*tool).to_string())
            .collect();
        assert_eq!(task.denied_tools, Some(expected_denied));
        assert_eq!(task.timeout_secs, 600);
        assert!(task.context.is_some());
        let ctx = task.context.as_ref().unwrap();
        assert_eq!(ctx.read_files.len(), 1);
        assert_eq!(ctx.read_files[0].lines.as_deref(), Some("40-80"));
        assert_eq!(ctx.anti_patterns.len(), 1);
        assert_eq!(task.verify.len(), 2);
        let rendered = toml::to_string(&parsed).unwrap();
        assert!(rendered.contains("frequency = \"delta\""));
        assert!(rendered.contains("replan_strategy = \"decompose\""));
    }

    #[test]
    fn effective_model_by_tier() {
        let task = TaskDef {
            id: "T1".into(),
            title: "test".into(),
            description: Some("test task".into()),
            role: None,
            status: "ready".into(),
            tier: "mechanical".into(),
            frequency: None,
            model_hint: None,
            replan_strategy: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![],
            timeout_secs: 600,
            max_retries: 3,
            acceptance: vec![],
            acceptance_contract: None,
            domain: None,
            sequence: 0,
        };
        assert_eq!(task.effective_model("fallback", None), "claude-haiku-4-5");

        let t2 = TaskDef {
            tier: "architectural".into(),
            ..task.clone()
        };
        assert_eq!(t2.effective_model("fallback", None), "claude-opus-4-6");

        let t3 = TaskDef {
            tier: "focused".into(),
            model_hint: Some("custom-model".into()),
            replan_strategy: None,
            ..task
        };
        assert_eq!(t3.effective_model("fallback", None), "custom-model");
    }

    #[test]
    fn operating_frequency_uses_explicit_frequency() {
        let task = TaskDef {
            id: "T1".into(),
            title: "test".into(),
            description: Some("test task".into()),
            role: None,
            status: "ready".into(),
            tier: "mechanical".into(),
            frequency: Some(OperatingFrequency::Gamma),
            model_hint: None,
            replan_strategy: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![],
            timeout_secs: 600,
            max_retries: 3,
            acceptance: vec![],
            acceptance_contract: None,
            domain: None,
            sequence: 0,
        };
        assert_eq!(task.operating_frequency(), OperatingFrequency::Gamma);
    }

    #[test]
    fn operating_frequency_infers_from_description() {
        let reactive = TaskDef {
            id: "T1".into(),
            title: "test".into(),
            description: Some("quick fix the typo in the rename flow".into()),
            role: None,
            status: "ready".into(),
            tier: "mechanical".into(),
            frequency: None,
            model_hint: None,
            replan_strategy: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![],
            timeout_secs: 600,
            max_retries: 3,
            acceptance: vec![],
            acceptance_contract: None,
            domain: None,
            sequence: 0,
        };
        assert_eq!(reactive.operating_frequency(), OperatingFrequency::Gamma);

        let reflective = TaskDef {
            id: "T2".into(),
            title: "plan".into(),
            description: Some("plan and design a consolidation pass".into()),
            role: None,
            status: "ready".into(),
            tier: "focused".into(),
            frequency: None,
            model_hint: None,
            replan_strategy: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![],
            timeout_secs: 600,
            max_retries: 3,
            acceptance: vec![],
            acceptance_contract: None,
            domain: None,
            sequence: 0,
        };
        assert_eq!(reflective.operating_frequency(), OperatingFrequency::Delta);

        let deliberative = TaskDef {
            id: "T3".into(),
            title: "implement".into(),
            description: Some("implement code change".into()),
            role: None,
            status: "ready".into(),
            tier: "architectural".into(),
            frequency: None,
            model_hint: None,
            replan_strategy: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![],
            timeout_secs: 600,
            max_retries: 3,
            acceptance: vec![],
            acceptance_contract: None,
            domain: None,
            sequence: 0,
        };
        assert_eq!(
            deliberative.operating_frequency(),
            OperatingFrequency::Theta
        );
    }

    #[test]
    fn ready_tasks_respects_deps() {
        let tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "test"
total = 3

[[task]]
id = "T1"
title = "first"
depends_on = []

[[task]]
id = "T2"
title = "second"
depends_on = ["T1"]

[[task]]
id = "T3"
title = "independent"
depends_on = []
"#,
        )
        .unwrap();

        let ready = tasks.ready_tasks(&[]);
        assert_eq!(ready.len(), 2); // T1 and T3
        assert!(ready.iter().any(|t| t.id == "T1"));
        assert!(ready.iter().any(|t| t.id == "T3"));

        let ready2 = tasks.ready_tasks(&["T1".into()]);
        assert_eq!(ready2.len(), 3); // T1 (already done but still "ready" status), T2, T3
    }

    #[test]
    fn parallel_groups_computes_levels() {
        let tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "test"
total = 4

[[task]]
id = "T1"
title = "first"
depends_on = []

[[task]]
id = "T2"
title = "also first"
depends_on = []

[[task]]
id = "T3"
title = "depends on T1"
depends_on = ["T1"]

[[task]]
id = "T4"
title = "depends on T3"
depends_on = ["T3"]
"#,
        )
        .unwrap();

        let groups = tasks.parallel_groups();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].len(), 2); // T1, T2 (parallel)
        assert_eq!(groups[1].len(), 1); // T3
        assert_eq!(groups[2].len(), 1); // T4
    }

    #[test]
    fn validate_structure_accepts_valid_plan() {
        let tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "test"
total = 2

[[task]]
id = "T1"
title = "first"
description = "bootstrap"
depends_on = []

[[task]]
id = "T2"
title = "second"
description = "follow up"
depends_on = ["T1"]
"#,
        )
        .unwrap();

        assert!(tasks.validate_structure().is_empty());
    }

    #[test]
    fn validate_structure_reports_graph_problems() {
        let tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "test"
total = 3

[[task]]
id = "T1"
title = "first"
description = "one"
depends_on = ["T2"]

[[task]]
id = "T2"
title = "second"
description = "two"
depends_on = ["T1"]

[[task]]
id = "T3"
title = "third"
description = "three"
depends_on = ["missing"]
"#,
        )
        .unwrap();

        let issues = tasks.validate_structure();
        assert!(
            issues
                .iter()
                .any(|issue| matches!(issue, TaskValidationIssue::UnknownDependency { .. }))
        );
        assert!(
            issues
                .iter()
                .any(|issue| matches!(issue, TaskValidationIssue::CircularDependency { .. }))
        );
        assert!(
            issues
                .iter()
                .any(|issue| matches!(issue, TaskValidationIssue::NoStartNode))
        );
    }

    #[test]
    fn validate_structure_accepts_valid_plan_dependencies() {
        let tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "consumer-plan"
total = 1

[[task]]
id = "T1"
title = "consume upstream work"
description = "wait for upstream"
depends_on = []
depends_on_plan = ["upstream-core", "shared-utils"]
"#,
        )
        .unwrap();

        assert!(tasks.validate_structure().is_empty());
    }

    #[test]
    fn validate_structure_reports_invalid_plan_dependencies() {
        let tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "consumer-plan"
total = 1

[[task]]
id = "T1"
title = "consume upstream work"
description = "wait for upstream"
depends_on = []
depends_on_plan = ["consumer-plan", "upstream:task"]
"#,
        )
        .unwrap();

        let issues = tasks.validate_structure();
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().any(|issue| matches!(
            issue,
            TaskValidationIssue::UnknownDependency { dependency, .. }
                if dependency == "consumer-plan"
        )));
        assert!(issues.iter().any(|issue| matches!(
            issue,
            TaskValidationIssue::UnknownDependency { dependency, .. }
                if dependency == "upstream:task"
        )));
    }

    #[test]
    fn validate_structure_reports_missing_description() {
        let tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "first"
depends_on = []
"#,
        )
        .unwrap();

        let issues = tasks.validate_structure();
        assert!(issues.iter().any(|issue| matches!(
            issue,
            TaskValidationIssue::MissingRequiredField {
                field: "description",
                ..
            }
        )));
    }

    #[test]
    fn quality_warnings_report_missing_context_and_verify() {
        let tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "first"
description = "small task"
depends_on = []
"#,
        )
        .unwrap();

        let warnings = tasks.quality_warnings();
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            TaskQualityWarning::MissingReadFiles { task_id } if task_id == "T1"
        )));
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            TaskQualityWarning::MissingVerify { task_id } if task_id == "T1"
        )));
    }

    #[test]
    fn quality_warnings_report_long_description_and_task_count() {
        let long_description = std::iter::repeat("word")
            .take(501)
            .collect::<Vec<_>>()
            .join(" ");

        let mut tasks = TasksFile {
            meta: TaskMeta {
                plan: "test".into(),
                iteration: 0,
                total: 21,
                done: 0,
                status: "ready".into(),
                max_parallel: 1,
                estimated_total_minutes: 0,
                skip_enrichment: false,
            },
            tasks: Vec::new(),
        };

        for idx in 0..21 {
            tasks.tasks.push(TaskDef {
                id: format!("T{}", idx + 1),
                title: "task".into(),
                description: if idx == 0 {
                    Some(long_description.clone())
                } else {
                    Some("small task".into())
                },
                role: None,
                status: "ready".into(),
                tier: "focused".into(),
                frequency: None,
                model_hint: None,
                replan_strategy: None,
                max_loc: None,
                files: vec![],
                allowed_tools: None,
                denied_tools: None,
                mcp_servers: None,
                depends_on: vec![],
                depends_on_plan: vec![],
                split_into: None,
                context: Some(TaskContext {
                    read_files: vec![ReadFile {
                        path: "src/lib.rs".into(),
                        lines: Some("1-10".into()),
                        why: "context".into(),
                    }],
                    symbols: vec![],
                    anti_patterns: vec![],
                    prior_failures: vec![],
                }),
                verify: vec![VerifyStep {
                    phase: "compile".into(),
                    command: "cargo check".into(),
                    fail_msg: None,
                    timeout_ms: 60_000,
                }],
                timeout_secs: 600,
                max_retries: 3,
                acceptance: vec![],
                acceptance_contract: None,
                domain: None,
                sequence: 0,
            });
        }

        let warnings = tasks.quality_warnings();
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            TaskQualityWarning::TooManyTasks { task_count } if *task_count == 21
        )));
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            TaskQualityWarning::LongDescription { task_id, word_count }
                if task_id == "T1" && *word_count == 501
        )));
    }

    #[test]
    fn extract_line_range_works() {
        let content = "line 1\nline 2\nline 3\nline 4\nline 5\n";
        assert_eq!(extract_line_range(content, "2-4"), "line 2\nline 3\nline 4");
        assert_eq!(extract_line_range(content, "3-"), "line 3\nline 4\nline 5");
    }

    #[test]
    fn update_cross_refs() {
        let mut tasks: TasksFile = toml::from_str(
            r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "depends on external"
depends_on = ["other-plan:T3"]
"#,
        )
        .unwrap();
        tasks.update_cross_refs("other-plan:T3", "other-plan:T5");
        assert_eq!(tasks.tasks[0].depends_on[0], "other-plan:T5");
    }

    #[test]
    fn build_fix_prompt_includes_error_output() {
        let task = TaskDef {
            id: "T1".into(),
            title: "test task".into(),
            description: Some("test task".into()),
            role: None,
            status: "ready".into(),
            tier: "focused".into(),
            frequency: None,
            model_hint: None,
            replan_strategy: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![],
            timeout_secs: 600,
            max_retries: 3,
            acceptance: vec![],
            acceptance_contract: None,
            domain: None,
            sequence: 0,
        };
        let original = "Original task prompt";
        let error_msg = "compilation failed: undefined symbol";
        let prompt = task.build_fix_prompt(original, "compile", error_msg);

        assert!(prompt.contains(original));
        assert!(prompt.contains(error_msg));
        assert!(prompt.contains("compile"));
    }

    #[test]
    fn build_fix_prompt_truncates_long_error() {
        let task = TaskDef {
            id: "T1".into(),
            title: "test task".into(),
            description: Some("test task".into()),
            role: None,
            status: "ready".into(),
            tier: "focused".into(),
            frequency: None,
            model_hint: None,
            replan_strategy: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![],
            timeout_secs: 600,
            max_retries: 3,
            acceptance: vec![],
            acceptance_contract: None,
            domain: None,
            sequence: 0,
        };
        let original = "Original prompt";
        let long_error = "x".repeat(5000);
        let prompt = task.build_fix_prompt(original, "test", &long_error);

        // The prompt should contain truncated error (4000 chars max)
        assert!(prompt.contains(original));
        // Should not contain the full 5000-char string
        assert!(!prompt.contains(&long_error));
        // But should contain a 4000-char substring of it
        assert!(prompt.contains(&"x".repeat(4000)));
    }

    #[test]
    fn explicit_denied_tools_override_role_defaults() {
        let toml = r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "R1"
title = "Research"
role = "researcher"
denied_tools = ["custom_block"]
depends_on = []
"#;
        let parsed: TasksFile = toml::from_str(toml).unwrap();
        let task = &parsed.tasks[0];
        assert_eq!(task.role.as_deref(), Some("researcher"));
        assert_eq!(task.denied_tools, Some(vec!["custom_block".into()]));
    }

    #[test]
    fn validate_modern_fields_reports_missing_keys() {
        let content = r#"
[meta]
plan = "demo"
iteration = 1
total = 1
done = 0
status = "ready"

[[task]]
id = "T1"
title = "Legacy task"
status = "ready"
depends_on = []
"#;
        let issues = validate_modern_fields_content(content).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0].missing_fields,
            vec!["tier", "model_hint", "read_files", "verify"]
        );
    }

    #[test]
    fn validate_modern_fields_accepts_full_metadata() {
        let content = r#"
[meta]
plan = "demo"
iteration = 1
total = 1
done = 0
status = "ready"

[[task]]
id = "T1"
title = "Modern task"
status = "ready"
tier = "focused"
model_hint = "claude-sonnet-4-6"
depends_on = []
verify = [{ phase = "compile", command = "cargo check" }]

[task.context]
read_files = [{ path = "src/lib.rs" }]
"#;
        let issues = validate_modern_fields_content(content).unwrap();
        assert!(issues.is_empty());
    }

    #[test]
    fn apply_role_tool_defaults_reviewer_denies_write_tools() {
        let toml = r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "Review code"
role = "reviewer"
depends_on = []
"#;
        let parsed: TasksFile = toml::from_str(toml).unwrap();
        let task = &parsed.tasks[0];
        let denied = task
            .denied_tools
            .as_ref()
            .expect("reviewer should have denied_tools");
        assert!(denied.contains(&"write_file".to_string()));
        assert!(denied.contains(&"edit_file".to_string()));
        assert!(!denied.contains(&"bash".to_string()));
    }

    #[test]
    fn apply_role_tool_defaults_strategist_denies_destructive_tools() {
        let toml = r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "Plan work"
role = "strategist"
depends_on = []
"#;
        let parsed: TasksFile = toml::from_str(toml).unwrap();
        let task = &parsed.tasks[0];
        let denied = task
            .denied_tools
            .as_ref()
            .expect("strategist should have denied_tools");
        assert!(denied.contains(&"write_file".to_string()));
        assert!(denied.contains(&"edit_file".to_string()));
        assert!(denied.contains(&"bash".to_string()));
        assert!(denied.contains(&"run_tests".to_string()));
    }

    #[test]
    fn apply_role_tool_defaults_scribe_denies_exec_tools() {
        let toml = r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "Write docs"
role = "scribe"
depends_on = []
"#;
        let parsed: TasksFile = toml::from_str(toml).unwrap();
        let task = &parsed.tasks[0];
        let denied = task
            .denied_tools
            .as_ref()
            .expect("scribe should have denied_tools");
        assert!(denied.contains(&"bash".to_string()));
        assert!(denied.contains(&"run_tests".to_string()));
        // Scribe CAN write files.
        assert!(!denied.contains(&"write_file".to_string()));
        assert!(!denied.contains(&"edit_file".to_string()));
    }

    #[test]
    fn apply_role_tool_defaults_implementer_has_no_denials() {
        let toml = r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "Write code"
role = "implementer"
depends_on = []
"#;
        let parsed: TasksFile = toml::from_str(toml).unwrap();
        let task = &parsed.tasks[0];
        assert!(
            task.denied_tools.is_none(),
            "implementer should have no denied tools"
        );
    }

    #[test]
    fn apply_role_tool_defaults_auditor_gets_reviewer_profile() {
        let toml = r#"
[meta]
plan = "test"
total = 1

[[task]]
id = "T1"
title = "Audit code"
role = "auditor"
depends_on = []
"#;
        let parsed: TasksFile = toml::from_str(toml).unwrap();
        let task = &parsed.tasks[0];
        let denied = task
            .denied_tools
            .as_ref()
            .expect("auditor should have denied_tools");
        assert!(denied.contains(&"write_file".to_string()));
        assert!(denied.contains(&"edit_file".to_string()));
    }
}
