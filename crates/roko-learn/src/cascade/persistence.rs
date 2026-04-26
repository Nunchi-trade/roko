//! Snapshot persistence and migration helpers for the cascade router.

use roko_core::agent::AgentRole;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::types::StageTransition;

/// Persisted snapshot of cascade router state.
#[derive(Serialize, Deserialize)]
pub(crate) struct CascadeSnapshot {
    pub(crate) model_slugs: Vec<String>,
    #[serde(default)]
    pub(crate) role_table: HashMap<AgentRole, String>,
    pub(crate) confidence_stats: HashMap<String, PersistedModelStats>,
    /// Total observations across all models (used to restore cascade stage).
    ///
    /// Defaults to 0 for backward compatibility with snapshots written before
    /// this field was added; in that case `load_or_new` recomputes the total
    /// from the sum of per-model trials.
    #[serde(default)]
    pub(crate) total_observations: u64,
    #[serde(default)]
    pub(crate) stage_transitions: Vec<StageTransition>,
}

/// Serializable form of per-model confidence stats.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct PersistedModelStats {
    pub(crate) trials: u64,
    pub(crate) successes: u64,
    #[serde(default)]
    pub(crate) total_citations: u64,
    #[serde(default)]
    pub(crate) total_search_latency_ms: u64,
    #[serde(default)]
    pub(crate) total_cost_usd: f64,
    #[serde(default)]
    pub(crate) perplexity_requests: u64,
    #[serde(default)]
    pub(crate) total_gemini_thinking_tokens: u64,
    #[serde(default)]
    pub(crate) total_gemini_cached_tokens: u64,
    #[serde(default)]
    pub(crate) total_gemini_grounding_queries: u64,
    #[serde(default)]
    pub(crate) gemini_code_execution_successes: u64,
    #[serde(default)]
    pub(crate) gemini_code_execution_failures: u64,
    #[serde(default)]
    pub(crate) gemini_context_window_le_200k_requests: u64,
    #[serde(default)]
    pub(crate) gemini_context_window_gt_200k_requests: u64,
    #[serde(default)]
    pub(crate) gemini_requests: u64,
}

impl PersistedModelStats {
    pub(crate) fn weighted_half(self) -> Self {
        Self {
            trials: self.trials / 2,
            successes: self.successes / 2,
            ..self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VersionChange {
    Added(String),
    Removed(String),
    Upgraded { old: String, new: String },
}

pub(crate) fn detect_version_changes(
    persisted_slugs: &[String],
    current_slugs: &[String],
) -> Vec<VersionChange> {
    let mut changes = Vec::new();
    let persisted_set: HashSet<&str> = persisted_slugs.iter().map(String::as_str).collect();
    let current_set: HashSet<&str> = current_slugs.iter().map(String::as_str).collect();

    for slug in current_slugs {
        if !persisted_set.contains(slug.as_str()) {
            let prefix = slug
                .rsplit_once('-')
                .map_or(slug.as_str(), |(prefix, _)| prefix);
            if let Some(old) = persisted_slugs
                .iter()
                .find(|candidate| candidate.starts_with(prefix))
            {
                changes.push(VersionChange::Upgraded {
                    old: old.clone(),
                    new: slug.clone(),
                });
            } else {
                changes.push(VersionChange::Added(slug.clone()));
            }
        }
    }

    for slug in persisted_slugs {
        if !current_set.contains(slug.as_str()) {
            changes.push(VersionChange::Removed(slug.clone()));
        }
    }

    changes
}

pub(crate) fn migrated_confidence_stats(
    persisted_stats: &HashMap<String, PersistedModelStats>,
    changes: &[VersionChange],
    active_slugs: &[String],
) -> HashMap<String, PersistedModelStats> {
    let active_set: HashSet<&str> = active_slugs.iter().map(String::as_str).collect();
    let mut migrated = persisted_stats
        .iter()
        .filter(|(slug, _)| active_set.contains(slug.as_str()))
        .map(|(slug, stats)| (slug.clone(), *stats))
        .collect::<HashMap<_, _>>();

    for change in changes {
        if let VersionChange::Upgraded { old, new } = change {
            let Some(old_stats) = persisted_stats.get(old) else {
                continue;
            };
            let transferred = old_stats.weighted_half();
            if transferred.trials == 0 && transferred.successes == 0 {
                continue;
            }

            let entry = migrated
                .entry(new.clone())
                .or_insert(PersistedModelStats::default());
            entry.trials += transferred.trials;
            entry.successes += transferred.successes;
        }
    }

    migrated
}

pub(crate) fn remap_role_table_entry(slug: String, changes: &[VersionChange]) -> String {
    for change in changes {
        if let VersionChange::Upgraded { old, new } = change {
            if slug == *old {
                return new.clone();
            }
        }
    }

    slug
}
