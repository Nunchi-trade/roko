//! Daimon subsystem scaffold and affect engine.

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine};

/// Normalized PAD affect state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffectState {
    /// Pleasure dimension in `[-1.0, 1.0]`.
    /// Success pushes this positive; failure pushes it negative.
    pub pleasure: f64,
    /// Arousal dimension in `[-1.0, 1.0]`.
    /// Time pressure and urgency push this positive; idle pushes it negative.
    pub arousal: f64,
    /// Dominance dimension in `[-1.0, 1.0]`.
    /// Agency and control push this positive; blocked or stuck pushes it negative.
    pub dominance: f64,
    /// Last time this affect state was updated.
    pub updated_at: DateTime<Utc>,
}

impl Default for AffectState {
    fn default() -> Self {
        Self {
            pleasure: 0.0,
            arousal: 0.0,
            dominance: 0.0,
            updated_at: Utc::now(),
        }
    }
}

impl AffectState {
    /// Construct a neutral affect state at the provided timestamp.
    #[must_use]
    pub fn neutral(updated_at: DateTime<Utc>) -> Self {
        Self {
            updated_at,
            ..Self::default()
        }
    }

    fn apply_delta(&mut self, pleasure: f64, arousal: f64, dominance: f64) {
        self.pleasure = (self.pleasure + pleasure).clamp(-1.0, 1.0);
        self.arousal = (self.arousal + arousal).clamp(-1.0, 1.0);
        self.dominance = (self.dominance + dominance).clamp(-1.0, 1.0);
        self.updated_at = Utc::now();
    }

    fn decay_by_factor(&mut self, factor: f64) {
        self.pleasure = (self.pleasure * factor).clamp(-1.0, 1.0);
        self.arousal = (self.arousal * factor).clamp(-1.0, 1.0);
        self.dominance = (self.dominance * factor).clamp(-1.0, 1.0);
        self.updated_at = Utc::now();
    }

    /// Resolve this PAD vector to its named octant.
    #[must_use]
    pub const fn octant(&self) -> AffectOctant {
        AffectOctant::from_pad(self.pleasure, self.arousal, self.dominance)
    }

    /// Human-readable label for logging and dashboards.
    #[must_use]
    pub const fn octant_label(&self) -> &'static str {
        self.octant().label()
    }

    /// Derive the behavioral modulation table entry for the current state.
    #[must_use]
    pub const fn behavior_modulation(&self) -> AffectBehaviorModulation {
        self.octant().behavior_modulation()
    }
}

/// Stateful affect engine keyed by task or agent id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AffectEngine {
    /// Per-task PAD state.
    #[serde(default)]
    states: HashMap<String, AffectState>,
    /// Half-life in hours for explicit decay operations.
    #[serde(default = "default_half_life_hours")]
    half_life_hours: f64,
}

fn default_half_life_hours() -> f64 {
    4.0
}

