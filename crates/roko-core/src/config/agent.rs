//! Agent / model configuration, including per-role overrides.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::temperament::Temperament;

// ---- helpers shared with other modules ------------------------------------

pub(crate) const fn default_true() -> bool {
    true
}

pub(crate) const fn default_context_window() -> u64 {
    128_000
}

pub(crate) fn default_tool_format() -> String {
    "openai_json".into()
}

// ---- [agent] -------------------------------------------------------------

/// Agent / model configuration, including per-role overrides.
#[allow(clippy::derive_partial_eq_without_eq)] // contains f32 via RoleOverride
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Default model slug (e.g. `"claude-sonnet-4-6"`).
    #[serde(default = "default_model", alias = "model")]
    pub default_model: String,
    /// Default backend (e.g. `"claude"`, `"codex"`, `"cursor"`).
    #[serde(default = "default_backend")]
    pub default_backend: String,
    /// Default reasoning effort (`"low"`, `"medium"`, `"high"`, `"max"`).
    #[serde(default = "default_effort", alias = "effort")]
    pub default_effort: String,
    /// Default agent temperament for roles without a local override.
    #[serde(default)]
    pub temperament: Temperament,
    /// Context window limit in thousands of tokens.
    #[serde(default = "default_context_limit_k")]
    pub context_limit_k: u32,
    /// When true, agents use `--bare` (skip built-in system prompt).
    #[serde(default = "default_true")]
    pub bare_mode: bool,
    /// Legacy agent command used when no provider registry is configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Legacy CLI args used for the Claude subprocess path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// Legacy subprocess timeout in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Legacy subprocess environment variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<(String, String)>>,
    /// Legacy per-tier model mapping used before `[models.*]` existed.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tier_models: HashMap<String, String>,
    /// Global fallback model: if an agent spawn fails, retry with this.
    #[serde(default)]
    pub fallback_model: Option<String>,
    /// Per-role overrides keyed by role label (e.g. `"implementer"`, `"architect"`).
    #[serde(default)]
    pub roles: HashMap<String, RoleOverride>,

    /// Default agent behavior overrides under `[agent.defaults]`.
    #[serde(default)]
    pub defaults: AgentDefaults,

    /// Reserved for future CaMeL dual-LLM isolation. The DataLlmConfig
    /// type and DataLlmRouter implementation are substantial enough to
    /// keep around, but no production dispatch path currently consults
    /// this field. See audit T2-21 / 39-config-schema-phantom-fields.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_llm: Option<DataLlmConfig>,

    /// Default agent mode: how long the agent lives.
    #[serde(default)]
    pub mode: AgentMode,

    /// Extensions loaded for all agents (can be overridden per-role).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

/// Agent execution mode controlling lifecycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    /// Agent runs a task then stops. Default for `plan run` tasks.
    #[default]
    Ephemeral,
    /// Agent runs continuously until explicitly stopped.
    /// Used for deployed agents (Railway, Fly) and `roko agent start`.
    Persistent,
    /// Agent sleeps until a trigger fires (webhook, cron, event).
    /// Conserves resources; wakes on matching event.
    Reactive,
}

fn default_model() -> String {
    crate::defaults::MODEL_FOCUSED.to_string()
}

fn default_backend() -> String {
    "claude".into()
}

fn default_effort() -> String {
    "medium".into()
}

const fn default_context_limit_k() -> u32 {
    200
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            default_model: default_model(),
            default_backend: default_backend(),
            default_effort: default_effort(),
            temperament: Temperament::default(),
            context_limit_k: default_context_limit_k(),
            bare_mode: default_true(),
            command: None,
            args: None,
            timeout_ms: None,
            env: None,
            tier_models: HashMap::new(),
            fallback_model: None,
            roles: HashMap::new(),
            defaults: AgentDefaults::default(),
            data_llm: None,
            mode: AgentMode::default(),
            extensions: Vec::new(),
        }
    }
}

/// Default agent behavior overrides. All fields are optional config-key references.
/// Lives under `[agent.defaults]` in `roko.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentDefaults {
    /// Default model key for generic agent dispatch (references `[models.*]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generic_agent_model: Option<String>,
    /// Model key for gate judging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_judge_model: Option<String>,
}

/// Per-role spend and token caps under `[agent.roles.<role>]`.
#[allow(clippy::derive_partial_eq_without_eq)] // contains f64 via derived helpers
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentBudget {
    /// Estimated token ceiling for a single turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens_per_turn: Option<u32>,
    /// Estimated spend ceiling for a single turn, expressed in USD cents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd_cents_per_turn: Option<u32>,
}

impl AgentBudget {
    /// Convert the configured USD-cent ceiling into a USD float.
    #[must_use]
    pub fn max_cost_usd_per_turn(&self) -> Option<f64> {
        self.max_cost_usd_cents_per_turn
            .map(|cents| f64::from(cents) / 100.0)
    }
}

/// Per-role adaptive-threshold overrides under `[agent.roles.<role>]`.
#[allow(clippy::derive_partial_eq_without_eq)] // contains f64
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentThresholds {
    /// Minimum pass-rate floor applied over adaptive gate thresholds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_pass_rate_floor: Option<f64>,
}

