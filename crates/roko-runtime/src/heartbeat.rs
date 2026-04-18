//! Runtime heartbeat policy, cognitive clock state, and tier-gating records.
//!
//! The heartbeat module is intentionally generic: it publishes typed tick
//! events on the runtime bus and exposes small data structures that higher
//! layers can fill with domain-specific cognition.

use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::{AtomicI8, AtomicU8, AtomicU16, AtomicU32, AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::time::MissedTickBehavior;

use crate::{
    cancel::CancelToken,
    event_bus::{BusSender, RokoEvent},
};

/// Bus topic emitted for fast reactive heartbeat ticks.
pub const HEARTBEAT_GAMMA_TICK: &str = "heartbeat.gamma.tick";

/// Bus topic emitted for medium reflective heartbeat ticks.
pub const HEARTBEAT_THETA_TICK: &str = "heartbeat.theta.tick";

/// Bus topic emitted for slow consolidation heartbeat ticks.
pub const HEARTBEAT_DELTA_TICK: &str = "heartbeat.delta.tick";

/// Cognitive speed handled by the heartbeat clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeartbeatSpeed {
    /// Reactive perception and action cadence.
    Gamma,
    /// Reflective planning and calibration cadence.
    Theta,
    /// Offline consolidation cadence.
    Delta,
}

impl HeartbeatSpeed {
    /// Return the canonical bus topic for this speed.
    pub const fn topic(self) -> &'static str {
        match self {
            Self::Gamma => HEARTBEAT_GAMMA_TICK,
            Self::Theta => HEARTBEAT_THETA_TICK,
            Self::Delta => HEARTBEAT_DELTA_TICK,
        }
    }
}

/// Cognitive inference tier selected for a heartbeat tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceTier {
    /// Deterministic probes and playbook rules only; no LLM call.
    T0,
    /// Fast model with reduced context for triage.
    T1,
    /// Full model with complete context for deep deliberation.
    T2,
}

impl From<InferenceTier> for u8 {
    fn from(value: InferenceTier) -> Self {
        match value {
            InferenceTier::T0 => 0,
            InferenceTier::T1 => 1,
            InferenceTier::T2 => 2,
        }
    }
}

/// Environmental regime used by the adaptive clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Regime {
    /// Low prediction error and stable external conditions.
    Calm,
    /// Expected day-to-day variation.
    Normal,
    /// Elevated volatility or repeated anomalies.
    Volatile,
    /// Critical conditions requiring near-continuous attention.
    Crisis,
}

impl Regime {
    /// Convert a compact atomic representation into a regime.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Calm,
            1 => Self::Normal,
            2 => Self::Volatile,
            _ => Self::Crisis,
        }
    }
}

impl From<Regime> for u8 {
    fn from(value: Regime) -> Self {
        match value {
            Regime::Calm => 0,
            Regime::Normal => 1,
            Regime::Volatile => 2,
            Regime::Crisis => 3,
        }
    }
}

/// PAD affect vector used by tier gating and attention allocation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PadVector {
    /// Pleasure axis in `[-1.0, 1.0]`.
    pub pleasure: f32,
    /// Arousal axis in `[-1.0, 1.0]`.
    pub arousal: f32,
    /// Dominance axis in `[-1.0, 1.0]`.
    pub dominance: f32,
}

impl PadVector {
    /// Neutral PAD vector.
    pub const fn neutral() -> Self {
        Self {
            pleasure: 0.0,
            arousal: 0.0,
            dominance: 0.0,
        }
    }
}

/// Personality preset used to initialize [`CorticalState`] affect signals.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalityPreset {
    /// Cautious startup: lower dominance and slightly elevated arousal.
    Cautious,
    /// Balanced startup with neutral PAD.
    Balanced,
    /// Aggressive startup: higher arousal and dominance.
    Aggressive,
    /// Explicit PAD values supplied by configuration.
    Custom(PadVector),
}

impl PersonalityPreset {
    /// Return the initial PAD vector for this preset.
    pub const fn pad(self) -> PadVector {
        match self {
            Self::Cautious => PadVector {
                pleasure: -0.1,
                arousal: 0.1,
                dominance: -0.2,
            },
            Self::Balanced => PadVector::neutral(),
            Self::Aggressive => PadVector {
                pleasure: 0.1,
                arousal: 0.3,
                dominance: 0.2,
            },
            Self::Custom(pad) => pad,
        }
    }
}

/// Behavioral state derived from PAD and recent outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BehavioralState {
    /// Balanced exploration and exploitation.
    Engaged,
    /// Repeated failures or low confidence require caution.
    Struggling,
    /// Success with low arousal may hide complacency.
    Coasting,
    /// Actively searching and tolerating uncertainty.
    Exploring,
    /// Deep-work state with high arousal and dominance.
    Focused,
    /// Low-arousal pre-consolidation state.
    Resting,
}

impl BehavioralState {
    /// Convert a compact atomic representation into a behavioral state.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Engaged,
            1 => Self::Struggling,
            2 => Self::Coasting,
            3 => Self::Exploring,
            4 => Self::Focused,
            _ => Self::Resting,
        }
    }
}

impl From<BehavioralState> for u8 {
    fn from(value: BehavioralState) -> Self {
        match value {
            BehavioralState::Engaged => 0,
            BehavioralState::Struggling => 1,
            BehavioralState::Coasting => 2,
            BehavioralState::Exploring => 3,
            BehavioralState::Focused => 4,
            BehavioralState::Resting => 5,
        }
    }
}

/// Primary emotion label stored in the shared perception surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlutchikLabel {
    /// Joy-like positive valence.
    Joy,
    /// Trust-like confidence signal.
    Trust,
    /// Fear-like threat signal.
    Fear,
    /// Surprise-like novelty signal.
    Surprise,
    /// Sadness-like negative valence.
    Sadness,
    /// Disgust-like rejection signal.
    Disgust,
    /// Anger-like blocked-goal signal.
    Anger,
    /// Anticipation-like opportunity signal.
    Anticipation,
}

impl PlutchikLabel {
    /// Convert a compact atomic representation into an emotion label.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Joy,
            1 => Self::Trust,
            2 => Self::Fear,
            3 => Self::Surprise,
            4 => Self::Sadness,
            5 => Self::Disgust,
            6 => Self::Anger,
            _ => Self::Anticipation,
        }
    }
}

