//! Learning, demurrage, attention, immune, temporal, and goals configuration.

use serde::{Deserialize, Serialize};

use super::agent::default_true;

// ---- [learning] ----------------------------------------------------------

/// Learning subsystem configuration.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LearningConfig {
    /// Auto-refresh playbook rules after successful tasks.
    #[serde(default = "default_true")]
    pub auto_playbook_refresh: bool,
    /// Inject file difficulty profiles into agent context.
    #[serde(default = "default_true")]
    pub knowledge_file_intel: bool,
    /// Inject grimoire warnings into agent context.
    #[serde(default = "default_true")]
    pub knowledge_warnings: bool,
    /// Enable cross-task wave context propagation.
    #[serde(default = "default_true")]
    pub knowledge_wave_context: bool,
    /// Enable error signature pattern matching.
    #[serde(default = "default_true")]
    pub knowledge_error_patterns: bool,
    /// Min occurrences before promoting learned rules.
    #[serde(default = "default_learning_min_occ")]
    pub learning_min_occurrences: usize,
    /// Max file-intel entries to inject per task.
    #[serde(default = "default_file_intel_max")]
    pub file_intel_max_entries: usize,
    /// Max warning entries to inject per task.
    #[serde(default = "default_warning_max")]
    pub warning_max_entries: usize,
    /// Whether repeated gate failures should trigger a plan revision.
    #[serde(default = "default_true")]
    pub replan_on_gate_failure: bool,
    /// Maximum number of gate-failure-triggered plan revisions per plan.
    #[serde(default = "default_replan_max_per_plan")]
    pub replan_max_per_plan: u32,
    /// Consecutive gate failures required before emitting a plan revision.
    #[serde(default = "default_replan_gate_attempts")]
    pub replan_gate_attempts: u32,
    /// Run dream consolidation after a plan completes.
    #[serde(default = "default_true")]
    pub dream_on_completion: bool,
    /// Enable the lookahead router for cost-saving tier downgrades.
    /// When true, the cascade router selection is post-filtered through
    /// `LookaheadRouter::route_with_lookahead()` which may downgrade to a
    /// cheaper model when calibration data indicates sufficient success
    /// probability.
    #[serde(default)]
    pub use_lookahead_router: bool,
    /// Success probability threshold for the lookahead router to accept a
    /// cheaper tier (0.0--1.0). Only used when `use_lookahead_router` is true.
    /// Defaults to 0.7.
    #[serde(default = "default_lookahead_threshold")]
    pub lookahead_threshold: f64,
}

fn default_lookahead_threshold() -> f64 {
    0.7
}

const fn default_learning_min_occ() -> usize {
    2
}

const fn default_file_intel_max() -> usize {
    15
}

const fn default_warning_max() -> usize {
    5
}

const fn default_replan_max_per_plan() -> u32 {
    2
}

const fn default_replan_gate_attempts() -> u32 {
    3
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            auto_playbook_refresh: true,
            knowledge_file_intel: true,
            knowledge_warnings: true,
            knowledge_wave_context: true,
            knowledge_error_patterns: true,
            learning_min_occurrences: default_learning_min_occ(),
            file_intel_max_entries: default_file_intel_max(),
            warning_max_entries: default_warning_max(),
            replan_on_gate_failure: true,
            replan_max_per_plan: default_replan_max_per_plan(),
            replan_gate_attempts: default_replan_gate_attempts(),
            dream_on_completion: default_true(),
            use_lookahead_router: false,
            lookahead_threshold: default_lookahead_threshold(),
        }
    }
}

// ---- [demurrage] ---------------------------------------------------------

