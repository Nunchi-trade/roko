//! Hypnagogia engine for sleep-onset creativity.
//!
//! The engine keeps the four-layer shape from the batch docs while remaining
//! small enough to slot into the existing dream-cycle scaffold.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use chrono::{DateTime, Utc};
use roko_learn::episode_logger::Episode;
use roko_neuro::{KnowledgeEntry, KnowledgeKind, KnowledgeTier};
use roko_primitives::hdc::text_fingerprint;
use serde::{Deserialize, Serialize};

/// Placeholder state for the thalamic gate layer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ThalamicGate {
    /// Minimum confidence retained by the gate before stochastic resonance.
    pub relevance_floor: f64,
    /// Fraction of low-confidence signals allowed through as noise.
    pub noise_floor: f64,
}

impl Default for ThalamicGate {
    fn default() -> Self {
        Self {
            relevance_floor: 0.45,
            noise_floor: 0.20,
        }
    }
}

/// Constraint-relaxation layer for associative search.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ExecutiveLoosener {
    /// Maximum neighborhood size to consider while loosening associations.
    pub neighborhood: usize,
    /// How aggressively the search should widen.
    pub looseness: f64,
}

impl Default for ExecutiveLoosener {
    fn default() -> Self {
        Self {
            neighborhood: 4,
            looseness: 0.35,
        }
    }
}

/// Random interrupt layer that breaks fixation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DaliInterrupt {
    /// How many signals to skip between injected interruptions.
    pub stride: usize,
    /// Probability-like weight that decides whether an interrupt is emitted.
    pub intensity: f64,
}

impl Default for DaliInterrupt {
    fn default() -> Self {
        Self {
            stride: 3,
            intensity: 0.55,
        }
    }
}

/// Final observer that keeps the most promising associations.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HomuncularObserver {
    /// Minimum score required for a candidate to survive.
    pub retention_floor: f64,
    /// Maximum number of candidate insights to keep.
    pub max_candidates: usize,
}

impl Default for HomuncularObserver {
    fn default() -> Self {
        Self {
            retention_floor: 0.40,
            max_candidates: 6,
        }
    }
}

/// Liminal creativity pipeline used during sleep onset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypnagogiaEngine {
    /// Thalamic gate settings.
    pub gate: ThalamicGate,
    /// Executive loosener settings.
    pub loosener: ExecutiveLoosener,
    /// Dali-style interrupt settings.
    pub interrupt: DaliInterrupt,
    /// Homuncular observer settings.
    pub observer: HomuncularObserver,
}

impl Default for HypnagogiaEngine {
    fn default() -> Self {
        Self {
            gate: ThalamicGate::default(),
            loosener: ExecutiveLoosener::default(),
            interrupt: DaliInterrupt::default(),
            observer: HomuncularObserver::default(),
        }
    }
}

impl HypnagogiaEngine {
    /// Stable subsystem id.
    pub const ID: crate::DreamsSubsystemId = crate::DreamsSubsystemId::Hypnagogia;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Hypnagogia";
    /// Marker string retained for compatibility with older summaries.
    pub const MARKER: &'static str = "roko-dreams subsystem: hypnagogia";