impl From<PlutchikLabel> for u8 {
    fn from(value: PlutchikLabel) -> Self {
        match value {
            PlutchikLabel::Joy => 0,
            PlutchikLabel::Trust => 1,
            PlutchikLabel::Fear => 2,
            PlutchikLabel::Surprise => 3,
            PlutchikLabel::Sadness => 4,
            PlutchikLabel::Disgust => 5,
            PlutchikLabel::Anger => 6,
            PlutchikLabel::Anticipation => 7,
        }
    }
}

/// Lock-free shared perception surface for heartbeat subsystems.
#[repr(C, align(64))]
pub struct CorticalState {
    pleasure: AtomicU32,
    arousal: AtomicU32,
    dominance: AtomicU32,
    primary_emotion: AtomicU8,
    aggregate_accuracy: AtomicU32,
    accuracy_trend: AtomicI8,
    category_accuracies: [AtomicU32; 16],
    surprise_rate: AtomicU32,
    universe_size: AtomicU32,
    active_count: AtomicU16,
    pending_predictions: AtomicU32,
    creative_mode: AtomicU8,
    fragments_captured: AtomicU32,
    last_novel_prediction_tick: AtomicU64,
    regime: AtomicU8,
    gas_gwei: AtomicU32,
    resource_health: AtomicU32,
    knowledge_health: AtomicU32,
    performance_trend: AtomicU32,
    behavioral_state: AtomicU8,
    compounding_momentum: AtomicU32,
}

impl CorticalState {
    /// Initialize all signals to neutral defaults for a personality preset.
    pub fn new(personality: PersonalityPreset) -> Self {
        let pad = personality.pad();
        Self {
            pleasure: AtomicU32::new(pad.pleasure.to_bits()),
            arousal: AtomicU32::new(pad.arousal.to_bits()),
            dominance: AtomicU32::new(pad.dominance.to_bits()),
            primary_emotion: AtomicU8::new(u8::from(PlutchikLabel::Joy)),
            aggregate_accuracy: AtomicU32::new(0.5f32.to_bits()),
            accuracy_trend: AtomicI8::new(0),
            category_accuracies: std::array::from_fn(|_| AtomicU32::new(0.5f32.to_bits())),
            surprise_rate: AtomicU32::new(0.0f32.to_bits()),
            universe_size: AtomicU32::new(0),
            active_count: AtomicU16::new(0),
            pending_predictions: AtomicU32::new(0),
            creative_mode: AtomicU8::new(0),
            fragments_captured: AtomicU32::new(0),
            last_novel_prediction_tick: AtomicU64::new(0),
            regime: AtomicU8::new(u8::from(Regime::Calm)),
            gas_gwei: AtomicU32::new(0.0f32.to_bits()),
            resource_health: AtomicU32::new(1.0f32.to_bits()),
            knowledge_health: AtomicU32::new(0.5f32.to_bits()),
            performance_trend: AtomicU32::new(0.0f32.to_bits()),
            behavioral_state: AtomicU8::new(u8::from(BehavioralState::Engaged)),
            compounding_momentum: AtomicU32::new(0.0f32.to_bits()),
        }
    }

    /// Read the current PAD vector.
    pub fn pad(&self) -> PadVector {
        PadVector {
            pleasure: load_f32(&self.pleasure),
            arousal: load_f32(&self.arousal),
            dominance: load_f32(&self.dominance),
        }
    }

    /// Write the current PAD vector.
    pub fn set_pad(&self, pad: PadVector) {
        store_f32(&self.pleasure, pad.pleasure);
        store_f32(&self.arousal, pad.arousal);
        store_f32(&self.dominance, pad.dominance);
    }

    /// Read current prediction accuracy.
    pub fn prediction_accuracy(&self) -> f32 {
        load_f32(&self.aggregate_accuracy)
    }

    /// Write current prediction accuracy.
    pub fn set_prediction_accuracy(&self, accuracy: f32) {
        store_f32(&self.aggregate_accuracy, accuracy.clamp(0.0, 1.0));
    }

    /// Read current environmental regime.
    pub fn regime(&self) -> Regime {
        Regime::from_u8(self.regime.load(Ordering::Acquire))
    }

    /// Write current environmental regime.
    pub fn set_regime(&self, regime: Regime) {
        self.regime.store(u8::from(regime), Ordering::Release);
    }

    /// Read current behavioral state.
    pub fn behavioral_state(&self) -> BehavioralState {
        BehavioralState::from_u8(self.behavioral_state.load(Ordering::Acquire))
    }

    /// Write current behavioral state.
    pub fn set_behavioral_state(&self, state: BehavioralState) {
        self.behavioral_state
            .store(u8::from(state), Ordering::Release);
    }

    /// Write resource health in `[0.0, 1.0]`.
    pub fn set_resource_health(&self, resource_health: f32) {
        store_f32(&self.resource_health, resource_health.clamp(0.0, 1.0));
    }

    /// Read a full eventually consistent snapshot.
    pub fn snapshot(&self) -> CorticalSnapshot {
        CorticalSnapshot {
            pad: self.pad(),
            primary_emotion: PlutchikLabel::from_u8(self.primary_emotion.load(Ordering::Acquire)),
            aggregate_accuracy: self.prediction_accuracy(),
            accuracy_trend: self.accuracy_trend.load(Ordering::Acquire),
            category_accuracies: self.category_accuracies.each_ref().map(load_f32),
            surprise_rate: load_f32(&self.surprise_rate),
            universe_size: self.universe_size.load(Ordering::Acquire),
            active_count: self.active_count.load(Ordering::Acquire),
            pending_predictions: self.pending_predictions.load(Ordering::Acquire),
            creative_mode: self.creative_mode.load(Ordering::Acquire) != 0,
            fragments_captured: self.fragments_captured.load(Ordering::Acquire),
            last_novel_prediction_tick: self.last_novel_prediction_tick.load(Ordering::Acquire),
            regime: self.regime(),
            gas_gwei: load_f32(&self.gas_gwei),
            resource_health: load_f32(&self.resource_health),
            knowledge_health: load_f32(&self.knowledge_health),
            performance_trend: load_f32(&self.performance_trend),
            behavioral_state: self.behavioral_state(),
            compounding_momentum: load_f32(&self.compounding_momentum),
        }
    }
}

impl Default for CorticalState {
    fn default() -> Self {
        Self::new(PersonalityPreset::Balanced)
    }
}

impl std::fmt::Debug for CorticalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CorticalState")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

