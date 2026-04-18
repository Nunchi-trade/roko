//! Task model — the unit of work an agent executes.
//!
//! A `tasks.toml` file under a plan directory enumerates [`Task`]s plus
//! [`TaskMeta`] describing plan-level state. Each task carries 30+ fields
//! describing what to build, how to route the model, what context to
//! preload, and which invariants to test.
//!
//! This module mirrors `apps/mori/src/orchestrator/tasks.rs` for
//! drop-in `.mori/plans/<plan>/tasks.toml` compatibility.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

// ─── Status enums ─────────────────────────────────────────────────────────

/// Plan-level status stored in `tasks.toml [meta]`.
///
/// Single source of truth for plan lifecycle — the state machine in
/// §12.1 of the parity checklist transitions a plan through these values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PlanStatus {
    /// Plan has not yet been picked up for execution.
    #[default]
    Pending,
    /// Agents are currently editing files.
    Implementing,
    /// Gates are running against the current worktree state.
    Gating,
    /// Custom verify-chain scripts are running.
    Verifying,
    /// Reviewers are inspecting the diff.
    Reviewing,
    /// All gates + reviews passed; waiting for merge queue.
    Ready,
    /// Being merged into the batch branch.
    Merging,
    /// Finished and merged successfully.
    Complete,
    /// Terminally failed (circuit breaker tripped or retries exhausted).
    Failed,
}

impl PlanStatus {
    /// Lowercase label matching the TOML serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Implementing => "implementing",
            Self::Gating => "gating",
            Self::Verifying => "verifying",
            Self::Reviewing => "reviewing",
            Self::Ready => "ready",
            Self::Merging => "merging",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }
}

/// Per-task status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum TaskStatus {
    /// Not yet started.
    Pending,
    /// Currently being worked on.
    Active,
    /// Complete and merged.
    Done,
    /// Cannot proceed — dependency failure or unresolved merge conflict.
    Blocked,
}

impl TaskStatus {
    /// Single-char icon for TUI display.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Pending => "·",
            Self::Active => "►",
            Self::Done => "✓",
            Self::Blocked => "✗",
        }
    }
}

// ─── Complexity / routing enums ───────────────────────────────────────────

/// Complexity band controls model routing and iteration caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum TaskComplexityBand {
    /// Trivial single-file change (use Fast tier).
    Fast,
    /// Multi-file change within one crate (Standard tier).
    Standard,
    /// Cross-crate or architectural work (Premium tier).
    Complex,
}

impl TaskComplexityBand {
    /// Lowercase kebab label matching TOML serialization.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Standard => "standard",
            Self::Complex => "complex",
        }
    }

    /// Escalate one step (Fast → Standard → Complex, saturates at Complex).
    ///
    /// Used on retry: if a task fails at its current band, we try again
    /// with a more capable model tier.
    #[must_use]
    pub const fn escalate(self) -> Self {
        match self {
            Self::Fast => Self::Standard,
            Self::Standard | Self::Complex => Self::Complex,
        }
    }
}

/// Broad class of work — drives playbook recall and prompt templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum TaskCategory {
    /// Creating new files, modules, crates.
    Scaffolding,
    /// Main feature work.
    Implementation,
    /// Wiring components together.
    Integration,
    /// Adding / fixing tests.
    Verification,
    /// Investigating unknowns before edits.
    Research,
    /// Structural rewrites without behavior change.
    Refactor,
    /// Infrastructure (CI, tooling, build config).
    Infra,
    /// Documentation.
    Docs,
}

impl TaskCategory {
    /// Lowercase kebab label (matches TOML serialization).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Scaffolding => "scaffolding",
            Self::Implementation => "implementation",
            Self::Integration => "integration",
            Self::Verification => "verification",
            Self::Research => "research",
            Self::Refactor => "refactor",
            Self::Infra => "infra",
            Self::Docs => "docs",
        }
    }
}

