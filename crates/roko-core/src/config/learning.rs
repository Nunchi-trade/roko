//! Learning subsystem configuration.

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
