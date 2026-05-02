//! Agent role taxonomy, backend inference, and per-role budgets.
//!
//! The [`AgentRole`] enum enumerates every distinct "persona" an LLM can
//! take on in Roko's orchestration loop — there are 28 roles, grouped by
//! responsibility (planning, implementing, reviewing, validating, etc.).
//!
//! Each role carries:
//! - A short/long label (for logs, TUI widgets)
//! - A default [`AgentBackend`] (which CLI to spawn)
//! - A default [`ModelTier`] (which capability class to route to)
//! - A default [`TurnBudget`] (dollar ceiling per turn)
//! - A default [`ToolPermissions`] (Read/Write/Exec scope)
//!
//! These defaults are starting points: a plan or policy can override any
//! of them at spawn time. They exist so a bare `AgentRole::Implementer`
//! is enough to dispatch a reasonable turn without threading config
//! through every call site.
//!
//! Mirrors Mori's `apps/mori/src/agent/roles.rs` (for the enum + backend
//! inference) and the per-role budget table in `mori-agents/03`.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

use crate::config::schema::{ModelProfile, ProviderConfig, RokoConfig};

// ─── ProviderKind (which protocol family to use) ─────────────────────────

/// Which protocol family a provider belongs to.
///
/// This is the primary dispatch key for the provider registry layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Anthropic Messages API over HTTP.
    AnthropicApi,
    /// `claude` CLI subprocess protocol.
    ClaudeCli,
    /// OpenAI chat completions-compatible HTTP APIs.
    #[serde(rename = "openai_compat", alias = "open_ai_compat")]
    OpenAiCompat,
    /// Cursor Agent Client Protocol.
    CursorAcp,
    /// Perplexity Sonar HTTP API (OpenAI-compatible base, Sonar extensions).
    PerplexityApi,
    /// Google Gemini API.
    GeminiApi,
    /// Cerebras Inference API (OpenAI-compatible, ultra-fast inference).
    CerebrasApi,
}

impl ProviderKind {
    /// Canonical snake_case label for logs, config, and display.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::AnthropicApi => "anthropic_api",
            Self::ClaudeCli => "claude_cli",
            Self::OpenAiCompat => "openai_compat",
            Self::CursorAcp => "cursor_acp",
            Self::PerplexityApi => "perplexity_api",
            Self::GeminiApi => "gemini_api",
            Self::CerebrasApi => "cerebras_api",
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

// ─── AgentBackend (which CLI to spawn) ────────────────────────────────────

/// Which backing CLI tool an agent role spawns.
///
/// Backend is inferred either from a model slug (see [`AgentBackend::from_model`])
/// or taken from the role's default (see [`AgentRole::backend`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentBackend {
    /// Anthropic's `claude` CLI (stream-json protocol).
    Claude,
    /// `OpenAI`'s `codex` CLI (JSON-RPC app-server protocol).
    Codex,
    /// Cursor's `cursor-agent` CLI (ACP JSON-RPC protocol).
    Cursor,
    /// Local Ollama server (`OpenAI`-compatible HTTP).
    Ollama,
    /// Raw `OpenAI` HTTP API (no CLI).
    OpenAi,
    /// Perplexity Sonar HTTP API.
    Perplexity,
    /// Cerebras Inference API (ultra-fast LLM inference).
    Cerebras,
}

impl AgentBackend {
    /// A 2-char mnemonic used for compact TUI displays.
    #[must_use]
    pub const fn short(self) -> &'static str {
        match self {
            Self::Claude => "cl",
            Self::Codex => "cd",
            Self::Cursor => "cx",
            Self::Ollama => "ol",
            Self::OpenAi => "oa",
            Self::Perplexity => "px",
            Self::Cerebras => "cb",
        }
    }

    /// Infer backend from a model slug.
    ///
    /// Rules (derived from `apps/mori/src/agent/roles.rs::from_model`):
    /// - `claude-*` → Claude
    /// - `composer-*`, `cursor-*`, `sonnet-*`, `opus-*`, `haiku-*`,
    ///   `gemini-*`, `auto`, `*-high`, `*-xhigh-fast` → Cursor
    /// - `ollama/*` or `llama*` → Ollama
    /// - `sonar*` or `perplexity/*` → Perplexity
    /// - everything else → Codex (default GPT routing)
    #[must_use]
    pub fn from_model(slug: &str) -> Self {
        let slug = slug.trim();
        if slug.starts_with("claude-") || matches!(slug, "sonnet" | "opus" | "haiku") {
            Self::Claude
        } else if slug.starts_with("ollama/") || slug.starts_with("llama") {
            Self::Ollama
        } else if slug.starts_with("sonar") || slug.starts_with("perplexity/") {
            Self::Perplexity
        } else if slug.starts_with("cerebras/") {
            Self::Cerebras
        } else if is_cursor_slug(slug) {
            Self::Cursor
        } else {
            Self::Codex
        }
    }
}

impl From<AgentBackend> for ProviderKind {
    fn from(backend: AgentBackend) -> Self {
        match backend {
            AgentBackend::Claude => ProviderKind::ClaudeCli,
            AgentBackend::Codex | AgentBackend::OpenAi => ProviderKind::OpenAiCompat,
            AgentBackend::Cursor => ProviderKind::CursorAcp,
            AgentBackend::Ollama => ProviderKind::OpenAiCompat,
            AgentBackend::Perplexity => ProviderKind::PerplexityApi,
            AgentBackend::Cerebras => ProviderKind::CerebrasApi,
        }
    }
}

fn is_cursor_slug(slug: &str) -> bool {
    slug.starts_with("composer-")
        || slug.starts_with("cursor-")
        || slug == "auto"
        || slug.starts_with("sonnet-")
        || slug.starts_with("opus-")
        || slug.starts_with("haiku-")
        || slug.starts_with("gemini-")
        || slug == "gpt-5.2"
        || slug.ends_with("-high")
        || slug.ends_with("-xhigh-fast")
}

