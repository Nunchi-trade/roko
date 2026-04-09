//! Parse `tasks.toml` into structured task definitions with tiers, context,
//! dependencies, model hints, and per-task verification pipelines.
//!
//! This is the bridge between the plan generator output and the orchestrator.
//! It reads the extended task format (with `tier`, `model_hint`, `context`,
//! and `verify` sections) and produces [`TaskDef`]s that the executor uses
//! to select models, assemble context, and verify results.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use roko_std::RESEARCHER_TOOL_PROFILE;
use serde::{Deserialize, Deserializer};

/// Parsed `[meta]` section of tasks.toml.
#[derive(Debug, Clone, Deserialize)]
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
}

fn default_max_parallel() -> u32 {
    1
}

/// A single task definition.
#[derive(Debug, Clone)]
pub struct TaskDef {
    pub id: String,
    pub title: String,
    pub role: Option<String>,
    pub status: String,
    /// Task complexity tier: mechanical, focused, integrative, architectural.
    pub tier: String,
    /// Suggested model for this task.
    pub model_hint: Option<String>,
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
    /// Surgical context specification.
    pub context: Option<TaskContext>,
    /// Verification pipeline.
    pub verify: Vec<VerifyStep>,
    /// Per-task timeout in seconds.
    pub timeout_secs: u64,
    /// Free-form acceptance criteria (legacy format, strings).
    pub acceptance: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TaskDefSerde {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default = "default_tier")]
    pub tier: String,
    #[serde(default)]
    pub model_hint: Option<String>,
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
    #[serde(default)]
    pub context: Option<TaskContext>,
    #[serde(default)]
    pub verify: Vec<VerifyStep>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub acceptance: Vec<String>,
}

impl From<TaskDefSerde> for TaskDef {
    fn from(raw: TaskDefSerde) -> Self {
        let mut task = Self {
            id: raw.id,
            title: raw.title,
            role: raw.role,
            status: raw.status,
            tier: raw.tier,
            model_hint: raw.model_hint,
            max_loc: raw.max_loc,
            files: raw.files,
            allowed_tools: raw.allowed_tools,
            denied_tools: raw.denied_tools,
            mcp_servers: raw.mcp_servers,
            depends_on: raw.depends_on,
            depends_on_plan: raw.depends_on_plan,
            context: raw.context,
            verify: raw.verify,
            timeout_secs: raw.timeout_secs,
            acceptance: raw.acceptance,
        };
        task.apply_role_tool_defaults();
        task
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

impl TaskDef {
    /// Get the model to use for this task, falling back through:
    /// 1. model_hint from task definition
    /// 2. tier_models from config (if provided)
    /// 3. built-in tier defaults
    /// 4. provided fallback
    pub fn effective_model(
        &self,
        fallback: &str,
        tier_models: Option<&std::collections::HashMap<String, String>>,
    ) -> String {
        if let Some(ref hint) = self.model_hint {
            return hint.clone();
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
                            // Show first 50 lines max
                            content.lines().take(50).collect::<Vec<_>>().join("\n")
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

        if self
            .role
            .as_deref()
            .is_some_and(|role| role.eq_ignore_ascii_case("researcher"))
        {
            self.denied_tools = Some(
                RESEARCHER_TOOL_PROFILE
                    .denied_tools
                    .iter()
                    .map(|tool| (*tool).to_string())
                    .collect(),
            );
        }
    }
}

/// Surgical context for a task.
#[derive(Debug, Clone, Deserialize, Default)]
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
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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
        toml::from_str(&content).with_context(|| format!("parse tasks.toml at {}", path.display()))
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
        issues
    }
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
model_hint = "claude-sonnet-4-6"
max_loc = 50
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
        assert_eq!(task.model_hint.as_deref(), Some("claude-sonnet-4-6"));
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
        assert_eq!(
            task.denied_tools,
            Some(
                RESEARCHER_TOOL_PROFILE
                    .denied_tools
                    .iter()
                    .map(|tool| (*tool).to_string())
                    .collect()
            )
        );
        assert_eq!(task.timeout_secs, 600);
        assert!(task.context.is_some());
        let ctx = task.context.as_ref().unwrap();
        assert_eq!(ctx.read_files.len(), 1);
        assert_eq!(ctx.read_files[0].lines.as_deref(), Some("40-80"));
        assert_eq!(ctx.anti_patterns.len(), 1);
        assert_eq!(task.verify.len(), 2);
    }

    #[test]
    fn effective_model_by_tier() {
        let task = TaskDef {
            id: "T1".into(),
            title: "test".into(),
            role: None,
            status: "ready".into(),
            tier: "mechanical".into(),
            model_hint: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            context: None,
            verify: vec![],
            timeout_secs: 600,
            acceptance: vec![],
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
            ..task
        };
        assert_eq!(t3.effective_model("fallback", None), "custom-model");
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
            role: None,
            status: "ready".into(),
            tier: "focused".into(),
            model_hint: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            context: None,
            verify: vec![],
            timeout_secs: 600,
            acceptance: vec![],
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
            role: None,
            status: "ready".into(),
            tier: "focused".into(),
            model_hint: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            context: None,
            verify: vec![],
            timeout_secs: 600,
            acceptance: vec![],
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
}
