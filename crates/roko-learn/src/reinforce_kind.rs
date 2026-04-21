//! Typed reinforcement signal categories (AS-11).
//!
//! Provides a structured enum for categorizing reinforcement signals flowing
//! through the learning subsystem. This replaces ad-hoc boolean success/failure
//! flags with semantically rich signal types that downstream consumers (cascade
//! router, bandits, prompt experiments, skill library) can dispatch on.

use serde::{Deserialize, Serialize};

/// Typed category for a reinforcement signal.
///
/// Each variant encodes why the signal was produced and what domain it
/// applies to, enabling richer feedback loops than bare `succeeded: bool`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReinforceKind {
    /// Gate pipeline passed for a task.
    GatePass {
        /// Which gate rung (0..=6) passed.
        rung: u32,
    },
    /// Gate pipeline failed for a task.
    GateFail {
        /// Which gate rung failed.
        rung: u32,
        /// Whether the failure was on compilation, tests, clippy, diff, etc.
        gate_name: String,
    },
    /// Model routing decision was validated by task outcome.
    RoutingSuccess {
        /// Model that was selected.
        model: String,
        /// Tier used.
        tier: String,
    },
    /// Model routing decision resulted in a poor outcome.
    RoutingFailure {
        /// Model that was selected.
        model: String,
        /// Tier used.
        tier: String,
    },
    /// A prompt section or template contributed to task success.
    PromptEffective {
        /// Section or template identifier.
        section_id: String,
    },
    /// A prompt section or template was present during a failure.
    PromptIneffective {
        /// Section or template identifier.
        section_id: String,
    },
    /// Skill from the skill library was applied successfully.
    SkillApplied {
        /// Skill identifier.
        skill_id: String,
    },
    /// Playbook sequence was followed and led to a positive outcome.
    PlaybookFollowed {
        /// Playbook identifier.
        playbook_id: String,
    },
    /// Conductor intervention was beneficial (task recovered after intervention).
    InterventionHelpful {
        /// Intervention type (restart, model-change, etc.).
        intervention_type: String,
    },
    /// Conductor intervention was not helpful (task still failed).
    InterventionUnhelpful {
        /// Intervention type.
        intervention_type: String,
    },
    /// Dream consolidation hypothesis was validated in waking.
    DreamHypothesisValidated {
        /// Hypothesis identifier.
        hypothesis_id: String,
    },
    /// Dream consolidation hypothesis was refuted in waking.
    DreamHypothesisRefuted {
        /// Hypothesis identifier.
        hypothesis_id: String,
    },
    /// Cost efficiency was acceptable for this task.
    CostEfficient {
        /// Actual cost in USD.
        cost_usd: f64,
        /// Budget limit in USD.
        budget_usd: f64,
    },
    /// Cost exceeded expectations for this task.
    CostOverrun {
        /// Actual cost in USD.
        cost_usd: f64,
        /// Budget limit in USD.
        budget_usd: f64,
    },
    /// Generic positive reinforcement with a freeform label.
    Positive {
        /// Domain-specific label.
        label: String,
    },
    /// Generic negative reinforcement with a freeform label.
    Negative {
        /// Domain-specific label.
        label: String,
    },
}

impl ReinforceKind {
    /// Whether this reinforcement signal is positive (success/reward).
    #[must_use]
    pub fn is_positive(&self) -> bool {
        matches!(
            self,
            Self::GatePass { .. }
                | Self::RoutingSuccess { .. }
                | Self::PromptEffective { .. }
                | Self::SkillApplied { .. }
                | Self::PlaybookFollowed { .. }
                | Self::InterventionHelpful { .. }
                | Self::DreamHypothesisValidated { .. }
                | Self::CostEfficient { .. }
                | Self::Positive { .. }
        )
    }

    /// Whether this reinforcement signal is negative (failure/penalty).
    #[must_use]
    pub fn is_negative(&self) -> bool {
        !self.is_positive()
    }

    /// A short stable label for logging, metrics, and key construction.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::GatePass { .. } => "gate_pass",
            Self::GateFail { .. } => "gate_fail",
            Self::RoutingSuccess { .. } => "routing_success",
            Self::RoutingFailure { .. } => "routing_failure",
            Self::PromptEffective { .. } => "prompt_effective",
            Self::PromptIneffective { .. } => "prompt_ineffective",
            Self::SkillApplied { .. } => "skill_applied",
            Self::PlaybookFollowed { .. } => "playbook_followed",
            Self::InterventionHelpful { .. } => "intervention_helpful",
            Self::InterventionUnhelpful { .. } => "intervention_unhelpful",
            Self::DreamHypothesisValidated { .. } => "dream_hypothesis_validated",
            Self::DreamHypothesisRefuted { .. } => "dream_hypothesis_refuted",
            Self::CostEfficient { .. } => "cost_efficient",
            Self::CostOverrun { .. } => "cost_overrun",
            Self::Positive { .. } => "positive",
            Self::Negative { .. } => "negative",
        }
    }

    /// Convert to a numeric reward value: +1.0 for positive, -1.0 for negative.
    #[must_use]
    pub fn reward_value(&self) -> f64 {
        if self.is_positive() { 1.0 } else { -1.0 }
    }
}