fn provider_kind_from_backend(backend: AgentBackend) -> ProviderKind {
    backend.into()
}

// ─── ModelSpec (slug + inferred backend) ──────────────────────────────────

/// Reasoning effort hint passed to capable backends (Codex, Claude).
///
/// Concrete backends map this to their native flag (e.g. Codex's
/// `reasoning_effort`, Claude's `--effort`). Backends that don't support
/// it silently ignore.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ReasoningEffort {
    /// Minimal thinking, fastest turn (validators, watchers).
    Low,
    /// Default.
    #[default]
    Medium,
    /// Thorough analysis (architects, critics).
    High,
    /// Max budget (saturates at whatever the backend supports).
    Max,
}

/// A fully-resolved model specification: slug + inferred backend + effort.
///
/// Mirrors Mori's `ModelSpec` in `apps/mori/src/agent/roles.rs`. Create
/// from any model string; the backend is always derived from the slug.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelSpec {
    /// The model slug (e.g. `"claude-sonnet-4-5"`, `"gpt-5"`).
    pub slug: String,
    /// The backend inferred from the slug.
    pub backend: AgentBackend,
    /// Reasoning effort hint.
    #[serde(default)]
    pub effort: ReasoningEffort,
}

impl ModelSpec {
    /// Construct from a slug, inferring the backend.
    #[must_use]
    pub fn from_slug(slug: impl Into<String>) -> Self {
        let slug = slug.into();
        let backend = AgentBackend::from_model(&slug);
        Self {
            slug,
            backend,
            effort: ReasoningEffort::default(),
        }
    }

    /// Set the reasoning effort.
    #[must_use]
    pub const fn with_effort(mut self, effort: ReasoningEffort) -> Self {
        self.effort = effort;
        self
    }

    /// Compact display label (strips common prefixes for TUI columns).
    #[must_use]
    pub fn short(&self) -> String {
        self.slug
            .replace("composer-", "cx-")
            .replace("claude-", "cl-")
            .replace("gpt-", "")
    }
}

/// Fully resolved model lookup result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedModel {
    /// The key used to resolve the model.
    pub model_key: String,
    /// The API model ID sent to the backend.
    pub slug: String,
    /// Protocol family for the resolved provider.
    pub provider_kind: ProviderKind,
    /// Provider-specific config, if the config registry has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_config: Option<ProviderConfig>,
    /// Model-specific config, if the config registry has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ModelProfile>,
    /// Legacy backend inference for backwards compatibility.
    pub backend: AgentBackend,
}

/// Resolve a model key against config, falling back to the legacy slug heuristic.
#[must_use]
pub fn resolve_model(config: &RokoConfig, model_key: &str) -> ResolvedModel {
    // 1. Direct lookup by config key (e.g. `--model llama3` matches `[models.llama3]`).
    if let Some(profile) = config.models.get(model_key) {
        return resolved_from_profile(config, model_key, profile);
    }

    // 2. Fallback: search by slug (e.g. `--model llama3.2` matches slug in `[models.llama3]`).
    for (key, profile) in &config.models {
        if profile.slug == model_key {
            return resolved_from_profile(config, key, profile);
        }
    }

    // 3. Prefix match on slug (e.g. "claude-opus-4" matches slug "claude-opus-4-6").
    //    Only accept the match if the slug starts with the key and the next char
    //    (if any) is a separator, avoiding false positives like "o3" matching "o3-mini".
    for (key, profile) in &config.models {
        if profile.slug.len() > model_key.len()
            && profile.slug.starts_with(model_key)
            && matches!(
                profile.slug.as_bytes().get(model_key.len()),
                Some(b'-' | b'.' | b'_')
            )
        {
            return resolved_from_profile(config, key, profile);
        }
    }

    let backend = AgentBackend::from_model(model_key);
    ResolvedModel {
        model_key: model_key.to_owned(),
        slug: model_key.trim().to_owned(),
        provider_kind: provider_kind_from_backend(backend),
        provider_config: None,
        profile: None,
        backend,
    }
}

fn resolved_from_profile(
    config: &RokoConfig,
    model_key: &str,
    profile: &crate::config::schema::ModelProfile,
) -> ResolvedModel {
    let provider_config = config.providers.get(&profile.provider).cloned();
    let backend = AgentBackend::from_model(&profile.slug);
    let provider_kind = provider_config
        .as_ref()
        .map(|provider| provider.kind)
        .unwrap_or_else(|| provider_kind_from_backend(backend));

    ResolvedModel {
        model_key: model_key.to_owned(),
        slug: profile.slug.clone(),
        provider_kind,
        provider_config,
        profile: Some(profile.clone()),
        backend,
    }
}

/// Task requirements that inform automatic provider/model selection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskRequirements {
    /// Does the task need web search / grounded retrieval?
    pub needs_web_search: bool,
    /// Does the task need provider-native code execution?
    pub needs_code_execution: bool,
    /// Does the task benefit from extended thinking / deep reasoning?
    pub needs_thinking: bool,
    /// Does the task need vision / image analysis?
    pub needs_vision: bool,
    /// Does the task need structured output support?
    pub needs_structured_output: bool,
    /// Minimum context window required in tokens.
    pub min_context_window: u64,
    /// Maximum acceptable cost per million output tokens.
    pub max_cost_output_per_m: Option<f64>,
    /// Maximum acceptable latency in milliseconds.
    pub max_latency_ms: Option<u64>,
}

