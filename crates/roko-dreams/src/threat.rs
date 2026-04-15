//! Threat simulation for dream consolidation.
//!
//! The module enumerates a small set of failure scenarios from observed
//! episodes and converts the high-severity ones into warning knowledge.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use roko_learn::episode_logger::Episode;
use roko_neuro::{KnowledgeEntry, KnowledgeKind, KnowledgeTier};
use serde::{Deserialize, Serialize};

/// One simulated threat scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreatScenario {
    /// Stable identifier derived from the failure pattern.
    pub id: String,
    /// Human-readable threat description.
    pub description: String,
    /// Estimated likelihood, normalized to `0.0..=1.0`.
    pub likelihood: f64,
    /// Estimated impact, normalized to `0.0..=1.0`.
    pub impact: f64,
    /// How hard it is to detect the threat before it causes damage.
    pub detection_difficulty: f64,
    /// Recommended mitigation.
    pub mitigation: String,
}

impl ThreatScenario {
    /// Compute a simple severity score.
    #[must_use]
    pub fn severity(&self) -> f64 {
        (self.likelihood * self.impact * (1.0 - self.detection_difficulty)).clamp(0.0, 1.0)
    }
}

/// Simulate threats from a batch of episodes.
#[must_use]
pub fn enumerate_threats(episodes: &[Episode]) -> Vec<ThreatScenario> {
    let mut by_key: BTreeMap<String, Vec<&Episode>> = BTreeMap::new();
    for episode in episodes {
        if episode.success {
            continue;
        }
        let key = threat_key(episode);
        by_key.entry(key).or_default().push(episode);
    }

    let total_failures = episodes
        .iter()
        .filter(|episode| !episode.success)
        .count()
        .max(1);
    let mut threats = Vec::new();
    for (key, failures) in by_key {
        let description = describe_threat(&failures);
        let likelihood = (failures.len() as f64 / total_failures as f64).clamp(0.05, 1.0);
        let impact = impact_score(&failures);
        let detection_difficulty = detection_difficulty(&failures);
        let mitigation = mitigation_for(&failures);
        threats.push(ThreatScenario {
            id: threat_id(&key, &description),
            description,
            likelihood,
            impact,
            detection_difficulty,
            mitigation,
        });
    }

    threats.sort_by(|left, right| {
        right
            .severity()
            .partial_cmp(&left.severity())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.description.cmp(&right.description))
    });
    threats
}

/// Convert threat scenarios into warning knowledge entries.
#[must_use]
pub fn threat_warning_entries(
    episodes: &[Episode],
    created_at: DateTime<Utc>,
) -> Vec<KnowledgeEntry> {
    let threats = enumerate_threats(episodes);
    let mut out = Vec::new();
    for threat in threats
        .into_iter()
        .filter(|threat| threat.severity() >= 0.20)
    {
        let source_episodes = source_episode_ids(episodes, &threat);
        let tags = vec![
            "dream".to_string(),
            "threat".to_string(),
            "warning".to_string(),
            "fmea".to_string(),
            "fta".to_string(),
        ];
        out.push(KnowledgeEntry {
            id: threat_entry_id(&threat, &source_episodes, &tags),
            kind: KnowledgeKind::Warning,
            source: Some("dream".to_string()),
            content: format!("{} Mitigation: {}.", threat.description, threat.mitigation),
            confidence: threat.severity().clamp(0.0, 1.0),
            confidence_weight: threat.severity().clamp(0.0, 1.0),
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes,
            tags,
            source_model: None,
            model_generality: 1.0,
            created_at,
            half_life_days: KnowledgeKind::Warning.default_half_life_days(),
            tier: KnowledgeTier::Working,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
        });
    }
    out
}

fn threat_key(episode: &Episode) -> String {
    let reason = episode
        .failure_reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .unwrap_or("unclassified");
    format!(
        "task:{}|model:{}|reason:{}",
        episode.task_id, episode.model, reason
    )
}

fn describe_threat(failures: &[&Episode]) -> String {
    let task_id = failures
        .first()
        .map(|episode| episode.task_id.as_str())
        .unwrap_or("unknown-task");
    let model = failures
        .first()
        .map(|episode| episode.model.as_str())
        .unwrap_or("unknown-model");
    let gates = failing_gates(failures);
    let reason = failures
        .first()
        .and_then(|episode| episode.failure_reason.as_deref())
        .unwrap_or("unknown failure");
    if gates.is_empty() {
        format!(
            "Task {} on model {} may repeatedly fail because {}.",
            task_id, model, reason
        )
    } else {
        format!(
            "Task {} on model {} may repeatedly fail because {} and the same gates keep failing: {}.",
            task_id, model, reason, gates
        )
    }
}