/// A timestamped reinforcement signal with context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReinforceSignal {
    /// When this signal was produced.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// The kind of reinforcement.
    pub kind: ReinforceKind,
    /// Task identifier this signal is associated with.
    pub task_id: String,
    /// Optional plan identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// Optional agent/model identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

impl ReinforceSignal {
    /// Construct a new reinforcement signal.
    #[must_use]
    pub fn new(kind: ReinforceKind, task_id: impl Into<String>) -> Self {
        Self {
            timestamp: chrono::Utc::now(),
            kind,
            task_id: task_id.into(),
            plan_id: None,
            agent_id: None,
        }
    }

    /// Attach a plan identifier.
    #[must_use]
    pub fn with_plan(mut self, plan_id: impl Into<String>) -> Self {
        self.plan_id = Some(plan_id.into());
        self
    }

    /// Attach an agent identifier.
    #[must_use]
    pub fn with_agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_pass_is_positive() {
        let kind = ReinforceKind::GatePass { rung: 2 };
        assert!(kind.is_positive());
        assert!(!kind.is_negative());
        assert_eq!(kind.label(), "gate_pass");
        assert!((kind.reward_value() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gate_fail_is_negative() {
        let kind = ReinforceKind::GateFail {
            rung: 1,
            gate_name: "compile".to_string(),
        };
        assert!(kind.is_negative());
        assert!(!kind.is_positive());
        assert_eq!(kind.label(), "gate_fail");
        assert!((kind.reward_value() - (-1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn all_positive_kinds() {
        let positives = [
            ReinforceKind::GatePass { rung: 0 },
            ReinforceKind::RoutingSuccess {
                model: "m".into(),
                tier: "t".into(),
            },
            ReinforceKind::PromptEffective {
                section_id: "s".into(),
            },
            ReinforceKind::SkillApplied {
                skill_id: "sk".into(),
            },
            ReinforceKind::PlaybookFollowed {
                playbook_id: "p".into(),
            },
            ReinforceKind::InterventionHelpful {
                intervention_type: "restart".into(),
            },
            ReinforceKind::DreamHypothesisValidated {
                hypothesis_id: "h".into(),
            },
            ReinforceKind::CostEfficient {
                cost_usd: 1.0,
                budget_usd: 2.0,
            },
            ReinforceKind::Positive {
                label: "custom".into(),
            },
        ];
        for kind in &positives {
            assert!(kind.is_positive(), "{:?} should be positive", kind);
        }
    }

    #[test]
    fn all_negative_kinds() {
        let negatives = [
            ReinforceKind::GateFail {
                rung: 0,
                gate_name: "test".into(),
            },
            ReinforceKind::RoutingFailure {
                model: "m".into(),
                tier: "t".into(),
            },
            ReinforceKind::PromptIneffective {
                section_id: "s".into(),
            },
            ReinforceKind::InterventionUnhelpful {
                intervention_type: "restart".into(),
            },
            ReinforceKind::DreamHypothesisRefuted {
                hypothesis_id: "h".into(),
            },
            ReinforceKind::CostOverrun {
                cost_usd: 3.0,
                budget_usd: 2.0,
            },
            ReinforceKind::Negative {
                label: "custom".into(),
            },
        ];
        for kind in &negatives {
            assert!(kind.is_negative(), "{:?} should be negative", kind);
        }
    }

    #[test]
    fn signal_construction() {
        let signal = ReinforceSignal::new(ReinforceKind::GatePass { rung: 3 }, "task-42")
            .with_plan("plan-1")
            .with_agent("claude-opus-4-6");

        assert_eq!(signal.task_id, "task-42");
        assert_eq!(signal.plan_id.as_deref(), Some("plan-1"));
        assert_eq!(signal.agent_id.as_deref(), Some("claude-opus-4-6"));
        assert!(signal.kind.is_positive());
    }

    #[test]
    fn serde_roundtrip() {
        let kind = ReinforceKind::CostOverrun {
            cost_usd: 5.5,
            budget_usd: 3.0,
        };
        let json = serde_json::to_string(&kind).unwrap();
        let deserialized: ReinforceKind = serde_json::from_str(&json).unwrap();
        assert_eq!(kind, deserialized);
    }
}
