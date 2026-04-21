//! Threat rehearsal: simulate failure scenarios and practice recovery paths.
//!
//! Given a set of episodes, the rehearsal engine generates threats from the
//! fault tree (via [`enumerate_threats`]), constructs hypothetical scenarios
//! for each, simulates recovery by matching against known mitigations, and
//! records outcomes as episodes for future learning.

use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use roko_learn::episode_logger::{Episode, GateVerdict};
use serde::{Deserialize, Serialize};

use crate::threat::{ThreatScenario, enumerate_threats};

/// Maximum number of scenarios rehearsed per cycle to bound compute.
const MAX_REHEARSALS_PER_CYCLE: usize = 20;

/// Outcome of rehearsing a single threat scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RehearsalOutcome {
    /// Threat that was rehearsed.
    pub threat_id: String,
    /// Human-readable description of the simulated scenario.
    pub scenario: String,
    /// Whether the simulated recovery succeeded.
    pub recovery_succeeded: bool,
    /// Simulated recovery path taken.
    pub recovery_path: String,
    /// Confidence in the recovery (0.0..=1.0).
    pub confidence: f64,
    /// When the rehearsal was performed.
    pub rehearsed_at: DateTime<Utc>,
}

/// Summary of a threat rehearsal cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RehearsalReport {
    /// When the rehearsal cycle started.
    pub started_at: DateTime<Utc>,
    /// When the rehearsal cycle completed.
    pub completed_at: DateTime<Utc>,
    /// Number of threats evaluated.
    pub threats_evaluated: usize,
    /// Number of rehearsals performed (bounded by max).
    pub rehearsals_performed: usize,
    /// Individual rehearsal outcomes.
    pub outcomes: Vec<RehearsalOutcome>,
    /// Episodes generated from rehearsal for future learning.
    pub generated_episodes: Vec<Episode>,
}

/// Run threat rehearsal against a batch of episodes.
///
/// Generates threats from the failure patterns, rehearses recovery for the
/// top-severity threats (up to `max_scenarios`), and produces synthetic
/// episodes recording the rehearsal outcomes.
#[must_use]
pub fn rehearse_threats(
    episodes: &[Episode],
    max_scenarios: Option<usize>,
    now: DateTime<Utc>,
) -> RehearsalReport {
    let started_at = now;
    let threats = enumerate_threats(episodes);
    let threats_evaluated = threats.len();
    let limit = max_scenarios.unwrap_or(MAX_REHEARSALS_PER_CYCLE);

    let mut outcomes = Vec::new();
    let mut generated_episodes = Vec::new();

    for threat in threats.iter().take(limit) {
        let outcome = rehearse_single(threat, now);
        let episode = outcome_to_episode(&outcome, threat);
        generated_episodes.push(episode);
        outcomes.push(outcome);
    }

    RehearsalReport {
        started_at,
        completed_at: Utc::now(),
        threats_evaluated,
        rehearsals_performed: outcomes.len(),
        outcomes,
        generated_episodes,
    }
}

/// Rehearse a single threat scenario.
fn rehearse_single(threat: &ThreatScenario, now: DateTime<Utc>) -> RehearsalOutcome {
    // Construct a hypothetical scenario description from the threat.
    let scenario = format!(
        "Simulated failure: {}. Severity={:.2}, likelihood={:.2}, impact={:.2}.",
        threat.description,
        threat.severity(),
        threat.likelihood,
        threat.impact
    );

    // Simulate recovery: if the threat has a clear mitigation and the
    // detection difficulty is low enough, the recovery is considered
    // successful. This is a heuristic simulation, not a real execution.
    let recovery_feasible = threat.detection_difficulty < 0.7
        && !threat.mitigation.is_empty()
        && threat.severity() < 0.8;

    let recovery_path = if recovery_feasible {
        format!("Apply mitigation: {}.", threat.mitigation)
    } else {
        format!(
            "Mitigation '{}' insufficient — escalate to human review.",
            threat.mitigation
        )
    };

    let confidence = if recovery_feasible {
        (1.0 - threat.detection_difficulty) * (1.0 - threat.severity() * 0.5)
    } else {
        (0.1 + (1.0 - threat.severity()) * 0.2).clamp(0.0, 0.4)
    };

    RehearsalOutcome {
        threat_id: threat.id.clone(),
        scenario,
        recovery_succeeded: recovery_feasible,
        recovery_path,
        confidence: confidence.clamp(0.0, 1.0),
        rehearsed_at: now,
    }
}

