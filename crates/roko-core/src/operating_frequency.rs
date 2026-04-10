//! Operating frequency bands for 3-speed cognition.
//!
//! These frequencies name the intended cadence of agent behavior:
//! - `Gamma`: reactive, ~10s
//! - `Theta`: strategic, ~2-5min
//! - `Delta`: consolidation, ~30min+

use crate::{Task, TaskContextWeight, TaskQualityProfile, TaskReasoningLevel};
use bardo_primitives::tier::InferenceTier;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Cognitive operating frequency for agent work.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingFrequency {
    /// Reactive mode: perceive, retrieve, act.
    ///
    /// Tool calls, cache lookups, and signal routing.
    Gamma,
    /// Strategic mode: re-plan, update goals, evaluate progress.
    ///
    /// Periodic step-back / course-correction passes.
    Theta,
    /// Consolidation mode: replay, distill, meta-cognate.
    ///
    /// Slow learning and knowledge consolidation.
    Delta,
}

impl OperatingFrequency {
    /// Map to the existing inference tier model.
    #[must_use]
    pub const fn inference_tier(self) -> InferenceTier {
        match self {
            Self::Gamma => InferenceTier::T0,
            Self::Theta => InferenceTier::T1,
            Self::Delta => InferenceTier::T2,
        }
    }

    /// Map operating frequency to the default agent turn limit.
    ///
    /// - `Gamma` reactive work does not dispatch an agent.
    /// - `Theta` deliberative work uses the default 20-turn budget.
    /// - `Delta` reflective work gets a 50-turn budget.
    #[must_use]
    pub const fn turn_limit(self) -> u32 {
        match self {
            Self::Gamma => 0,
            Self::Theta => 20,
            Self::Delta => 50,
        }
    }

    /// Select the operating frequency for a task and its current affect state.
    ///
    /// Rules:
    /// - `Gamma` for reactive work like `quick_fix`, gate re-checks, permission checks,
    ///   and subscription-filter evaluation.
    /// - `Delta` for reflective work like dream cycles, plan regeneration, and retrospectives.
    /// - `Theta` for the default deliberative path.
    /// - Low-confidence, substantial tasks are promoted to `Delta` so the loop can step back
    ///   and reconsider the approach instead of blindly continuing.
    #[must_use]
    pub fn select(task: &Task, affect: &impl OperatingFrequencyAffect) -> Self {
        if is_reactive_task(task) {
            return Self::Gamma;
        }

        if is_reflective_task(task) {
            return Self::Delta;
        }

        if affect_suggests_reflection(affect) && task.is_substantial() {
            return Self::Delta;
        }

        Self::Theta
    }
}

/// Runtime inputs used by the operating-frequency scheduler.
///
/// This is intentionally small and pure: the scheduler only needs timing,
/// throughput, and affect signals to choose the next loop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OperatingFrequencyScheduleContext {
    /// Time elapsed since the last theta evaluation.
    pub time_since_last_theta: Duration,
    /// Number of currently active tasks.
    pub active_tasks: usize,
    /// Recent task completion rate in `[0.0, 1.0]`.
    pub completion_rate: f64,
    /// Motivational confidence in `[0.0, 1.0]`.
    pub confidence: f64,
    /// Arousal in `[-1.0, 1.0]`.
    pub arousal: f64,
    /// Dominance in `[-1.0, 1.0]`.
    pub dominance: f64,
}

impl OperatingFrequencyScheduleContext {
    /// Construct a scheduling context from a read-only affect view.
    #[must_use]
    pub fn from_affect(
        time_since_last_theta: Duration,
        active_tasks: usize,
        completion_rate: f64,
        affect: &impl OperatingFrequencyAffect,
    ) -> Self {
        Self {
            time_since_last_theta,
            active_tasks,
            completion_rate,
            confidence: affect.confidence(),
            arousal: affect.arousal(),
            dominance: affect.dominance(),
        }
    }

    /// Returns `true` when the system has no active tasks.
    #[must_use]
    pub const fn is_idle(self) -> bool {
        self.active_tasks == 0
    }

    /// Returns `true` when work is stalling.
    #[must_use]
    pub fn is_stalling(self) -> bool {
        self.completion_rate <= STALLING_COMPLETION_RATE_THRESHOLD
    }