fn impact_score(failures: &[&Episode]) -> f64 {
    let tokens = failures
        .iter()
        .map(|episode| episode.tokens_used.max(1))
        .sum::<u64>() as f64;
    let duration = failures
        .iter()
        .map(|episode| episode.duration_secs.max(0.0))
        .sum::<f64>();
    ((tokens.log10().max(0.0) * 0.12) + (duration.log10().max(0.0) * 0.10) + 0.35).clamp(0.0, 1.0)
}

fn detection_difficulty(failures: &[&Episode]) -> f64 {
    let gate_count = failures
        .iter()
        .flat_map(|episode| &episode.gate_verdicts)
        .filter(|verdict| !verdict.passed)
        .count();
    let reason_available = failures.iter().any(|episode| {
        episode
            .failure_reason
            .as_deref()
            .is_some_and(|reason| !reason.trim().is_empty())
    });
    let mut difficulty: f64 = 0.35;
    if gate_count == 0 {
        difficulty += 0.2;
    }
    if !reason_available {
        difficulty += 0.2;
    }
    difficulty.clamp(0.0, 1.0)
}

fn mitigation_for(failures: &[&Episode]) -> String {
    let gates = failing_gates(failures);
    if !gates.is_empty() {
        format!(
            "tighten verification around {} and replay the earliest failure first",
            gates
        )
    } else if failures
        .first()
        .map(|episode| episode.model.as_str())
        .is_some_and(|model| model.contains("haiku"))
    {
        "escalate the model tier and re-run the fragile step under a stricter budget".to_string()
    } else {
        "insert a preflight guard and capture richer failure diagnostics".to_string()
    }
}

fn failing_gates(failures: &[&Episode]) -> String {
    let mut counts = BTreeMap::<String, usize>::new();
    for verdict in failures.iter().flat_map(|episode| &episode.gate_verdicts) {
        if !verdict.passed {
            *counts.entry(verdict.gate.clone()).or_insert(0) += 1;
        }
    }
    let mut gates: Vec<(String, usize)> = counts.into_iter().collect();
    gates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    gates
        .into_iter()
        .take(3)
        .map(|(gate, count)| format!("{gate} ({count})"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn source_episode_ids(episodes: &[Episode], threat: &ThreatScenario) -> Vec<String> {
    let mut out = Vec::new();
    for episode in episodes {
        if threat.description.contains(&episode.task_id)
            || threat.description.contains(&episode.model)
            || episode
                .failure_reason
                .as_deref()
                .is_some_and(|reason| threat.description.contains(reason))
        {
            out.push(episode.id.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn threat_id(key: &str, description: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    description.hash(&mut hasher);
    format!("dream-threat-{:016x}", hasher.finish())
}

fn threat_entry_id(threat: &ThreatScenario, source_episodes: &[String], tags: &[String]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    threat.id.hash(&mut hasher);
    threat.description.hash(&mut hasher);
    source_episodes.hash(&mut hasher);
    tags.hash(&mut hasher);
    format!("dream-threat-entry-{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(id: &str, task_id: &str, model: &str, reason: Option<&str>) -> Episode {
        let mut episode = Episode::new("agent", task_id);
        episode.id = id.to_string();
        episode.task_id = task_id.to_string();
        episode.model = model.to_string();
        episode.success = reason.is_none();
        episode.failure_reason = reason.map(ToOwned::to_owned);
        episode.tokens_used = 200;
        episode.duration_secs = 42.0;
        episode
    }

    #[test]
    fn enumerate_threats_ranks_repeated_failures() {
        let episodes = vec![
            episode("a", "task-1", "haiku", Some("timeout")),
            episode("b", "task-1", "haiku", Some("timeout")),
            episode("c", "task-2", "haiku", None),
        ];
        let threats = enumerate_threats(&episodes);
        assert_eq!(threats.len(), 1);
        assert!(threats[0].severity() > 0.0);
    }

    #[test]
    fn threat_warning_entries_emit_warning_kind() {
        let episodes = vec![
            episode("a", "task-1", "haiku", Some("timeout")),
            episode("b", "task-1", "haiku", Some("timeout")),
        ];
        let entries = threat_warning_entries(&episodes, Utc::now());
        assert!(!entries.is_empty());
        assert!(
            entries
                .iter()
                .all(|entry| entry.kind == KnowledgeKind::Warning)
        );
    }
}