/// How much multi-step reasoning this task requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum TaskReasoningLevel {
    /// Mechanical edits; a fast model suffices.
    Low,
    /// Standard feature work.
    Medium,
    /// Multi-step planning or architectural analysis.
    High,
}

impl TaskReasoningLevel {
    /// Lowercase kebab label (matches TOML serialization).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Latency vs correctness tradeoff dial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum TaskSpeedPriority {
    /// Optimize for turnaround (pick faster models).
    Latency,
    /// Default blend.
    Balanced,
    /// Optimize for correctness (pick deeper models).
    Accuracy,
}

impl TaskSpeedPriority {
    /// Lowercase kebab label (matches TOML serialization).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Latency => "latency",
            Self::Balanced => "balanced",
            Self::Accuracy => "accuracy",
        }
    }
}

/// Quality bar for this task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum TaskQualityProfile {
    /// Minimal viable (skip property tests, skip hardening).
    Pragmatic,
    /// Default middle path.
    Balanced,
    /// Belt-and-suspenders (full rung stack, property tests, invariants).
    Hardened,
}

impl TaskQualityProfile {
    /// Lowercase kebab label (matches TOML serialization).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pragmatic => "pragmatic",
            Self::Balanced => "balanced",
            Self::Hardened => "hardened",
        }
    }
}

/// How much inline/file context the prompt should preload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum TaskContextWeight {
    /// Minimal (just task + role).
    Slim,
    /// Default (includes workspace map, recent learnings).
    Standard,
    /// Maximal (full PRD, research, playbook, invariants).
    Deep,
}

impl TaskContextWeight {
    /// Lowercase kebab label (matches TOML serialization).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Slim => "slim",
            Self::Standard => "standard",
            Self::Deep => "deep",
        }
    }
}

// ─── Task struct (30+ fields) ─────────────────────────────────────────────

/// The unit of work an agent executes. Serialized from/to `tasks.toml`.
///
/// Mirrors `apps/mori/src/orchestrator/tasks.rs::Task` — every field here
/// is either required for basic execution (`id`, `title`, `files`,
/// `depends_on`) or a routing/context hint that tunes how the prompt is
/// assembled and which model tier is selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Stable identifier unique within the plan (e.g. `"t3"`).
    pub id: String,
    /// Human-readable summary shown in TUI and logs.
    pub title: String,
    /// Current status.
    pub status: TaskStatus,
    /// Files this task will create or modify.
    #[serde(default)]
    pub files: Vec<String>,
    /// Optional role hint for task-specific tool policy defaults.
    #[serde(default)]
    pub role: Option<String>,
    /// Acceptance criteria (human-readable bullets the auditor checks).
    #[serde(default)]
    pub acceptance: Vec<String>,
    /// Task IDs that must complete before this one starts.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Tasks sharing a `parallel_group` value can run simultaneously.
    #[serde(default)]
    pub parallel_group: Option<String>,
    /// When true, no other task should touch this task's files.
    #[serde(default = "default_exclusive_files")]
    pub exclusive_files: bool,
    /// Estimated minutes for this task (Codex/Claude agent wall time).
    #[serde(default)]
    pub estimated_minutes: Option<u32>,

    // ── Enhanced context fields ────────────────────────────────────
    /// Type signatures this task must define (from plan Quick Reference).
    #[serde(default)]
    pub types_to_define: Option<Vec<String>>,
    /// Formulas to implement verbatim (from PRD2).
    #[serde(default)]
    pub formulas: Option<Vec<String>>,
    /// Invariant IDs this task must test (from `## Verification`).
    #[serde(default)]
    pub test_invariants: Option<Vec<String>>,
    /// Imports needed from other crates/modules.
    #[serde(default)]
    pub imports: Option<Vec<String>>,
    /// Path to similar existing code to follow as pattern.
    #[serde(default)]
    pub example_pattern: Option<String>,
    /// Context files to read before implementing (injected into prompt).
    #[serde(default)]
    pub context_files: Option<Vec<String>>,
    /// Specific section of plan to focus on.
    #[serde(default)]
    pub plan_section: Option<String>,
    /// Skills to inject into prompts for this task (additive to role defaults).
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// Broad task class used for playbook recall and routing summaries.
    #[serde(default)]
    pub category: Option<TaskCategory>,
    /// How much multi-step reasoning this task needs.
    #[serde(default)]
    pub reasoning_level: Option<TaskReasoningLevel>,
    /// Whether to optimize for latency or correctness depth.
    #[serde(default)]
    pub speed_priority: Option<TaskSpeedPriority>,
    /// Expected implementation rigor.
    #[serde(default)]
    pub quality_profile: Option<TaskQualityProfile>,
    /// How much inline/file context the prompt should preload.
    #[serde(default)]
    pub context_weight: Option<TaskContextWeight>,
    /// Whether the agent should research patterns before editing code.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_boolish",
        serialize_with = "serialize_optional_boolish"
    )]
    pub research_before_edit: Option<bool>,
    /// Free-form routing and memory tags used by playbook recall.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Reusable dependency labels that this task relies on.
    #[serde(default)]
    pub dependency_tags: Option<Vec<String>>,
    /// Fixture keys available while executing this task.
    #[serde(default)]
    pub fixture_keys: Option<Vec<String>>,
    /// Sidecars or local services this task expects.
    #[serde(default)]
    pub sidecar_requirements: Option<Vec<String>>,
    /// High-level integration surfaces touched.
    #[serde(default)]
    pub integration_surfaces: Option<Vec<String>>,
    /// Optional complexity override for task-aware model routing.
    #[serde(default)]
    pub complexity_band: Option<TaskComplexityBand>,
    /// Explicit model override for this task.
    #[serde(default)]
    pub preferred_model: Option<String>,
    /// Optional backend/provider preference (`codex`, `cursor`, `claude`).
    #[serde(default)]
    pub preferred_provider: Option<String>,
    /// Whether this task should escalate to a stronger band on retry.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_boolish",
        serialize_with = "serialize_optional_boolish"
    )]
    pub escalate_on_retry: Option<bool>,
}