    /// Returns `true` when affect suggests the loop should step back sooner.
    #[must_use]
    pub fn is_anxious(self) -> bool {
        self.confidence <= ANXIOUS_CONFIDENCE_THRESHOLD
            && self.arousal >= ANXIOUS_AROUSAL_THRESHOLD
            && self.dominance <= ANXIOUS_DOMINANCE_THRESHOLD
    }
}

/// Selects the next operating frequency from runtime context.
///
/// - Idle systems consolidate with `Delta`.
/// - Stalling or anxious systems shorten the theta cadence.
/// - Otherwise the scheduler stays in `Gamma` until theta becomes due.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OperatingFrequencyScheduler {
    theta_interval: Duration,
    delta_interval: Duration,
}

impl Default for OperatingFrequencyScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl OperatingFrequencyScheduler {
    /// Default scheduler:
    /// - theta every 3 minutes
    /// - delta every 30 minutes
    #[must_use]
    pub fn new() -> Self {
        Self {
            theta_interval: Duration::from_secs(180),
            delta_interval: Duration::from_secs(30 * 60),
        }
    }

    /// Override the base theta interval.
    #[must_use]
    pub fn with_theta_interval(mut self, theta_interval: Duration) -> Self {
        assert!(
            theta_interval > Duration::ZERO,
            "theta interval must be positive"
        );
        self.theta_interval = theta_interval;
        self
    }

    /// Override the delta consolidation interval.
    #[must_use]
    pub fn with_delta_interval(mut self, delta_interval: Duration) -> Self {
        assert!(
            delta_interval > Duration::ZERO,
            "delta interval must be positive"
        );
        self.delta_interval = delta_interval;
        self
    }

    /// Choose the next loop to run.
    #[must_use]
    pub fn select(&self, context: &OperatingFrequencyScheduleContext) -> OperatingFrequency {
        if context.is_idle() {
            return OperatingFrequency::Delta;
        }

        if context.time_since_last_theta >= self.delta_interval {
            return OperatingFrequency::Delta;
        }

        let theta_due = self.theta_interval_for(context);
        if context.time_since_last_theta >= theta_due {
            OperatingFrequency::Theta
        } else {
            OperatingFrequency::Gamma
        }
    }

    fn theta_interval_for(&self, context: &OperatingFrequencyScheduleContext) -> Duration {
        let mut factor = 1.0;

        if context.is_stalling() {
            factor *= STALLING_THETA_MULTIPLIER;
        }

        if context.is_anxious() {
            factor *= ANXIOUS_THETA_MULTIPLIER;
        }

        scale_duration(self.theta_interval, factor)
    }
}

/// Read-only affect view used by frequency selection.
pub trait OperatingFrequencyAffect {
    /// Motivational confidence in `[0.0, 1.0]`.
    fn confidence(&self) -> f64;
    /// Arousal in `[-1.0, 1.0]`.
    fn arousal(&self) -> f64;
    /// Dominance in `[-1.0, 1.0]`.
    fn dominance(&self) -> f64;
}

const LOW_CONFIDENCE_THRESHOLD: f64 = 0.3;
const STALLING_COMPLETION_RATE_THRESHOLD: f64 = 0.25;
const ANXIOUS_CONFIDENCE_THRESHOLD: f64 = 0.35;
const ANXIOUS_AROUSAL_THRESHOLD: f64 = 0.25;
const ANXIOUS_DOMINANCE_THRESHOLD: f64 = -0.1;
const STALLING_THETA_MULTIPLIER: f64 = 0.5;
const ANXIOUS_THETA_MULTIPLIER: f64 = 0.66;

impl Task {
    /// Return `true` when the task is large enough to justify a reflective step-back.
    #[must_use]
    pub fn is_substantial(&self) -> bool {
        self.estimated_minutes.is_some_and(|minutes| minutes >= 30)
            || matches!(self.reasoning_level, Some(TaskReasoningLevel::High))
            || matches!(self.context_weight, Some(TaskContextWeight::Deep))
            || matches!(self.quality_profile, Some(TaskQualityProfile::Hardened))
    }
}

fn is_reactive_task(task: &Task) -> bool {
    task_tag_matches(task, "quick_fix")
        || task_text_matches(
            task,
            &[
                "quick fix",
                "quick-fix",
                "gate re-check",
                "gate recheck",
                "permission check",
                "permission checks",
                "tool permission",
                "tool permissions",
                "subscription filter",
                "filter evaluation",
            ],
        )
}

