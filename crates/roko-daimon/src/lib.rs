//! Daimon affect state, somatic markers, and dispatch modulation.
//!
//! This crate provides a standalone affect engine for Roko's plan runner.
//! It owns the current PAD state, appraises task events into that state,
//! stores situation-specific somatic markers, and modulates dispatch
//! parameters for future task runs.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use kiddo::{KdTree, SquaredEuclidean};
use roko_core::{BehavioralState, ContentHash, EmotionalTag, OperatingFrequencyAffect, PadVector};
use serde::{Deserialize, Serialize};

const STRATEGY_DIMENSIONS: usize = 8;
const DEFAULT_SOMATIC_NEIGHBORS: usize = 5;
const CONTRARIAN_FRACTION: f64 = 0.15;
const SOMATIC_MERGE_DISTANCE_SQUARED: f64 = 0.25;

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
}