    /// Construct a default hypnagogia engine.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            gate: ThalamicGate {
                relevance_floor: 0.45,
                noise_floor: 0.20,
            },
            loosener: ExecutiveLoosener {
                neighborhood: 4,
                looseness: 0.35,
            },
            interrupt: DaliInterrupt {
                stride: 3,
                intensity: 0.55,
            },
            observer: HomuncularObserver {
                retention_floor: 0.40,
                max_candidates: 6,
            },
        }
    }

    /// Summary metadata for this subsystem.
    #[must_use]
    pub const fn summary(self) -> crate::DreamsSubsystemSummary {
        crate::DreamsSubsystemSummary::new(Self::ID, Self::LABEL, Self::MARKER)
    }

    /// Run the full hypnagogic pipeline and return candidate insights.
    #[must_use]
    pub fn run(
        &self,
        signals: &[KnowledgeEntry],
        episodes: &[Episode],
        created_at: DateTime<Utc>,
    ) -> Vec<KnowledgeEntry> {
        let gated = self.thalamic_gate(signals);
        let loosened = self.executive_loosen(gated, episodes, created_at);
        let interrupted = self.dali_interrupt(episodes, created_at);
        self.homuncular_observer(loosened.into_iter().chain(interrupted).collect())
    }

    /// Returns the compatibility marker string used by older summaries.
    #[must_use]
    pub const fn interrupt(self) -> &'static str {
        Self::MARKER
    }

    fn thalamic_gate(&self, signals: &[KnowledgeEntry]) -> Vec<KnowledgeEntry> {
        signals
            .iter()
            .filter(|signal| {
                signal.confidence >= self.gate.relevance_floor
                    || resonance_score(&signal.content) >= self.gate.noise_floor
            })
            .cloned()
            .collect()
    }

    fn executive_loosen(
        &self,
        signals: Vec<KnowledgeEntry>,
        episodes: &[Episode],
        created_at: DateTime<Utc>,
    ) -> Vec<KnowledgeEntry> {
        let mut out = Vec::new();
        let neighborhood = self.loosener.neighborhood.max(2);
        for (index, signal) in signals.iter().enumerate() {
            out.push(signal.clone());
            let neighborhood_signal = signals
                .iter()
                .skip(index + 1)
                .take(neighborhood - 1)
                .find(|candidate| shares_signal(signal, candidate))
                .cloned();
            if let Some(candidate) = neighborhood_signal {
                out.push(loosened_association(
                    signal,
                    &candidate,
                    self.loosener.looseness,
                    created_at,
                ));
            } else if let Some(episode) = episodes.get(index % episodes.len().max(1)) {
                out.push(interrupt_to_insight(signal, episode, created_at));
            }
        }
        out
    }

    fn dali_interrupt(
        &self,
        episodes: &[Episode],
        created_at: DateTime<Utc>,
    ) -> Vec<KnowledgeEntry> {
        let stride = self.interrupt.stride.max(1);
        episodes
            .iter()
            .enumerate()
            .filter_map(|(index, episode)| {
                if index % stride != 0 {
                    return None;
                }
                let weight = resonance_score(&episode_summary(episode));
                if weight < self.interrupt.intensity {
                    return None;
                }
                Some(dali_insight(episode, created_at))
            })
            .collect()
    }

    fn homuncular_observer(&self, candidates: Vec<KnowledgeEntry>) -> Vec<KnowledgeEntry> {
        let mut ranked = candidates
            .into_iter()
            .map(|candidate| {
                let score = candidate.confidence
                    + novelty_score(&candidate.content)
                    + candidate.source_episodes.len().min(4) as f64 * 0.04;
                (candidate, score)
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.id.cmp(&right.0.id))
        });

        let mut seen = BTreeSet::new();
        ranked
            .into_iter()
            .filter_map(|(candidate, score)| {
                if score < self.observer.retention_floor {
                    return None;
                }
                if !seen.insert(candidate.id.clone()) {
                    return None;
                }
                Some(candidate)
            })
            .take(self.observer.max_candidates)
            .collect()
    }
}

fn resonance_score(text: &str) -> f64 {
    let fingerprint = text_fingerprint(text);
    let bytes = fingerprint.to_bytes();
    let sum: u64 = bytes.iter().map(|byte| u64::from(*byte)).sum();
    (sum % 100) as f64 / 100.0
}

fn novelty_score(text: &str) -> f64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    let value = hasher.finish();
    ((value % 10_000) as f64 / 10_000.0).clamp(0.0, 1.0)
}

fn shares_signal(left: &KnowledgeEntry, right: &KnowledgeEntry) -> bool {
    if left.kind != right.kind {
        return false;
    }
    left.tags.iter().any(|tag| right.tags.contains(tag))
}

fn loosened_association(
    left: &KnowledgeEntry,
    right: &KnowledgeEntry,
    looseness: f64,
    created_at: DateTime<Utc>,
) -> KnowledgeEntry {
    let mut source_episodes = left.source_episodes.clone();
    source_episodes.extend(right.source_episodes.iter().cloned());
    source_episodes.sort();
    source_episodes.dedup();
    let mut tags = left.tags.clone();
    tags.extend(right.tags.iter().cloned());
    tags.push("hypnagogia".to_string());
    tags.push("loosened-association".to_string());
    tags.sort();
    tags.dedup();
    KnowledgeEntry {
        id: hypnagogia_id(
            "loosened",
            &left.content,
            &right.content,
            &source_episodes,
            &tags,
        ),
        kind: left.kind,
        source: Some("dream".to_string()),
        content: format!(
            "Sleep-onset association: {} / {}",
            left.content, right.content
        ),
        confidence: (left.confidence + right.confidence) * 0.5 * (0.5 + looseness),
        confidence_weight: (left.confidence + right.confidence) * 0.5 * (0.5 + looseness),
        refuted_insight_id: None,
        refutation_evidence: None,
        source_episodes,
        tags,
        source_model: left
            .source_model
            .clone()
            .or_else(|| right.source_model.clone()),
        model_generality: 0.9,
        created_at,
        half_life_days: left.half_life_days.max(right.half_life_days),
        tier: KnowledgeTier::Working,
        emotional_tag: None,
        emotional_provenance: None,
        hdc_vector: None,

        confirmation_count: 0,

        distinct_contexts: Vec::new(),

        deprecated: false,
        balance: 1.0,
        frozen: false,
        catalytic_score: 0,
    }
}

