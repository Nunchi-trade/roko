//! Daimon affect state, somatic markers, and dispatch modulation.
//!
//! This crate provides a standalone affect engine for Roko's plan runner.
//! It owns the current PAD state, appraises task events into that state,
//! stores situation-specific somatic markers, and modulates dispatch
//! parameters for future task runs.

#![allow(
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::clone_on_copy,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::manual_clamp,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::option_if_let_else,
    clippy::ref_option,
    clippy::suboptimal_flops,
    clippy::too_many_arguments,
    clippy::useless_conversion
)]

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use kiddo::{KdTree, SquaredEuclidean};
use roko_core::{
    BehavioralState, ContentHash, EmotionalTag, OperatingFrequencyAffect, PadVector, Task,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const STRATEGY_DIMENSIONS: usize = 8;
const DEFAULT_SOMATIC_NEIGHBORS: usize = 5;
const CONTRARIAN_FRACTION: f64 = 0.15;
const SOMATIC_MERGE_DISTANCE_SQUARED: f64 = 0.25;
const SOMATIC_EVENT_VALENCE_THRESHOLD: f64 = 0.30;
const SOMATIC_EVENT_INTENSITY_THRESHOLD: f64 = 0.50;
const DEPOTENTIATION_DELTA_MIN: f64 = 0.30;
const DEPOTENTIATION_DELTA_MAX: f64 = 0.50;
const DEPOTENTIATION_FLOOR: f64 = 0.05;

type SomaticTree = KdTree<f64, STRATEGY_DIMENSIONS>;

fn default_somatic_tree() -> SomaticTree {
    KdTree::new()
}

/// Current affect snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffectState {
    /// Current PAD vector.
    pub pad: PadVector,
    /// Motivational confidence in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Explicit behavioral state derived from PAD plus confidence.
    #[serde(default)]
    pub behavioral_state: BehavioralState,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

impl Default for AffectState {
    fn default() -> Self {
        Self {
            pad: PadVector::neutral(),
            confidence: 0.5,
            behavioral_state: BehavioralState::Engaged,
            updated_at: Utc::now(),
        }
    }
}

impl AffectState {
    /// Construct a neutral state at `updated_at`.
    #[must_use]
    pub fn neutral(updated_at: DateTime<Utc>) -> Self {
        Self {
            updated_at,
            ..Self::default()
        }
    }

    fn decay(&mut self, half_life_hours: f64, now: DateTime<Utc>) {
        let elapsed_hours =
            now.signed_duration_since(self.updated_at).num_seconds() as f64 / 3600.0;
        if elapsed_hours <= 0.0 {
            return;
        }

        let factor = decay_factor(elapsed_hours, half_life_hours);
        if factor != 1.0 {
            self.pad.decay_by_factor(factor);
            self.confidence = (0.5 + (self.confidence - 0.5) * factor).clamp(0.0, 1.0);
        }
        self.refresh_behavioral_state();
        self.updated_at = now;
    }

    fn apply_delta(
        &mut self,
        pleasure: f64,
        arousal: f64,
        dominance: f64,
        confidence: f64,
        now: DateTime<Utc>,
    ) {
        self.pad.apply_delta(pleasure, arousal, dominance);
        self.confidence = (self.confidence + confidence).clamp(0.0, 1.0);
        self.refresh_behavioral_state();
        self.updated_at = now;
    }

    fn refresh_behavioral_state(&mut self) {
        self.behavioral_state = BehavioralState::classify(self.pad, self.confidence);
    }

    /// Build an emotional annotation from the current affect state.
    #[must_use]
    pub fn emotional_tag(&self, trigger: impl Into<String>) -> EmotionalTag {
        let normalized_intensity = (self.pad.magnitude() / 3.0_f64.sqrt()).clamp(0.0, 1.0) as f32;
        EmotionalTag::new(self.pad, normalized_intensity, trigger, self.pad)
    }
}

impl OperatingFrequencyAffect for AffectState {
    fn confidence(&self) -> f64 {
        self.confidence
    }

    fn arousal(&self) -> f64 {
        self.pad.arousal
    }

    fn dominance(&self) -> f64 {
        self.pad.dominance
    }
}

/// Coordinates in the coding-oriented 8D strategy space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrategyCoordinates {
    /// Structural difficulty of the change.
    pub complexity: f64,
    /// Blast radius and failure cost.
    pub risk: f64,
    /// How unfamiliar the task is.
    pub novelty: f64,
    /// Local confidence for this attempt.
    pub confidence: f64,
    /// Deadline / blockage pressure.
    pub time_pressure: f64,
    /// Spatial extent of the change.
    pub scope: f64,
    /// Ease of undoing the work.
    pub reversibility: f64,
    /// Reverse-dependency or dependency-chain depth.
    pub dependency_depth: f64,
}

impl Default for StrategyCoordinates {
    fn default() -> Self {
        Self::neutral()
    }
}

impl StrategyCoordinates {
    /// Construct coordinates and clamp them into `[0.0, 1.0]`.
    #[must_use]
    pub fn new(
        complexity: f64,
        risk: f64,
        novelty: f64,
        confidence: f64,
        time_pressure: f64,
        scope: f64,
        reversibility: f64,
        dependency_depth: f64,
    ) -> Self {
        Self {
            complexity,
            risk,
            novelty,
            confidence,
            time_pressure,
            scope,
            reversibility,
            dependency_depth,
        }
        .clamped()
    }

    /// A neutral mid-space point.
    #[must_use]
    pub const fn neutral() -> Self {
        Self {
            complexity: 0.5,
            risk: 0.5,
            novelty: 0.5,
            confidence: 0.5,
            time_pressure: 0.5,
            scope: 0.5,
            reversibility: 0.5,
            dependency_depth: 0.5,
        }
    }

    /// Return the coordinates as an array for k-d tree queries.
    #[must_use]
    pub const fn as_array(self) -> [f64; STRATEGY_DIMENSIONS] {
        [
            self.complexity,
            self.risk,
            self.novelty,
            self.confidence,
            self.time_pressure,
            self.scope,
            self.reversibility,
            self.dependency_depth,
        ]
    }

    /// Clamp all dimensions into `[0.0, 1.0]`.
    #[must_use]
    pub fn clamped(mut self) -> Self {
        self.complexity = clamp_unit(self.complexity);
        self.risk = clamp_unit(self.risk);
        self.novelty = clamp_unit(self.novelty);
        self.confidence = clamp_unit(self.confidence);
        self.time_pressure = clamp_unit(self.time_pressure);
        self.scope = clamp_unit(self.scope);
        self.reversibility = clamp_unit(self.reversibility);
        self.dependency_depth = clamp_unit(self.dependency_depth);
        self
    }
}

/// Persisted strategy-space definition for the somatic landscape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategySpaceDefinition {
    /// Domain identifier for this strategy-space mapping.
    #[serde(default = "StrategySpaceDefinition::default_domain")]
    pub domain: String,
    /// Human-readable labels for each of the fixed 8 dimensions.
    #[serde(default = "StrategySpaceDefinition::default_dimensions")]
    pub dimensions: [String; STRATEGY_DIMENSIONS],
}

impl StrategySpaceDefinition {
    fn default_domain() -> String {
        "coding".to_string()
    }

    fn default_dimensions() -> [String; STRATEGY_DIMENSIONS] {
        [
            "complexity".to_string(),
            "risk".to_string(),
            "novelty".to_string(),
            "confidence".to_string(),
            "time_pressure".to_string(),
            "scope".to_string(),
            "reversibility".to_string(),
            "dependency_depth".to_string(),
        ]
    }

    /// Coding-domain default strategy-space definition.
    #[must_use]
    pub fn coding() -> Self {
        Self::default()
    }

    /// Construct and validate a strategy-space definition.
    ///
    /// # Errors
    ///
    /// Returns an error when the domain or any dimension name is empty, or
    /// when dimension names are duplicated.
    pub fn validate(self) -> Result<Self> {
        if self.domain.trim().is_empty() {
            return Err(anyhow!("daimon.strategy_space.domain must not be empty"));
        }

        let mut seen = HashSet::with_capacity(STRATEGY_DIMENSIONS);
        for dimension in &self.dimensions {
            let normalized = dimension.trim();
            if normalized.is_empty() {
                return Err(anyhow!(
                    "daimon.strategy_space.dimensions must not contain empty names"
                ));
            }
            if !seen.insert(normalized.to_ascii_lowercase()) {
                return Err(anyhow!(
                    "daimon.strategy_space.dimensions must be unique: duplicate `{normalized}`"
                ));
            }
        }

        Ok(Self {
            domain: self.domain.trim().to_string(),
            dimensions: self
                .dimensions
                .map(|dimension| dimension.trim().to_string()),
        })
    }

    /// Borrow the axis labels.
    #[must_use]
    pub const fn labels(&self) -> &[String; STRATEGY_DIMENSIONS] {
        &self.dimensions
    }

    /// Build the registered coordinate computer for this strategy space.
    #[must_use]
    pub fn computer(&self) -> RegisteredStrategySpaceComputer {
        RegisteredStrategySpaceComputer::new(self.clone())
    }
}

impl Default for StrategySpaceDefinition {
    fn default() -> Self {
        Self {
            domain: Self::default_domain(),
            dimensions: Self::default_dimensions(),
        }
    }
}

/// Normalized task signals used to project work into strategy space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskStrategyObservation {
    /// Task tier label (for example `mechanical` or `architectural`).
    pub task_tier: String,
    /// Number of files touched by the task.
    pub file_count: usize,
    /// Number of verification commands or checks attached to the task.
    pub verification_count: usize,
    /// Number of upstream task dependencies.
    pub dependency_count: usize,
    /// Maximum lines-of-code budget for the task.
    pub max_loc: u32,
    /// Familiarity with the affected area in `[0.0, 1.0]`.
    pub familiarity: f64,
    /// Daimon confidence in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Failure / gate pressure in `[0.0, 1.0]`.
    pub failure_pressure: f64,
    /// Immediate urgency in `[0.0, 1.0]`.
    pub urgency_pressure: f64,
}

impl Default for TaskStrategyObservation {
    fn default() -> Self {
        Self {
            task_tier: "focused".to_string(),
            file_count: 0,
            verification_count: 0,
            dependency_count: 0,
            max_loc: 50,
            familiarity: 0.5,
            confidence: 0.5,
            failure_pressure: 0.0,
            urgency_pressure: 0.0,
        }
    }
}

