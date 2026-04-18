//! Predictive-foraging helpers for context retrieval.

use std::time::{Duration, SystemTime};

use crate::{ContextChunk, ContextSource, TaskInput};

/// Calibrated gain-curve parameters for one context source.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceForagingProfile {
    /// Source this profile describes.
    pub source: ContextSource,
    /// Asymptotic relevance available from this source.
    pub g_max: f64,
    /// Saturation rate for the diminishing-returns curve.
    pub lambda: f64,
    /// Setup and switching cost for the source.
    pub travel_cost: f64,
}

/// Multi-patch foraging strategy for context assembly.
///
/// `ContextSource` is not hashable in the current workspace, so the runtime
/// representation stores per-source profiles directly instead of using a map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MultiPatchForager {
    /// Per-source gain-curve parameters and travel costs.
    pub source_profiles: Vec<SourceForagingProfile>,
    /// Current average gain rate across the environment.
    pub environment_rate: f64,
}

impl MultiPatchForager {
    /// Determine the visitation order by expected initial gain.
    #[must_use]
    pub fn optimal_order(&self) -> Vec<ContextSource> {
        let mut profiles = self.source_profiles.clone();
        profiles.sort_by(|left, right| {
            self.expected_initial_gain(&right.source)
                .partial_cmp(&self.expected_initial_gain(&left.source))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        profiles.into_iter().map(|profile| profile.source).collect()
    }

    fn expected_initial_gain(&self, source: &ContextSource) -> f64 {
        self.profile_for(source)
            .map(|profile| profile.g_max * profile.lambda)
            .unwrap_or(0.0)
    }

    /// Check whether the source's initial gain justifies visiting it.
    #[must_use]
    pub fn should_visit(&self, source: &ContextSource) -> bool {
        let Some(profile) = self.profile_for(source) else {
            return false;
        };
        self.expected_initial_gain(source) > self.environment_rate * profile.travel_cost.max(0.0)
    }

    /// Solve for the approximate number of iterations to spend in one source.
    #[must_use]
    pub fn optimal_iterations(&self, source: &ContextSource) -> usize {
        let Some(profile) = self.profile_for(source) else {
            return 1;
        };

        let mut lo = 1usize;
        let mut hi = 20usize;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let marginal = profile.g_max * profile.lambda * (-profile.lambda * mid as f64).exp();
            let threshold = self.environment_rate + profile.travel_cost / mid as f64;
            if marginal > threshold {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.clamp(1, 10)
    }

    fn profile_for(&self, source: &ContextSource) -> Option<&SourceForagingProfile> {
        self.source_profiles
            .iter()
            .find(|profile| profile.source == *source)
    }
}

/// A retrieval signal deposited after one successful retrieval episode.
#[derive(Clone, Debug, PartialEq)]
pub struct RetrievalSignal {
    /// Task category that consumed the retrieved entry.
    pub task_category: String,
    /// Knowledge or context entry identifier.
    pub entry_id: String,
    /// Relevance score assigned during retrieval.
    pub relevance: f64,
    /// Whether the downstream gate passed when the entry was included.
    pub gate_passed: bool,
    /// Timestamp used for decay.
    pub timestamp: SystemTime,
    /// Agent identifier that produced the signal.
    pub agent_id: String,
}

/// Apply a capped social-foraging boost to retrieved entries.
pub fn social_foraging_boost(
    candidate_entries: &mut [ContextChunk],
    recent_signals: &[RetrievalSignal],
    task_category: &str,
    decay_half_life: Duration,
) {
    let now = SystemTime::now();
    let half_life_secs = decay_half_life.as_secs_f64().max(1.0);

    for entry in candidate_entries {
        let Some(entry_id) = context_chunk_identifier(entry) else {
            continue;
        };

        let social_evidence = recent_signals
            .iter()
            .filter(|signal| signal.entry_id == entry_id && signal.task_category == task_category)
            .filter(|signal| signal.gate_passed)
            .map(|signal| {
                let age_secs = now
                    .duration_since(signal.timestamp)
                    .unwrap_or_default()
                    .as_secs_f64();
                let decay = 0.5_f64.powf(age_secs / half_life_secs);
                signal.relevance * decay
            })
            .sum::<f64>();

        entry.relevance += (social_evidence * 0.1).min(0.3);
    }
}

/// Estimate whether the retrieved context is sufficient for the task.
#[must_use]
pub fn estimate_context_sufficiency(retrieved_chunks: &[ContextChunk], task: &TaskInput) -> f64 {
    let mut task_terms = tokenize(&task.title);
    if let Some(description) = &task.description {
        task_terms.extend(tokenize(description));
    }
    for file in &task.files {
        task_terms.extend(tokenize(file));
    }
    for symbol in &task.symbols {
        task_terms.extend(tokenize(symbol));
    }

    if task_terms.is_empty() {
        return 0.0;
    }

    let covered = task_terms
        .iter()
        .filter(|term| {
            retrieved_chunks.iter().any(|chunk| {
                let content = chunk.content.to_ascii_lowercase();
                content.contains(term.as_str())
            })
        })
        .count();

    covered as f64 / task_terms.len() as f64
}

/// Stop when either the MVT ratio falls below 1.0 or sufficiency is high enough.
#[must_use]
pub fn should_stop_searching(mvt_ratio: f64, sufficiency: f64, sufficiency_threshold: f64) -> bool {
    mvt_ratio <= 1.0 || sufficiency >= sufficiency_threshold
}

fn context_chunk_identifier(chunk: &ContextChunk) -> Option<String> {
    match &chunk.source {
        ContextSource::KnowledgeEntry { entry_id, .. } => Some(entry_id.clone()),
        ContextSource::Episode { episode_id, .. } => Some(episode_id.clone()),
        ContextSource::InlineFile { path, lines } => {
            Some(format!("{path}:{}", lines.as_deref().unwrap_or("*")))
        }
        ContextSource::RecentSignal { signal_id, .. } => Some(signal_id.clone()),
        ContextSource::SymbolSignature { symbol, file } => Some(format!("{file}:{symbol}")),
        ContextSource::PriorTaskOutput { task_id } => Some(task_id.clone()),
        _ => None,
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_patch_ordering_prefers_higher_initial_gain() {
        let file_source = ContextSource::InlineFile {
            path: "src/lib.rs".into(),
            lines: None,
        };
        let knowledge_source = ContextSource::KnowledgeEntry {
            entry_id: "k1".into(),
            kind: "heuristic".into(),
            source: None,
        };
        let forager = MultiPatchForager {
            source_profiles: vec![
                SourceForagingProfile {
                    source: knowledge_source.clone(),
                    g_max: 0.9,
                    lambda: 0.25,
                    travel_cost: 0.1,
                },
                SourceForagingProfile {
                    source: file_source.clone(),
                    g_max: 0.8,
                    lambda: 0.5,
                    travel_cost: 0.2,
                },
            ],
            environment_rate: 0.05,
        };

        let order = forager.optimal_order();
        assert_eq!(order[0], file_source);
        assert_eq!(order[1], knowledge_source);
    }

    #[test]
    fn social_boost_is_capped() {
        let mut chunks = vec![ContextChunk {
            content: "retrieved entry".into(),
            source: ContextSource::KnowledgeEntry {
                entry_id: "entry-1".into(),
                kind: "heuristic".into(),
                source: None,
            },
            relevance: 0.2,
            track_record: None,
            confidence: None,
            recency: None,
            emotional_tag: None,
        }];
        let signals = vec![RetrievalSignal {
            task_category: "integration".into(),
            entry_id: "entry-1".into(),
            relevance: 5.0,
            gate_passed: true,
            timestamp: SystemTime::now(),
            agent_id: "agent-a".into(),
        }];

        social_foraging_boost(
            &mut chunks,
            &signals,
            "integration",
            Duration::from_secs(3600),
        );

        assert!((chunks[0].relevance - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn sufficiency_can_stop_search() {
        assert!(should_stop_searching(1.2, 0.9, 0.85));
        assert!(should_stop_searching(0.9, 0.1, 0.85));
        assert!(!should_stop_searching(1.2, 0.2, 0.85));
    }
}
