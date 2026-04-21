//! Position-aware attention helpers for prompt placement.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Placement, PromptSection, SectionPriority};

/// Attention multiplier based on position within a context window.
///
/// This is the scaffold-level model described in the composition docs for
/// approximating the U-shaped "lost in the middle" curve.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PositionAttentionModel {
    /// Primacy contribution at the beginning of the prompt.
    pub primacy_weight: f64,
    /// Decay applied to the primacy contribution as position increases.
    pub primacy_decay: f64,
    /// Recency contribution near the end of the prompt.
    pub recency_weight: f64,
    /// Decay applied to the recency contribution as position approaches zero.
    pub recency_decay: f64,
    /// Baseline attention that remains across the full prompt.
    pub baseline: f64,
}

impl Default for PositionAttentionModel {
    fn default() -> Self {
        Self {
            primacy_weight: 0.35,
            primacy_decay: 3.0,
            recency_weight: 0.30,
            recency_decay: 3.0,
            baseline: 0.35,
        }
    }
}

impl PositionAttentionModel {
    /// Compute the attention multiplier for a normalized position in `[0.0, 1.0]`.
    #[must_use]
    pub fn attention_at(&self, normalized_pos: f64) -> f64 {
        let pos = normalized_pos.clamp(0.0, 1.0);
        let primacy = self.primacy_weight * (-self.primacy_decay * pos).exp();
        let recency = self.recency_weight * (-self.recency_decay * (1.0 - pos)).exp();
        (primacy + recency + self.baseline).clamp(0.0, 1.0)
    }

    /// Apply the attention multiplier to a base score.
    #[must_use]
    pub fn effective_score(&self, base_score: f64, normalized_pos: f64) -> f64 {
        base_score * self.attention_at(normalized_pos)
    }
}

/// Per-model fitted attention curves derived from prompt-placement experiments.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelAttentionCurves {
    /// Model id to fitted curve mapping.
    pub curves: HashMap<String, PositionAttentionModel>,
    /// Fallback curve used when a model-specific fit is unavailable.
    pub default_curve: PositionAttentionModel,
}

impl ModelAttentionCurves {
    /// Return the curve for one model, or the default curve when unknown.
    #[must_use]
    pub fn for_model(&self, model_id: &str) -> &PositionAttentionModel {
        self.curves.get(model_id).unwrap_or(&self.default_curve)
    }

    /// Persist the fitted curve set as JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if the curve set cannot be serialized or written.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

/// Adjust a score for the prompt placement zone it will occupy.
#[must_use]
pub const fn placement_adjusted_score(base_score: f64, placement: Placement) -> f64 {
    match placement {
        Placement::Start => base_score,
        Placement::End => base_score * 0.95,
        Placement::Middle => base_score * 0.70,
    }
}

/// Reassign non-critical sections toward higher-attention prompt edges.
///
/// Critical sections keep their existing placement. Remaining sections are
/// ranked by a cheap information-density proxy relative to the task query.
pub fn dynamic_placement(sections: &mut [PromptSection], query: &str) {
    let mut scored_indices = sections
        .iter()
        .enumerate()
        .filter(|(_, section)| section.priority != SectionPriority::Critical)
        .map(|(index, section)| (index, information_density(section, query)))
        .collect::<Vec<_>>();

    scored_indices.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });

    let total = scored_indices.len();
    for (rank, (index, _)) in scored_indices.into_iter().enumerate() {
        sections[index].placement = if rank < total.div_ceil(3) {
            Placement::Start
        } else if rank >= (2 * total) / 3 {
            Placement::End
        } else {
            Placement::Middle
        };
    }
}

fn information_density(section: &PromptSection, query: &str) -> f64 {
    let query_terms = tokenize(query);
    let section_terms = tokenize(&format!("{} {}", section.name, section.content));
    if section_terms.is_empty() {
        return 0.0;
    }

    let overlap = section_terms.intersection(&query_terms).count() as f64;
    let uniqueness = section_terms.len() as f64;
    let compactness = 1.0 / (section.content.len().max(1) as f64).sqrt();
    overlap * 0.7 + uniqueness.sqrt() * 0.2 + compactness * 10.0 * 0.1
}

fn tokenize(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_attention_model_has_u_shape() {
        let model = PositionAttentionModel::default();
        let start = model.attention_at(0.0);
        let mid = model.attention_at(0.5);
        let end = model.attention_at(1.0);

        assert!(start > mid);
        assert!(end > mid);
    }

    #[test]
    fn placement_adjustment_matches_expected_order() {
        let base = 1.0;
        assert!(
            placement_adjusted_score(base, Placement::Start)
                >= placement_adjusted_score(base, Placement::End)
        );
        assert!(
            placement_adjusted_score(base, Placement::End)
                > placement_adjusted_score(base, Placement::Middle)
        );
    }

    #[test]
    fn dynamic_placement_preserves_critical_sections() {
        let mut sections = vec![
            PromptSection::new("critical", "must keep at start")
                .with_priority(SectionPriority::Critical)
                .with_placement(Placement::Start),
            PromptSection::new("dense", "query query query focused section"),
            PromptSection::new("sparse", "background context"),
        ];

        dynamic_placement(&mut sections, "query focused");

        assert_eq!(sections[0].placement, Placement::Start);
        assert_ne!(sections[1].placement, Placement::Middle);
    }
}