impl Default for AffectEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AffectEngine {
    /// Construct a new affect engine with the default 4 hour half-life.
    #[must_use]
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            half_life_hours: default_half_life_hours(),
        }
    }

    /// Construct an affect engine with a custom decay half-life.
    #[must_use]
    pub fn with_half_life_hours(half_life_hours: f64) -> Self {
        Self {
            states: HashMap::new(),
            half_life_hours,
        }
    }

    /// Return the configured decay half-life in hours.
    #[must_use]
    pub const fn half_life_hours(&self) -> f64 {
        self.half_life_hours
    }

    /// Returns the current state for a task or agent.
    #[must_use]
    pub fn get_state(&mut self, task_id: impl AsRef<str>) -> AffectState {
        let task_id = task_id.as_ref();
        let now = Utc::now();
        let state = self
            .states
            .entry(task_id.to_owned())
            .or_insert_with(|| AffectState::neutral(now));

        let elapsed_hours = now
            .signed_duration_since(state.updated_at)
            .num_seconds() as f64
            / 3600.0;
        if elapsed_hours > 0.0 {
            decay_state(state, elapsed_hours, self.half_life_hours);
        }

        state.clone()
    }

    /// Appraisal trigger: task succeeded.
    #[must_use]
    pub fn on_task_success(&mut self, task_id: impl Into<String>) -> AffectState {
        self.adjust(task_id.into(), 0.1, 0.0, 0.1)
    }

    /// Appraisal trigger: task failed.
    #[must_use]
    pub fn on_task_failure(&mut self, task_id: impl Into<String>) -> AffectState {
        self.adjust(task_id.into(), -0.2, 0.0, -0.15)
    }

    /// Appraisal trigger: gate passed.
    #[must_use]
    pub fn on_gate_pass(&mut self, task_id: impl Into<String>) -> AffectState {
        self.adjust(task_id.into(), 0.05, 0.0, 0.0)
    }

    /// Appraisal trigger: gate failed.
    #[must_use]
    pub fn on_gate_fail(&mut self, task_id: impl Into<String>) -> AffectState {
        self.adjust(task_id.into(), -0.1, 0.0, -0.05)
    }

    /// Appraisal trigger: deadline proximity raises arousal as deadline approaches.
    #[must_use]
    pub fn on_time_pressure(
        &mut self,
        task_id: impl Into<String>,
        deadline_proximity: f64,
    ) -> AffectState {
        let proximity = deadline_proximity.clamp(0.0, 1.0);
        self.adjust(task_id.into(), 0.0, proximity * 0.4, 0.0)
    }

    /// Appraisal trigger: blocked work raises arousal and lowers dominance.
    #[must_use]
    pub fn on_blocked(&mut self, task_id: impl Into<String>, blocker_count: usize) -> AffectState {
        let blockers = blocker_count.max(1).min(5) as f64;
        self.adjust(task_id.into(), 0.0, blockers * 0.05, -(blockers * 0.08))
    }

    /// Queue-wait motivation signal.
    ///
    /// Returns a positive arousal bump for work that has sat in a queue
    /// without being executed. The bump starts after 24 hours and grows by
    /// `+0.1` per day beyond that. Once work is older than 7 days, the
    /// signal saturates at a very high arousal level to mean "do this or
    /// drop it".
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

    /// Apply explicit decay to every tracked state.
    pub fn decay(&mut self, delta_hours: f64) {
        let factor = decay_factor(delta_hours, self.half_life_hours);
        if factor == 1.0 {
            return;
        }

        for state in self.states.values_mut() {
            state.decay_by_factor(factor);
        }
    }

    fn adjust(
        &mut self,
        task_id: String,
        pleasure: f64,
        arousal: f64,
        dominance: f64,
    ) -> AffectState {
        let now = Utc::now();
        let state = self
            .states
            .entry(task_id)
            .or_insert_with(|| AffectState::neutral(now));

        let elapsed_hours = now
            .signed_duration_since(state.updated_at)
            .num_seconds() as f64
            / 3600.0;
        if elapsed_hours > 0.0 {
            decay_state(state, elapsed_hours, self.half_life_hours);
        }

        state.apply_delta(pleasure, arousal, dominance);
        state.clone()
    }

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

fn decay_state(state: &mut AffectState, delta_hours: f64, half_life_hours: f64) {
    let factor = decay_factor(delta_hours, half_life_hours);
    if factor != 1.0 {
        state.decay_by_factor(factor);
    }
}

/// Named PAD octant for logging and dashboard display.
///
/// The octants correspond to the sign of the PAD vector:
/// - `+P+A+D` => `Excited`
/// - `+P+A-D` => `Surprised`
/// - `+P-A+D` => `Confident`
/// - `+P-A-D` => `Relaxed`
/// - `-P+A+D` => `Angry`
/// - `-P+A-D` => `Anxious`
/// - `-P-A+D` => `Bored`
/// - `-P-A-D` => `Depressed`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AffectOctant {
    /// Succeeding under pressure.
    Excited,
    /// Unexpected success.
    Surprised,
    /// Calm, in control, succeeding.
    Confident,
    /// Nothing urgent, things are fine.
    Relaxed,
    /// Frustrated but still trying.
    Angry,
    /// Failing, pressured, no control.
    Anxious,
    /// Nothing happening, agent idle.
    Bored,
    /// Repeated failures, no agency.
    Depressed,
}