impl Task {
    /// Construct a minimal task — id, title, status only; everything else defaulted.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            status: TaskStatus::Pending,
            files: Vec::new(),
            role: None,
            acceptance: Vec::new(),
            depends_on: Vec::new(),
            parallel_group: None,
            exclusive_files: true,
            estimated_minutes: None,
            types_to_define: None,
            formulas: None,
            test_invariants: None,
            imports: None,
            example_pattern: None,
            context_files: None,
            plan_section: None,
            skills: None,
            category: None,
            reasoning_level: None,
            speed_priority: None,
            quality_profile: None,
            context_weight: None,
            research_before_edit: None,
            tags: None,
            dependency_tags: None,
            fixture_keys: None,
            sidecar_requirements: None,
            integration_surfaces: None,
            complexity_band: None,
            preferred_model: None,
            preferred_provider: None,
            escalate_on_retry: None,
        }
    }
}

const fn default_exclusive_files() -> bool {
    true
}

// ─── TaskMeta (plan-level state) ──────────────────────────────────────────

/// Plan-level metadata stored in `[meta]` header of `tasks.toml`.
///
/// Mirrors `apps/mori/src/orchestrator/tasks.rs::TaskMeta`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskMeta {
    /// Plan base name (e.g. `"46-reputation-engine"`).
    #[serde(default)]
    pub plan: String,
    /// Current iteration number (starts at 1; bumps on replan).
    #[serde(default = "default_iteration")]
    pub iteration: u32,
    /// Total number of tasks in the plan.
    #[serde(default)]
    pub total: usize,
    /// Number completed so far.
    #[serde(default)]
    pub done: usize,
    /// Plan-level status — single source of truth for plan phase.
    #[serde(default)]
    pub status: PlanStatus,
    /// Whether the last verify-chain run passed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_boolish",
        serialize_with = "serialize_optional_boolish"
    )]
    pub verify_passed: Option<bool>,
    /// Last gate result label (e.g. `"compile-pass"`, `"verify-fail"`).
    #[serde(default)]
    pub last_gate: Option<String>,
    /// ISO timestamp when plan was merged to batch.
    #[serde(default)]
    pub completed_at: Option<String>,
    /// Cap on how many tasks can execute at once in this plan.
    #[serde(default)]
    pub max_parallel: Option<usize>,
    /// Total estimated minutes for all tasks in this plan.
    #[serde(default)]
    pub estimated_total_minutes: Option<u32>,
}