impl TaskStrategyObservation {
    /// Build a strategy observation from a task and somatic context.
    #[must_use]
    pub fn from_task(task: &Task, context: &TaskContext) -> Self {
        let task_tier = task_tier_label(task);
        let file_count = task.files.len();
        let verification_count = task.test_invariants.as_ref().map_or(0, Vec::len)
            + task.acceptance.len()
            + task.formulas.as_ref().map_or(0, Vec::len)
            + task.types_to_define.as_ref().map_or(0, Vec::len)
            + task.imports.as_ref().map_or(0, Vec::len);
        let dependency_count = task.depends_on.len()
            + task.integration_surfaces.as_ref().map_or(0, Vec::len)
            + task.sidecar_requirements.as_ref().map_or(0, Vec::len)
            + context.dag_depth.round().clamp(0.0, 3.0) as usize;
        let max_loc = task_max_loc_estimate(task);
        let familiarity = task_familiarity(task, context);
        let confidence = context.model_tier_confidence.clamp(0.0, 1.0);
        let failure_pressure = task_failure_pressure(task, context);
        let urgency_pressure = context.deadline_proximity.clamp(0.0, 1.0);

        Self {
            task_tier,
            file_count,
            verification_count,
            dependency_count,
            max_loc,
            familiarity,
            confidence,
            failure_pressure,
            urgency_pressure,
        }
    }
}

/// Extra somatic inputs that are not encoded directly on [`Task`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TaskContext {
    /// Deadline proximity in `[0.0, 1.0]`.
    pub deadline_proximity: f64,
    /// How familiar the task area is in `[0.0, 1.0]`.
    pub existing_code_familiarity: f64,
    /// Effective test coverage or reversibility in `[0.0, 1.0]`.
    pub test_coverage: f64,
    /// DAG depth / reverse-dependency pressure in `[0.0, 1.0]`.
    pub dag_depth: f64,
    /// Confidence in the selected model tier in `[0.0, 1.0]`.
    pub model_tier_confidence: f64,
}

impl Default for TaskContext {
    fn default() -> Self {
        Self {
            deadline_proximity: 0.5,
            existing_code_familiarity: 0.5,
            test_coverage: 0.5,
            dag_depth: 0.5,
            model_tier_confidence: 0.5,
        }
    }
}

impl TaskContext {
    /// Construct a best-effort context snapshot from a task alone.
    #[must_use]
    pub fn from_task(task: &Task) -> Self {
        let complexity_bias = match task.complexity_band {
            Some(roko_core::TaskComplexityBand::Fast) => 0.25,
            Some(roko_core::TaskComplexityBand::Standard) => 0.55,
            Some(roko_core::TaskComplexityBand::Complex) => 0.85,
            _ => match task.estimated_minutes.unwrap_or(90) {
                0..=45 => 0.25,
                46..=120 => 0.55,
                121..=240 => 0.72,
                _ => 0.85,
            },
        };
        let deadline_proximity = match task.speed_priority {
            Some(roko_core::TaskSpeedPriority::Latency) => 0.85,
            Some(roko_core::TaskSpeedPriority::Balanced) => complexity_bias,
            Some(roko_core::TaskSpeedPriority::Accuracy) => 0.35,
            _ => (f64::from(task.estimated_minutes.unwrap_or(90)) / 240.0).clamp(0.25, 0.95),
        };
        let surface_pressure = task_surface_pressure(task);
        let existing_code_familiarity: f64 =
            if task.example_pattern.is_some() {
                0.75
            } else {
                0.45
            } + if task
                .context_files
                .as_ref()
                .is_some_and(|files| !files.is_empty())
            {
                0.10
            } else {
                0.0
            } + if matches!(task.category, Some(roko_core::TaskCategory::Research)) {
                0.05
            } else {
                0.0
            } - if matches!(task.research_before_edit, Some(true)) {
                0.10
            } else {
                0.0
            } - (surface_pressure * 0.15);
        let test_coverage: f64 = if task
            .test_invariants
            .as_ref()
            .is_some_and(|tests| !tests.is_empty())
            || task.quality_profile == Some(roko_core::TaskQualityProfile::Hardened)
        {
            0.8
        } else {
            0.45
        } + if task.acceptance.is_empty() {
            0.0
        } else {
            0.05
        };
        let dag_depth = (task.depends_on.len() as f64
            + optional_vec_len(&task.integration_surfaces) as f64 * 0.5
            + optional_vec_len(&task.sidecar_requirements) as f64 * 0.5
            + optional_vec_len(&task.dependency_tags) as f64 * 0.25)
            / 6.0;
        let model_tier_confidence: f64 =
            match task.complexity_band {
                Some(roko_core::TaskComplexityBand::Fast) => 0.35,
                Some(roko_core::TaskComplexityBand::Standard) => 0.65,
                Some(roko_core::TaskComplexityBand::Complex) => 0.90,
                _ => {
                    if task.quality_profile == Some(roko_core::TaskQualityProfile::Hardened) {
                        0.80
                    } else {
                        0.55
                    }
                }
            } + if task.preferred_model.is_some() || task.preferred_provider.is_some() {
                0.05
            } else {
                0.0
            };

        Self {
            deadline_proximity: deadline_proximity.clamp(0.0, 1.0),
            existing_code_familiarity: existing_code_familiarity.clamp(0.0, 1.0),
            test_coverage: test_coverage.clamp(0.0, 1.0),
            dag_depth: dag_depth.clamp(0.0, 1.0),
            model_tier_confidence: model_tier_confidence.clamp(0.0, 1.0),
        }
    }
}

/// Normalized episode signals used to reconstruct strategy-space placement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeStrategyObservation {
    /// Task tier label captured on the original episode.
    pub task_tier: String,
    /// Number of files touched by the underlying task.
    pub file_count: usize,
    /// Number of verification commands or checks attached to the task.
    pub verification_count: usize,
    /// Number of upstream task dependencies.
    pub dependency_count: usize,
    /// Maximum lines-of-code budget for the task.
    pub max_loc: u32,
    /// Familiarity with the affected area in `[0.0, 1.0]`.
    pub familiarity: f64,
    /// Affect confidence in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Failure / stress pressure in `[0.0, 1.0]`.
    pub failure_pressure: f64,
    /// Emotional intensity in `[0.0, 1.0]`.
    pub emotional_intensity: f64,
}

impl Default for EpisodeStrategyObservation {
    fn default() -> Self {
        Self {
            task_tier: "focused".to_string(),
            file_count: 0,
            verification_count: 0,
            dependency_count: 0,
            max_loc: 50,
            familiarity: 0.5,
            confidence: 0.5,
            failure_pressure: 0.0,
            emotional_intensity: 0.5,
        }
    }
}

/// Strategy-space projector for a particular observation type.
pub trait StrategySpaceComputer<Observation> {
    /// Strategy-space definition owned by this computer.
    fn definition(&self) -> &StrategySpaceDefinition;

    /// Compute normalized 8D coordinates.
    fn compute_coords(&self, observation: &Observation) -> StrategyCoordinates;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DimensionRole {
    Difficulty,
    Danger,
    Familiarity,
    SelfAssessment,
    Urgency,
    Breadth,
    Recoverability,
    Coupling,
}

impl DimensionRole {
    const fn default_for_index(index: usize) -> Self {
        match index {
            0 => Self::Difficulty,
            1 => Self::Danger,
            2 => Self::Familiarity,
            3 => Self::SelfAssessment,
            4 => Self::Urgency,
            5 => Self::Breadth,
            6 => Self::Recoverability,
            _ => Self::Coupling,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CanonicalStrategyProfile {
    difficulty: f64,
    danger: f64,
    familiarity: f64,
    self_assessment: f64,
    urgency: f64,
    breadth: f64,
    recoverability: f64,
    coupling: f64,
}

impl CanonicalStrategyProfile {
    fn into_coords(self) -> StrategyCoordinates {
        StrategyCoordinates::new(
            self.difficulty,
            self.danger,
            self.familiarity,
            self.self_assessment,
            self.urgency,
            self.breadth,
            self.recoverability,
            self.coupling,
        )
    }

    fn value_for_role(self, role: DimensionRole) -> f64 {
        match role {
            DimensionRole::Difficulty => self.difficulty,
            DimensionRole::Danger => self.danger,
            DimensionRole::Familiarity => self.familiarity,
            DimensionRole::SelfAssessment => self.self_assessment,
            DimensionRole::Urgency => self.urgency,
            DimensionRole::Breadth => self.breadth,
            DimensionRole::Recoverability => self.recoverability,
            DimensionRole::Coupling => self.coupling,
        }
    }
}

fn classify_dimension_role(label: &str, index: usize) -> DimensionRole {
    let normalized = label.trim().to_ascii_lowercase();

    if contains_role_keyword(
        &normalized,
        &["complex", "difficulty", "volatility", "unstable"],
    ) {
        return DimensionRole::Difficulty;
    }
    if contains_role_keyword(
        &normalized,
        &[
            "risk",
            "danger",
            "exposure",
            "leverage",
            "slippage",
            "counterparty",
            "blast",
        ],
    ) {
        return DimensionRole::Danger;
    }
    if contains_role_keyword(
        &normalized,
        &[
            "novel",
            "familiar",
            "correlation",
            "similarity",
            "ambiguity",
        ],
    ) {
        return DimensionRole::Familiarity;
    }
    if contains_role_keyword(&normalized, &["confidence", "conviction", "certainty"]) {
        return DimensionRole::SelfAssessment;
    }
    if contains_role_keyword(
        &normalized,
        &["time", "deadline", "horizon", "urgency", "latency"],
    ) {
        return DimensionRole::Urgency;
    }
    if contains_role_keyword(
        &normalized,
        &[
            "scope",
            "breadth",
            "concentration",
            "liquidity",
            "surface",
            "coverage",
        ],
    ) {
        return DimensionRole::Breadth;
    }
    if contains_role_keyword(
        &normalized,
        &[
            "revers",
            "rollback",
            "recover",
            "counterparty",
            "exit",
            "undo",
        ],
    ) {
        return DimensionRole::Recoverability;
    }
    if contains_role_keyword(
        &normalized,
        &[
            "dependency",
            "coupling",
            "regulatory",
            "compliance",
            "integration",
        ],
    ) {
        return DimensionRole::Coupling;
    }

    DimensionRole::default_for_index(index)
}

fn contains_role_keyword(label: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| label.contains(keyword))
}

fn project_profile_for_definition(
    definition: &StrategySpaceDefinition,
    profile: CanonicalStrategyProfile,
) -> StrategyCoordinates {
    let mut values = [0.5_f64; STRATEGY_DIMENSIONS];
    for (index, label) in definition.labels().iter().enumerate() {
        let role = classify_dimension_role(label, index);
        values[index] = profile.value_for_role(role);
    }

    StrategyCoordinates::new(
        values[0], values[1], values[2], values[3], values[4], values[5], values[6], values[7],
    )
}

/// Built-in coding-domain strategy-space computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingStrategySpace {
    definition: StrategySpaceDefinition,
}

impl Default for CodingStrategySpace {
    fn default() -> Self {
        Self {
            definition: StrategySpaceDefinition::coding(),
        }
    }
}

impl CodingStrategySpace {
    fn complexity_from_tier(tier: &str) -> f64 {
        match tier.trim().to_ascii_lowercase().as_str() {
            "mechanical" | "fast" => 0.15,
            "focused" | "standard" => 0.35,
            "integrative" => 0.60,
            "architectural" | "complex" => 0.85,
            "premium" => 0.95,
            _ => 0.50,
        }
    }