/// Knowledge demurrage configuration.
///
/// Controls the Gesellian decay applied to playbook rules and knowledge
/// entries so that stale, unvalidated heuristics naturally fade.
#[allow(clippy::derive_partial_eq_without_eq)] // contains f64
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DemurrageConfig {
    /// Exponential decay rate per hour applied to knowledge entry balances.
    #[serde(default = "default_demurrage_rate_per_hour")]
    pub rate_per_hour: f64,
    /// Entries with balance below this threshold are deprioritized in retrieval.
    #[serde(default = "default_demurrage_min_balance")]
    pub min_balance: f64,
    /// Balance below which entries are frozen into cold storage.
    #[serde(default = "default_demurrage_freeze_threshold")]
    pub freeze_threshold: f64,
    /// Starting balance for thawed (resurrected) entries.
    #[serde(default = "default_demurrage_thaw_balance")]
    pub thaw_balance: f64,
    /// Maximum balance an entry can accumulate from reinforcement.
    #[serde(default = "default_demurrage_max_balance")]
    pub max_balance: f64,
    /// How often to run demurrage GC (in seconds, 0 = manual only).
    #[serde(default)]
    pub gc_interval_secs: u64,
    /// Per-kind rate multipliers (e.g., Warnings decay faster).
    /// Keys are knowledge kind strings ("warning", "insight", etc.).
    #[serde(default)]
    pub kind_rate_multipliers: std::collections::HashMap<String, f64>,
    /// Whether to freeze entries before deleting (true = preserve for resurrection).
    #[serde(default = "default_true")]
    pub freeze_before_delete: bool,
    /// Death threshold: entries with recency factor below this are considered dead.
    #[serde(default = "default_demurrage_death_threshold")]
    pub death_threshold: f64,
}

const fn default_demurrage_rate_per_hour() -> f64 {
    0.01
}

const fn default_demurrage_min_balance() -> f64 {
    0.1
}

const fn default_demurrage_freeze_threshold() -> f64 {
    0.05
}

const fn default_demurrage_thaw_balance() -> f64 {
    0.6
}

const fn default_demurrage_max_balance() -> f64 {
    5.0
}

const fn default_demurrage_death_threshold() -> f64 {
    0.01
}

impl Default for DemurrageConfig {
    fn default() -> Self {
        Self {
            rate_per_hour: default_demurrage_rate_per_hour(),
            min_balance: default_demurrage_min_balance(),
            freeze_threshold: default_demurrage_freeze_threshold(),
            thaw_balance: default_demurrage_thaw_balance(),
            max_balance: default_demurrage_max_balance(),
            gc_interval_secs: 0,
            kind_rate_multipliers: std::collections::HashMap::new(),
            freeze_before_delete: true,
            death_threshold: default_demurrage_death_threshold(),
        }
    }
}

// ---- [attention] ---------------------------------------------------------

/// Attention token budget allocation and context window management.
///
/// Controls how the runtime distributes token budget across prompt layers
/// and manages context window pressure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttentionConfig {
    /// Maximum tokens to allocate per prompt layer (0 = unlimited).
    #[serde(default = "default_attention_max_tokens_per_layer")]
    pub max_tokens_per_layer: usize,
    /// Context window utilization target as a fraction in `[0.0, 1.0]`.
    #[serde(default = "default_attention_utilization_target")]
    pub utilization_target: f64,
    /// Enable attention auction where layers bid for token budget.
    #[serde(default)]
    pub auction_enabled: bool,
    /// Minimum tokens reserved for task context regardless of auction.
    #[serde(default = "default_attention_task_reserve")]
    pub task_reserve_tokens: usize,
}

const fn default_attention_max_tokens_per_layer() -> usize {
    4096
}

const fn default_attention_utilization_target() -> f64 {
    0.85
}

const fn default_attention_task_reserve() -> usize {
    512
}

impl Default for AttentionConfig {
    fn default() -> Self {
        Self {
            max_tokens_per_layer: default_attention_max_tokens_per_layer(),
            utilization_target: default_attention_utilization_target(),
            auction_enabled: false,
            task_reserve_tokens: default_attention_task_reserve(),
        }
    }
}

// ---- [immune] ------------------------------------------------------------