impl AffectOctant {
    /// Human-readable label for this octant.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Excited => "Excited",
            Self::Surprised => "Surprised",
            Self::Confident => "Confident",
            Self::Relaxed => "Relaxed",
            Self::Angry => "Angry",
            Self::Anxious => "Anxious",
            Self::Bored => "Bored",
            Self::Depressed => "Depressed",
        }
    }

    /// Resolve a PAD vector to its octant.
    ///
    /// Exact zero vectors are treated as `Relaxed` so the neutral dashboard
    /// state stays readable instead of collapsing to an arbitrary octant.
    #[must_use]
    pub const fn from_pad(pleasure: f64, arousal: f64, dominance: f64) -> Self {
        if pleasure == 0.0 && arousal == 0.0 && dominance == 0.0 {
            return Self::Relaxed;
        }

        let positive_pleasure = !pleasure.is_sign_negative();
        let positive_arousal = !arousal.is_sign_negative();
        let positive_dominance = !dominance.is_sign_negative();

        match (positive_pleasure, positive_arousal, positive_dominance) {
            (true, true, true) => Self::Excited,
            (true, true, false) => Self::Surprised,
            (true, false, true) => Self::Confident,
            (true, false, false) => Self::Relaxed,
            (false, true, true) => Self::Angry,
            (false, true, false) => Self::Anxious,
            (false, false, true) => Self::Bored,
            (false, false, false) => Self::Depressed,
        }
    }

    /// Translate this affect state into behavioral parameters.
    #[must_use]
    pub const fn behavior_modulation(self) -> AffectBehaviorModulation {
        match self {
            Self::Anxious => AffectBehaviorModulation::anxious(),
            Self::Confident => AffectBehaviorModulation::confident(),
            Self::Angry => AffectBehaviorModulation::angry(),
            Self::Bored => AffectBehaviorModulation::bored(),
            _ => AffectBehaviorModulation::balanced(),
        }
    }
}

/// High-level behavior mode selected by affect state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AffectBehaviorStrategy {
    /// Stick to proven playbooks and limit exploration.
    Conservative,
    /// Default mixed mode.
    Balanced,
    /// Bias toward novel approaches and broader search.
    Exploratory,
    /// Escalate capability and persist longer before giving up.
    Escalating,
    /// Run maintenance and background cognitive tasks.
    Proactive,
}

/// Behavioral parameters modulated by the current affect state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AffectBehaviorModulation {
    /// Coarse strategy preference.
    pub strategy: AffectBehaviorStrategy,
    /// Exploration rate in `[0.0, 1.0]`.
    pub exploration_rate: f64,
    /// Whether to prefer proven playbooks over novel approaches.
    pub prefer_proven_playbooks: bool,
    /// How many model-tier steps to escalate when selecting a model.
    pub model_tier_escalation: u8,
    /// Additional retry attempts before giving up.
    pub extra_retries: u32,
    /// Whether to trigger dream cycles proactively.
    pub trigger_dream_cycles: bool,
    /// Whether to run maintenance tasks proactively.
    pub run_maintenance_tasks: bool,
}

impl AffectBehaviorModulation {
    const fn balanced() -> Self {
        Self {
            strategy: AffectBehaviorStrategy::Balanced,
            exploration_rate: 0.20,
            prefer_proven_playbooks: true,
            model_tier_escalation: 0,
            extra_retries: 0,
            trigger_dream_cycles: false,
            run_maintenance_tasks: false,
        }
    }

    const fn anxious() -> Self {
        Self {
            strategy: AffectBehaviorStrategy::Conservative,
            exploration_rate: 0.05,
            prefer_proven_playbooks: true,
            model_tier_escalation: 0,
            extra_retries: 0,
            trigger_dream_cycles: false,
            run_maintenance_tasks: false,
        }
    }