    fn scope(complexity: f64, file_count: usize, max_loc: u32) -> f64 {
        (0.55 * complexity
            + 0.30 * (file_count as f64 / 8.0).min(1.0)
            + 0.15 * (f64::from(max_loc) / 400.0).min(1.0))
        .clamp(0.0, 1.0)
    }

    fn novelty(familiarity: f64) -> f64 {
        (1.0 - familiarity).clamp(0.0, 1.0)
    }

    fn risk(
        complexity: f64,
        novelty: f64,
        verification_count: usize,
        failure_pressure: f64,
    ) -> f64 {
        (0.40 * complexity
            + 0.25 * novelty
            + 0.20 * (verification_count as f64 / 4.0).min(1.0)
            + 0.15 * clamp_unit(failure_pressure))
        .clamp(0.0, 1.0)
    }

    fn reversibility(scope: f64, dependency_count: usize, failure_pressure: f64) -> f64 {
        (1.0 - (0.60 * scope
            + 0.20 * (dependency_count as f64 / 6.0).min(1.0)
            + 0.20 * clamp_unit(failure_pressure)))
        .clamp(0.0, 1.0)
    }

    fn dependency_depth(complexity: f64, dependency_count: usize) -> f64 {
        (0.60 * (dependency_count as f64 / 6.0).min(1.0) + 0.40 * complexity).clamp(0.0, 1.0)
    }

    fn task_time_pressure(failure_pressure: f64, urgency_pressure: f64) -> f64 {
        (0.70 * clamp_unit(failure_pressure) + 0.30 * clamp_unit(urgency_pressure)).clamp(0.0, 1.0)
    }

    fn episode_time_pressure(failure_pressure: f64, emotional_intensity: f64) -> f64 {
        (0.60 * clamp_unit(emotional_intensity) + 0.40 * clamp_unit(failure_pressure))
            .clamp(0.0, 1.0)
    }

    fn task_profile(observation: &TaskStrategyObservation) -> CanonicalStrategyProfile {
        let complexity = Self::complexity_from_tier(&observation.task_tier);
        let scope = Self::scope(complexity, observation.file_count, observation.max_loc);
        let novelty = Self::novelty(observation.familiarity);
        let risk = Self::risk(
            complexity,
            novelty,
            observation.verification_count,
            observation.failure_pressure,
        );
        let time_pressure =
            Self::task_time_pressure(observation.failure_pressure, observation.urgency_pressure);
        let reversibility = Self::reversibility(
            scope,
            observation.dependency_count,
            observation.failure_pressure,
        );
        let dependency_depth = Self::dependency_depth(complexity, observation.dependency_count);

        CanonicalStrategyProfile {
            difficulty: complexity,
            danger: risk,
            familiarity: novelty,
            self_assessment: observation.confidence,
            urgency: time_pressure,
            breadth: scope,
            recoverability: reversibility,
            coupling: dependency_depth,
        }
    }

    fn episode_profile(observation: &EpisodeStrategyObservation) -> CanonicalStrategyProfile {
        let complexity = Self::complexity_from_tier(&observation.task_tier);
        let scope = Self::scope(complexity, observation.file_count, observation.max_loc);
        let novelty = Self::novelty(observation.familiarity);
        let risk = Self::risk(
            complexity,
            novelty,
            observation.verification_count,
            observation.failure_pressure,
        );
        let time_pressure = Self::episode_time_pressure(
            observation.failure_pressure,
            observation.emotional_intensity,
        );
        let reversibility = Self::reversibility(
            scope,
            observation.dependency_count,
            observation.failure_pressure,
        );
        let dependency_depth = Self::dependency_depth(complexity, observation.dependency_count);

        CanonicalStrategyProfile {
            difficulty: complexity,
            danger: risk,
            familiarity: novelty,
            self_assessment: observation.confidence,
            urgency: time_pressure,
            breadth: scope,
            recoverability: reversibility,
            coupling: dependency_depth,
        }
    }
}

impl StrategySpaceComputer<TaskStrategyObservation> for CodingStrategySpace {
    fn definition(&self) -> &StrategySpaceDefinition {
        &self.definition
    }

    fn compute_coords(&self, observation: &TaskStrategyObservation) -> StrategyCoordinates {
        Self::task_profile(observation).into_coords()
    }
}

impl StrategySpaceComputer<EpisodeStrategyObservation> for CodingStrategySpace {
    fn definition(&self) -> &StrategySpaceDefinition {
        &self.definition
    }

    fn compute_coords(&self, observation: &EpisodeStrategyObservation) -> StrategyCoordinates {
        Self::episode_profile(observation).into_coords()
    }
}

/// Registered strategy-space computer. Built-in domains get specialized
/// extraction; non-coding domains use a label/role-aware projection until
/// they supply a dedicated extractor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredStrategySpaceComputer {
    definition: StrategySpaceDefinition,
}

impl RegisteredStrategySpaceComputer {
    /// Create a new registered strategy-space computer.
    #[must_use]
    pub fn new(definition: StrategySpaceDefinition) -> Self {
        Self { definition }
    }

    /// Borrow the registered strategy-space definition.
    #[must_use]
    pub const fn definition(&self) -> &StrategySpaceDefinition {
        &self.definition
    }

    /// Whether this definition uses the built-in coding extractor.
    #[must_use]
    pub fn is_builtin_coding(&self) -> bool {
        self.definition.domain.eq_ignore_ascii_case("coding")
    }

    /// Compute live task coordinates for this strategy-space definition.
    #[must_use]
    pub fn task_coords(&self, observation: &TaskStrategyObservation) -> StrategyCoordinates {
        let profile = CodingStrategySpace::task_profile(observation);
        if self.is_builtin_coding() {
            profile.into_coords()
        } else {
            project_profile_for_definition(&self.definition, profile)
        }
    }

    /// Compute dream / replay coordinates for this strategy-space definition.
    #[must_use]
    pub fn episode_coords(&self, observation: &EpisodeStrategyObservation) -> StrategyCoordinates {
        let profile = CodingStrategySpace::episode_profile(observation);
        if self.is_builtin_coding() {
            profile.into_coords()
        } else {
            project_profile_for_definition(&self.definition, profile)
        }
    }
}

impl StrategySpaceComputer<TaskStrategyObservation> for RegisteredStrategySpaceComputer {
    fn definition(&self) -> &StrategySpaceDefinition {
        &self.definition
    }

    fn compute_coords(&self, observation: &TaskStrategyObservation) -> StrategyCoordinates {
        self.task_coords(observation)
    }
}

impl StrategySpaceComputer<EpisodeStrategyObservation> for RegisteredStrategySpaceComputer {
    fn definition(&self) -> &StrategySpaceDefinition {
        &self.definition
    }

    fn compute_coords(&self, observation: &EpisodeStrategyObservation) -> StrategyCoordinates {
        self.episode_coords(observation)
    }
}

/// Project a task and somatic context into the canonical 8D strategy space.
#[must_use]
pub fn extract_strategy_point(task: &Task, context: &TaskContext) -> [f64; STRATEGY_DIMENSIONS] {
    StrategySpaceDefinition::coding()
        .computer()
        .task_coords(&TaskStrategyObservation::from_task(task, context))
        .as_array()
}

/// Situation-specific emotional memory stored in the somatic landscape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SomaticMarker {
    /// Coordinates of the strategy region that produced this marker.
    pub strategy_coords: StrategyCoordinates,
    /// Aggregate valence in `[-1.0, 1.0]`.
    pub valence: f64,
    /// Marker strength in `[0.0, 1.0]`.
    pub intensity: f64,
    /// Supporting episodes that formed the marker.
    pub episodes: Vec<ContentHash>,
    /// Last time the marker was reinforced or updated.
    pub updated_at: DateTime<Utc>,
}

impl SomaticMarker {
    fn clamped(mut self) -> Self {
        self.strategy_coords = self.strategy_coords.clamped();
        self.valence = self.valence.clamp(-1.0, 1.0);
        self.intensity = clamp_unit(self.intensity);
        self
    }
}

/// Aggregate signal returned by the somatic landscape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SomaticSignal {
    /// Weighted average valence after contrarian blending.
    pub valence: f64,
    /// Aggregate signal strength in `[0.0, 1.0]`.
    pub intensity: f64,
    /// Number of same-valence neighbours used.
    pub neighbor_count: usize,
    /// Number of contrarian neighbours mixed in.
    pub contrarian_count: usize,
    /// Episodes that contributed to the signal.
    pub source_episodes: Vec<ContentHash>,
}

impl Default for SomaticSignal {
    fn default() -> Self {
        Self {
            valence: 0.0,
            intensity: 0.0,
            neighbor_count: 0,
            contrarian_count: 0,
            source_episodes: Vec::new(),
        }
    }
}

impl SomaticSignal {
    /// Whether the signal is strong enough to modulate dispatch.
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        self.intensity >= 0.15 && self.valence.abs() >= 0.10
    }

    /// Whether the signal is strong enough to emit an explicit runtime event.
    #[must_use]
    pub fn should_emit_event(&self) -> bool {
        self.intensity > SOMATIC_EVENT_INTENSITY_THRESHOLD
            && self.valence.abs() > SOMATIC_EVENT_VALENCE_THRESHOLD
    }
}