/// Score a model profile against task requirements.
///
/// Returns `None` if the profile fails any hard requirement.
#[must_use]
pub fn score_model_for_task(
    profile: &ModelProfile,
    requirements: &TaskRequirements,
) -> Option<f64> {
    let supports_web_search =
        profile.supports_web_search || profile.supports_search || profile.supports_grounding;
    let supports_structured = profile.supports_tools || profile.supports_partial;

    if requirements.needs_web_search && !supports_web_search {
        return None;
    }
    if requirements.needs_code_execution && !profile.supports_code_execution {
        return None;
    }
    if requirements.needs_thinking && !profile.supports_thinking {
        return None;
    }
    if requirements.needs_vision && !profile.supports_vision {
        return None;
    }
    if requirements.needs_structured_output && !supports_structured {
        return None;
    }
    if profile.context_window < requirements.min_context_window {
        return None;
    }
    if let (Some(max_cost), Some(model_cost)) = (
        requirements.max_cost_output_per_m,
        profile.cost_output_per_m,
    ) {
        if model_cost > max_cost {
            return None;
        }
    }

    let mut score = 1.0;
    if requirements.needs_web_search && supports_web_search {
        score += 0.2;
    }
    if requirements.needs_code_execution && profile.supports_code_execution {
        score += 0.15;
    }
    if requirements.needs_thinking && profile.supports_thinking {
        score += 0.2;
    }
    if requirements.needs_vision && profile.supports_vision {
        score += 0.15;
    }
    if requirements.needs_structured_output && supports_structured {
        score += 0.1;
    }
    if profile.supports_caching {
        score += 0.05;
    }

    if requirements.min_context_window > 0 {
        let ratio = profile.context_window as f64 / requirements.min_context_window as f64;
        score += (ratio.min(2.0) - 1.0).max(0.0) * 0.15;
    }

    match (
        requirements.max_cost_output_per_m,
        profile.cost_output_per_m,
    ) {
        (Some(max_cost), Some(model_cost)) if max_cost > 0.0 => {
            score += ((max_cost - model_cost) / max_cost).max(0.0) * 0.35;
        }
        (None, Some(model_cost)) => {
            score += (1.0 / (1.0 + model_cost)).min(0.2);
        }
        _ => {}
    }

    if let Some(max_latency_ms) = requirements.max_latency_ms {
        if max_latency_ms <= 5_000 && !profile.supports_thinking {
            score += 0.1;
        }
    }

    Some(score)
}

/// Select the best model for a task from the configured model registry.
#[must_use]
pub fn select_model_for_task(
    config: &RokoConfig,
    requirements: &TaskRequirements,
) -> Option<String> {
    select_model_for_task_with_bonus(config, requirements, |_| 0.0)
}

/// Select the best model for a task, with an additional learned bonus per model.
#[must_use]
pub fn select_model_for_task_with_bonus<F>(
    config: &RokoConfig,
    requirements: &TaskRequirements,
    mut learned_bonus: F,
) -> Option<String>
where
    F: FnMut(&str) -> f64,
{
    let mut candidates: Vec<(String, f64, u64, f64)> = config
        .effective_models()
        .into_iter()
        .filter_map(|(key, profile)| {
            let score = score_model_for_task(&profile, requirements)?;
            Some((
                key.clone(),
                score + learned_bonus(&key) * 0.5,
                profile.context_window,
                profile.cost_output_per_m.unwrap_or(f64::MAX),
            ))
        })
        .collect();

    candidates.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(Ordering::Equal)
            .then(right.2.cmp(&left.2))
            .then_with(|| left.3.partial_cmp(&right.3).unwrap_or(Ordering::Equal))
            .then(left.0.cmp(&right.0))
    });
    candidates.into_iter().next().map(|(key, _, _, _)| key)
}

// ─── ModelTier (capability class) ─────────────────────────────────────────

/// Capability tier a role routes to by default.
///
/// Concrete model selection happens via the model router (`LinUCB` bandit
/// in `roko-learn`), but every role has a default tier so routing has a
/// reasonable starting point before learning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ModelTier {
    /// Cheap, fast (Haiku-class). For classification, watchers, orchestration.
    Fast,
    /// Balanced (Sonnet-class). The workhorse for implementation and review.
    Standard,
    /// Premium reasoning (Opus/GPT-5-class). For architecture, hard debugging.
    Premium,
}

// ─── TurnBudget (per-turn dollar ceiling) ─────────────────────────────────

/// Per-turn spend ceiling for a role, in US dollars.
///
/// Defaults come from the budget table in `mori-agents/03` — e.g.
/// Implementer=$1.50, Conductor=$0.10, Architect=$3.00. The `multiplier`
/// adjusts this for escalation (2.0x on Opus, 0.6x on Haiku).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TurnBudget {
    /// Base ceiling in USD.
    pub base_usd: f32,
    /// Multiplier applied when escalating (e.g. Opus = 2.0, Haiku = 0.6).
    pub multiplier: f32,
}

impl TurnBudget {
    /// Construct a budget with multiplier 1.0.
    #[must_use]
    pub const fn new(base_usd: f32) -> Self {
        Self {
            base_usd,
            multiplier: 1.0,
        }
    }

    /// Effective ceiling after applying multiplier.
    #[must_use]
    pub fn effective_usd(&self) -> f32 {
        self.base_usd * self.multiplier
    }

    /// Set the multiplier (for tier escalation).
    #[must_use]
    pub const fn with_multiplier(mut self, m: f32) -> Self {
        self.multiplier = m;
        self
    }
}

// ─── ToolPermissions (per-role tool allowlist) ────────────────────────────

/// What a role is allowed to do with the filesystem and shell.
///
/// Mirrors the per-role permission matrix in `mori-agents/03`. Roles with
/// `write=false` run with `--dangerously-skip-permissions` still enforcing
/// read-only; roles with `exec=false` cannot spawn subprocesses.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolPermissions {
    /// Can read files in the worktree.
    pub read: bool,
    /// Can write/edit files in the worktree.
    pub write: bool,
    /// Can execute shell commands (cargo, git, scripts).
    pub exec: bool,
    /// Can spawn git operations (commit, branch).
    pub git: bool,
    /// Can call external network services.
    pub network: bool,
}