/// Eventually consistent snapshot of [`CorticalState`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorticalSnapshot {
    /// Current affect vector.
    pub pad: PadVector,
    /// Current primary emotion label.
    pub primary_emotion: PlutchikLabel,
    /// Aggregate prediction accuracy.
    pub aggregate_accuracy: f32,
    /// Accuracy trend: `-1`, `0`, or `1`.
    pub accuracy_trend: i8,
    /// Per-category prediction accuracies.
    pub category_accuracies: [f32; 16],
    /// Recent surprise rate.
    pub surprise_rate: f32,
    /// Number of tracked attention items.
    pub universe_size: u32,
    /// Number of active attention items.
    pub active_count: u16,
    /// Number of unresolved predictions.
    pub pending_predictions: u32,
    /// Whether creative mode is active.
    pub creative_mode: bool,
    /// Number of dream fragments captured.
    pub fragments_captured: u32,
    /// Last tick that produced a novel prediction.
    pub last_novel_prediction_tick: u64,
    /// Current environmental regime.
    pub regime: Regime,
    /// Chain-domain gas price signal.
    pub gas_gwei: f32,
    /// Remaining resource health in `[0.0, 1.0]`.
    pub resource_health: f32,
    /// Knowledge quality health in `[0.0, 1.0]`.
    pub knowledge_health: f32,
    /// Performance trend in `[-1.0, 1.0]`.
    pub performance_trend: f32,
    /// Current behavioral state.
    pub behavioral_state: BehavioralState,
    /// Derived compounding momentum signal.
    pub compounding_momentum: f32,
}

/// Configuration for heartbeat cadence, budget throttling, and triggers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClockConfig {
    /// Minimum gamma interval in seconds.
    pub gamma_min_interval_secs: u64,
    /// Maximum gamma interval in seconds.
    pub gamma_max_interval_secs: u64,
    /// Base gamma interval in seconds.
    pub gamma_base_interval_secs: u64,
    /// Minimum theta interval in seconds.
    pub theta_min_interval_secs: u64,
    /// Maximum theta interval in seconds.
    pub theta_max_interval_secs: u64,
    /// Base theta interval in seconds.
    pub theta_base_interval_secs: u64,
    /// Number of gamma ticks that should trigger theta.
    pub theta_gamma_count: u32,
    /// Episode threshold that should trigger delta.
    pub delta_episode_threshold: usize,
    /// Idle timeout in seconds that should trigger delta.
    pub delta_idle_timeout_secs: u64,
    /// Daily budget in USD.
    pub daily_budget_usd: f64,
    /// Budget percentage at which throttling begins.
    pub throttle_at_percent: u8,
    /// Budget percentage at which expensive tiers should stop.
    pub hard_stop_at_percent: u8,
    /// Scheduler poll interval in milliseconds.
    pub scheduler_poll_interval_millis: u64,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            gamma_min_interval_secs: 5,
            gamma_max_interval_secs: 15,
            gamma_base_interval_secs: 10,
            theta_min_interval_secs: 15,
            theta_max_interval_secs: 120,
            theta_base_interval_secs: 75,
            theta_gamma_count: 5,
            delta_episode_threshold: 50,
            delta_idle_timeout_secs: 300,
            daily_budget_usd: 50.0,
            throttle_at_percent: 80,
            hard_stop_at_percent: 95,
            scheduler_poll_interval_millis: 1_000,
        }
    }
}

/// A heartbeat tick pulse published on the runtime bus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatTick {
    /// Monotonic tick id for this policy instance.
    pub tick_id: u64,
    /// Cognitive speed of the tick.
    pub speed: HeartbeatSpeed,
    /// Canonical heartbeat topic.
    pub topic: String,
    /// UTC timestamp when the tick was emitted.
    pub emitted_at: DateTime<Utc>,
    /// Interval selected for this speed at emission time.
    pub interval_millis: u64,
    /// Current environmental regime.
    pub regime: Regime,
}

/// Inter-loop control signal produced by heartbeat policies and meta-cognition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveSignal {
    /// Stop all cognitive work as soon as possible.
    Shutdown,
    /// Pause lower-priority consolidation work.
    Pause,
    /// Resume paused work.
    Resume,
    /// Escalate to a stronger tier or request review.
    Escalate,
    /// Slow down and reduce thrashing.
    Cooldown,
    /// Reprioritize toward the provided target.
    Reprioritize(String),
    /// Inject a context note into the next deliberation.
    InjectContext(String),
    /// Seek novel information or alternatives.
    Explore,
}

impl CognitiveSignal {
    /// Return lower numeric values for higher-priority signals.
    pub const fn priority(&self) -> u8 {
        match self {
            Self::Shutdown => 1,
            Self::Pause => 2,
            Self::Escalate => 3,
            Self::Cooldown => 4,
            Self::Reprioritize(_) => 5,
            Self::InjectContext(_) => 6,
            Self::Explore => 7,
            Self::Resume => 8,
        }
    }
}

/// Condition that should emit an early gamma tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeupCondition {
    /// External intervention from a user or operator.
    UserIntervention,
    /// Internal intervention from a safety system.
    SafetyAlert,
    /// Coordination threat or opportunity signal.
    PheromoneAlert {
        /// Alert intensity in `[0.0, 1.0]`.
        intensity: f32,
    },
    /// Budget state changed enough to require attention.
    BudgetAlert,
    /// Scheduled event became due.
    ScheduledEvent {
        /// Event identifier supplied by the scheduler.
        event_id: String,
    },
}

/// Runtime heartbeat policy that publishes tick pulses on the bus.
pub struct HeartbeatPolicy {
    config: ClockConfig,
    bus: BusSender<RokoEvent>,
    cancel: CancelToken,
    gamma_interval_millis: AtomicU64,
    theta_interval_millis: AtomicU64,
    tick_seq: AtomicU64,
    regime: AtomicU8,
}

impl HeartbeatPolicy {
    /// Create a heartbeat policy backed by a runtime bus sender.
    pub fn new(config: ClockConfig, bus: BusSender<RokoEvent>, cancel: CancelToken) -> Self {
        let gamma_interval = Duration::from_secs(config.gamma_base_interval_secs);
        let theta_interval = Duration::from_secs(config.theta_base_interval_secs);
        Self {
            config,
            bus,
            cancel,
            gamma_interval_millis: AtomicU64::new(duration_millis(gamma_interval)),
            theta_interval_millis: AtomicU64::new(duration_millis(theta_interval)),
            tick_seq: AtomicU64::new(0),
            regime: AtomicU8::new(u8::from(Regime::Calm)),
        }
    }

    /// Access the clock configuration.
    pub const fn config(&self) -> &ClockConfig {
        &self.config
    }

