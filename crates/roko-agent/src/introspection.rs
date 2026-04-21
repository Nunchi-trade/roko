//! Agent introspection and metacognition helpers.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use roko_core::{AgentRole, Temperament, ToolPermissions, tool::ToolCall};

use crate::translate::BackendResponse;

/// A lightweight snapshot of the current agent identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIdentity {
    /// Current role label.
    pub role: AgentRole,
    /// Default model tier for the role.
    pub model_tier: roko_core::ModelTier,
    /// Typed execution temperament.
    pub temperament: Temperament,
    /// Runtime capability mask.
    pub capabilities: ToolPermissions,
}

impl AgentIdentity {
    /// Build an identity from a role plus a typed temperament.
    #[must_use]
    pub fn new(role: AgentRole, temperament: Temperament) -> Self {
        Self {
            role,
            model_tier: role.model_tier(),
            temperament,
            capabilities: role.tool_permissions(),
        }
    }

    /// Build an identity from a role plus a temperament label.
    #[must_use]
    pub fn from_label(role: AgentRole, temperament: &str) -> Self {
        Self::new(
            role,
            Temperament::from_label(temperament).unwrap_or_default(),
        )
    }
}

/// A single tool / reasoning turn observed by the metacognitive monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    /// Turn index in the current run.
    pub index: usize,
    /// Assistant text returned for this turn.
    pub assistant_text: String,
    /// Reasoning / thinking content, if any.
    pub reasoning: Option<String>,
    /// Tool calls emitted in the turn.
    pub tool_calls: Vec<ToolCall>,
    /// Optional model confidence in the turn.
    pub confidence: Option<f32>,
}

impl Turn {
    /// Construct a turn from a backend response.
    #[must_use]
    pub fn from_response(
        index: usize,
        response: &BackendResponse,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            index,
            assistant_text: response.extract_text(),
            reasoning: response.extract_reasoning(),
            tool_calls,
            confidence: extract_confidence(response),
        }
    }

    fn tool_fingerprints(&self) -> impl Iterator<Item = String> + '_ {
        self.tool_calls
            .iter()
            .map(|call| format!("{}:{}", call.name, call.arguments))
    }
}

/// Intervention suggested by the monitor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intervention {
    /// Escalate to a larger / more capable model.
    EscalateModel,
    /// Route the current task to a human.
    HumanHandoff,
    /// Abort the current run.
    Abort,
    /// Inject a reflection prompt and continue.
    InjectReflection(String),
}

/// Simple metacognitive monitor with configurable thresholds.
#[derive(Debug, Clone)]
pub struct MetacognitiveMonitor {
    /// Number of repeated tool-call fingerprints that indicate a stuck loop.
    pub repeat_threshold: usize,
    /// Sliding-window size for contradiction detection.
    pub contradiction_window: usize,
    /// Confidence below this threshold triggers an escalation.
    pub confidence_threshold: f32,
    /// Confidence below this threshold triggers a human handoff.
    pub human_handoff_threshold: f32,
}

impl Default for MetacognitiveMonitor {
    fn default() -> Self {
        Self {
            repeat_threshold: 3,
            contradiction_window: 4,
            confidence_threshold: 0.35,
            human_handoff_threshold: 0.15,
        }
    }
}

impl MetacognitiveMonitor {
    /// Inspect recent turns and decide whether to intervene.
    #[must_use]
    pub fn check(&self, turns: &[Turn]) -> Option<Intervention> {
        if turns.is_empty() {
            return None;
        }

        if self.repeated_tool_calls(turns) {
            return Some(Intervention::InjectReflection(
                "the same tool call is repeating; pause, inspect the result, and reconcile the plan"
                    .into(),
            ));
        }

        if self.contradiction_detected(turns) {
            return Some(Intervention::InjectReflection(
                "the recent turns contradict each other; restate the current state before continuing"
                    .into(),
            ));
        }

        if let Some(confidence) = turns.last().and_then(|turn| turn.confidence) {
            if confidence < self.human_handoff_threshold {
                return Some(Intervention::HumanHandoff);
            }
            if confidence < self.confidence_threshold {
                return Some(Intervention::EscalateModel);
            }
        }

        None
    }