/// Per-role routing overrides under `[agent.roles.<role>]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingOverrides {
    /// Force routing to a specific backend/provider family when possible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_backend: Option<String>,
    /// Force routing to the configured model tier when possible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_tier: Option<String>,
}

// ---- CaMeL dual-LLM configuration (SAFE-07) ────────────────────────────

/// Configuration for the Data LLM in the CaMeL dual-LLM architecture.
///
/// The Data LLM processes untrusted external content (web fetches, plugin
/// output, user-provided files) with tool-call capability stripped. It
/// receives the untrusted content plus a schema for valid outputs, and
/// returns a structured extraction that is safe for the Control LLM.
///
/// Three defense layers:
/// 1. Input sanitization (strip known injection patterns)
/// 2. Data LLM isolation (no tools, schema-constrained output)
/// 3. Output validation (schema check + anomaly detection)
///
/// ```toml
/// [agent.data_llm]
/// model = "claude-haiku-4-5"
/// max_tokens = 4096
/// temperature = 0.0
/// strip_tool_calls = true
/// ```
#[allow(clippy::derive_partial_eq_without_eq)] // contains f64
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataLlmConfig {
    /// Model slug for the Data LLM (can be a smaller/cheaper model).
    #[serde(default = "default_data_llm_model")]
    pub model: String,

    /// Maximum tokens the Data LLM is allowed to generate.
    #[serde(default = "default_data_llm_max_tokens")]
    pub max_tokens: u64,

    /// Sampling temperature (0.0 for deterministic extraction).
    #[serde(default)]
    pub temperature: f64,

    /// Whether to strip tool-call capability from the Data LLM dispatch.
    ///
    /// When `true` (the default), the Data LLM cannot generate tool calls
    /// and can only produce text/JSON output.
    #[serde(default = "default_true")]
    pub strip_tool_calls: bool,

    /// Optional JSON Schema that the Data LLM output must conform to.
    ///
    /// When set, the router validates the Data LLM response against this
    /// schema before passing it to the Control LLM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,

    /// Whether to sanitize the input before sending to the Data LLM.
    ///
    /// When `true`, known prompt injection patterns are stripped from the
    /// untrusted content before it reaches the Data LLM.
    #[serde(default = "default_true")]
    pub sanitize_input: bool,
}

fn default_data_llm_model() -> String {
    crate::defaults::MODEL_FAST.to_string()
}

const fn default_data_llm_max_tokens() -> u64 {
    4096
}

impl Default for DataLlmConfig {
    fn default() -> Self {
        Self {
            model: default_data_llm_model(),
            max_tokens: default_data_llm_max_tokens(),
            temperature: 0.0,
            strip_tool_calls: true,
            output_schema: None,
            sanitize_input: true,
        }
    }
}

/// Per-role override under `[agent.roles.<role>]`.
///
/// Every field is optional; absent means "use the agent-level default".
#[allow(clippy::derive_partial_eq_without_eq)] // contains f32/f64 via nested fields
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleOverride {
    /// Explicit runtime role label override; defaults to the section name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Model slug override for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Backend override for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// Reasoning effort override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Temperament override for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperament: Option<Temperament>,
    /// Context window override (in thousands of tokens).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit_k: Option<u32>,
    /// Role-local tool whitelist; absent means no additional restriction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// Per-turn token and cost caps for this role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<AgentBudget>,
    /// Per-role adaptive-threshold overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<AgentThresholds>,
    /// Per-role routing overrides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_overrides: Option<RoutingOverrides>,
    /// Turn budget override (USD).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_budget_usd: Option<f32>,
}

impl RoleOverride {
    /// Resolve the effective runtime role label for this config section.
    #[must_use]
    pub fn resolved_role_name<'a>(&'a self, section_name: &'a str) -> &'a str {
        self.role
            .as_deref()
            .map(str::trim)
            .filter(|role| !role.is_empty())
            .unwrap_or(section_name)
    }

    /// Return the effective per-turn budget, folding the legacy
    /// `turn_budget_usd` field into the nested `budget` block.
    #[must_use]
    pub fn effective_budget(&self) -> Option<AgentBudget> {
        let mut budget = self.budget.clone().unwrap_or_default();
        if budget.max_cost_usd_cents_per_turn.is_none() {
            budget.max_cost_usd_cents_per_turn =
                self.turn_budget_usd.and_then(usd_to_cents_per_turn);
        }
        (budget.max_tokens_per_turn.is_some() || budget.max_cost_usd_cents_per_turn.is_some())
            .then_some(budget)
    }

    /// Resolve the effective temperament for this role override.
    #[must_use]
    pub fn resolved_temperament(&self, default: Temperament) -> Temperament {
        self.temperament.unwrap_or(default)
    }
}

impl AgentConfig {
    /// Resolve the effective temperament for `role_label`.
    #[must_use]
    pub fn temperament_for_role(&self, role_label: &str) -> Temperament {
        self.roles
            .get(role_label)
            .map_or(self.temperament, |override_cfg| {
                override_cfg.resolved_temperament(self.temperament)
            })
    }
}

fn usd_to_cents_per_turn(usd: f32) -> Option<u32> {
    let usd = f64::from(usd);
    if !usd.is_finite() || usd.is_sign_negative() {
        return None;
    }
    let cents = (usd * 100.0).round();
    if cents > f64::from(u32::MAX) {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        Some(cents as u32)
    }
}