    /// Read the current gamma interval.
    pub fn gamma_interval(&self) -> Duration {
        Duration::from_millis(self.gamma_interval_millis.load(Ordering::Acquire))
    }

    /// Read the current theta interval.
    pub fn theta_interval(&self) -> Duration {
        Duration::from_millis(self.theta_interval_millis.load(Ordering::Acquire))
    }

    /// Set the current gamma interval.
    pub fn set_gamma_interval(&self, interval: Duration) {
        self.gamma_interval_millis
            .store(duration_millis(interval), Ordering::Release);
    }

    /// Set the current theta interval.
    pub fn set_theta_interval(&self, interval: Duration) {
        self.theta_interval_millis
            .store(duration_millis(interval), Ordering::Release);
    }

    /// Set the environmental regime used by future ticks.
    pub fn set_regime(&self, regime: Regime) {
        self.regime.store(u8::from(regime), Ordering::Release);
    }

    /// Compute the gamma interval from recent anomaly count.
    pub fn compute_gamma_interval(&self, anomaly_count: usize) -> Duration {
        compute_gamma_interval(anomaly_count, &self.config)
    }

    /// Compute the theta interval from the current regime.
    pub fn compute_theta_interval(&self, regime: Regime) -> Duration {
        compute_theta_interval(regime, &self.config)
    }

    /// Emit one tick for a specific speed.
    pub fn emit_tick(&self, speed: HeartbeatSpeed) -> HeartbeatTick {
        let interval = match speed {
            HeartbeatSpeed::Gamma => self.gamma_interval(),
            HeartbeatSpeed::Theta => self.theta_interval(),
            HeartbeatSpeed::Delta => Duration::from_secs(self.config.delta_idle_timeout_secs),
        };
        let tick = HeartbeatTick {
            tick_id: self.tick_seq.fetch_add(1, Ordering::Relaxed),
            speed,
            topic: speed.topic().to_string(),
            emitted_at: Utc::now(),
            interval_millis: duration_millis(interval),
            regime: Regime::from_u8(self.regime.load(Ordering::Acquire)),
        };
        self.bus.emit(RokoEvent::HeartbeatTick(tick.clone()));
        tick
    }

    /// Emit a cognitive signal on the runtime bus.
    pub fn emit_signal(&self, signal: CognitiveSignal) {
        self.bus.emit(RokoEvent::CognitiveSignal {
            signal,
            issued_at: Utc::now(),
        });
    }

    /// Emit an early gamma tick because a wakeup condition fired.
    pub fn wakeup(&self, condition: WakeupCondition) -> HeartbeatTick {
        self.bus.emit(RokoEvent::HeartbeatWakeup {
            condition,
            issued_at: Utc::now(),
        });
        self.emit_tick(HeartbeatSpeed::Gamma)
    }

    /// Run the three tick producers until cancellation.
    pub async fn run(&self) {
        let mut gamma = tokio::time::interval(self.gamma_interval());
        gamma.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut theta = tokio::time::interval(self.theta_interval());
        theta.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = self.cancel.cancelled() => break,
                _ = gamma.tick() => {
                    self.emit_tick(HeartbeatSpeed::Gamma);
                    gamma = tokio::time::interval(self.gamma_interval());
                    gamma.set_missed_tick_behavior(MissedTickBehavior::Delay);
                }
                _ = theta.tick() => {
                    self.emit_tick(HeartbeatSpeed::Theta);
                    theta = tokio::time::interval(self.theta_interval());
                    theta.set_missed_tick_behavior(MissedTickBehavior::Delay);
                }
            }
        }
    }
}

/// Compute the gamma tick interval from recent anomaly count.
pub fn compute_gamma_interval(anomaly_count: usize, config: &ClockConfig) -> Duration {
    let max = Duration::from_secs(config.gamma_max_interval_secs);
    let min = Duration::from_secs(config.gamma_min_interval_secs);
    let anomaly_count = usize_to_f64(anomaly_count);
    let adjusted = max.mul_f64(1.0 / anomaly_count.mul_add(0.3, 1.0));
    adjusted.max(min).min(max)
}

/// Compute the theta interval from the current regime.
pub fn compute_theta_interval(regime: Regime, config: &ClockConfig) -> Duration {
    let multiplier = match regime {
        Regime::Calm => 1.6,
        Regime::Normal => 1.0,
        Regime::Volatile => 0.4,
        Regime::Crisis => 0.2,
    };
    let base = Duration::from_secs(config.theta_base_interval_secs);
    Duration::from_secs_f64(base.as_secs_f64() * multiplier)
        .max(Duration::from_secs(config.theta_min_interval_secs))
        .min(Duration::from_secs(config.theta_max_interval_secs))
}

/// Apply budget-aware throttling to a non-gamma interval.
pub fn apply_budget_throttle(
    interval: Duration,
    budget_pct: f64,
    config: &ClockConfig,
) -> Duration {
    let hard_stop = f64::from(config.hard_stop_at_percent) / 100.0;
    let throttle_at = f64::from(config.throttle_at_percent) / 100.0;
    let max = Duration::from_secs(config.theta_max_interval_secs);
    if budget_pct >= hard_stop {
        max
    } else if budget_pct >= 0.90 {
        interval.mul_f64(4.0).min(max)
    } else if budget_pct >= throttle_at {
        interval.mul_f64(2.0).min(max)
    } else {
        interval
    }
}

/// Determine whether theta should fire from gamma count or episode completion.
pub const fn should_fire_theta(
    gamma_since_last_theta: u32,
    episode_completed: bool,
    config: &ClockConfig,
) -> bool {
    gamma_since_last_theta >= config.theta_gamma_count || episode_completed
}

/// Determine whether delta consolidation should enter.
pub fn should_enter_delta(
    idle_duration: Duration,
    episodes_since_last_delta: usize,
    scheduled_delta_time: Option<SystemTime>,
    explicit_trigger: bool,
    config: &ClockConfig,
) -> bool {
    explicit_trigger
        || idle_duration > Duration::from_secs(config.delta_idle_timeout_secs)
        || episodes_since_last_delta >= config.delta_episode_threshold
        || scheduled_delta_time.is_some_and(|time| SystemTime::now() >= time)
}

