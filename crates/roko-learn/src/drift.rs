//! Behavioral drift detection for agent roles.
//!
//! This module builds lightweight action distributions from
//! [`AgentEfficiencyEvent`] telemetry and compares recent behavior against a
//! reference distribution using Jensen-Shannon divergence (JSD). Tool usage is
//! compared as a categorical distribution, while token intensity and tool-call
//! intensity are compared as bounded Bernoulli distributions. The three JSD
//! components are averaged into a single score in `[0, 1]`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::efficiency::AgentEfficiencyEvent;

/// JSD threshold above which drift is considered meaningful.
pub const DRIFT_ALERT_THRESHOLD: f64 = 0.15;

/// Drift alert emitted when recent behavior diverges from the reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftAlert {
    /// Agent role whose behavior drifted.
    pub role: String,
    /// Composite Jensen-Shannon divergence score in `[0, 1]`.
    pub jsd: f64,
    /// Human-readable explanation of the detected drift.
    pub description: String,
}

/// Aggregate action distribution for a role across a window of turns.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActionDistribution {
    /// Relative frequency of each tool across all observed tool calls.
    pub tool_frequencies: HashMap<String, f64>,
    /// Mean tokens consumed per turn.
    pub avg_tokens_per_turn: f64,
    /// Mean tool calls per turn.
    pub avg_tool_calls_per_turn: f64,
}

impl ActionDistribution {
    /// Build an action distribution from efficiency events.
    #[must_use]
    pub fn from_events(events: &[AgentEfficiencyEvent]) -> Self {
        if events.is_empty() {
            return Self::default();
        }

        let mut tool_counts: HashMap<String, u64> = HashMap::new();
        let mut total_tool_calls = 0_u64;
        let mut total_tokens = 0.0;
        let mut total_tool_calls_per_turn = 0.0;

        for event in events {
            total_tokens += event.total_tokens() as f64 + event.reasoning_tokens as f64;

            let tool_call_count = if event.tool_calls.is_empty() {
                u64::from(event.tools_used)
            } else {
                event.tool_calls.len() as u64
            };
            total_tool_calls_per_turn += tool_call_count as f64;

            for call in &event.tool_calls {
                *tool_counts.entry(call.tool_name.clone()).or_default() += 1;
                total_tool_calls += 1;
            }
        }

        let mut tool_frequencies = HashMap::new();
        if total_tool_calls > 0 {
            let total_tool_calls = total_tool_calls as f64;
            for (tool_name, count) in tool_counts {
                tool_frequencies.insert(tool_name, count as f64 / total_tool_calls);
            }
        }

        let turns = events.len() as f64;
        Self {
            tool_frequencies,
            avg_tokens_per_turn: total_tokens / turns,
            avg_tool_calls_per_turn: total_tool_calls_per_turn / turns,
        }
    }
}

/// Detects drift by comparing recent role behavior against a reference window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftDetector {
    reference_distributions: HashMap<String, ActionDistribution>,
    window_size: usize,
}

impl DriftDetector {
    /// Create a detector with precomputed reference distributions.
    #[must_use]
    pub fn new(
        reference_distributions: HashMap<String, ActionDistribution>,
        window_size: usize,
    ) -> Self {
        Self {
            reference_distributions,
            window_size: window_size.max(1),
        }
    }

    /// Register or replace the reference distribution for a role.
    pub fn set_reference_distribution(
        &mut self,
        role: impl Into<String>,
        distribution: ActionDistribution,
    ) {
        self.reference_distributions
            .insert(role.into(), distribution);
    }

    /// Read the reference distribution for a role.
    #[must_use]
    pub fn reference_distribution(&self, role: &str) -> Option<&ActionDistribution> {
        self.reference_distributions.get(role)
    }

    /// Compute the composite Jensen-Shannon divergence for a role.
    ///
    /// Returns `0.0` when no reference exists for the role.
    #[must_use]
    pub fn compute_jsd(&self, role: &str, recent: &ActionDistribution) -> f64 {
        let Some(reference) = self.reference_distributions.get(role) else {
            return 0.0;
        };

        let tool_jsd = categorical_jsd(&reference.tool_frequencies, &recent.tool_frequencies);
        let token_jsd = scalar_jsd(reference.avg_tokens_per_turn, recent.avg_tokens_per_turn);
        let tool_call_jsd = scalar_jsd(
            reference.avg_tool_calls_per_turn,
            recent.avg_tool_calls_per_turn,
        );

        ((tool_jsd + token_jsd + tool_call_jsd) / 3.0).clamp(0.0, 1.0)
    }