impl ToolPermissions {
    /// Full access (for implementers, refactorers).
    #[must_use]
    pub const fn full() -> Self {
        Self {
            read: true,
            write: true,
            exec: true,
            git: true,
            network: false,
        }
    }

    /// Read-only (for reviewers, auditors, critics).
    #[must_use]
    pub const fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            exec: false,
            git: false,
            network: false,
        }
    }

    /// Read + exec (for validators, testers — can run code but not edit).
    #[must_use]
    pub const fn read_exec() -> Self {
        Self {
            read: true,
            write: false,
            exec: true,
            git: false,
            network: false,
        }
    }

    /// Full access including network (for researchers).
    #[must_use]
    pub const fn networked() -> Self {
        Self {
            read: true,
            write: false,
            exec: true,
            git: false,
            network: true,
        }
    }
}

// ─── AgentRole (the 28-variant enum) ──────────────────────────────────────

/// Every distinct persona an LLM-backed agent can assume.
///
/// Grouped by responsibility:
/// - **Orchestration**: `Conductor`, `PlanLifecycleManager`, `PrePlanner`
/// - **Planning**: `Strategist`, `Architect`
/// - **Implementation**: `Implementer`, `AutoFixer`, `Refactorer`, `MergeResolver`
/// - **Review**: `Auditor`, `Critic`, `QuickReviewer`, `Scribe`
/// - **Research**: `Researcher`, `PatternExtractor`, `ErrorDiagnoser`
/// - **Validation**: `IntegrationTester`, `TerminalValidator`, `GolemLifecycleTester`,
///   `CrossSystemTester`, `FullLoopValidator`, `SnapshotComparator`, `DocVerifier`,
///   `DependencyValidator`, `RegressionDetector`, `SpecDriftDetector`
/// - **Observability**: `PerformanceSentinel`, `CoverageTracker`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum AgentRole {
    /// Meta-orchestrator that watches all other agents and intervenes.
    Conductor,
    /// Writes the plan brief, decomposes PRDs into tasks.
    Strategist,
    /// Writes code (the main "coding agent").
    Implementer,
    /// Reviews architecture before implementation.
    Architect,
    /// Broad research reader (docs, code, external).
    Researcher,
    /// Post-impl review for correctness and safety.
    Auditor,
    /// Single-pass reviewer for Standard-complexity plans.
    QuickReviewer,
    /// Drafts documentation.
    Scribe,
    /// Devil's advocate / alternative-approach reviewer.
    Critic,
    /// Lightweight patcher used in express mode after gate failure.
    AutoFixer,
    /// Structural rewrite without behavior change.
    Refactorer,
    /// Validates pre-plan artifacts before expensive enrichment.
    PrePlanner,
    /// Verifies docs still match code after edits.
    DocVerifier,
    /// Runs integration-level tests against a live system.
    IntegrationTester,
    /// Resolves merge conflicts across parallel workstreams.
    MergeResolver,
    /// Tests CLI/terminal entry points end-to-end.
    TerminalValidator,
    /// Exercises Golem agent lifecycle (spawn/tick/teardown).
    GolemLifecycleTester,
    /// Detects divergence between PRD and implementation.
    SpecDriftDetector,
    /// Watches for regression in test-pass rate and cost.
    RegressionDetector,
    /// Tracks performance metrics across runs.
    PerformanceSentinel,
    /// Tracks coverage/rung satisfaction.
    CoverageTracker,
    /// Manages plan lifecycle state transitions.
    PlanLifecycleManager,
    /// Tests cross-system flows across Roko runtime boundaries.
    CrossSystemTester,
    /// Diagnoses errors into actionable root causes.
    ErrorDiagnoser,
    /// Validates dependency additions/upgrades.
    DependencyValidator,
    /// Extracts reusable patterns from completed work.
    PatternExtractor,
    /// Compares snapshots across runs for drift.
    SnapshotComparator,
    /// Validates end-to-end pipeline (mirage + terminal + runtime).
    FullLoopValidator,
}

impl AgentRole {
    /// Every role except Conductor — the Conductor is a meta-watcher,
    /// not a working agent.
    pub const ALL_AGENTS: [Self; 27] = [
        Self::Strategist,
        Self::Implementer,
        Self::Architect,
        Self::Researcher,
        Self::Auditor,
        Self::QuickReviewer,
        Self::Scribe,
        Self::Critic,
        Self::AutoFixer,
        Self::Refactorer,
        Self::PrePlanner,
        Self::DocVerifier,
        Self::IntegrationTester,
        Self::MergeResolver,
        Self::TerminalValidator,
        Self::GolemLifecycleTester,
        Self::SpecDriftDetector,
        Self::RegressionDetector,
        Self::PerformanceSentinel,
        Self::CoverageTracker,
        Self::PlanLifecycleManager,
        Self::CrossSystemTester,
        Self::ErrorDiagnoser,
        Self::DependencyValidator,
        Self::PatternExtractor,
        Self::SnapshotComparator,
        Self::FullLoopValidator,
    ];