/// Prediction-state vector used to compute world-model drift.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PredictionState {
    /// Predicted environmental regime encoded as `[0.0, 1.0]`.
    pub predicted_regime: f32,
    /// Observed environmental regime encoded as `[0.0, 1.0]`.
    pub observed_regime: f32,
    /// Predicted accuracy.
    pub predicted_accuracy: f32,
    /// Observed accuracy.
    pub observed_accuracy: f32,
    /// Predicted resource health.
    pub predicted_resource_health: f32,
    /// Observed resource health.
    pub observed_resource_health: f32,
    /// Predicted active-item ratio.
    pub predicted_active_ratio: f32,
    /// Observed active-item ratio.
    pub observed_active_ratio: f32,
    /// Predicted arousal.
    pub predicted_arousal: f32,
    /// Observed arousal.
    pub observed_arousal: f32,
}

impl PredictionState {
    /// Compute normalized Euclidean drift in `[0.0, 1.0]`.
    pub fn compute_drift(&self) -> f32 {
        let pairs = [
            (self.predicted_regime, self.observed_regime),
            (self.predicted_accuracy, self.observed_accuracy),
            (
                self.predicted_resource_health,
                self.observed_resource_health,
            ),
            (self.predicted_active_ratio, self.observed_active_ratio),
            (self.predicted_arousal, self.observed_arousal),
        ];
        let sum_sq = pairs
            .into_iter()
            .map(|(predicted, observed)| (predicted - observed).powi(2))
            .sum::<f32>();
        (sum_sq / 5.0).sqrt().clamp(0.0, 1.0)
    }
}

/// Inputs for aggregate prediction-error computation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PredictionErrorInput {
    /// Number of anomalous probes.
    pub anomaly_count: u32,
    /// Whether the environmental regime changed.
    pub regime_changed: bool,
    /// World-model drift in `[0.0, 1.0]`.
    pub drift: f32,
    /// Pending intervention count.
    pub pending_interventions: u32,
}

/// Tunable weights for prediction-error computation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PredictionErrorWeights {
    /// Contribution of each anomalous probe.
    pub anomaly_weight: f32,
    /// Contribution of a regime change.
    pub regime_change_weight: f32,
    /// Contribution multiplier for world-model drift.
    pub drift_weight: f32,
    /// Contribution of each pending intervention.
    pub intervention_weight: f32,
}

impl Default for PredictionErrorWeights {
    fn default() -> Self {
        Self {
            anomaly_weight: 0.05,
            regime_change_weight: 0.40,
            drift_weight: 0.30,
            intervention_weight: 0.10,
        }
    }
}

/// Compute aggregate prediction error from probe, regime, drift, and intervention signals.
pub fn compute_prediction_error(
    input: PredictionErrorInput,
    weights: PredictionErrorWeights,
) -> f32 {
    let mut error = u32_to_f32(input.anomaly_count) * weights.anomaly_weight;
    if input.regime_changed {
        error += weights.regime_change_weight;
    }
    error += input.drift.clamp(0.0, 1.0) * weights.drift_weight;
    error += u32_to_f32(input.pending_interventions) * weights.intervention_weight;
    error.min(1.0)
}

/// Tunable configuration for adaptive tier gating.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GatingConfig {
    /// Base T0-to-T1 prediction-error threshold.
    pub base_threshold: f32,
    /// Multiplier from T1 threshold to T2 threshold.
    pub t1_t2_multiplier: f32,
    /// Minimum adaptive threshold.
    pub threshold_min: f32,
    /// Maximum adaptive threshold.
    pub threshold_max: f32,
    /// Resource-health threshold below which T2 downgrades model class.
    pub t2_resource_threshold: f32,
}

impl Default for GatingConfig {
    fn default() -> Self {
        Self {
            base_threshold: 0.20,
            t1_t2_multiplier: 2.0,
            threshold_min: 0.05,
            threshold_max: 0.50,
            t2_resource_threshold: 0.30,
        }
    }
}

/// Inputs that modulate adaptive gating threshold.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveThresholdInputs {
    /// Current PAD vector.
    pub pad: PadVector,
    /// Budget usage fraction in `[0.0, 1.0]`.
    pub budget_usage_pct: f32,
    /// Strategy confidence in `[0.0, 1.0]`.
    pub strategy_confidence: f32,
}

/// Compute adaptive threshold from affect, resources, and confidence.
pub fn compute_adaptive_threshold(input: AdaptiveThresholdInputs, config: GatingConfig) -> f32 {
    let affect_adj = if input.pad.dominance < -0.2 {
        -0.05
    } else if input.pad.dominance > 0.3 {
        0.05
    } else {
        0.0
    };
    let resource_adj = if input.budget_usage_pct > 0.80 {
        0.10
    } else {
        0.0
    };
    let arousal_adj = if input.pad.arousal > 0.5 { -0.05 } else { 0.0 };
    let confidence_adj = input.strategy_confidence.clamp(0.0, 1.0) * 0.05;
    (config.base_threshold + affect_adj + resource_adj + arousal_adj + confidence_adj)
        .clamp(config.threshold_min, config.threshold_max)
}

/// Select a tier from prediction error and threshold.
pub fn gate_tier(
    prediction_error: f32,
    threshold: f32,
    forced: bool,
    config: GatingConfig,
) -> InferenceTier {
    if forced {
        return InferenceTier::T2;
    }
    if prediction_error < threshold {
        InferenceTier::T0
    } else if prediction_error < threshold * config.t1_t2_multiplier {
        InferenceTier::T1
    } else {
        InferenceTier::T2
    }
}

/// Complete record of a tier gating decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierDecision {
    /// Tick identifier.
    pub tick_id: u64,
    /// Timestamp of the decision.
    pub timestamp: DateTime<Utc>,
    /// Selected tier.
    pub tier: InferenceTier,
    /// Aggregate prediction error in `[0.0, 1.0]`.
    pub prediction_error: f32,
    /// Adaptive threshold at decision time.
    pub threshold: f32,
    /// Number of anomalous probes.
    pub anomaly_count: u32,
    /// Whether regime changed.
    pub regime_changed: bool,
    /// World-model drift in `[0.0, 1.0]`.
    pub drift: f32,
    /// Pending intervention count.
    pub pending_interventions: u32,
    /// Whether forced escalation was triggered.
    pub forced: bool,
    /// Optional forced-escalation reason.
    pub force_reason: Option<String>,
    /// PAD vector at decision time.
    pub pad: PadVector,
    /// Budget usage fraction at decision time.
    pub budget_usage_pct: f32,
    /// Strategy confidence at decision time.
    pub strategy_confidence: f32,
    /// Matching playbook rule identifier, if any.
    pub playbook_rule_id: Option<String>,
    /// Concrete model selected for this tier.
    pub model: Option<String>,
    /// Resource health at decision time.
    pub resource_health: f32,
}