    const fn confident() -> Self {
        Self {
            strategy: AffectBehaviorStrategy::Exploratory,
            exploration_rate: 0.35,
            prefer_proven_playbooks: false,
            model_tier_escalation: 0,
            extra_retries: 0,
            trigger_dream_cycles: false,
            run_maintenance_tasks: false,
        }
    }

    const fn angry() -> Self {
        Self {
            strategy: AffectBehaviorStrategy::Escalating,
            exploration_rate: 0.10,
            prefer_proven_playbooks: true,
            model_tier_escalation: 1,
            extra_retries: 2,
            trigger_dream_cycles: false,
            run_maintenance_tasks: false,
        }
    }

    const fn bored() -> Self {
        Self {
            strategy: AffectBehaviorStrategy::Proactive,
            exploration_rate: 0.25,
            prefer_proven_playbooks: true,
            model_tier_escalation: 0,
            extra_retries: 0,
            trigger_dream_cycles: true,
            run_maintenance_tasks: true,
        }
    }
}

impl fmt::Display for AffectOctant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Placeholder daimon engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DaimonEngine;

impl DaimonEngine {
    /// Stable subsystem id.
    pub const ID: GolemSubsystemId = GolemSubsystemId::Daimon;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Daimon";
    /// Static scaffold marker string.
    pub const MARKER: &'static str = "roko-golem scaffold: daimon";

    /// Construct a placeholder daimon engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Summary metadata for this scaffold subsystem.
    #[must_use]
    pub const fn summary(self) -> GolemSubsystemSummary {
        GolemSubsystemSummary::new(Self::ID, Self::LABEL, Self::MARKER)
    }

    /// Returns a static marker describing scaffold behavior.
    #[must_use]
    pub const fn evaluate(self) -> &'static str {
        Self::MARKER
    }
}

impl ScaffoldEngine for DaimonEngine {
    fn summary(self) -> GolemSubsystemSummary {
        self.summary()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AffectBehaviorStrategy, AffectEngine, AffectOctant, AffectState,
    };
    use chrono::Utc;

    #[test]
    fn maps_all_pad_octants_to_named_states() {
        let now = Utc::now();

        let cases = [
            (1.0, 1.0, 1.0, AffectOctant::Excited),
            (1.0, 1.0, -1.0, AffectOctant::Surprised),
            (1.0, -1.0, 1.0, AffectOctant::Confident),
            (1.0, -1.0, -1.0, AffectOctant::Relaxed),
            (-1.0, 1.0, 1.0, AffectOctant::Angry),
            (-1.0, 1.0, -1.0, AffectOctant::Anxious),
            (-1.0, -1.0, 1.0, AffectOctant::Bored),
            (-1.0, -1.0, -1.0, AffectOctant::Depressed),
        ];

        for (pleasure, arousal, dominance, expected) in cases {
            let state = AffectState {
                pleasure,
                arousal,
                dominance,
                updated_at: now,
            };

            assert_eq!(state.octant(), expected);
            assert_eq!(state.octant_label(), expected.label());
            assert_eq!(expected.to_string(), expected.label());
        }
    }

    #[test]
    fn neutral_vector_defaults_to_relaxed() {
        let state = AffectState {
            pleasure: 0.0,
            arousal: 0.0,
            dominance: 0.0,
            updated_at: Utc::now(),
        };

        assert_eq!(state.octant(), AffectOctant::Relaxed);
    }

    #[test]
    fn appraisal_triggers_update_task_state() {
        let mut engine = AffectEngine::new();

        let state = engine.on_task_success("task-1");
        assert_eq!(state.pleasure, 0.1);
        assert_eq!(state.dominance, 0.1);

        let state = engine.on_task_failure("task-1");
        assert!((state.pleasure - -0.1).abs() < f64::EPSILON);
        assert!((state.dominance - -0.05).abs() < f64::EPSILON);

        let state = engine.on_gate_pass("task-1");
        assert!((state.pleasure - -0.05).abs() < f64::EPSILON);

        let state = engine.on_gate_fail("task-1");
        assert!((state.pleasure - -0.15).abs() < f64::EPSILON);
        assert!((state.dominance - -0.10).abs() < f64::EPSILON);
    }