fn interrupt_to_insight(
    signal: &KnowledgeEntry,
    episode: &Episode,
    created_at: DateTime<Utc>,
) -> KnowledgeEntry {
    let mut source_episodes = signal.source_episodes.clone();
    source_episodes.push(episode.id.clone());
    source_episodes.sort();
    source_episodes.dedup();
    let mut tags = signal.tags.clone();
    tags.push("dali-interrupt".to_string());
    tags.push("hypnagogia".to_string());
    tags.sort();
    tags.dedup();
    KnowledgeEntry {
        id: hypnagogia_id(
            "interrupt",
            &signal.content,
            &episode.id,
            &source_episodes,
            &tags,
        ),
        kind: signal.kind,
        source: Some("dream".to_string()),
        content: format!(
            "Interruptive association from {}: maybe the repeating pattern is {}",
            episode.task_id,
            episode.failure_reason.as_deref().unwrap_or("unclear")
        ),
        confidence: (signal.confidence * 0.8).clamp(0.0, 1.0),
        confidence_weight: (signal.confidence * 0.8).clamp(0.0, 1.0),
        refuted_insight_id: None,
        refutation_evidence: None,
        source_episodes,
        tags,
        source_model: Some(episode.model.clone()),
        model_generality: 0.85,
        created_at,
        half_life_days: signal.half_life_days,
        tier: KnowledgeTier::Working,
        emotional_tag: None,
        emotional_provenance: None,
        hdc_vector: None,

        confirmation_count: 0,

        distinct_contexts: Vec::new(),

        deprecated: false,
        balance: 1.0,
        frozen: false,
        catalytic_score: 0,
    }
}

fn dali_insight(episode: &Episode, created_at: DateTime<Utc>) -> KnowledgeEntry {
    let content = format!(
        "Dali interrupt: if {} keeps failing, try a wider search around {} with the {} model.",
        episode.task_id,
        episode
            .failure_reason
            .as_deref()
            .unwrap_or("the current assumption"),
        episode.model
    );
    let tags = vec![
        "dream".to_string(),
        "hypnagogia".to_string(),
        "dali-interrupt".to_string(),
        "creative-break".to_string(),
    ];
    let source_episodes = vec![episode.id.clone()];
    KnowledgeEntry {
        id: hypnagogia_id("dali", &content, &episode.id, &source_episodes, &tags),
        kind: KnowledgeKind::Insight,
        source: Some("dream".to_string()),
        content,
        confidence: 0.70,
        confidence_weight: 0.70,
        refuted_insight_id: None,
        refutation_evidence: None,
        source_episodes,
        tags,
        source_model: Some(episode.model.clone()),
        model_generality: 0.8,
        created_at,
        half_life_days: KnowledgeKind::Insight.default_half_life_days(),
        tier: KnowledgeTier::Working,
        emotional_tag: None,
        emotional_provenance: None,
        hdc_vector: None,

        confirmation_count: 0,

        distinct_contexts: Vec::new(),

        deprecated: false,
        balance: 1.0,
        frozen: false,
        catalytic_score: 0,
    }
}

fn episode_summary(episode: &Episode) -> String {
    format!(
        "task_id={} model={} outcome={} failure_reason={}",
        episode.task_id,
        episode.model,
        if episode.success {
            "success"
        } else {
            "failure"
        },
        episode.failure_reason.as_deref().unwrap_or(""),
    )
}

fn hypnagogia_id(
    kind: &str,
    left: &str,
    right: &str,
    source_episodes: &[String],
    tags: &[String],
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    left.hash(&mut hasher);
    right.hash(&mut hasher);
    source_episodes.hash(&mut hasher);
    tags.hash(&mut hasher);
    format!("dream-hypnagogia-{:016x}", hasher.finish())
}