/// Summary of one dream-driven depotentiation pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DepotentiationReport {
    /// PAD arousal before the dream pass.
    pub pre_arousal: f64,
    /// PAD arousal after the dream pass.
    pub post_arousal: f64,
    /// Number of somatic markers whose intensity was reduced.
    pub cooled_markers: usize,
    /// Aggregate reduction applied to somatic marker intensity.
    pub total_marker_intensity_reduction: f64,
}

/// Compact view of the current somatic landscape for UI display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SomaticSummary {
    /// Total number of stored markers.
    pub marker_count: usize,
    /// Number of positively valenced markers.
    pub positive_markers: usize,
    /// Number of negatively valenced markers.
    pub negative_markers: usize,
    /// Number of near-neutral markers.
    pub neutral_markers: usize,
    /// Intensity-weighted average valence.
    pub mean_valence: f64,
    /// Arithmetic mean intensity across markers.
    pub mean_intensity: f64,
    /// Strongest intensity observed in the landscape.
    pub strongest_intensity: f64,
    /// Most recent reinforcement timestamp, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated_at: Option<DateTime<Utc>>,
}

/// Mutable store of somatic markers indexed by a k-d tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SomaticLandscape {
    /// Persisted marker payloads.
    #[serde(default)]
    pub markers: Vec<SomaticMarker>,
    #[serde(skip, default = "default_somatic_tree")]
    tree: SomaticTree,
}

impl Default for SomaticLandscape {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for SomaticLandscape {
    fn eq(&self, other: &Self) -> bool {
        self.markers == other.markers
    }
}

impl SomaticLandscape {
    /// Construct an empty landscape.
    #[must_use]
    pub fn new() -> Self {
        Self {
            markers: Vec::new(),
            tree: default_somatic_tree(),
        }
    }

    /// Rebuild the in-memory k-d tree from persisted markers.
    pub fn rebuild_index(&mut self) {
        self.tree = default_somatic_tree();
        for (idx, marker) in self.markers.iter().enumerate() {
            self.tree
                .add(&marker.strategy_coords.as_array(), idx as u64);
        }
    }

    /// Record a new live outcome into the landscape.
    pub fn record_outcome(
        &mut self,
        strategy_coords: StrategyCoordinates,
        pleasure: f64,
        arousal: f64,
        episode_hash: ContentHash,
        now: DateTime<Utc>,
    ) {
        let marker = SomaticMarker {
            strategy_coords: strategy_coords.clamped(),
            valence: pleasure.clamp(-1.0, 1.0),
            intensity: arousal.abs().clamp(0.0, 1.0),
            episodes: vec![episode_hash],
            updated_at: now,
        };
        self.record_marker(marker);
    }

    /// Insert or reinforce a marker.
    pub fn record_marker(&mut self, marker: SomaticMarker) {
        let marker = marker.clamped();
        if marker.intensity <= 0.0 {
            return;
        }

        let coords = marker.strategy_coords.as_array();
        if !self.markers.is_empty() {
            let nearest = self.tree.nearest_one::<SquaredEuclidean>(&coords);
            if nearest.distance <= SOMATIC_MERGE_DISTANCE_SQUARED {
                let idx = nearest.item as usize;
                if let Some(existing) = self.markers.get_mut(idx) {
                    let same_valence_family = existing.valence.signum() == marker.valence.signum()
                        || existing.valence.abs() < 0.10
                        || marker.valence.abs() < 0.10;
                    if same_valence_family {
                        merge_markers(existing, &marker);
                        self.rebuild_index();
                        return;
                    }
                }
            }
        }

        let item = self.markers.len() as u64;
        self.markers.push(marker);
        self.tree.add(&coords, item);
    }

    /// Query the landscape for the emotional shape of a strategy region.
    #[must_use]
    pub fn query(&self, strategy_coords: StrategyCoordinates, k: usize) -> SomaticSignal {
        if self.markers.is_empty() {
            return SomaticSignal::default();
        }

        let coords = strategy_coords.clamped().as_array();
        let neighbor_count = k.max(1).min(self.markers.len());
        let neighbors = self
            .tree
            .nearest_n::<SquaredEuclidean>(&coords, neighbor_count);
        let dominant_sign = dominant_valence_sign(&neighbors, &self.markers);
        let congruent = self.aggregate_signal(neighbors.iter().filter_map(|neighbor| {
            let idx = neighbor.item as usize;
            let marker = self.markers.get(idx)?;
            (marker.valence.signum() == dominant_sign
                || (dominant_sign == 0.0 && marker.valence.abs() < 0.10))
                .then_some((neighbor.distance, idx))
        }));

        let contrarian_target = ((neighbor_count as f64) * CONTRARIAN_FRACTION).ceil() as usize;
        let contrarian_target = contrarian_target.min(self.markers.len());
        if contrarian_target == 0 || congruent.valence.abs() < 0.05 {
            return SomaticSignal {
                neighbor_count,
                ..congruent
            };
        }

        let congruent_sign = if congruent.valence.abs() >= 0.05 {
            congruent.valence.signum()
        } else {
            dominant_sign
        };
        let mut contrarian_candidates = self
            .markers
            .iter()
            .enumerate()
            .filter(|(_, marker)| {
                marker.valence.signum() != 0.0 && marker.valence.signum() != congruent_sign
            })
            .map(|(idx, marker)| {
                (
                    squared_euclidean(&coords, &marker.strategy_coords.as_array()),
                    idx,
                )
            })
            .collect::<Vec<_>>();
        contrarian_candidates.sort_by(|left, right| left.0.total_cmp(&right.0));
        contrarian_candidates.truncate(contrarian_target);

        if contrarian_candidates.is_empty() {
            return SomaticSignal {
                neighbor_count,
                ..congruent
            };
        }

        let contrarian = self.aggregate_signal(contrarian_candidates.into_iter());
        SomaticSignal {
            valence: (0.85 * congruent.valence + 0.15 * contrarian.valence).clamp(-1.0, 1.0),
            intensity: (0.85 * congruent.intensity + 0.15 * contrarian.intensity).clamp(0.0, 1.0),
            neighbor_count,
            contrarian_count: contrarian.neighbor_count,
            source_episodes: union_hashes(
                congruent.source_episodes.into_iter(),
                contrarian.source_episodes.into_iter(),
            ),
        }
    }

    fn aggregate_signal<I>(&self, items: I) -> SomaticSignal
    where
        I: IntoIterator<Item = (f64, usize)>,
    {
        let mut total_valence = 0.0;
        let mut total_intensity = 0.0;
        let mut total_weight = 0.0;
        let mut count = 0_usize;
        let mut episodes = Vec::new();

        for (distance_sq, idx) in items {
            let Some(marker) = self.markers.get(idx) else {
                continue;
            };
            let distance_weight = 1.0 / (1.0 + distance_sq.max(0.0));
            let weight = distance_weight * marker.intensity.max(0.05);
            total_valence += weight * marker.valence;
            total_intensity += weight * marker.intensity;
            total_weight += weight;
            count += 1;
            extend_unique_hashes(&mut episodes, marker.episodes.iter().copied());
        }

        if total_weight <= 0.0 || count == 0 {
            return SomaticSignal::default();
        }

        SomaticSignal {
            valence: (total_valence / total_weight).clamp(-1.0, 1.0),
            intensity: (total_intensity / total_weight).clamp(0.0, 1.0),
            neighbor_count: count,
            contrarian_count: 0,
            source_episodes: episodes,
        }
    }

    /// Reduce the intensity of highly charged markers during dream processing.
    pub fn apply_dream_depotentiation(&mut self) -> (usize, f64) {
        let mut cooled_markers = 0_usize;
        let mut total_reduction = 0.0;

        for marker in &mut self.markers {
            if marker.intensity <= 0.5 {
                continue;
            }

            let before = marker.intensity;
            let after = depotentiate_magnitude(before);
            if after < before {
                marker.intensity = after;
                cooled_markers += 1;
                total_reduction += before - after;
            }
        }

        (cooled_markers, total_reduction)
    }

    /// Summarize the current landscape for the TUI and runtime telemetry.
    #[must_use]
    pub fn summary(&self) -> SomaticSummary {
        if self.markers.is_empty() {
            return SomaticSummary::default();
        }

        let marker_count = self.markers.len();
        let mut positive_markers = 0_usize;
        let mut negative_markers = 0_usize;
        let mut neutral_markers = 0_usize;
        let mut weighted_valence = 0.0_f64;
        let mut weighted_intensity = 0.0_f64;
        let mut total_weight = 0.0_f64;
        let mut strongest_intensity = 0.0_f64;
        let mut last_updated_at: Option<DateTime<Utc>> = None;

        for marker in &self.markers {
            if marker.valence > 0.10 {
                positive_markers += 1;
            } else if marker.valence < -0.10 {
                negative_markers += 1;
            } else {
                neutral_markers += 1;
            }

            let weight = marker.intensity.max(0.05);
            weighted_valence += weight * marker.valence;
            weighted_intensity += marker.intensity;
            total_weight += weight;
            strongest_intensity = strongest_intensity.max(marker.intensity);
            last_updated_at = Some(match last_updated_at {
                Some(current) => current.max(marker.updated_at.clone()),
                None => marker.updated_at.clone(),
            });
        }

        SomaticSummary {
            marker_count,
            positive_markers,
            negative_markers,
            neutral_markers,
            mean_valence: (weighted_valence / total_weight).clamp(-1.0, 1.0),
            mean_intensity: (weighted_intensity / marker_count as f64).clamp(0.0, 1.0),
            strongest_intensity,
            last_updated_at,
        }
    }
}

/// Behavioral strategy selected by the affect engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchStrategy {
    /// Lower-risk, lower-budget dispatch.
    Conservative,
    /// Default mixed mode.
    Balanced,
    /// More exploratory / broader-search dispatch.
    Exploratory,
    /// Stronger-model, higher-turn-limit dispatch.
    Escalating,
    /// Background-maintenance oriented dispatch.
    Proactive,
}

impl DispatchStrategy {
    /// Claude reasoning-effort hint associated with this strategy.
    #[must_use]
    pub const fn effort_label(self) -> &'static str {
        match self {
            Self::Conservative => "low",
            Self::Balanced => "medium",
            Self::Exploratory => "high",
            Self::Escalating => "high",
            Self::Proactive => "medium",
        }
    }
}