    #[test]
    fn pressure_and_blockage_raise_arousal() {
        let mut engine = AffectEngine::new();

        let low_pressure = engine.on_time_pressure("task-2", 0.2);
        let high_pressure = engine.on_time_pressure("task-3", 0.9);
        assert!(high_pressure.arousal > low_pressure.arousal);

        let light_blocked = engine.on_blocked("task-4", 1);
        let heavy_blocked = engine.on_blocked("task-5", 5);
        assert!(heavy_blocked.arousal > light_blocked.arousal);
        assert!(heavy_blocked.dominance < light_blocked.dominance);
    }

    #[test]
    fn behavior_modulation_matches_named_affect_states() {
        let anxious = AffectOctant::Anxious.behavior_modulation();
        assert_eq!(anxious.strategy, AffectBehaviorStrategy::Conservative);
        assert!(anxious.prefer_proven_playbooks);
        assert!(anxious.exploration_rate < 0.1);
        assert_eq!(anxious.model_tier_escalation, 0);
        assert_eq!(anxious.extra_retries, 0);
        assert!(!anxious.trigger_dream_cycles);
        assert!(!anxious.run_maintenance_tasks);

        let confident = AffectOctant::Confident.behavior_modulation();
        assert_eq!(confident.strategy, AffectBehaviorStrategy::Exploratory);
        assert!(!confident.prefer_proven_playbooks);
        assert!(confident.exploration_rate > anxious.exploration_rate);

        let angry = AffectOctant::Angry.behavior_modulation();
        assert_eq!(angry.strategy, AffectBehaviorStrategy::Escalating);
        assert_eq!(angry.model_tier_escalation, 1);
        assert!(angry.extra_retries >= 2);

        let bored = AffectOctant::Bored.behavior_modulation();
        assert_eq!(bored.strategy, AffectBehaviorStrategy::Proactive);
        assert!(bored.trigger_dream_cycles);
        assert!(bored.run_maintenance_tasks);
    }

    #[test]
    fn decay_halves_state_at_half_life() {
        let mut engine = AffectEngine::with_half_life_hours(4.0);
        engine.on_task_success("task-6");
        engine.on_time_pressure("task-6", 1.0);

        engine.decay(4.0);
        let state = engine.get_state("task-6");

        assert!((state.pleasure - 0.05).abs() < 1e-9);
        assert!(state.arousal > 0.0 && state.arousal < 0.2);
        assert!((state.dominance - 0.05).abs() < 1e-9);
    }

    #[test]
    fn get_state_creates_neutral_default() {
        let mut engine = AffectEngine::new();
        let state = engine.get_state("missing-task");

        assert_eq!(state.pleasure, 0.0);
        assert_eq!(state.arousal, 0.0);
        assert_eq!(state.dominance, 0.0);
    }

    #[test]
    fn queue_wait_arousal_stays_idle_for_fresh_work() {
        assert_eq!(AffectEngine::queue_wait_arousal(12.0), 0.0);
        assert_eq!(AffectEngine::queue_wait_arousal(24.0), 0.0);
    }

    #[test]
    fn queue_wait_arousal_grows_after_one_day() {
        let bump = AffectEngine::queue_wait_arousal(48.0);
        assert!((bump - 0.1).abs() < 1e-9);

        let bigger_bump = AffectEngine::queue_wait_arousal(72.0);
        assert!(bigger_bump > bump);
    }

    #[test]
    fn queue_wait_arousal_saturates_for_very_stale_work() {
        assert_eq!(AffectEngine::queue_wait_arousal(24.0 * 8.0), 1.0);
    }
}