fn is_reflective_task(task: &Task) -> bool {
    task_text_matches(
        task,
        &[
            "dream",
            "dream cycle",
            "plan regeneration",
            "regeneration",
            "retrospective",
            "retrospective analysis",
            "retro",
            "meta-cognition",
            "meta cognition",
            "consolidation",
        ],
    )
}

fn affect_suggests_reflection(affect: &impl OperatingFrequencyAffect) -> bool {
    affect.confidence() < LOW_CONFIDENCE_THRESHOLD
        && (affect.arousal() > 0.25 || affect.dominance() < -0.1)
}

fn task_text_matches(task: &Task, needles: &[&str]) -> bool {
    let haystack = task_haystack(task);
    needles.iter().any(|needle| haystack.contains(needle))
}

fn task_tag_matches(task: &Task, needle: &str) -> bool {
    let normalized_needle = normalize_token(needle);
    task.tags.as_deref().is_some_and(|tags| {
        tags.iter()
            .any(|tag| normalize_token(tag) == normalized_needle)
    })
}

fn task_haystack(task: &Task) -> String {
    let mut parts = Vec::new();
    parts.push(task.title.to_ascii_lowercase());
    if let Some(role) = &task.role {
        parts.push(role.to_ascii_lowercase());
    }
    if let Some(category) = task.category {
        parts.push(category.label().to_string());
    }
    if let Some(tags) = &task.tags {
        parts.extend(tags.iter().map(|tag| tag.to_ascii_lowercase()));
    }
    parts.push(task.id.to_ascii_lowercase());
    parts.join(" ")
}

fn normalize_token(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "_")
}

fn scale_duration(duration: Duration, factor: f64) -> Duration {
    debug_assert!(factor > 0.0);
    Duration::from_secs_f64(duration.as_secs_f64() * factor)
}

impl From<OperatingFrequency> for InferenceTier {
    fn from(value: OperatingFrequency) -> Self {
        value.inference_tier()
    }
}