    /// Check the recent window for behavioral drift.
    #[must_use]
    pub fn check_drift(
        &self,
        role: &str,
        recent_events: &[AgentEfficiencyEvent],
    ) -> Option<DriftAlert> {
        if !self.reference_distributions.contains_key(role) {
            return None;
        }

        let mut scoped_events: Vec<AgentEfficiencyEvent> = recent_events
            .iter()
            .filter(|event| event.role == role)
            .cloned()
            .collect();

        if scoped_events.is_empty() {
            return None;
        }

        if scoped_events.len() > self.window_size {
            let start = scoped_events.len() - self.window_size;
            scoped_events = scoped_events.split_off(start);
        }

        let recent_dist = ActionDistribution::from_events(&scoped_events);
        let jsd = self.compute_jsd(role, &recent_dist);

        if jsd > DRIFT_ALERT_THRESHOLD {
            Some(DriftAlert {
                role: role.to_string(),
                jsd,
                description: format!(
                    "Behavioral drift detected for role {role}: JSD {jsd:.3} exceeded {DRIFT_ALERT_THRESHOLD:.3}"
                ),
            })
        } else {
            None
        }
    }
}

fn categorical_jsd(left: &HashMap<String, f64>, right: &HashMap<String, f64>) -> f64 {
    let left = normalize_distribution(left);
    let right = normalize_distribution(right);

    if left.is_empty() && right.is_empty() {
        return 0.0;
    }

    let keys: HashSet<&str> = left
        .keys()
        .map(String::as_str)
        .chain(right.keys().map(String::as_str))
        .collect();

    let mut divergence = 0.0;
    for key in keys {
        let p = left.get(key).copied().unwrap_or(0.0);
        let q = right.get(key).copied().unwrap_or(0.0);
        let m = 0.5 * (p + q);
        divergence += 0.5 * kl_term(p, m) + 0.5 * kl_term(q, m);
    }

    divergence.clamp(0.0, 1.0)
}

fn scalar_jsd(reference: f64, recent: f64) -> f64 {
    let reference = sanitize_metric(reference);
    let recent = sanitize_metric(recent);
    let scale = reference.max(recent).max(1.0);
    let p = (reference / scale).clamp(0.0, 1.0);
    let q = (recent / scale).clamp(0.0, 1.0);

    bernoulli_jsd(p, q)
}

fn bernoulli_jsd(p: f64, q: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    let q = q.clamp(0.0, 1.0);
    let m = 0.5 * (p + q);

    (0.5 * kl_term(p, m)
        + 0.5 * kl_term(1.0 - p, 1.0 - m)
        + 0.5 * kl_term(q, m)
        + 0.5 * kl_term(1.0 - q, 1.0 - m))
    .clamp(0.0, 1.0)
}

fn normalize_distribution(distribution: &HashMap<String, f64>) -> HashMap<String, f64> {
    let total: f64 = distribution
        .values()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .sum();

    if total <= 0.0 {
        return HashMap::new();
    }

    distribution
        .iter()
        .filter_map(|(key, value)| {
            if value.is_finite() && *value > 0.0 {
                Some((key.clone(), value / total))
            } else {
                None
            }
        })
        .collect()
}