/// Domain observation summary captured during a heartbeat tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// Human-readable observation summary.
    pub summary: String,
    /// Domain-specific scalar signals.
    pub signals: HashMap<String, f32>,
}

/// Probe output summary included in decision-cycle records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeReading {
    /// Probe name.
    pub name: String,
    /// Probe value in `[0.0, 1.0]`.
    pub value: f32,
    /// Probe weight in aggregate prediction error.
    pub weight: f32,
    /// Whether the probe was anomalous.
    pub anomalous: bool,
}

/// Anomaly detected during a heartbeat tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anomaly {
    /// Source probe or subsystem.
    pub source: String,
    /// Severity in `[0.0, 1.0]`.
    pub severity: f32,
    /// Human-readable description.
    pub description: String,
}

/// Context bundle summary included in a tick record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSummary {
    /// Number of tokens allocated.
    pub tokens_allocated: usize,
    /// Context sections included.
    pub sections: Vec<String>,
}

/// Retrieved durable entry summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngramSummary {
    /// Entry identifier or content hash.
    pub id: String,
    /// Entry kind.
    pub kind: String,
    /// Entry summary.
    pub summary: String,
}

/// Intervention summary active during a heartbeat tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionSummary {
    /// Intervention kind.
    pub kind: String,
    /// Intervention reason.
    pub reason: String,
}

/// Deliberation record for T1/T2 ticks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliberationRecord {
    /// Model used for deliberation.
    pub model: String,
    /// Prompt token count.
    pub input_tokens: u64,
    /// Output token count.
    pub output_tokens: u64,
    /// Latency in milliseconds.
    pub latency_millis: u64,
}

/// Action emitted during a heartbeat tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRecord {
    /// Action kind.
    pub kind: String,
    /// Action target.
    pub target: String,
    /// Human-readable action summary.
    pub summary: String,
}

/// Outcome produced after actions and verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutcomeRecord {
    /// Whether the tick outcome passed its verification criteria.
    pub passed: bool,
    /// Outcome summary.
    pub summary: String,
    /// Optional scalar score.
    pub score: Option<f32>,
}

/// Neuro mutation summary produced by learning hooks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeuroMutation {
    /// Mutation target identifier.
    pub target: String,
    /// Mutation kind.
    pub kind: String,
}

/// Reference to a somatic marker fired during a tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SomaticMarkerRef {
    /// Marker identifier.
    pub id: String,
    /// Marker label.
    pub label: String,
}

/// The structured output of a single gamma tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionCycleRecord {
    /// Tick identifier.
    pub tick: u64,
    /// UTC timestamp for this tick.
    pub timestamp: DateTime<Utc>,
    /// Agent identifier.
    pub agent_id: String,
    /// PERCEIVE/SENSE observation.
    pub observation: Observation,
    /// Environmental regime at tick time.
    pub regime: Regime,
    /// Probe results.
    pub probe_results: Vec<ProbeReading>,
    /// Detected anomalies.
    pub anomalies: Vec<Anomaly>,
    /// Aggregate prediction error.
    pub prediction_error: f32,
    /// Deliberation threshold used for tier selection.
    pub deliberation_threshold: f32,
    /// Selected inference tier.
    pub tier: InferenceTier,
    /// Human-readable gating reason.
    pub gating_reason: String,
    /// Context bundle summary.
    pub context_bundle_summary: ContextSummary,
    /// Retrieved durable entries.
    pub retrieved_entries: Vec<EngramSummary>,
    /// Active interventions.
    pub active_interventions: Vec<InterventionSummary>,
    /// Optional deliberation record for T1/T2 ticks.
    pub deliberation: Option<DeliberationRecord>,
    /// Actions emitted by the tick.
    pub actions: Vec<ActionRecord>,
    /// Optional outcome after verification.
    pub outcome: Option<OutcomeRecord>,
    /// Episode identifiers written.
    pub episodes_written: Vec<String>,
    /// Neuro mutations applied.
    pub neuro_mutations: Vec<NeuroMutation>,
    /// PAD before tick adaptation.
    pub pad_before: PadVector,
    /// PAD after tick adaptation.
    pub pad_after: PadVector,
    /// Somatic markers fired.
    pub somatic_markers_fired: Vec<SomaticMarkerRef>,
    /// Primary emotion after tick adaptation.
    pub primary_emotion: PlutchikLabel,
    /// Inference cost in USD.
    pub inference_cost: f64,
    /// Domain cost in USD.
    pub domain_cost: f64,
    /// Total cost in USD.
    pub total_cost: f64,
}

/// Aggregate summary of recent gamma ticks for theta reflection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GammaSummary {
    /// Number of ticks summarized.
    pub tick_count: u32,
    /// Distribution across T0/T1/T2.
    pub tier_distribution: [u32; 3],
    /// Success rate among records with outcomes.
    pub success_rate: f32,
    /// Total cost across summarized records.
    pub total_cost: f64,
    /// Recurring anomaly source names.
    pub recurring_anomalies: Vec<String>,
    /// Total action count.
    pub action_count: usize,
}

/// Summarize recent gamma records for theta reflection.
pub fn summarize_gamma_history(records: &[DecisionCycleRecord]) -> GammaSummary {
    let mut tier_distribution = [0_u32; 3];
    let mut outcome_count = 0_u32;
    let mut passed_count = 0_u32;
    let mut anomaly_counts: HashMap<String, u32> = HashMap::new();
    let mut total_cost = 0.0;
    let mut action_count = 0_usize;
    for record in records {
        tier_distribution[usize::from(u8::from(record.tier))] += 1;
        if let Some(outcome) = &record.outcome {
            outcome_count += 1;
            if outcome.passed {
                passed_count += 1;
            }
        }
        for anomaly in &record.anomalies {
            *anomaly_counts.entry(anomaly.source.clone()).or_default() += 1;
        }
        total_cost += record.total_cost;
        action_count += record.actions.len();
    }
    let recurring_anomalies = anomaly_counts
        .into_iter()
        .filter_map(|(source, count)| (count > 1).then_some(source))
        .collect();
    GammaSummary {
        tick_count: usize_to_u32(records.len()),
        tier_distribution,
        success_rate: if outcome_count == 0 {
            0.0
        } else {
            u32_to_f32(passed_count) / u32_to_f32(outcome_count)
        },
        total_cost,
        recurring_anomalies,
        action_count,
    }
}

/// Retry and approach history used by meta-cognition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryTracker {
    /// Current task identifier.
    pub current_task_id: Option<String>,
    /// Retry count for the current task.
    pub retry_count: u32,
    /// Recent approach tags.
    pub approach_history: VecDeque<String>,
    /// Maximum retained approach count.
    pub max_history: usize,
}