impl HypnagogiaEngine {
    /// Returns the compatibility marker string used by older summaries.
    #[must_use]
    pub const fn replay(self) -> &'static str {
        Self::MARKER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_engine_identity_and_legacy_markers_remain_stable() {
        let engine = HypnagogiaEngine::new();

        assert_eq!(engine, HypnagogiaEngine::default());
        assert_eq!(HypnagogiaEngine::ID, crate::DreamsSubsystemId::Hypnagogia);
        assert_eq!(HypnagogiaEngine::LABEL, "Hypnagogia");
        assert_eq!(
            HypnagogiaEngine::new().summary(),
            crate::DreamsSubsystemSummary::new(
                HypnagogiaEngine::ID,
                HypnagogiaEngine::LABEL,
                HypnagogiaEngine::MARKER,
            )
        );
        assert_eq!(
            HypnagogiaEngine::new().interrupt(),
            HypnagogiaEngine::MARKER
        );
        assert_eq!(HypnagogiaEngine::new().replay(), HypnagogiaEngine::MARKER);
    }

    #[test]
    fn pipeline_emits_bounded_candidate_insights() {
        let engine = HypnagogiaEngine::default();
        let signals = vec![
            KnowledgeEntry {
                id: "sig-1".to_string(),
                kind: KnowledgeKind::Insight,
                source: Some("dream".to_string()),
                content: "recurrent compile failure".to_string(),
                confidence: 1.0,
                confidence_weight: 1.0,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-1".to_string()],
                tags: vec!["compile".to_string(), "dream".to_string()],
                source_model: Some("claude-haiku-4-5".to_string()),
                model_generality: 1.0,
                created_at: Utc::now(),
                half_life_days: 30.0,
                tier: KnowledgeTier::Working,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
                balance: 1.0,
                frozen: false,
                catalytic_score: 0,
            },
            KnowledgeEntry {
                id: "sig-2".to_string(),
                kind: KnowledgeKind::Insight,
                source: Some("dream".to_string()),
                content: "recurrent test failure".to_string(),
                confidence: 1.0,
                confidence_weight: 1.0,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-2".to_string()],
                tags: vec!["test".to_string(), "dream".to_string()],
                source_model: Some("claude-haiku-4-5".to_string()),
                model_generality: 1.0,
                created_at: Utc::now(),
                half_life_days: 30.0,
                tier: KnowledgeTier::Working,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
                balance: 1.0,
                frozen: false,
                catalytic_score: 0,
            },
            KnowledgeEntry {
                id: "sig-3".to_string(),
                kind: KnowledgeKind::Insight,
                source: Some("dream".to_string()),
                content: "recurrent review failure".to_string(),
                confidence: 1.0,
                confidence_weight: 1.0,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-3".to_string()],
                tags: vec!["review".to_string(), "dream".to_string()],
                source_model: Some("claude-haiku-4-5".to_string()),
                model_generality: 1.0,
                created_at: Utc::now(),
                half_life_days: 30.0,
                tier: KnowledgeTier::Working,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
                balance: 1.0,
                frozen: false,
                catalytic_score: 0,
            },
            KnowledgeEntry {
                id: "sig-4".to_string(),
                kind: KnowledgeKind::Insight,
                source: Some("dream".to_string()),
                content: "recurrent deployment failure".to_string(),
                confidence: 1.0,
                confidence_weight: 1.0,
                refuted_insight_id: None,
                refutation_evidence: None,
                source_episodes: vec!["ep-4".to_string()],
                tags: vec!["deploy".to_string(), "dream".to_string()],
                source_model: Some("claude-haiku-4-5".to_string()),
                model_generality: 1.0,
                created_at: Utc::now(),
                half_life_days: 30.0,
                tier: KnowledgeTier::Working,
                emotional_tag: None,
                emotional_provenance: None,
                hdc_vector: None,

                confirmation_count: 0,

                distinct_contexts: Vec::new(),

                deprecated: false,
                balance: 1.0,
                frozen: false,
                catalytic_score: 0,
            },
        ];
        let output = engine.run(&signals, &[], Utc::now());

        assert_eq!(output.len(), engine.observer.max_candidates);
        assert!(
            output
                .iter()
                .any(|entry| entry.content.starts_with("Sleep-onset association:"))
        );
        assert!(
            output
                .iter()
                .all(|entry| entry.source.as_deref() == Some("dream"))
        );
        assert!(
            output
                .iter()
                .any(|entry| entry.id.starts_with("dream-hypnagogia-"))
        );

        let unique_ids = output
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique_ids.len(), output.len());
    }
}
