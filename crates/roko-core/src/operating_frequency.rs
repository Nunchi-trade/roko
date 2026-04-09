//! Operating frequency bands for 3-speed cognition.
//!
//! These frequencies name the intended cadence of agent behavior:
//! - `Gamma`: reactive, ~10s
//! - `Theta`: strategic, ~2-5min
//! - `Delta`: consolidation, ~30min+

use crate::{Task, TaskContextWeight, TaskQualityProfile, TaskReasoningLevel};
use bardo_primitives::tier::InferenceTier;
use serde::{Deserialize, Serialize};

/// Cognitive operating frequency for agent work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
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
    task.tags
        .as_deref()
        .is_some_and(|tags| tags.iter().any(|tag| normalize_token(tag) == normalized_needle))
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
    value.trim().to_ascii_lowercase().replace('-', "_").replace(' ', "_")
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
    use super::OperatingFrequency;
    use bardo_primitives::tier::InferenceTier;
    use crate::{Task, TaskCategory, TaskContextWeight, TaskQualityProfile, TaskReasoningLevel};

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
        assert_eq!(OperatingFrequency::Gamma.inference_tier(), InferenceTier::T0);
        assert_eq!(OperatingFrequency::Theta.inference_tier(), InferenceTier::T1);
        assert_eq!(OperatingFrequency::Delta.inference_tier(), InferenceTier::T2);
    }

    #[test]
    fn round_trips_from_inference_tiers() {
        assert_eq!(OperatingFrequency::from(InferenceTier::T0), OperatingFrequency::Gamma);
        assert_eq!(OperatingFrequency::from(InferenceTier::T1), OperatingFrequency::Theta);
        assert_eq!(OperatingFrequency::from(InferenceTier::T2), OperatingFrequency::Delta);
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

        assert_eq!(OperatingFrequency::select(&task, &affect), OperatingFrequency::Gamma);
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

        assert_eq!(OperatingFrequency::select(&task, &affect), OperatingFrequency::Delta);
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

        assert_eq!(OperatingFrequency::select(&task, &affect), OperatingFrequency::Delta);
    }

    #[test]
    fn defaults_to_theta_for_standard_tasks() {
        let task = Task::new("t4", "Implement code change");
        let affect = TestAffect {
            confidence: 0.8,
            arousal: 0.2,
            dominance: 0.1,
        };

        assert_eq!(OperatingFrequency::select(&task, &affect), OperatingFrequency::Theta);
    }
}