impl RetryTracker {
    /// Create an empty retry tracker.
    pub fn new(max_history: usize) -> Self {
        Self {
            current_task_id: None,
            retry_count: 0,
            approach_history: VecDeque::with_capacity(max_history),
            max_history,
        }
    }

    /// Record an attempt for a task and approach.
    pub fn record_attempt(&mut self, task_id: impl Into<String>, approach: impl Into<String>) {
        let task_id = task_id.into();
        if self.current_task_id.as_ref() == Some(&task_id) {
            self.retry_count += 1;
        } else {
            self.current_task_id = Some(task_id);
            self.retry_count = 0;
            self.approach_history.clear();
        }
        self.approach_history.push_back(approach.into());
        if self.approach_history.len() > self.max_history {
            self.approach_history.pop_front();
        }
    }

    /// Count approach changes in the last `n` attempts.
    pub fn approach_changes_last_n(&self, n: usize) -> u32 {
        let recent: Vec<&String> = self.approach_history.iter().rev().take(n).collect();
        recent
            .windows(2)
            .filter(|pair| pair[0] != pair[1])
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }
}

impl Default for RetryTracker {
    fn default() -> Self {
        Self::new(10)
    }
}

/// Meta-cognition issue detected by theta or delta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MetaIssue {
    /// Current task has retried too many times.
    Stuck {
        /// Task identifier, if known.
        task: Option<String>,
        /// Retry count.
        retries: u32,
        /// Suggested remediation.
        suggestion: String,
    },
    /// Agent is changing approaches without progress.
    Thrashing {
        /// Number of approach changes.
        changes: u32,
        /// Suggested remediation.
        suggestion: String,
    },
    /// Performance trend is declining.
    PerformanceDecline {
        /// Trend value.
        trend: f32,
        /// Suggested remediation.
        suggestion: String,
    },
    /// High accuracy with low arousal may indicate complacency.
    Complacency {
        /// Current accuracy.
        accuracy: f32,
        /// Current arousal.
        arousal: f32,
        /// Suggested remediation.
        suggestion: String,
    },
}

/// Result of a meta-cognition pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaCognitionResult {
    /// Issues detected during the pass.
    pub issues: Vec<MetaIssue>,
}

impl MetaCognitionResult {
    /// Convert issues to cognitive signals in priority order.
    pub fn signals(&self) -> Vec<CognitiveSignal> {
        let mut signals = self
            .issues
            .iter()
            .map(|issue| match issue {
                MetaIssue::Stuck { .. } | MetaIssue::PerformanceDecline { .. } => {
                    CognitiveSignal::Escalate
                }
                MetaIssue::Thrashing { .. } => CognitiveSignal::Cooldown,
                MetaIssue::Complacency { .. } => CognitiveSignal::Explore,
            })
            .collect::<Vec<_>>();
        signals.sort_by_key(CognitiveSignal::priority);
        signals
    }
}

/// Run a meta-cognition pass over retry history and cortical state.
pub fn meta_cognize(
    retry_tracker: &RetryTracker,
    cortical: &CorticalState,
    stuck_threshold: u32,
    thrash_threshold: u32,
    thrash_window: usize,
) -> MetaCognitionResult {
    let mut issues = Vec::new();
    if retry_tracker.retry_count > stuck_threshold {
        issues.push(MetaIssue::Stuck {
            task: retry_tracker.current_task_id.clone(),
            retries: retry_tracker.retry_count,
            suggestion: "Escalate to T2 with a different approach, or request human review".into(),
        });
    }
    let changes = retry_tracker.approach_changes_last_n(thrash_window);
    if changes > thrash_threshold {
        issues.push(MetaIssue::Thrashing {
            changes,
            suggestion: "Commit to one approach for several attempts before switching".into(),
        });
    }
    let snapshot = cortical.snapshot();
    if snapshot.performance_trend < -0.3 {
        issues.push(MetaIssue::PerformanceDecline {
            trend: snapshot.performance_trend,
            suggestion: "Switch to a stronger model or request a different task".into(),
        });
    }
    if snapshot.aggregate_accuracy > 0.8 && snapshot.pad.arousal < -0.2 {
        issues.push(MetaIssue::Complacency {
            accuracy: snapshot.aggregate_accuracy,
            arousal: snapshot.pad.arousal,
            suggestion: "Seek novel challenges or increase exploration rate".into(),
        });
    }
    MetaCognitionResult { issues }
}

/// Complete Expected Free Energy estimate for a runtime decision target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EFEEstimate {
    /// What this estimate is for.
    pub target: EFETarget,
    /// Expected utility of preferred outcomes.
    pub pragmatic_value: f64,
    /// Expected information gain.
    pub epistemic_value: f64,
    /// Normalized inference and latency cost.
    pub cost: f64,
    /// Token-cost component.
    pub token_cost: f64,
    /// Net value after discount and cost.
    pub net_efe: f64,
    /// Temporal discount applied.
    pub discount: f64,
    /// Number of observations informing this estimate.
    pub observation_count: u64,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f64,
}

impl EFEEstimate {
    /// Compute net EFE from components.
    pub fn compute(pragmatic_value: f64, epistemic_value: f64, cost: f64, discount: f64) -> f64 {
        discount.mul_add(pragmatic_value + epistemic_value, -cost)
    }
}

/// Target of an Expected Free Energy estimate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EFETarget {
    /// Tier selection target.
    Tier {
        /// Inference tier being evaluated.
        tier: InferenceTier,
    },
    /// Model selection target.
    Model {
        /// Model identifier.
        model: String,
    },
    /// Context-entry inclusion target.
    ContextEntry {
        /// Engram or entry identifier.
        engram_id: String,
    },
}

/// Dirichlet-categorical posterior for one factorized signal dimension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirichletCategorical {
    /// Concentration parameters.
    pub alphas: Vec<f64>,
    /// Number of categories.
    pub num_categories: usize,
}

impl DirichletCategorical {
    /// Create a dimension with a flat prior.
    pub fn new_uniform(num_categories: usize) -> Self {
        Self {
            alphas: vec![1.0; num_categories],
            num_categories,
        }
    }

    /// Observe category `k` and update the posterior.
    pub fn observe(&mut self, k: usize) {
        if k < self.num_categories {
            self.alphas[k] += 1.0;
        }
    }

    /// Expected posterior probability for category `k`.
    pub fn expected_prob(&self, k: usize) -> f64 {
        let total = self.alphas.iter().sum::<f64>();
        self.alphas.get(k).copied().unwrap_or(0.0) / total.max(f64::EPSILON)
    }