    /// Full kebab-case label (for logs, config files, TUI).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Conductor => "conductor",
            Self::Strategist => "strategist",
            Self::Implementer => "implementer",
            Self::Architect => "architect",
            Self::Researcher => "researcher",
            Self::Auditor => "auditor",
            Self::QuickReviewer => "quick-reviewer",
            Self::Scribe => "scribe",
            Self::Critic => "critic",
            Self::AutoFixer => "auto-fixer",
            Self::Refactorer => "refactorer",
            Self::PrePlanner => "pre-planner",
            Self::DocVerifier => "doc-verifier",
            Self::IntegrationTester => "integration-tester",
            Self::MergeResolver => "merge-resolver",
            Self::TerminalValidator => "terminal-validator",
            Self::GolemLifecycleTester => "golem-lifecycle-tester",
            Self::SpecDriftDetector => "spec-drift-detector",
            Self::RegressionDetector => "regression-detector",
            Self::PerformanceSentinel => "performance-sentinel",
            Self::CoverageTracker => "coverage-tracker",
            Self::PlanLifecycleManager => "plan-lifecycle-mgr",
            Self::CrossSystemTester => "cross-system-tester",
            Self::ErrorDiagnoser => "error-diagnoser",
            Self::DependencyValidator => "dep-validator",
            Self::PatternExtractor => "pattern-extractor",
            Self::SnapshotComparator => "snapshot-comparator",
            Self::FullLoopValidator => "full-loop-validator",
        }
    }

    /// 4-6 char mnemonic for compact TUI columns.
    #[must_use]
    pub const fn short(self) -> &'static str {
        match self {
            Self::Conductor => "cond",
            Self::Strategist => "strat",
            Self::Implementer => "impl",
            Self::Architect => "arch",
            Self::Researcher => "rsrch",
            Self::Auditor => "audit",
            Self::QuickReviewer => "qrev",
            Self::Scribe => "scribe",
            Self::Critic => "critic",
            Self::AutoFixer => "afix",
            Self::Refactorer => "refac",
            Self::PrePlanner => "prepl",
            Self::DocVerifier => "docvf",
            Self::IntegrationTester => "itest",
            Self::MergeResolver => "merge",
            Self::TerminalValidator => "tval",
            Self::GolemLifecycleTester => "glct",
            Self::SpecDriftDetector => "sdrf",
            Self::RegressionDetector => "regd",
            Self::PerformanceSentinel => "perf",
            Self::CoverageTracker => "covr",
            Self::PlanLifecycleManager => "plcm",
            Self::CrossSystemTester => "xsys",
            Self::ErrorDiagnoser => "errdx",
            Self::DependencyValidator => "depv",
            Self::PatternExtractor => "patrn",
            Self::SnapshotComparator => "snapc",
            Self::FullLoopValidator => "FLV",
        }
    }

    /// Default CLI backend for this role. Can be overridden per-call.
    ///
    /// Strategy: agent-mode roles that need rich tool access default to
    /// Claude CLI; structured review/validation roles default to Codex.
    #[must_use]
    pub const fn backend(self) -> AgentBackend {
        match self {
            Self::Conductor
            | Self::Strategist
            | Self::Implementer
            | Self::Researcher
            | Self::Auditor
            | Self::QuickReviewer
            | Self::Scribe
            | Self::Critic
            | Self::AutoFixer
            | Self::FullLoopValidator => AgentBackend::Claude,

            Self::Architect
            | Self::Refactorer
            | Self::PrePlanner
            | Self::DocVerifier
            | Self::IntegrationTester
            | Self::MergeResolver
            | Self::TerminalValidator
            | Self::GolemLifecycleTester
            | Self::SpecDriftDetector
            | Self::RegressionDetector
            | Self::PerformanceSentinel
            | Self::CoverageTracker
            | Self::PlanLifecycleManager
            | Self::CrossSystemTester
            | Self::ErrorDiagnoser
            | Self::DependencyValidator
            | Self::PatternExtractor
            | Self::SnapshotComparator => AgentBackend::Codex,
        }
    }

    /// Default model tier this role routes to before the bandit learns.
    ///
    /// Strategy (from `mori-agents/03`):
    /// - **Fast**: orchestration, classification, quick patches
    /// - **Standard**: implementation, review, most validators
    /// - **Premium**: architecture, cross-system correctness
    #[must_use]
    pub const fn model_tier(self) -> ModelTier {
        match self {
            // Orchestration / lightweight
            Self::Conductor
            | Self::PrePlanner
            | Self::PlanLifecycleManager
            | Self::AutoFixer
            | Self::DependencyValidator
            | Self::PatternExtractor
            | Self::SnapshotComparator
            | Self::CoverageTracker
            | Self::PerformanceSentinel
            | Self::RegressionDetector => ModelTier::Fast,

            // Premium reasoning
            Self::Architect
            | Self::Critic
            | Self::CrossSystemTester
            | Self::SpecDriftDetector
            | Self::FullLoopValidator => ModelTier::Premium,

            // Standard workhorse
            Self::Strategist
            | Self::Implementer
            | Self::Researcher
            | Self::Auditor
            | Self::QuickReviewer
            | Self::Scribe
            | Self::Refactorer
            | Self::DocVerifier
            | Self::IntegrationTester
            | Self::MergeResolver
            | Self::TerminalValidator
            | Self::GolemLifecycleTester
            | Self::ErrorDiagnoser => ModelTier::Standard,
        }
    }

    /// Default per-turn dollar budget (base USD × 1.0 multiplier).
    ///
    /// Figures from `mori-agents/03` budget table.
    #[must_use]
    pub const fn turn_budget(self) -> TurnBudget {
        let base = match self {
            // Orchestration turns are cheap and frequent.
            Self::Conductor | Self::PlanLifecycleManager => 0.10,
            Self::AutoFixer | Self::PrePlanner => 0.25,
            Self::DependencyValidator
            | Self::PatternExtractor
            | Self::SnapshotComparator
            | Self::CoverageTracker
            | Self::PerformanceSentinel
            | Self::RegressionDetector => 0.30,

            // Standard tier — the workhorses.
            Self::DocVerifier | Self::QuickReviewer | Self::Scribe | Self::ErrorDiagnoser => 0.75,
            Self::Strategist
            | Self::Researcher
            | Self::Refactorer
            | Self::MergeResolver
            | Self::IntegrationTester
            | Self::TerminalValidator
            | Self::GolemLifecycleTester => 1.00,
            Self::Implementer | Self::Auditor => 1.50,

            // Premium reasoning — expensive turns.
            Self::Critic => 2.00,
            Self::Architect
            | Self::CrossSystemTester
            | Self::SpecDriftDetector
            | Self::FullLoopValidator => 3.00,
        };
        TurnBudget::new(base)
    }

    /// Default tool permissions for this role.
    #[must_use]
    pub const fn tool_permissions(self) -> ToolPermissions {
        match self {
            // Write access for code-producing roles.
            Self::Implementer
            | Self::AutoFixer
            | Self::Refactorer
            | Self::MergeResolver
            | Self::Scribe => {
                let mut p = ToolPermissions::full();
                p.git = matches!(self, Self::MergeResolver);
                p
            }

            // Read + exec (can run tests, never edit).
            Self::IntegrationTester
            | Self::TerminalValidator
            | Self::GolemLifecycleTester
            | Self::CrossSystemTester
            | Self::FullLoopValidator
            | Self::DependencyValidator
            | Self::RegressionDetector
            | Self::PerformanceSentinel
            | Self::CoverageTracker
            | Self::ErrorDiagnoser
            | Self::SnapshotComparator => ToolPermissions::read_exec(),

            // Network-enabled research.
            Self::Researcher => ToolPermissions::networked(),

            // Pure reviewers / watchers — read only.
            Self::Conductor
            | Self::Strategist
            | Self::Architect
            | Self::Auditor
            | Self::QuickReviewer
            | Self::Critic
            | Self::PrePlanner
            | Self::DocVerifier
            | Self::SpecDriftDetector
            | Self::PlanLifecycleManager
            | Self::PatternExtractor => ToolPermissions::read_only(),
        }
    }

    /// Stable numeric index for array indexing.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Conductor => 0,
            Self::Strategist => 1,
            Self::Implementer => 2,
            Self::Architect => 3,
            Self::Researcher => 4,
            Self::Auditor => 5,
            Self::QuickReviewer => 6,
            Self::Scribe => 7,
            Self::Critic => 8,
            Self::AutoFixer => 9,
            Self::Refactorer => 10,
            Self::PrePlanner => 11,
            Self::DocVerifier => 12,
            Self::IntegrationTester => 13,
            Self::MergeResolver => 14,
            Self::TerminalValidator => 15,
            Self::GolemLifecycleTester => 16,
            Self::SpecDriftDetector => 17,
            Self::RegressionDetector => 18,
            Self::PerformanceSentinel => 19,
            Self::CoverageTracker => 20,
            Self::PlanLifecycleManager => 21,
            Self::CrossSystemTester => 22,
            Self::ErrorDiagnoser => 23,
            Self::DependencyValidator => 24,
            Self::PatternExtractor => 25,
            Self::SnapshotComparator => 26,
            Self::FullLoopValidator => 27,
        }
    }
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_TTFT_TIMEOUT_MS;
    use crate::config::schema::{ModelProfile, ProviderConfig, RokoConfig};

    #[test]
    fn backend_from_claude_slug() {
        assert_eq!(
            AgentBackend::from_model("claude-sonnet-4-5"),
            AgentBackend::Claude
        );
    }

    #[test]
    fn backend_from_cursor_slug() {
        assert_eq!(AgentBackend::from_model("composer-1"), AgentBackend::Cursor);
        assert_eq!(AgentBackend::from_model("sonnet-4"), AgentBackend::Cursor);
        assert_eq!(AgentBackend::from_model("auto"), AgentBackend::Cursor);
        assert_eq!(AgentBackend::from_model("gpt-5-high"), AgentBackend::Cursor);
    }

    #[test]
    fn backend_from_ollama_slug() {
        assert_eq!(
            AgentBackend::from_model("ollama/llama3"),
            AgentBackend::Ollama
        );
        assert_eq!(AgentBackend::from_model("llama3-8b"), AgentBackend::Ollama);
    }

    #[test]
    fn backend_from_codex_slug() {
        assert_eq!(AgentBackend::from_model("gpt-5"), AgentBackend::Codex);
        assert_eq!(AgentBackend::from_model("o3-mini"), AgentBackend::Codex);
    }

    #[test]
    fn backend_from_perplexity_slug() {
        assert_eq!(AgentBackend::from_model("sonar"), AgentBackend::Perplexity);
        assert_eq!(
            AgentBackend::from_model("sonar-pro"),
            AgentBackend::Perplexity
        );
        assert_eq!(
            AgentBackend::from_model("perplexity/sonar"),
            AgentBackend::Perplexity
        );
        assert_eq!(
            ProviderKind::from(AgentBackend::Perplexity),
            ProviderKind::PerplexityApi
        );
    }

    #[test]
    fn kimi_not_cursor() {
        assert!(!is_cursor_slug("kimi-k2.5"));
        assert_eq!(AgentBackend::from_model("kimi-k2.5"), AgentBackend::Codex);
        assert!(!is_cursor_slug("glm-5.1"));
        assert_eq!(AgentBackend::from_model("glm-5.1"), AgentBackend::Codex);
    }

    #[test]
    fn backend_to_provider_kind() {
        assert_eq!(
            ProviderKind::from(AgentBackend::Claude),
            ProviderKind::ClaudeCli
        );
        assert_eq!(
            ProviderKind::from(AgentBackend::Codex),
            ProviderKind::OpenAiCompat
        );
        assert_eq!(
            ProviderKind::from(AgentBackend::OpenAi),
            ProviderKind::OpenAiCompat
        );
        assert_eq!(
            ProviderKind::from(AgentBackend::Cursor),
            ProviderKind::CursorAcp
        );
        assert_eq!(
            ProviderKind::from(AgentBackend::Ollama),
            ProviderKind::OpenAiCompat
        );
        assert_eq!(
            ProviderKind::from(AgentBackend::Perplexity),
            ProviderKind::PerplexityApi
        );
    }

    #[test]
    fn provider_kind_labels_and_display() {
        let kinds = [
            (ProviderKind::AnthropicApi, "anthropic_api"),
            (ProviderKind::ClaudeCli, "claude_cli"),
            (ProviderKind::OpenAiCompat, "openai_compat"),
            (ProviderKind::CursorAcp, "cursor_acp"),
            (ProviderKind::PerplexityApi, "perplexity_api"),
            (ProviderKind::GeminiApi, "gemini_api"),
        ];

        for (kind, label) in kinds {
            assert_eq!(kind.label(), label);
            assert_eq!(kind.to_string(), label);
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{label}\""));
            let decoded: ProviderKind = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, kind);
        }
    }

    #[test]
    fn all_roles_unique_labels_and_indices() {
        let mut labels = std::collections::HashSet::new();
        let mut indices = std::collections::HashSet::new();
        let all_with_conductor: Vec<AgentRole> = std::iter::once(AgentRole::Conductor)
            .chain(AgentRole::ALL_AGENTS.iter().copied())
            .collect();
        for r in &all_with_conductor {
            assert!(labels.insert(r.label()), "dup label: {}", r.label());
            assert!(indices.insert(r.index()), "dup index: {}", r.index());
        }
        assert_eq!(all_with_conductor.len(), 28);
    }

    #[test]
    fn all_roles_have_distinct_short_names() {
        let mut shorts = std::collections::HashSet::new();
        let all_with_conductor: Vec<AgentRole> = std::iter::once(AgentRole::Conductor)
            .chain(AgentRole::ALL_AGENTS.iter().copied())
            .collect();
        for r in &all_with_conductor {
            assert!(shorts.insert(r.short()), "dup short: {}", r.short());
        }
    }

    #[test]
    fn conductor_excluded_from_all_agents() {
        assert!(!AgentRole::ALL_AGENTS.contains(&AgentRole::Conductor));
        assert_eq!(AgentRole::ALL_AGENTS.len(), 27);
    }

    #[test]
    fn implementer_budget_higher_than_conductor() {
        assert!(
            AgentRole::Implementer.turn_budget().effective_usd()
                > AgentRole::Conductor.turn_budget().effective_usd()
        );
    }

    #[test]
    fn architect_is_premium_tier() {
        assert_eq!(AgentRole::Architect.model_tier(), ModelTier::Premium);
        assert_eq!(AgentRole::Conductor.model_tier(), ModelTier::Fast);
        assert_eq!(AgentRole::Implementer.model_tier(), ModelTier::Standard);
    }

    #[test]
    fn auditor_is_read_only() {
        let p = AgentRole::Auditor.tool_permissions();
        assert!(p.read);
        assert!(!p.write);
        assert!(!p.exec);
    }

    #[test]
    fn implementer_has_write_and_exec() {
        let p = AgentRole::Implementer.tool_permissions();
        assert!(p.read);
        assert!(p.write);
        assert!(p.exec);
    }

    #[test]
    fn researcher_has_network() {
        assert!(AgentRole::Researcher.tool_permissions().network);
    }

    #[test]
    fn merge_resolver_has_git() {
        assert!(AgentRole::MergeResolver.tool_permissions().git);
    }

    #[test]
    fn display_matches_label() {
        assert_eq!(AgentRole::Implementer.to_string(), "implementer");
        assert_eq!(AgentRole::QuickReviewer.to_string(), "quick-reviewer");
    }

    #[test]
    fn turn_budget_applies_multiplier() {
        let b = TurnBudget::new(1.0).with_multiplier(2.0);
        assert!((b.effective_usd() - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn backend_short_labels_are_two_chars() {
        for b in [
            AgentBackend::Claude,
            AgentBackend::Codex,
            AgentBackend::Cursor,
            AgentBackend::Ollama,
            AgentBackend::OpenAi,
            AgentBackend::Perplexity,
        ] {
            assert_eq!(b.short().len(), 2);
        }
    }

    #[test]
    fn model_spec_infers_backend_from_slug() {
        let m = ModelSpec::from_slug("claude-sonnet-4-5");
        assert_eq!(m.backend, AgentBackend::Claude);
        assert_eq!(m.effort, ReasoningEffort::Medium);
    }

    #[test]
    fn model_spec_with_effort() {
        let m = ModelSpec::from_slug("gpt-5").with_effort(ReasoningEffort::High);
        assert_eq!(m.effort, ReasoningEffort::High);
    }

    #[test]
    fn model_spec_short_strips_prefixes() {
        assert_eq!(
            ModelSpec::from_slug("claude-sonnet-4-5").short(),
            "cl-sonnet-4-5"
        );
        assert_eq!(ModelSpec::from_slug("composer-1").short(), "cx-1");
        assert_eq!(ModelSpec::from_slug("gpt-5-high").short(), "5-high");
    }

    #[test]
    fn resolve_model_uses_config_lookup() {
        let mut config = RokoConfig::default();
        config.providers.insert(
            "zai".to_owned(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: Some("https://api.z.ai/api/paas/v4".to_owned()),
                api_key_env: Some("ZAI_API_KEY".to_owned()),
                command: None,
                args: None,
                timeout_ms: Some(120_000),
                ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
                connect_timeout_ms: Some(5_000),
                extra_headers: None,
                max_concurrent: Some(8),
            },
        );
        config.models.insert(
            "glm-5-1".to_owned(),
            ModelProfile {
                provider: "zai".to_owned(),
                slug: "glm-5.1".to_owned(),
                context_window: 200_000,
                max_output: Some(131_072),
                supports_tools: true,
                supports_thinking: true,
                supports_vision: false,
                supports_web_search: false,
                supports_mcp_tools: true,
                supports_partial: false,
                supports_grounding: false,
                supports_code_execution: false,
                supports_caching: false,
                provider_routing: None,
                tool_format: "openai_json".to_owned(),
                cost_input_per_m: Some(1.4),
                cost_output_per_m: Some(4.4),
                cost_input_per_m_high: None,
                cost_output_per_m_high: None,
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                thinking_level: None,
                max_tools: None,
                tokenizer_ratio: None,
                ..Default::default()
            },
        );

        let resolved = resolve_model(&config, "glm-5-1");
        assert_eq!(resolved.model_key, "glm-5-1");
        assert_eq!(resolved.slug, "glm-5.1");
        assert_eq!(resolved.provider_kind, ProviderKind::OpenAiCompat);
        assert_eq!(resolved.backend, AgentBackend::Codex);
        assert!(resolved.provider_config.is_some());
        assert!(resolved.profile.is_some());
    }

    #[test]
    fn resolve_model_falls_back_to_legacy_backend() {
        let config = RokoConfig::default();

        let resolved = resolve_model(&config, "claude-sonnet-4-6");
        assert_eq!(resolved.model_key, "claude-sonnet-4-6");
        assert_eq!(resolved.slug, "claude-sonnet-4-6");
        assert_eq!(resolved.provider_kind, ProviderKind::ClaudeCli);
        assert_eq!(resolved.backend, AgentBackend::Claude);
        assert!(resolved.provider_config.is_none());
        assert!(resolved.profile.is_none());
    }

    #[test]
    fn score_model_for_task_disqualifies_missing_hard_requirements() {
        let profile = ModelProfile {
            provider: "openai".to_owned(),
            slug: "gpt-5-mini".to_owned(),
            context_window: 128_000,
            max_output: Some(8_192),
            supports_tools: true,
            supports_thinking: false,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            supports_grounding: false,
            supports_code_execution: false,
            supports_caching: false,
            provider_routing: None,
            tool_format: "openai_json".to_owned(),
            cost_input_per_m: None,
            cost_output_per_m: Some(2.0),
            cost_input_per_m_high: None,
            cost_output_per_m_high: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            thinking_level: None,
            max_tools: None,
            tokenizer_ratio: None,
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
        };

        let requirements = TaskRequirements {
            needs_thinking: true,
            ..TaskRequirements::default()
        };
        assert!(score_model_for_task(&profile, &requirements).is_none());
    }

    #[test]
    fn select_model_for_task_prefers_matching_capabilities_and_cost() {
        let mut config = RokoConfig::default();
        config.models.insert(
            "cheap".to_owned(),
            ModelProfile {
                provider: "openai".to_owned(),
                slug: "gpt-5-mini".to_owned(),
                context_window: 128_000,
                max_output: Some(8_192),
                supports_tools: true,
                supports_thinking: false,
                supports_vision: false,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                supports_grounding: false,
                supports_code_execution: false,
                supports_caching: false,
                provider_routing: None,
                tool_format: "openai_json".to_owned(),
                cost_input_per_m: None,
                cost_output_per_m: Some(2.0),
                cost_input_per_m_high: None,
                cost_output_per_m_high: None,
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                thinking_level: None,
                max_tools: None,
                tokenizer_ratio: None,
                supports_search: false,
                supports_citations: false,
                supports_async: false,
                is_embedding_model: false,
                search_context_size: None,
                cost_per_request: None,
            },
        );
        config.models.insert(
            "capable".to_owned(),
            ModelProfile {
                provider: "gemini".to_owned(),
                slug: "gemini-2.5-pro".to_owned(),
                context_window: 1_048_576,
                max_output: Some(65_536),
                supports_tools: true,
                supports_thinking: true,
                supports_vision: true,
                supports_web_search: true,
                supports_mcp_tools: true,
                supports_partial: true,
                supports_grounding: true,
                supports_code_execution: true,
                supports_caching: true,
                provider_routing: None,
                tool_format: "openai_json".to_owned(),
                cost_input_per_m: None,
                cost_output_per_m: Some(8.0),
                cost_input_per_m_high: None,
                cost_output_per_m_high: None,
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                thinking_level: None,
                max_tools: None,
                tokenizer_ratio: None,
                supports_search: true,
                supports_citations: false,
                supports_async: false,
                is_embedding_model: false,
                search_context_size: None,
                cost_per_request: None,
            },
        );
        let requirements = TaskRequirements {
            needs_web_search: true,
            needs_code_execution: true,
            needs_thinking: true,
            min_context_window: 150_000,
            max_cost_output_per_m: Some(15.0),
            ..TaskRequirements::default()
        };

        let selected = select_model_for_task(&config, &requirements).expect("selected model");
        assert_eq!(selected, "capable");
    }

    #[test]
    fn resolve_model_prefix_matches_slug() {
        let mut config = RokoConfig::default();
        config.models.insert(
            "opus".to_owned(),
            ModelProfile {
                provider: "anthropic".to_owned(),
                slug: "claude-opus-4-6".to_owned(),
                ..Default::default()
            },
        );

        // "claude-opus-4" is a prefix of slug "claude-opus-4-6" separated by '-'
        let resolved = resolve_model(&config, "claude-opus-4");
        assert_eq!(resolved.model_key, "opus");
        assert_eq!(resolved.slug, "claude-opus-4-6");
        assert!(resolved.profile.is_some());
    }

    #[test]
    fn resolve_model_prefix_requires_separator() {
        let mut config = RokoConfig::default();
        config.models.insert(
            "o3".to_owned(),
            ModelProfile {
                provider: "openai".to_owned(),
                slug: "o3".to_owned(),
                ..Default::default()
            },
        );
        config.models.insert(
            "o3-mini".to_owned(),
            ModelProfile {
                provider: "openai".to_owned(),
                slug: "o3-mini".to_owned(),
                ..Default::default()
            },
        );

        // "o3" should match exactly, not prefix-match "o3-mini"
        let resolved = resolve_model(&config, "o3");
        assert_eq!(resolved.model_key, "o3");
        assert_eq!(resolved.slug, "o3");
    }

    #[test]
    fn serde_kebab_case_roundtrip() {
        let r = AgentRole::QuickReviewer;
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, "\"quick-reviewer\"");
        let decoded: AgentRole = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, r);
    }
}