const fn default_iteration() -> u32 {
    1
}

// ─── GlobalTaskId (cross-plan task reference) ─────────────────────────────

/// Globally unique task identifier — plan base name + task ID within plan.
///
/// Used by the cross-plan DAG (`UnifiedTaskDag`) to express dependencies
/// across plans. Displays as `plan:task` (e.g. `46-reputation-engine:t3`).
///
/// Mirrors `apps/mori/src/orchestrator/unified_dag.rs::GlobalTaskId`.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct GlobalTaskId {
    /// Plan base name (directory name under `plans/`).
    pub plan: String,
    /// Task ID within that plan.
    pub task: String,
}

impl GlobalTaskId {
    /// Construct from parts.
    #[must_use]
    pub fn new(plan: impl Into<String>, task: impl Into<String>) -> Self {
        Self {
            plan: plan.into(),
            task: task.into(),
        }
    }

    /// Parse a `"plan:task"` string back into a [`GlobalTaskId`].
    /// Returns None if the input lacks a colon or either side is empty.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let (plan, task) = s.split_once(':')?;
        if plan.is_empty() || task.is_empty() {
            return None;
        }
        Some(Self {
            plan: plan.to_string(),
            task: task.to_string(),
        })
    }
}

impl std::fmt::Display for GlobalTaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.plan, self.task)
    }
}

// ─── Serde helpers: bool-ish deserialization ──────────────────────────────

/// Accepts either a real boolean or the strings `"true"`/`"false"`.
/// Human-edited TOML often has `research_before_edit = "true"` with quotes.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum BoolishValue {
    Bool(bool),
    String(String),
}

/// Deserialize an optional boolean that may appear as a string in TOML.
///
/// # Errors
///
/// Returns an error if the value is a string other than `"true"`/`"false"`.
pub fn deserialize_optional_boolish<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<BoolishValue>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(BoolishValue::Bool(b)) => Ok(Some(b)),
        Some(BoolishValue::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            match trimmed.to_ascii_lowercase().as_str() {
                "true" => Ok(Some(true)),
                "false" => Ok(Some(false)),
                _ => Err(de::Error::custom(format!(
                    "expected boolean or \"true\"/\"false\", got {raw:?}"
                ))),
            }
        }
    }
}