/// Dispatch parameters modulated by the current affect state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchParams {
    /// Model slug chosen for the next dispatch.
    pub model: String,
    /// Maximum turns allowed for the next dispatch.
    pub turn_limit: u32,
    /// Behavioral strategy to apply.
    pub strategy: DispatchStrategy,
    /// Reasoning-effort label for Claude-compatible backends.
    pub effort: String,
}

impl DispatchParams {
    /// Construct a new dispatch parameter set.
    #[must_use]
    pub fn new(model: impl Into<String>, turn_limit: u32) -> Self {
        Self {
            model: model.into(),
            turn_limit,
            strategy: DispatchStrategy::Balanced,
            effort: DispatchStrategy::Balanced.effort_label().to_string(),
        }
    }
}

/// Affect event fed into the daimon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AffectEvent {
    /// Task gate evaluation result.
    GateResult {
        /// Plan identifier.
        plan_id: String,
        /// Task identifier.
        task_id: String,
        /// Whether the gate passed.
        passed: bool,
        /// Gate rung for the task.
        rung: u32,
    },
    /// Final task outcome.
    TaskOutcome {
        /// Task identifier.
        task_id: String,
        /// Whether the task succeeded.
        succeeded: bool,
    },
    /// Work was blocked by dependencies or safety gates.
    Blocked {
        /// Task identifier.
        task_id: String,
        /// Number of blockers observed.
        blocker_count: usize,
    },
    /// Deadline pressure is increasing.
    TimePressure {
        /// Task identifier.
        task_id: String,
        /// Deadline proximity in `[0.0, 1.0]`.
        deadline_proximity: f64,
    },
    /// Work has been waiting in a queue.
    QueueWait {
        /// Task identifier.
        task_id: String,
        /// Hours spent waiting.
        wait_hours: f64,
    },
    /// Repeated failure during dream consolidation.
    DreamFailure {
        /// Task or topic identifier.
        task_type: String,
        /// Number of failing episodes observed.
        failure_count: usize,
    },
}

/// Single entry point for affect operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaimonState {
    /// Current affect snapshot.
    pub state: AffectState,
    /// Half-life in hours for state decay.
    #[serde(default = "default_half_life_hours")]
    pub half_life_hours: f64,
    /// Situation-specific somatic markers.
    #[serde(default)]
    pub somatic_landscape: SomaticLandscape,
    /// Active strategy-space definition for interpreting 8D coordinates.
    #[serde(default)]
    pub strategy_space: StrategySpaceDefinition,
    /// Optional persistence path for best-effort autosaves.
    #[serde(skip, default)]
    persistence_path: Option<PathBuf>,
}

impl Default for DaimonState {
    fn default() -> Self {
        Self::new()
    }
}

impl DaimonState {
    /// Construct a fresh neutral state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: AffectState::default(),
            half_life_hours: default_half_life_hours(),
            somatic_landscape: SomaticLandscape::new(),
            strategy_space: StrategySpaceDefinition::default(),
            persistence_path: None,
        }
    }

    /// Construct a state with a custom half-life.
    #[must_use]
    pub fn with_half_life_hours(half_life_hours: f64) -> Self {
        Self {
            half_life_hours,
            ..Self::new()
        }
    }

    /// Load the state from disk, or return a fresh neutral state.
    #[must_use]
    pub fn load_or_new(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let mut state = fs::read_to_string(path)
            .ok()
            .and_then(|json| serde_json::from_str::<Self>(&json).ok())
            .unwrap_or_default();
        state.rebuild_indexes();
        state.persistence_path = Some(path.to_path_buf());
        state
    }

    /// Attach a persistence path for best-effort autosaves.
    #[must_use]
    pub fn with_persistence_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.persistence_path = Some(path.into());
        self
    }

    /// Borrow the current affect snapshot.
    #[must_use]
    pub const fn query_state(&self) -> &AffectState {
        &self.state
    }

    /// Build an emotional annotation from the current Daimon state.
    #[must_use]
    pub fn emotional_tag(&self, trigger: impl Into<String>) -> EmotionalTag {
        self.state.emotional_tag(trigger)
    }

    /// Borrow the active strategy-space definition.
    #[must_use]
    pub const fn strategy_space(&self) -> &StrategySpaceDefinition {
        &self.strategy_space
    }

    /// Reconfigure the strategy-space definition used by the somatic landscape.
    ///
    /// When the definition changes, previously stored markers are discarded so
    /// incompatible domains do not silently share the same coordinate system.
    ///
    /// # Panics
    ///
    /// Panics if `strategy_space` fails validation. Call
    /// [`StrategySpaceDefinition::validate`] first when the input may be
    /// malformed.
    pub fn configure_strategy_space(&mut self, strategy_space: StrategySpaceDefinition) {
        let strategy_space = strategy_space
            .validate()
            .expect("strategy space should be validated before configuring DaimonState");
        if self.strategy_space == strategy_space {
            return;
        }

        self.strategy_space = strategy_space;
        self.somatic_landscape = SomaticLandscape::new();
        self.autosave();
    }

    /// Query the somatic landscape for a strategy region.
    #[must_use]
    pub fn query_somatic(&self, strategy_coords: StrategyCoordinates) -> SomaticSignal {
        self.somatic_landscape
            .query(strategy_coords, DEFAULT_SOMATIC_NEIGHBORS)
    }

    /// Record a task outcome into the somatic landscape using the current affect state.
    pub fn record_somatic_outcome(
        &mut self,
        strategy_coords: StrategyCoordinates,
        episode_hash: ContentHash,
    ) {
        let now = Utc::now();
        self.somatic_landscape.record_outcome(
            strategy_coords,
            self.state.pad.pleasure,
            self.state.pad.arousal,
            episode_hash,
            now,
        );
        self.autosave();
    }

    /// Record a synthesized somatic marker directly into the landscape.
    pub fn record_somatic_marker(&mut self, marker: SomaticMarker) {
        self.somatic_landscape.record_marker(marker);
        self.autosave();
    }

    /// Summarize the current somatic landscape for display and telemetry.
    #[must_use]
    pub fn somatic_summary(&self) -> SomaticSummary {
        self.somatic_landscape.summary()
    }

    /// Modulate dispatch parameters using both the global affect state and the
    /// situation-specific somatic landscape.
    pub fn modulate_with_strategy(
        &self,
        params: &mut DispatchParams,
        strategy_coords: StrategyCoordinates,
    ) {
        self.modulate(params);
        let state = self.query();
        let signal = self.query_somatic(strategy_coords);
        apply_somatic_bias(params, &state, &signal);
    }

    /// Apply dream-time emotional depotentiation to the live PAD state and the
    /// persisted somatic landscape.
    pub fn apply_dream_depotentiation(&mut self) -> DepotentiationReport {
        let pre_arousal = self.state.pad.arousal;
        self.state.pad.arousal = depotentiate_signed_charge(self.state.pad.arousal);
        let (cooled_markers, total_marker_intensity_reduction) =
            self.somatic_landscape.apply_dream_depotentiation();
        self.state.refresh_behavioral_state();
        self.state.updated_at = Utc::now();
        self.autosave();
        DepotentiationReport {
            pre_arousal,
            post_arousal: self.state.pad.arousal,
            cooled_markers,
            total_marker_intensity_reduction,
        }
    }

    fn autosave(&self) {
        if let Some(path) = self.persistence_path.as_ref() {
            let _ = self.persist(path);
        }
    }

    fn rebuild_indexes(&mut self) {
        self.somatic_landscape.rebuild_index();
        self.state.refresh_behavioral_state();
    }
}

/// Public affect-engine interface for the daimon subsystem.
pub trait AffectEngine {
    /// Appraise one event and return the updated PAD vector.
    fn appraise(&mut self, event: AffectEvent) -> PadVector;
    /// Query the current affect state.
    fn query(&self) -> AffectState;
    /// Modulate dispatch parameters in place.
    fn modulate(&self, params: &mut DispatchParams);
    /// Persist the current engine state to `path`.
    ///
    /// # Errors
    ///
    /// Returns any persistence error encountered while serializing or writing
    /// the engine state to disk.
    fn persist(&self, path: &Path) -> Result<()>;
}

impl AffectEngine for DaimonState {
    fn appraise(&mut self, event: AffectEvent) -> PadVector {
        let now = Utc::now();
        self.state.decay(self.half_life_hours, now);

        match event {
            AffectEvent::GateResult {
                plan_id: _,
                task_id: _,
                passed,
                rung,
            } => {
                let rung_scale = 1.0 + (rung.min(3) as f64 * 0.15);
                if passed {
                    self.state.apply_delta(
                        0.05 * rung_scale,
                        -0.01 * rung_scale,
                        0.03 * rung_scale,
                        0.03 * rung_scale,
                        now,
                    );
                } else {
                    self.state.apply_delta(
                        -0.10 * rung_scale,
                        0.04 * rung_scale,
                        -0.08 * rung_scale,
                        -0.08 * rung_scale,
                        now,
                    );
                }
            }
            AffectEvent::TaskOutcome {
                task_id: _,
                succeeded,
            } => {
                if succeeded {
                    self.state.apply_delta(0.10, 0.00, 0.10, 0.08, now);
                } else {
                    self.state.apply_delta(-0.20, 0.00, -0.15, -0.15, now);
                }
            }
            AffectEvent::Blocked {
                task_id: _,
                blocker_count,
            } => {
                let blockers = blocker_count.max(1).min(5) as f64;
                self.state.apply_delta(
                    0.0,
                    blockers * 0.05,
                    -(blockers * 0.08),
                    -0.02 * blockers,
                    now,
                );
            }
            AffectEvent::TimePressure {
                task_id: _,
                deadline_proximity,
            } => {
                let proximity = deadline_proximity.clamp(0.0, 1.0);
                self.state.apply_delta(0.0, proximity * 0.40, 0.0, 0.0, now);
            }
            AffectEvent::QueueWait {
                task_id: _,
                wait_hours,
            } => {
                let bump = queue_wait_arousal(wait_hours);
                self.state.apply_delta(0.0, bump, 0.0, 0.0, now);
            }
            AffectEvent::DreamFailure {
                task_type: _,
                failure_count,
            } => {
                let failures = failure_count.max(1).min(5) as f64;
                let confidence_drop = -(0.07 * failures).min(0.35);
                self.state.apply_delta(0.0, 0.0, 0.0, confidence_drop, now);
            }
        }

        self.autosave();
        self.state.pad
    }