fn sanitize_metric(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn kl_term(p: f64, q: f64) -> f64 {
    if p <= 0.0 || q <= 0.0 {
        0.0
    } else {
        p * (p / q).log2()
    }
}

#[cfg(test)]
mod tests {
    use super::{ActionDistribution, DRIFT_ALERT_THRESHOLD, DriftDetector};
    use crate::efficiency::{AgentEfficiencyEvent, ToolCallMeta};

    use std::collections::HashMap;

    fn make_event(
        role: &str,
        input_tokens: u64,
        output_tokens: u64,
        reasoning_tokens: u64,
        tool_names: &[&str],
    ) -> AgentEfficiencyEvent {
        AgentEfficiencyEvent {
            agent_id: "agent-1".into(),
            role: role.into(),
            backend: "claude".into(),
            model: "claude-sonnet-4-5".into(),
            plan_id: "plan-1".into(),
            task_id: "task-1".into(),
            input_tokens,
            output_tokens,
            reasoning_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.15,
            cost_usd_without_cache: 0.15,
            prompt_sections: Vec::new(),
            total_prompt_tokens: input_tokens,
            system_prompt_tokens: 128,
            tools_available: 6,
            tools_used: tool_names.len() as u32,
            tool_calls: tool_names
                .iter()
                .map(|tool_name| ToolCallMeta {
                    tool_name: (*tool_name).to_string(),
                    duration_ms: 20,
                    result_tokens: 40,
                    succeeded: true,
                })
                .collect(),
            wall_time_ms: 1_000,
            duration_ms: 1_000,
            time_to_first_token_ms: 150,
            was_warm_start: true,
            iteration: 1,
            gate_passed: true,
            outcome: "success".into(),
            gate_errors: Vec::new(),
            model_used: "claude-sonnet-4-5".into(),
            frequency: roko_core::OperatingFrequency::Theta,
            strategy_attempted: "none".into(),
            timestamp: "2026-04-11T00:00:00Z".into(),
        }
    }

    fn make_detector(
        reference_events: &[AgentEfficiencyEvent],
        window_size: usize,
    ) -> DriftDetector {
        let mut references = HashMap::new();
        references.insert(
            "Implementer".to_string(),
            ActionDistribution::from_events(reference_events),
        );
        DriftDetector::new(references, window_size)
    }

    #[test]
    fn drift_detection_action_distribution_from_events_aggregates_tool_and_token_metrics() {
        let events = vec![
            make_event("Implementer", 600, 300, 100, &["Read", "Edit"]),
            make_event("Implementer", 500, 200, 0, &["Read"]),
        ];

        let distribution = ActionDistribution::from_events(&events);

        assert!((distribution.avg_tokens_per_turn - 850.0).abs() < 1e-9);
        assert!((distribution.avg_tool_calls_per_turn - 1.5).abs() < 1e-9);
        assert!((distribution.tool_frequencies["Read"] - (2.0 / 3.0)).abs() < 1e-9);
        assert!((distribution.tool_frequencies["Edit"] - (1.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn drift_detection_compute_jsd_is_zero_for_identical_distribution() {
        let reference_events = vec![
            make_event("Implementer", 700, 300, 50, &["Read", "Edit"]),
            make_event("Implementer", 650, 350, 50, &["Read", "Bash"]),
        ];
        let detector = make_detector(&reference_events, 8);
        let recent = ActionDistribution::from_events(&reference_events);

        let jsd = detector.compute_jsd("Implementer", &recent);

        assert!(jsd.abs() < 1e-12);
    }

    #[test]
    fn drift_detection_normal_variation_stays_below_threshold() {
        let reference_events: Vec<_> = (0..50)
            .map(|idx| {
                let tool_names = if idx % 5 == 0 {
                    vec!["Read", "Edit", "Bash"]
                } else if idx % 2 == 0 {
                    vec!["Read", "Edit"]
                } else {
                    vec!["Read", "Bash"]
                };
                make_event("Implementer", 720 + idx, 280 + idx, 30, &tool_names)
            })
            .collect();
        let detector = make_detector(&reference_events, 12);

        let recent_events: Vec<_> = (0..12)
            .map(|idx| {
                let tool_names = if idx % 4 == 0 {
                    vec!["Read", "Edit", "Bash"]
                } else if idx % 3 == 0 {
                    vec!["Read", "Bash"]
                } else {
                    vec!["Read", "Edit"]
                };
                make_event("Implementer", 735 + idx, 295 + idx, 35, &tool_names)
            })
            .collect();

        let alert = detector.check_drift("Implementer", &recent_events);
        let jsd = detector.compute_jsd(
            "Implementer",
            &ActionDistribution::from_events(&recent_events),
        );

        assert!(
            jsd < DRIFT_ALERT_THRESHOLD,
            "expected normal variation, got {jsd}"
        );
        assert!(alert.is_none());
    }

    #[test]
    fn drift_detection_significant_shift_triggers_alert() {
        let reference_events: Vec<_> = (0..50)
            .map(|idx| {
                let tool_names = if idx % 5 == 0 {
                    vec!["Read", "Edit"]
                } else {
                    vec!["Read"]
                };
                make_event("Implementer", 650 + idx, 250, 20, &tool_names)
            })
            .collect();
        let detector = make_detector(&reference_events, 10);

        let mut recent_events = vec![make_event("Reviewer", 500, 150, 0, &["Read"])];
        recent_events.extend((0..10).map(|idx| {
            let tool_names = if idx % 2 == 0 {
                vec!["Bash", "Edit", "Edit", "Write"]
            } else {
                vec!["Write", "Bash", "Write", "Edit"]
            };
            make_event("Implementer", 1_900 + idx * 20, 900, 250, &tool_names)
        }));

        let alert = detector
            .check_drift("Implementer", &recent_events)
            .expect("drift alert");

        assert!(
            alert.jsd > DRIFT_ALERT_THRESHOLD,
            "unexpected jsd {}",
            alert.jsd
        );
        assert!(alert.description.contains("JSD"));
        assert_eq!(alert.role, "Implementer");
    }
}