    /// Entropy of the predictive distribution.
    pub fn entropy(&self) -> f64 {
        let total = self.alphas.iter().sum::<f64>().max(f64::EPSILON);
        -self
            .alphas
            .iter()
            .map(|alpha| {
                let probability = *alpha / total;
                if probability > 0.0 {
                    probability * probability.ln()
                } else {
                    0.0
                }
            })
            .sum::<f64>()
    }
}

/// Factorized generative model over cortical signal dimensions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerativeModel {
    /// Per-signal Dirichlet-categorical dimensions.
    pub dimensions: HashMap<String, DirichletCategorical>,
    /// Total observations seen.
    pub observation_count: u64,
}

impl GenerativeModel {
    /// Create an empty generative model.
    pub fn new() -> Self {
        Self {
            dimensions: HashMap::new(),
            observation_count: 0,
        }
    }

    /// Add or replace a named dimension with a uniform prior.
    pub fn add_uniform_dimension(&mut self, signal_id: impl Into<String>, categories: usize) {
        self.dimensions.insert(
            signal_id.into(),
            DirichletCategorical::new_uniform(categories),
        );
    }

    /// Observe a category for a signal dimension.
    pub fn observe(&mut self, signal_id: &str, category: usize) {
        if let Some(dimension) = self.dimensions.get_mut(signal_id) {
            dimension.observe(category);
            self.observation_count += 1;
        }
    }
}

impl Default for GenerativeModel {
    fn default() -> Self {
        Self::new()
    }
}

/// Task lifecycle phase for the 90-state active inference POMDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPhase {
    /// Parsing the task and identifying requirements.
    Understanding,
    /// Generating an approach and decomposing steps.
    Planning,
    /// Retrieving relevant knowledge and code context.
    GatheringContext,
    /// Writing code or executing actions.
    Implementing,
    /// Running gates and checking results.
    Verifying,
    /// Task is finished.
    Complete,
}

/// Quality of currently available context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextQuality {
    /// No relevant context retrieved.
    None,
    /// Some context exists but critical gaps remain.
    Insufficient,
    /// Moderate coverage with ambiguity.
    Partial,
    /// Good coverage for the current phase.
    Adequate,
    /// Full coverage with high confidence.
    Comprehensive,
}

/// Current uncertainty level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Uncertainty {
    /// High uncertainty.
    High,
    /// Medium uncertainty.
    Medium,
    /// Low uncertainty.
    Low,
}

/// Factorized active-inference state with 90 possible combinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PomdpState {
    /// Task lifecycle phase.
    pub task_phase: TaskPhase,
    /// Quality of current context.
    pub context_quality: ContextQuality,
    /// Current uncertainty.
    pub uncertainty: Uncertainty,
}

impl PomdpState {
    /// Total number of factorized states.
    pub const COUNT: usize = 90;

    /// Return this state as an index in `0..90`.
    pub const fn index(self) -> usize {
        task_phase_index(self.task_phase) * 15
            + context_quality_index(self.context_quality) * 3
            + uncertainty_index(self.uncertainty)
    }
}

#[inline]
fn load_f32(atomic: &AtomicU32) -> f32 {
    f32::from_bits(atomic.load(Ordering::Acquire))
}

#[inline]
fn store_f32(atomic: &AtomicU32, value: f32) {
    atomic.store(value.to_bits(), Ordering::Release);
}

#[allow(clippy::cast_possible_truncation)]
fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[allow(clippy::cast_precision_loss)]
const fn usize_to_f64(value: usize) -> f64 {
    value as f64
}

fn usize_to_u32(value: usize) -> u32 {
    value.try_into().unwrap_or(u32::MAX)
}

#[allow(clippy::cast_precision_loss)]
const fn u32_to_f32(value: u32) -> f32 {
    value as f32
}

const fn task_phase_index(phase: TaskPhase) -> usize {
    match phase {
        TaskPhase::Understanding => 0,
        TaskPhase::Planning => 1,
        TaskPhase::GatheringContext => 2,
        TaskPhase::Implementing => 3,
        TaskPhase::Verifying => 4,
        TaskPhase::Complete => 5,
    }
}

const fn context_quality_index(quality: ContextQuality) -> usize {
    match quality {
        ContextQuality::None => 0,
        ContextQuality::Insufficient => 1,
        ContextQuality::Partial => 2,
        ContextQuality::Adequate => 3,
        ContextQuality::Comprehensive => 4,
    }
}

const fn uncertainty_index(uncertainty: Uncertainty) -> usize {
    match uncertainty {
        Uncertainty::High => 0,
        Uncertainty::Medium => 1,
        Uncertainty::Low => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::EventBus;

    #[test]
    fn cortical_pad_round_trips() {
        let state = CorticalState::default();
        let pad = PadVector {
            pleasure: -0.2,
            arousal: 0.7,
            dominance: 0.3,
        };
        state.set_pad(pad);
        assert_eq!(state.pad(), pad);
    }

    #[test]
    fn gamma_interval_accelerates_with_anomalies() {
        let config = ClockConfig::default();
        let calm = compute_gamma_interval(0, &config);
        let volatile = compute_gamma_interval(7, &config);
        assert_eq!(calm, Duration::from_secs(15));
        assert_eq!(volatile, Duration::from_secs(5));
    }

    #[test]
    fn tier_gating_matches_thresholds() {
        let config = GatingConfig::default();
        assert_eq!(gate_tier(0.0, 0.2, false, config), InferenceTier::T0);
        assert_eq!(gate_tier(0.2, 0.2, false, config), InferenceTier::T1);
        assert_eq!(gate_tier(0.4, 0.2, false, config), InferenceTier::T2);
        assert_eq!(gate_tier(0.0, 0.2, true, config), InferenceTier::T2);
    }

    #[test]
    fn heartbeat_policy_emits_bus_tick() {
        let bus = EventBus::new(16);
        let policy = HeartbeatPolicy::new(ClockConfig::default(), bus.sender(), CancelToken::new());
        let tick = policy.emit_tick(HeartbeatSpeed::Gamma);
        assert_eq!(tick.topic, HEARTBEAT_GAMMA_TICK);
        let events = bus.replay_from(0);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn dirichlet_observe_updates_probability() {
        let mut dimension = DirichletCategorical::new_uniform(3);
        let before = dimension.expected_prob(1);
        dimension.observe(1);
        assert!(dimension.expected_prob(1) > before);
    }
}