    fn query(&self) -> AffectState {
        let mut state = self.state.clone();
        state.refresh_behavioral_state();
        state
    }

    fn modulate(&self, params: &mut DispatchParams) {
        let state = self.query();
        match state.behavioral_state {
            BehavioralState::Struggling => {
                if state.pad.pleasure < -0.30 && state.pad.arousal > 0.30 {
                    params.strategy = DispatchStrategy::Conservative;
                    params.turn_limit = params.turn_limit.saturating_sub(3);
                    params.model = demote_model(&params.model);
                } else {
                    params.strategy = DispatchStrategy::Escalating;
                    params.turn_limit = params.turn_limit.saturating_add(10);
                    params.model = promote_model(&params.model);
                }
            }
            BehavioralState::Coasting => {
                params.strategy = DispatchStrategy::Exploratory;
                params.turn_limit = params.turn_limit.saturating_sub(5);
                params.model = demote_model(&params.model);
            }
            BehavioralState::Focused => {
                params.strategy = DispatchStrategy::Balanced;
                params.turn_limit = params.turn_limit.saturating_sub(2);
            }
            BehavioralState::Resting => {
                params.strategy = DispatchStrategy::Proactive;
                params.turn_limit = params.turn_limit.saturating_add(5);
            }
            BehavioralState::Exploring | BehavioralState::Engaged => {
                params.strategy = DispatchStrategy::Balanced;
            }
        }

        params.effort = params.strategy.effort_label().to_string();
    }