/// Convert a rehearsal outcome into a synthetic episode for learning.
fn outcome_to_episode(outcome: &RehearsalOutcome, threat: &ThreatScenario) -> Episode {
    let mut episode = Episode::new("dream-rehearsal", &format!("rehearsal-{}", threat.id));
    episode.id = rehearsal_episode_id(outcome);
    episode.success = outcome.recovery_succeeded;
    episode.model = "dream-rehearsal".to_string();
    episode.tokens_used = 0;
    episode.duration_secs = 0.0;
    episode.timestamp = outcome.rehearsed_at;
    episode.started_at = outcome.rehearsed_at;
    episode.completed_at = outcome.rehearsed_at;

    if !outcome.recovery_succeeded {
        episode.failure_reason = Some(format!("rehearsal failed: {}", outcome.recovery_path));
    }

    // Add a synthetic gate verdict representing the rehearsal assessment.
    episode.gate_verdicts.push(GateVerdict {
        gate: "threat-rehearsal".to_string(),
        passed: outcome.recovery_succeeded,
        signature: Some(format!(
            "confidence={:.2}: {}",
            outcome.confidence, outcome.scenario
        )),
    });

    episode
}

fn rehearsal_episode_id(outcome: &RehearsalOutcome) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    outcome.threat_id.hash(&mut hasher);
    outcome.scenario.hash(&mut hasher);
    outcome.rehearsed_at.hash(&mut hasher);
    format!("rehearsal-{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(id: &str, task_id: &str, model: &str, reason: Option<&str>) -> Episode {
        let mut ep = Episode::new("agent", task_id);
        ep.id = id.to_string();
        ep.model = model.to_string();
        ep.success = reason.is_none();
        ep.failure_reason = reason.map(ToOwned::to_owned);
        ep.tokens_used = 200;
        ep.duration_secs = 10.0;
        ep
    }

    #[test]
    fn rehearsal_produces_bounded_outcomes() {
        let episodes = vec![
            episode("a", "task-1", "haiku", Some("timeout")),
            episode("b", "task-1", "haiku", Some("timeout")),
            episode("c", "task-2", "sonnet", Some("compile error")),
        ];
        let report = rehearse_threats(&episodes, Some(5), Utc::now());
        assert!(report.rehearsals_performed <= 5);
        assert_eq!(report.outcomes.len(), report.generated_episodes.len());
        assert!(report.threats_evaluated >= 1);
    }

    #[test]
    fn rehearsal_generates_episodes() {
        let episodes = vec![
            episode("a", "task-1", "haiku", Some("timeout")),
            episode("b", "task-1", "haiku", Some("timeout")),
        ];
        let report = rehearse_threats(&episodes, None, Utc::now());
        assert!(!report.generated_episodes.is_empty());
        for ep in &report.generated_episodes {
            assert!(ep.id.starts_with("rehearsal-"));
            assert_eq!(ep.model, "dream-rehearsal");
            assert!(!ep.gate_verdicts.is_empty());
        }
    }

    #[test]
    fn no_failures_produces_empty_rehearsal() {
        let episodes = vec![{
            let mut ep = Episode::new("agent", "task-1");
            ep.success = true;
            ep
        }];
        let report = rehearse_threats(&episodes, None, Utc::now());
        assert_eq!(report.threats_evaluated, 0);
        assert_eq!(report.rehearsals_performed, 0);
        assert!(report.outcomes.is_empty());
    }

    #[test]
    fn rehearsal_episode_ids_are_stable() {
        let episodes = vec![
            episode("a", "task-1", "haiku", Some("timeout")),
            episode("b", "task-1", "haiku", Some("timeout")),
        ];
        let now = Utc::now();
        let r1 = rehearse_threats(&episodes, None, now);
        let r2 = rehearse_threats(&episodes, None, now);
        assert_eq!(
            r1.generated_episodes
                .iter()
                .map(|e| &e.id)
                .collect::<Vec<_>>(),
            r2.generated_episodes
                .iter()
                .map(|e| &e.id)
                .collect::<Vec<_>>(),
        );
    }
}