    fn repeated_tool_calls(&self, turns: &[Turn]) -> bool {
        let fingerprints: Vec<String> = turns
            .iter()
            .rev()
            .flat_map(|turn| turn.tool_fingerprints())
            .take(self.repeat_threshold)
            .collect();
        if fingerprints.len() < self.repeat_threshold || fingerprints.is_empty() {
            return false;
        }

        fingerprints.windows(2).all(|pair| pair[0] == pair[1])
    }

    fn contradiction_detected(&self, turns: &[Turn]) -> bool {
        let recent = turns.iter().rev().take(self.contradiction_window);
        let mut saw_positive = false;
        let mut saw_negative = false;

        for turn in recent {
            let text = format!(
                "{} {}",
                turn.assistant_text,
                turn.reasoning.as_deref().unwrap_or("")
            )
            .to_lowercase();
            if contains_positive_commitment(&text) {
                saw_positive = true;
            }
            if contains_negative_commitment(&text) {
                saw_negative = true;
            }
            if saw_positive && saw_negative {
                return true;
            }
        }

        false
    }
}

fn extract_confidence(response: &BackendResponse) -> Option<f32> {
    let value = match response {
        BackendResponse::Json(value) => Some(value),
        BackendResponse::StreamJson(events) => events.last(),
        BackendResponse::Text(_) => None,
    }?;

    for key in [
        "confidence",
        "confidence_score",
        "score",
        "message.confidence",
        "choices.0.message.confidence",
    ] {
        if let Some(value) = get_value(value, key).and_then(Value::as_f64) {
            return Some(value as f32);
        }
    }

    None
}

fn get_value<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let mut cursor = value;
    for part in key.split('.') {
        if let Ok(index) = part.parse::<usize>() {
            cursor = cursor.get(index)?;
        } else {
            cursor = cursor.get(part)?;
        }
    }
    Some(cursor)
}

fn contains_positive_commitment(text: &str) -> bool {
    [
        "i can",
        "we can",
        "possible",
        "works",
        "feasible",
        "confirmed",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn contains_negative_commitment(text: &str) -> bool {
    [
        "i can't",
        "cannot",
        "not possible",
        "won't work",
        "fails",
        "impossible",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::ToolCall;

    #[test]
    fn repeated_tool_calls_trigger_reflection() {
        let monitor = MetacognitiveMonitor::default();
        let turn = Turn {
            index: 1,
            assistant_text: String::new(),
            reasoning: None,
            tool_calls: vec![ToolCall::new(
                "1",
                "read_file",
                serde_json::json!({"path": "x"}),
            )],
            confidence: Some(0.9),
        };
        let turns = vec![turn.clone(), turn.clone(), turn];
        assert!(matches!(
            monitor.check(&turns),
            Some(Intervention::InjectReflection(_))
        ));
    }

    #[test]
    fn confidence_can_escalate() {
        let monitor = MetacognitiveMonitor::default();
        let turn = Turn {
            index: 1,
            assistant_text: "answer".into(),
            reasoning: None,
            tool_calls: Vec::new(),
            confidence: Some(0.2),
        };
        assert!(matches!(
            monitor.check(&[turn]),
            Some(Intervention::EscalateModel)
        ));
    }

    #[test]
    fn agent_identity_uses_role_defaults() {
        let identity = AgentIdentity::new(AgentRole::Implementer, Temperament::Balanced);
        assert_eq!(identity.role, AgentRole::Implementer);
        assert_eq!(identity.model_tier, AgentRole::Implementer.model_tier());
        assert_eq!(identity.temperament, Temperament::Balanced);
        assert!(identity.capabilities.read);
    }
}