/// Counterpart to [`deserialize_optional_boolish`] that always writes as a
/// real bool (no stringified bools in our output TOML).
///
/// # Errors
///
/// Returns any error the inner serializer raises.
pub fn serialize_optional_boolish<S>(
    value: &Option<bool>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(b) => serializer.serialize_bool(*b),
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complexity_band_escalates_and_saturates() {
        assert_eq!(
            TaskComplexityBand::Fast.escalate(),
            TaskComplexityBand::Standard
        );
        assert_eq!(
            TaskComplexityBand::Standard.escalate(),
            TaskComplexityBand::Complex
        );
        assert_eq!(
            TaskComplexityBand::Complex.escalate(),
            TaskComplexityBand::Complex
        );
    }

    #[test]
    fn complexity_band_orders_fast_to_complex() {
        assert!(TaskComplexityBand::Fast < TaskComplexityBand::Standard);
        assert!(TaskComplexityBand::Standard < TaskComplexityBand::Complex);
    }

    #[test]
    fn task_status_icons_distinct() {
        let icons: Vec<_> = [
            TaskStatus::Pending,
            TaskStatus::Active,
            TaskStatus::Done,
            TaskStatus::Blocked,
        ]
        .iter()
        .map(|s| s.icon())
        .collect();
        let unique: std::collections::HashSet<_> = icons.iter().copied().collect();
        assert_eq!(icons.len(), unique.len());
    }

    #[test]
    fn task_new_has_exclusive_files_default_true() {
        let t = Task::new("t1", "scaffold");
        assert!(t.exclusive_files);
        assert_eq!(t.status, TaskStatus::Pending);
        assert!(t.files.is_empty());
    }

    #[test]
    fn task_meta_defaults_iteration_to_one() {
        let m = TaskMeta::default();
        // Note: Default uses derive which zeroes — the #[serde(default)]
        // only kicks in during deserialization. Verify deser path here:
        let json = "{}";
        let parsed: TaskMeta = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.iteration, 1);
        assert_eq!(m.iteration, 0); // derived Default
    }

    #[test]
    fn global_task_id_roundtrips_display() {
        let id = GlobalTaskId::new("46-reputation", "t3");
        assert_eq!(id.to_string(), "46-reputation:t3");
        let parsed = GlobalTaskId::parse("46-reputation:t3").unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn global_task_id_parse_rejects_bad_forms() {
        assert!(GlobalTaskId::parse("noseparator").is_none());
        assert!(GlobalTaskId::parse(":empty-plan").is_none());
        assert!(GlobalTaskId::parse("empty-task:").is_none());
    }

    #[test]
    fn boolish_accepts_real_bool_and_strings() {
        #[derive(Deserialize)]
        struct Wrap {
            #[serde(default, deserialize_with = "deserialize_optional_boolish")]
            v: Option<bool>,
        }
        let a: Wrap = serde_json::from_str(r#"{"v": true}"#).unwrap();
        assert_eq!(a.v, Some(true));
        let b: Wrap = serde_json::from_str(r#"{"v": "false"}"#).unwrap();
        assert_eq!(b.v, Some(false));
        let c: Wrap = serde_json::from_str(r#"{"v": "True"}"#).unwrap();
        assert_eq!(c.v, Some(true));
        let d: Wrap = serde_json::from_str(r#"{"v": ""}"#).unwrap();
        assert_eq!(d.v, None);
        let e: Wrap = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(e.v, None);
    }

    #[test]
    fn boolish_rejects_invalid_strings() {
        #[derive(Deserialize)]
        struct Wrap {
            #[serde(default, deserialize_with = "deserialize_optional_boolish")]
            v: Option<bool>,
        }
        assert!(serde_json::from_str::<Wrap>(r#"{"v": "maybe"}"#).is_err());
        let parsed: Wrap = serde_json::from_str(r#"{"v": "false"}"#).unwrap();
        assert_eq!(parsed.v, Some(false));
    }

    #[test]
    fn task_serde_roundtrip_with_optional_fields() {
        let mut t = Task::new("t1", "implement login");
        t.files = vec!["src/auth.rs".into()];
        t.role = Some("researcher".into());
        t.acceptance = vec!["login succeeds".into()];
        t.depends_on = vec!["t0".into()];
        t.category = Some(TaskCategory::Implementation);
        t.complexity_band = Some(TaskComplexityBand::Standard);
        t.research_before_edit = Some(true);
        let json = serde_json::to_string(&t).unwrap();
        let decoded: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, t);
    }

    #[test]
    fn serde_kebab_case_for_enums() {
        assert_eq!(
            serde_json::to_string(&TaskCategory::Implementation).unwrap(),
            "\"implementation\""
        );
        assert_eq!(
            serde_json::to_string(&TaskComplexityBand::Complex).unwrap(),
            "\"complex\""
        );
        assert_eq!(
            serde_json::to_string(&TaskContextWeight::Slim).unwrap(),
            "\"slim\""
        );
    }

    #[test]
    fn plan_status_lowercase_serde() {
        assert_eq!(
            serde_json::to_string(&PlanStatus::Implementing).unwrap(),
            "\"implementing\""
        );
        assert_eq!(PlanStatus::Complete.as_str(), "complete");
    }
}