    fn persist(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

fn default_half_life_hours() -> f64 {
    4.0
}

fn decay_factor(delta_hours: f64, half_life_hours: f64) -> f64 {
    if delta_hours <= 0.0 {
        return 1.0;
    }
    if half_life_hours <= 0.0 {
        return 0.0;
    }
    0.5_f64.powf(delta_hours / half_life_hours)
}

fn clamp_unit(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

fn task_tier_label(task: &Task) -> String {
    if let Some(band) = task.complexity_band {
        return match band {
            roko_core::TaskComplexityBand::Fast => "mechanical".to_string(),
            roko_core::TaskComplexityBand::Standard => "focused".to_string(),
            roko_core::TaskComplexityBand::Complex => "architectural".to_string(),
            _ => "focused".to_string(),
        };
    }

    if matches!(task.category, Some(roko_core::TaskCategory::Research)) {
        return "integrative".to_string();
    }

    match task.estimated_minutes.unwrap_or(90) {
        0..=45 => "mechanical".to_string(),
        46..=120 => "focused".to_string(),
        121..=240 => "integrative".to_string(),
        _ => "architectural".to_string(),
    }
}

fn task_max_loc_estimate(task: &Task) -> u32 {
    let base = task.estimated_minutes.unwrap_or(90).saturating_mul(4);
    let file_budget = (task.files.len() as u32).saturating_mul(120);
    let context_budget = task
        .context_files
        .as_ref()
        .map(|files| (files.len() as u32).saturating_mul(80))
        .unwrap_or(0);
    let requirement_budget = optional_vec_len(&task.types_to_define) as u32 * 30
        + optional_vec_len(&task.formulas) as u32 * 45
        + task.acceptance.len() as u32 * 18
        + optional_vec_len(&task.imports) as u32 * 20
        + optional_vec_len(&task.sidecar_requirements) as u32 * 90
        + optional_vec_len(&task.integration_surfaces) as u32 * 70
        + optional_vec_len(&task.dependency_tags) as u32 * 15
        + optional_vec_len(&task.fixture_keys) as u32 * 15
        + if task.exclusive_files { 30 } else { 60 };
    base.saturating_add(file_budget)
        .saturating_add(context_budget)
        .saturating_add(requirement_budget)
        .clamp(40, 1_200)
}

fn task_familiarity(task: &Task, context: &TaskContext) -> f64 {
    let mut familiarity = context.existing_code_familiarity.clamp(0.0, 1.0);
    if task.example_pattern.is_some() {
        familiarity += 0.15;
    }
    if task
        .context_files
        .as_ref()
        .is_some_and(|files| !files.is_empty())
    {
        familiarity += 0.10;
    }
    if matches!(task.research_before_edit, Some(true)) {
        familiarity -= 0.10;
    }
    familiarity -= task_surface_pressure(task) * 0.10;
    familiarity.clamp(0.0, 1.0)
}

fn task_failure_pressure(task: &Task, context: &TaskContext) -> f64 {
    let file_pressure = (task.files.len() as f64 / 8.0).min(1.0);
    let dependency_pressure = (task.depends_on.len() as f64 / 6.0).min(1.0);
    let integration_pressure = task
        .integration_surfaces
        .as_ref()
        .map(|surfaces| (surfaces.len() as f64 / 4.0).min(1.0))
        .unwrap_or(0.0);
    let sidecar_pressure = task
        .sidecar_requirements
        .as_ref()
        .map(|requirements| (requirements.len() as f64 / 4.0).min(1.0))
        .unwrap_or(0.0);
    let surface_pressure = task_surface_pressure(task);
    let reversibility_gap = (1.0 - context.test_coverage.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let urgency_pressure = context.deadline_proximity.clamp(0.0, 1.0);
    let complexity_pressure = match task.complexity_band {
        Some(roko_core::TaskComplexityBand::Fast) => 0.2,
        Some(roko_core::TaskComplexityBand::Standard) => 0.5,
        Some(roko_core::TaskComplexityBand::Complex) => 0.85,
        _ => 0.45,
    };
    let routing_uncertainty = if task.preferred_model.is_some() || task.preferred_provider.is_some()
    {
        0.0
    } else {
        0.05
    };
    let retry_pressure = if matches!(task.escalate_on_retry, Some(true)) {
        0.05
    } else {
        0.0
    };
    let exclusivity_pressure = if task.exclusive_files { 0.02 } else { 0.0 };

    ((0.18 * file_pressure
        + 0.18 * dependency_pressure
        + 0.16 * integration_pressure
        + 0.10 * sidecar_pressure
        + 0.12 * surface_pressure
        + 0.18 * reversibility_gap
        + 0.12 * urgency_pressure
        + 0.08 * complexity_pressure)
        + routing_uncertainty
        + retry_pressure
        + exclusivity_pressure)
        .clamp(0.0, 1.0)
}

fn optional_vec_len(value: &Option<Vec<String>>) -> usize {
    value.as_ref().map_or(0, Vec::len)
}

fn task_surface_load(task: &Task) -> usize {
    task.files.len()
        + optional_vec_len(&task.context_files)
        + optional_vec_len(&task.types_to_define)
        + optional_vec_len(&task.formulas)
        + task.acceptance.len()
        + optional_vec_len(&task.imports)
        + optional_vec_len(&task.sidecar_requirements)
        + optional_vec_len(&task.integration_surfaces)
        + optional_vec_len(&task.dependency_tags)
        + optional_vec_len(&task.fixture_keys)
}

fn task_surface_pressure(task: &Task) -> f64 {
    (task_surface_load(task) as f64 / 18.0).clamp(0.0, 1.0)
}

fn squared_euclidean(left: &[f64; STRATEGY_DIMENSIONS], right: &[f64; STRATEGY_DIMENSIONS]) -> f64 {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| {
            let delta = a - b;
            delta * delta
        })
        .sum()
}

fn dominant_valence_sign(
    neighbors: &[kiddo::NearestNeighbour<f64, u64>],
    markers: &[SomaticMarker],
) -> f64 {
    let mut weighted_valence = 0.0;
    for neighbor in neighbors {
        let Some(marker) = markers.get(neighbor.item as usize) else {
            continue;
        };
        let distance_weight = 1.0 / (1.0 + neighbor.distance.max(0.0));
        weighted_valence += distance_weight * marker.intensity.max(0.05) * marker.valence;
    }
    let sign = weighted_valence.signum();
    if sign != 0.0 {
        return sign;
    }
    neighbors
        .iter()
        .find_map(|neighbor| {
            markers
                .get(neighbor.item as usize)
                .map(|marker| marker.valence.signum())
                .filter(|sign| *sign != 0.0)
        })
        .unwrap_or(0.0)
}

fn merge_markers(existing: &mut SomaticMarker, incoming: &SomaticMarker) {
    let total_intensity = existing.intensity + incoming.intensity;
    let existing_weight = if total_intensity > 0.0 {
        existing.intensity / total_intensity
    } else {
        0.5
    };
    let incoming_weight = 1.0 - existing_weight;

    let left = existing.strategy_coords.as_array();
    let right = incoming.strategy_coords.as_array();
    existing.strategy_coords = StrategyCoordinates::new(
        left[0] * existing_weight + right[0] * incoming_weight,
        left[1] * existing_weight + right[1] * incoming_weight,
        left[2] * existing_weight + right[2] * incoming_weight,
        left[3] * existing_weight + right[3] * incoming_weight,
        left[4] * existing_weight + right[4] * incoming_weight,
        left[5] * existing_weight + right[5] * incoming_weight,
        left[6] * existing_weight + right[6] * incoming_weight,
        left[7] * existing_weight + right[7] * incoming_weight,
    );
    existing.valence =
        (existing.valence * existing_weight + incoming.valence * incoming_weight).clamp(-1.0, 1.0);
    existing.intensity = (total_intensity).min(1.0);
    existing.updated_at = existing.updated_at.max(incoming.updated_at);
    extend_unique_hashes(&mut existing.episodes, incoming.episodes.iter().copied());
}

fn union_hashes<I, J>(left: I, right: J) -> Vec<ContentHash>
where
    I: IntoIterator<Item = ContentHash>,
    J: IntoIterator<Item = ContentHash>,
{
    let mut hashes = Vec::new();
    extend_unique_hashes(&mut hashes, left);
    extend_unique_hashes(&mut hashes, right);
    hashes
}

fn extend_unique_hashes<I>(hashes: &mut Vec<ContentHash>, incoming: I)
where
    I: IntoIterator<Item = ContentHash>,
{
    for hash in incoming {
        if !hashes.contains(&hash) {
            hashes.push(hash);
        }
    }
}

fn apply_somatic_bias(params: &mut DispatchParams, state: &AffectState, signal: &SomaticSignal) {
    if !signal.is_actionable() {
        return;
    }

    if signal.valence <= -0.35 {
        params.model = promote_model(&params.model);
        params.turn_limit = params.turn_limit.saturating_add(6);
        if state.behavioral_state != BehavioralState::Struggling {
            params.strategy = DispatchStrategy::Conservative;
        }
    } else if signal.valence <= -0.15 {
        params.turn_limit = params.turn_limit.saturating_add(3);
        if matches!(
            params.strategy,
            DispatchStrategy::Balanced | DispatchStrategy::Exploratory
        ) {
            params.strategy = DispatchStrategy::Conservative;
        }
    } else if signal.valence >= 0.35 {
        if state.behavioral_state != BehavioralState::Struggling {
            params.model = demote_model(&params.model);
            params.turn_limit = params.turn_limit.saturating_sub(3);
            params.strategy = DispatchStrategy::Exploratory;
        }
    } else if signal.valence >= 0.15 && state.behavioral_state != BehavioralState::Struggling {
        params.turn_limit = params.turn_limit.saturating_sub(1);
    }

    params.effort = params.strategy.effort_label().to_string();
}

fn depotentiation_delta(magnitude: f64) -> f64 {
    (DEPOTENTIATION_DELTA_MIN
        + (magnitude - 0.5).max(0.0) * (DEPOTENTIATION_DELTA_MAX - DEPOTENTIATION_DELTA_MIN) / 0.5)
        .clamp(DEPOTENTIATION_DELTA_MIN, DEPOTENTIATION_DELTA_MAX)
}

fn depotentiate_magnitude(magnitude: f64) -> f64 {
    let magnitude = clamp_unit(magnitude);
    if magnitude <= DEPOTENTIATION_FLOOR {
        return magnitude;
    }

    let reduced = magnitude - depotentiation_delta(magnitude);
    reduced.max(DEPOTENTIATION_FLOOR).min(magnitude)
}

fn depotentiate_signed_charge(value: f64) -> f64 {
    if value == 0.0 {
        return 0.0;
    }
    let sign = value.signum();
    let magnitude = value.abs().clamp(0.0, 1.0);
    if magnitude <= DEPOTENTIATION_FLOOR {
        return value.clamp(-1.0, 1.0);
    }
    sign * depotentiate_magnitude(magnitude)
}

/// Queue-wait arousal bump used by runtime feedback and dispatch heuristics.
#[must_use]
pub fn queue_wait_arousal(wait_hours: f64) -> f64 {
    if !wait_hours.is_finite() || wait_hours <= 24.0 {
        return 0.0;
    }
    if wait_hours > 24.0 * 7.0 {
        return 1.0;
    }

    ((wait_hours - 24.0) / 24.0 * 0.1).clamp(0.0, 1.0)
}

fn promote_model(model: &str) -> String {
    if model.contains("haiku") {
        model.replacen("haiku", "sonnet", 1)
    } else if model.contains("sonnet") {
        model.replacen("sonnet", "opus", 1)
    } else if model.ends_with("-high") {
        model.replace("-high", "-xhigh")
    } else {
        model.to_string()
    }
}

fn demote_model(model: &str) -> String {
    if model.contains("opus") {
        model.replacen("opus", "sonnet", 1)
    } else if model.contains("sonnet") {
        model.replacen("sonnet", "haiku", 1)
    } else if model.ends_with("-xhigh") {
        model.replace("-xhigh", "-high")
    } else {
        model.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_state_path(tmp: &TempDir) -> PathBuf {
        tmp.path().join(".roko").join("daimon").join("affect.json")
    }

    fn strategy(complexity: f64, risk: f64, novelty: f64) -> StrategyCoordinates {
        StrategyCoordinates::new(complexity, risk, novelty, 0.5, 0.5, complexity, 0.5, risk)
    }

    #[test]
    fn appraise_updates_state_and_persists() {
        let tmp = TempDir::new().unwrap_or_else(|err| panic!("tempdir: {err}"));
        let path = temp_state_path(&tmp);
        let mut state = DaimonState::load_or_new(&path);

        let pad = state.appraise(AffectEvent::GateResult {
            plan_id: "plan-a".to_string(),
            task_id: "task-a".to_string(),
            passed: false,
            rung: 2,
        });

        assert!(pad.pleasure < 0.0);
        assert!(pad.arousal > 0.0);
        assert!(pad.dominance < 0.0);
        assert!(matches!(
            state.query().behavioral_state,
            BehavioralState::Exploring | BehavioralState::Struggling
        ));
        assert!(path.exists());

        let reloaded = DaimonState::load_or_new(&path);
        let a = reloaded.query().pad;
        let b = state.query().pad;
        assert!((a.pleasure - b.pleasure).abs() < 1e-10);
        assert!((a.arousal - b.arousal).abs() < 1e-10);
        assert!((a.dominance - b.dominance).abs() < 1e-10);
    }

    #[test]
    fn modulation_escalates_on_negative_state() {
        let mut state = DaimonState::new();
        let _ = state.appraise(AffectEvent::TaskOutcome {
            task_id: "task-a".to_string(),
            succeeded: false,
        });
        let _ = state.appraise(AffectEvent::TaskOutcome {
            task_id: "task-a".to_string(),
            succeeded: false,
        });

        let mut params = DispatchParams::new("claude-haiku-4-5", 20);
        state.modulate(&mut params);

        assert!(params.model.contains("sonnet") || params.model.contains("opus"));
        assert!(params.turn_limit >= 20);
        assert_eq!(params.strategy, DispatchStrategy::Escalating);
        assert_eq!(params.effort, "high");
    }

    #[test]
    fn modulation_demotes_on_positive_state() {
        let mut state = DaimonState::new();
        let _ = state.appraise(AffectEvent::TaskOutcome {
            task_id: "task-a".to_string(),
            succeeded: true,
        });
        let _ = state.appraise(AffectEvent::GateResult {
            plan_id: "plan-a".to_string(),
            task_id: "task-a".to_string(),
            passed: true,
            rung: 3,
        });

        let mut params = DispatchParams::new("claude-sonnet-4-6", 20);
        state.modulate(&mut params);

        assert!(params.model.contains("haiku") || params.model == "claude-sonnet-4-6");
        assert!(params.turn_limit <= 20);
        assert!(matches!(
            params.strategy,
            DispatchStrategy::Exploratory | DispatchStrategy::Balanced
        ));
    }

    #[test]
    fn neutral_state_exposes_engaged_behavior() {
        let state = DaimonState::new();
        assert_eq!(state.query().behavioral_state, BehavioralState::Engaged);
    }

    #[test]
    fn affect_state_builds_emotional_tag() {
        let mut state = DaimonState::new();
        let _ = state.appraise(AffectEvent::TaskOutcome {
            task_id: "task-a".to_string(),
            succeeded: false,
        });

        let tag = state.emotional_tag("task_outcome");

        assert_eq!(tag.trigger, "task_outcome");
        assert!(tag.intensity > 0.0);
        assert_eq!(tag.pad, state.query().pad);
    }

    #[test]
    fn extract_strategy_point_responds_to_task_structure_and_context() {
        let mut task = roko_core::Task::new("task-a", "Refactor a risky cross-crate path");
        task.status = roko_core::TaskStatus::Active;
        task.files = vec![
            "crates/one/src/lib.rs".to_string(),
            "crates/two/src/lib.rs".to_string(),
            "Cargo.toml".to_string(),
        ];
        task.depends_on = vec![
            "task-b".to_string(),
            "task-c".to_string(),
            "task-d".to_string(),
        ];
        task.test_invariants = Some(vec!["compile".to_string(), "tests".to_string()]);
        task.estimated_minutes = Some(180);
        task.complexity_band = Some(roko_core::TaskComplexityBand::Complex);
        task.quality_profile = Some(roko_core::TaskQualityProfile::Hardened);

        let context = TaskContext {
            deadline_proximity: 0.9,
            existing_code_familiarity: 0.1,
            test_coverage: 0.2,
            dag_depth: 0.8,
            model_tier_confidence: 0.85,
        };

        let point = extract_strategy_point(&task, &context);

        assert!(point[0] > 0.8);
        assert!(point[1] > 0.7);
        assert!(point[2] > 0.7);
        assert!(point[3] > 0.8);
        assert!(point[4] > 0.6);
        assert!(point[5] > 0.6);
        assert!(point[6] < 0.6);
        assert!(point[7] > 0.6);
    }

    #[test]
    fn task_strategy_observation_accounts_for_richer_requirements() {
        let mut task = roko_core::Task::new("task-b", "Wire the daemon heartbeat through dreams");
        task.files = vec![
            "crates/roko-dreams/src/runner.rs".to_string(),
            "crates/roko-dreams/src/lib.rs".to_string(),
        ];
        task.depends_on = vec![
            "task-a".to_string(),
            "task-c".to_string(),
            "task-d".to_string(),
        ];
        task.acceptance = vec![
            "add heartbeat report".to_string(),
            "pause delta while active".to_string(),
        ];
        task.test_invariants = Some(vec!["heartbeat".to_string(), "delta-loop".to_string()]);
        task.types_to_define = Some(vec![
            "DreamHeartbeatPolicy".to_string(),
            "DreamHeartbeatReport".to_string(),
        ]);
        task.formulas = Some(vec![
            "delta_due_in = max(0, last_dream + interval - now)".to_string(),
        ]);
        task.imports = Some(vec![
            "chrono::DateTime".to_string(),
            "std::time::Duration".to_string(),
        ]);
        task.context_files = Some(vec![
            "tmp/ux-refactoring/D-architectural-gaps.md".to_string(),
        ]);
        task.sidecar_requirements = Some(vec!["daemon".to_string()]);
        task.integration_surfaces = Some(vec![
            "heartbeat".to_string(),
            "runtime-controls".to_string(),
        ]);
        task.dependency_tags = Some(vec!["dreams".to_string(), "delta".to_string()]);
        task.fixture_keys = Some(vec!["tempdir".to_string()]);
        task.estimated_minutes = Some(240);
        task.complexity_band = Some(roko_core::TaskComplexityBand::Complex);
        task.research_before_edit = Some(true);
        task.exclusive_files = false;
        task.preferred_model = Some("claude-sonnet-4-6".to_string());
        task.escalate_on_retry = Some(true);

        let context = TaskContext::from_task(&task);
        let observation = TaskStrategyObservation::from_task(&task, &context);

        assert!(observation.verification_count >= 6);
        assert!(observation.max_loc >= 600);
        assert!(observation.failure_pressure > 0.6);
        assert!(observation.familiarity < 0.7);
        assert!(context.dag_depth > 0.5);
    }

    #[test]
    fn somatic_landscape_merges_nearby_markers() {
        let mut landscape = SomaticLandscape::new();
        let first = ContentHash::of(b"episode-a");
        let second = ContentHash::of(b"episode-b");

        landscape.record_outcome(strategy(0.7, 0.8, 0.6), -0.7, 0.8, first, Utc::now());
        landscape.record_outcome(strategy(0.72, 0.82, 0.58), -0.5, 0.6, second, Utc::now());

        assert_eq!(landscape.markers.len(), 1);
        assert_eq!(landscape.markers[0].episodes.len(), 2);
        assert!(landscape.markers[0].valence < -0.55);
    }

    #[test]
    fn somatic_summary_reflects_landscape_balance() {
        let mut landscape = SomaticLandscape::new();
        landscape.record_outcome(
            strategy(0.2, 0.2, 0.4),
            0.6,
            0.4,
            ContentHash::of(b"positive"),
            Utc::now(),
        );
        landscape.record_outcome(
            strategy(0.8, 0.7, 0.6),
            -0.7,
            0.9,
            ContentHash::of(b"negative"),
            Utc::now(),
        );

        let summary = landscape.summary();

        assert_eq!(summary.marker_count, 2);
        assert_eq!(summary.positive_markers, 1);
        assert_eq!(summary.negative_markers, 1);
        assert!(summary.mean_intensity > 0.5);
        assert!(summary.strongest_intensity >= 0.9);
        assert!(summary.last_updated_at.is_some());
    }

    #[test]
    fn somatic_query_blends_contrarian_markers() {
        let mut landscape = SomaticLandscape::new();
        landscape.record_outcome(
            strategy(0.8, 0.8, 0.6),
            -0.8,
            0.9,
            ContentHash::of(b"negative-near"),
            Utc::now(),
        );
        landscape.record_outcome(
            strategy(0.82, 0.78, 0.62),
            -0.6,
            0.8,
            ContentHash::of(b"negative-near-2"),
            Utc::now(),
        );
        landscape.record_outcome(
            strategy(0.79, 0.81, 0.59),
            0.9,
            0.7,
            ContentHash::of(b"positive-contrarian"),
            Utc::now(),
        );

        let signal = landscape.query(strategy(0.8, 0.8, 0.6), 5);

        assert!(signal.valence < 0.0);
        assert!(signal.valence > -0.8);
        assert_eq!(signal.contrarian_count, 1);
        assert_eq!(signal.source_episodes.len(), 3);
    }

    #[test]
    fn modulate_with_strategy_uses_negative_somatic_signal() {
        let mut state = DaimonState::new();
        state.somatic_landscape.record_outcome(
            strategy(0.8, 0.8, 0.7),
            -0.9,
            0.9,
            ContentHash::of(b"negative-marker"),
            Utc::now(),
        );

        let mut params = DispatchParams::new("claude-haiku-4-5", 20);
        state.modulate_with_strategy(&mut params, strategy(0.8, 0.8, 0.7));

        assert!(params.model.contains("sonnet") || params.model.contains("opus"));
        assert!(params.turn_limit >= 26);
        assert_eq!(params.strategy, DispatchStrategy::Conservative);
    }

    #[test]
    fn load_or_new_rebuilds_somatic_index() {
        let tmp = TempDir::new().unwrap_or_else(|err| panic!("tempdir: {err}"));
        let path = temp_state_path(&tmp);
        let mut state = DaimonState::new().with_persistence_path(&path);
        state.somatic_landscape.record_outcome(
            strategy(0.3, 0.2, 0.4),
            0.7,
            0.6,
            ContentHash::of(b"persisted-marker"),
            Utc::now(),
        );
        state.persist(&path).expect("persist daimon");

        let reloaded = DaimonState::load_or_new(&path);
        let signal = reloaded.query_somatic(strategy(0.3, 0.2, 0.4));

        assert_eq!(reloaded.somatic_landscape.markers.len(), 1);
        assert!(signal.valence > 0.5);
        assert!(signal.intensity > 0.5);
    }

    #[test]
    fn dream_depotentiation_cools_arousal_and_high_intensity_markers() {
        let mut state = DaimonState::new();
        state.state.pad.arousal = 0.9;
        state.somatic_landscape.record_outcome(
            strategy(0.8, 0.7, 0.6),
            -0.7,
            0.8,
            ContentHash::of(b"charged-marker"),
            Utc::now(),
        );
        state.somatic_landscape.record_outcome(
            strategy(0.2, 0.3, 0.4),
            0.4,
            0.4,
            ContentHash::of(b"stable-marker"),
            Utc::now(),
        );

        let report = state.apply_dream_depotentiation();

        assert!(report.post_arousal < report.pre_arousal);
        assert_eq!(report.cooled_markers, 1);
        assert!(report.total_marker_intensity_reduction > 0.0);
        assert!(state.somatic_landscape.markers[0].intensity < 0.8);
        assert_eq!(state.somatic_landscape.markers[1].intensity, 0.4);
    }

    #[test]
    fn strong_somatic_signal_emits_runtime_threshold() {
        let strong = SomaticSignal {
            valence: -0.6,
            intensity: 0.7,
            neighbor_count: 2,
            contrarian_count: 1,
            source_episodes: vec![ContentHash::of(b"episode")],
        };
        let weak = SomaticSignal {
            valence: -0.2,
            intensity: 0.7,
            neighbor_count: 1,
            contrarian_count: 0,
            source_episodes: Vec::new(),
        };

        assert!(strong.should_emit_event());
        assert!(!weak.should_emit_event());
    }

    #[test]
    fn configure_strategy_space_clears_incompatible_markers() {
        let mut state = DaimonState::new();
        state.somatic_landscape.record_outcome(
            strategy(0.3, 0.2, 0.4),
            0.6,
            0.7,
            ContentHash::of(b"marker"),
            Utc::now(),
        );

        state.configure_strategy_space(
            StrategySpaceDefinition {
                domain: "chain".to_string(),
                dimensions: [
                    "volatility".to_string(),
                    "liquidity".to_string(),
                    "correlation".to_string(),
                    "leverage".to_string(),
                    "time_horizon".to_string(),
                    "concentration".to_string(),
                    "counterparty_risk".to_string(),
                    "regulatory_exposure".to_string(),
                ],
            }
            .validate()
            .unwrap(),
        );

        assert_eq!(state.strategy_space.domain, "chain");
        assert!(state.somatic_landscape.markers.is_empty());
    }

    #[test]
    fn strategy_space_validation_rejects_duplicates() {
        let err = StrategySpaceDefinition {
            domain: "coding".to_string(),
            dimensions: [
                "risk".to_string(),
                "risk".to_string(),
                "novelty".to_string(),
                "confidence".to_string(),
                "time_pressure".to_string(),
                "scope".to_string(),
                "reversibility".to_string(),
                "dependency_depth".to_string(),
            ],
        }
        .validate()
        .unwrap_err();

        assert!(err.to_string().contains("must be unique"));
    }

    #[test]
    fn coding_strategy_space_projects_task_observations() {
        let coords =
            StrategySpaceDefinition::coding()
                .computer()
                .task_coords(&TaskStrategyObservation {
                    task_tier: "architectural".to_string(),
                    file_count: 6,
                    verification_count: 3,
                    dependency_count: 4,
                    max_loc: 320,
                    familiarity: 0.2,
                    confidence: 0.65,
                    failure_pressure: 0.6,
                    urgency_pressure: 1.0,
                });

        assert!(coords.complexity > 0.8);
        assert!(coords.risk > 0.7);
        assert!(coords.scope > 0.7);
        assert!(coords.reversibility < 0.4);
        assert!(coords.time_pressure > 0.7);
        assert_eq!(coords.confidence, 0.65);
    }

    #[test]
    fn registered_strategy_space_preserves_custom_domain_metadata() {
        let definition = StrategySpaceDefinition {
            domain: "chain".to_string(),
            dimensions: [
                "volatility".to_string(),
                "exposure".to_string(),
                "correlation".to_string(),
                "confidence".to_string(),
                "time_horizon".to_string(),
                "concentration".to_string(),
                "counterparty_risk".to_string(),
                "regulatory_exposure".to_string(),
            ],
        }
        .validate()
        .unwrap();
        let computer = definition.computer();
        let coords = computer.task_coords(&TaskStrategyObservation {
            task_tier: "focused".to_string(),
            file_count: 2,
            verification_count: 1,
            dependency_count: 1,
            max_loc: 80,
            familiarity: 0.6,
            confidence: 0.55,
            failure_pressure: 0.1,
            urgency_pressure: 0.2,
        });

        assert_eq!(computer.definition().domain, "chain");
        assert!(!computer.is_builtin_coding());
        assert!((0.0..=1.0).contains(&coords.complexity));
        assert_eq!(computer.definition().labels()[0], "volatility");
    }

    #[test]
    fn registered_strategy_space_reorders_non_coding_dimensions_by_role() {
        let observation = TaskStrategyObservation {
            task_tier: "architectural".to_string(),
            file_count: 6,
            verification_count: 3,
            dependency_count: 4,
            max_loc: 320,
            familiarity: 0.2,
            confidence: 0.65,
            failure_pressure: 0.6,
            urgency_pressure: 1.0,
        };
        let baseline = StrategySpaceDefinition::coding()
            .computer()
            .task_coords(&observation);
        let custom = StrategySpaceDefinition {
            domain: "research".to_string(),
            dimensions: [
                "confidence".to_string(),
                "time_horizon".to_string(),
                "complexity".to_string(),
                "risk".to_string(),
                "novelty".to_string(),
                "scope".to_string(),
                "reversibility".to_string(),
                "dependency_depth".to_string(),
            ],
        }
        .validate()
        .unwrap()
        .computer()
        .task_coords(&observation);

        assert_eq!(custom.complexity, baseline.confidence);
        assert_eq!(custom.risk, baseline.time_pressure);
        assert_eq!(custom.novelty, baseline.complexity);
        assert_eq!(custom.confidence, baseline.risk);
        assert_eq!(custom.time_pressure, baseline.novelty);
        assert_eq!(custom.scope, baseline.scope);
        assert_eq!(custom.reversibility, baseline.reversibility);
        assert_eq!(custom.dependency_depth, baseline.dependency_depth);
    }
}