impl From<InferenceTier> for OperatingFrequency {
    fn from(value: InferenceTier) -> Self {
        match value {
            InferenceTier::T0 => Self::Gamma,
            InferenceTier::T1 => Self::Theta,
            InferenceTier::T2 => Self::Delta,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        OperatingFrequency, OperatingFrequencyScheduleContext, OperatingFrequencyScheduler,
    };
    use crate::{Task, TaskCategory, TaskContextWeight, TaskQualityProfile, TaskReasoningLevel};
    use bardo_primitives::tier::InferenceTier;

    struct TestAffect {
        confidence: f64,
        arousal: f64,
        dominance: f64,
    }

    impl super::OperatingFrequencyAffect for TestAffect {
        fn confidence(&self) -> f64 {
            self.confidence
        }

        fn arousal(&self) -> f64 {
            self.arousal
        }

        fn dominance(&self) -> f64 {
            self.dominance
        }
    }

    #[test]
    fn maps_to_inference_tiers() {
        assert_eq!(
            OperatingFrequency::Gamma.inference_tier(),
            InferenceTier::T0
        );
        assert_eq!(
            OperatingFrequency::Theta.inference_tier(),
            InferenceTier::T1
        );
        assert_eq!(
            OperatingFrequency::Delta.inference_tier(),
            InferenceTier::T2
        );
    }

    #[test]
    fn round_trips_from_inference_tiers() {
        assert_eq!(
            OperatingFrequency::from(InferenceTier::T0),
            OperatingFrequency::Gamma
        );
        assert_eq!(
            OperatingFrequency::from(InferenceTier::T1),
            OperatingFrequency::Theta
        );
        assert_eq!(
            OperatingFrequency::from(InferenceTier::T2),
            OperatingFrequency::Delta
        );
    }

    #[test]
    fn selects_gamma_for_reactive_tasks() {
        let mut task = Task::new("t1", "Quick fix gate re-check");
        task.tags = Some(vec!["quick_fix".into()]);
        task.category = Some(TaskCategory::Verification);
        let affect = TestAffect {
            confidence: 0.9,
            arousal: 0.1,
            dominance: 0.4,
        };

        assert_eq!(
            OperatingFrequency::select(&task, &affect),
            OperatingFrequency::Gamma
        );
    }

    #[test]
    fn selects_delta_for_reflective_tasks() {
        let mut task = Task::new("t2", "Plan regeneration after retrospection");
        task.context_weight = Some(TaskContextWeight::Deep);
        task.quality_profile = Some(TaskQualityProfile::Hardened);
        let affect = TestAffect {
            confidence: 0.9,
            arousal: 0.0,
            dominance: 0.3,
        };

        assert_eq!(
            OperatingFrequency::select(&task, &affect),
            OperatingFrequency::Delta
        );
    }

    #[test]
    fn selects_delta_for_low_confidence_substantial_tasks() {
        let mut task = Task::new("t3", "Implement complex feature");
        task.reasoning_level = Some(TaskReasoningLevel::High);
        let affect = TestAffect {
            confidence: 0.2,
            arousal: 0.4,
            dominance: -0.3,
        };

        assert_eq!(
            OperatingFrequency::select(&task, &affect),
            OperatingFrequency::Delta
        );
    }

    #[test]
    fn defaults_to_theta_for_standard_tasks() {
        let task = Task::new("t4", "Implement code change");
        let affect = TestAffect {
            confidence: 0.8,
            arousal: 0.2,
            dominance: 0.1,
        };

        assert_eq!(
            OperatingFrequency::select(&task, &affect),
            OperatingFrequency::Theta
        );
    }

    #[test]
    fn scheduler_defaults_to_gamma_before_theta_is_due() {
        let scheduler = OperatingFrequencyScheduler::default();
        let context = OperatingFrequencyScheduleContext {
            time_since_last_theta: Duration::from_secs(90),
            active_tasks: 1,
            completion_rate: 0.8,
            confidence: 0.8,
            arousal: 0.0,
            dominance: 0.2,
        };

        assert_eq!(scheduler.select(&context), OperatingFrequency::Gamma);
    }

    #[test]
    fn scheduler_triggers_theta_when_elapsed_reaches_base_interval() {
        let scheduler = OperatingFrequencyScheduler::default();
        let context = OperatingFrequencyScheduleContext {
            time_since_last_theta: Duration::from_secs(180),
            active_tasks: 2,
            completion_rate: 0.8,
            confidence: 0.8,
            arousal: 0.0,
            dominance: 0.2,
        };

        assert_eq!(scheduler.select(&context), OperatingFrequency::Theta);
    }

    #[test]
    fn scheduler_triggers_delta_when_idle() {
        let scheduler = OperatingFrequencyScheduler::default();
        let context = OperatingFrequencyScheduleContext {
            time_since_last_theta: Duration::from_secs(15),
            active_tasks: 0,
            completion_rate: 0.0,
            confidence: 0.4,
            arousal: -0.1,
            dominance: 0.0,
        };

        assert_eq!(scheduler.select(&context), OperatingFrequency::Delta);
    }

    #[test]
    fn scheduler_shortens_theta_cadence_when_stalling() {
        let scheduler = OperatingFrequencyScheduler::default();
        let context = OperatingFrequencyScheduleContext {
            time_since_last_theta: Duration::from_secs(120),
            active_tasks: 1,
            completion_rate: 0.1,
            confidence: 0.8,
            arousal: 0.0,
            dominance: 0.2,
        };

        assert_eq!(scheduler.select(&context), OperatingFrequency::Theta);
    }

    #[test]
    fn scheduler_shortens_theta_cadence_when_anxious() {
        let scheduler = OperatingFrequencyScheduler::default();
        let calm = OperatingFrequencyScheduleContext {
            time_since_last_theta: Duration::from_secs(120),
            active_tasks: 1,
            completion_rate: 0.8,
            confidence: 0.8,
            arousal: 0.0,
            dominance: 0.2,
        };
        let anxious = OperatingFrequencyScheduleContext {
            time_since_last_theta: Duration::from_secs(120),
            active_tasks: 1,
            completion_rate: 0.8,
            confidence: 0.2,
            arousal: 0.5,
            dominance: -0.4,
        };

        assert_eq!(scheduler.select(&calm), OperatingFrequency::Gamma);
        assert_eq!(scheduler.select(&anxious), OperatingFrequency::Theta);
    }

    #[test]
    fn turn_limits_match_frequency_bands() {
        assert_eq!(OperatingFrequency::Gamma.turn_limit(), 0);
        assert_eq!(OperatingFrequency::Theta.turn_limit(), 20);
        assert_eq!(OperatingFrequency::Delta.turn_limit(), 50);
    }
}