/// Anomaly detection thresholds and quarantine settings.
///
/// Configures the cognitive immune system that detects anomalous outputs,
/// quarantines suspect results, and classifies taint levels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImmuneConfig {
    /// Anomaly score threshold above which outputs are quarantined.
    #[serde(default = "default_immune_quarantine_threshold")]
    pub quarantine_threshold: f64,
    /// Maximum number of quarantined items before triggering escalation.
    #[serde(default = "default_immune_max_quarantined")]
    pub max_quarantined: usize,
    /// Whether to auto-reject quarantined outputs or hold for review.
    #[serde(default)]
    pub auto_reject: bool,
    /// Taint classification levels: low, medium, high.
    #[serde(default = "default_immune_taint_levels")]
    pub taint_levels: Vec<String>,
}

const fn default_immune_quarantine_threshold() -> f64 {
    0.8
}

const fn default_immune_max_quarantined() -> usize {
    50
}

fn default_immune_taint_levels() -> Vec<String> {
    vec!["low".to_string(), "medium".to_string(), "high".to_string()]
}

impl Default for ImmuneConfig {
    fn default() -> Self {
        Self {
            quarantine_threshold: default_immune_quarantine_threshold(),
            max_quarantined: default_immune_max_quarantined(),
            auto_reject: false,
            taint_levels: default_immune_taint_levels(),
        }
    }
}

// ---- [temporal] ----------------------------------------------------------

/// Time horizon preferences and planning depth configuration.
///
/// Controls how deep the planner looks ahead and how temporal relations
/// between tasks are evaluated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemporalConfig {
    /// Maximum planning depth (number of future task levels to consider).
    #[serde(default = "default_temporal_max_depth")]
    pub max_depth: usize,
    /// Default epoch duration in seconds for batching temporal events.
    #[serde(default = "default_temporal_epoch_secs")]
    pub epoch_secs: u64,
    /// Whether to enforce Allen temporal relations between dependent tasks.
    #[serde(default = "default_true")]
    pub enforce_allen_relations: bool,
}

const fn default_temporal_max_depth() -> usize {
    5
}

const fn default_temporal_epoch_secs() -> u64 {
    3600
}

impl Default for TemporalConfig {
    fn default() -> Self {
        Self {
            max_depth: default_temporal_max_depth(),
            epoch_secs: default_temporal_epoch_secs(),
            enforce_allen_relations: true,
        }
    }
}

// ---- [goals] -------------------------------------------------------------

/// Goal hierarchy configuration with priority weights and completion criteria.
///
/// Controls how goals are ranked, pruned, and when they are considered complete.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoalsConfig {
    /// Maximum number of active goals at any level of the hierarchy.
    #[serde(default = "default_goals_max_active")]
    pub max_active: usize,
    /// Priority weight for correctness vs. speed tradeoff in `[0.0, 1.0]`.
    /// Higher values favor correctness.
    #[serde(default = "default_goals_correctness_weight")]
    pub correctness_weight: f64,
    /// Minimum completion ratio in `[0.0, 1.0]` for a goal to be considered done.
    #[serde(default = "default_goals_completion_threshold")]
    pub completion_threshold: f64,
    /// Prune goals with priority below this value.
    #[serde(default = "default_goals_prune_threshold")]
    pub prune_threshold: f64,
}

const fn default_goals_max_active() -> usize {
    10
}

const fn default_goals_correctness_weight() -> f64 {
    0.7
}

const fn default_goals_completion_threshold() -> f64 {
    0.95
}

const fn default_goals_prune_threshold() -> f64 {
    0.1
}

impl Default for GoalsConfig {
    fn default() -> Self {
        Self {
            max_active: default_goals_max_active(),
            correctness_weight: default_goals_correctness_weight(),
            completion_threshold: default_goals_completion_threshold(),
            prune_threshold: default_goals_prune_threshold(),
        }
    }
}
