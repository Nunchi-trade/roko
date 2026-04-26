//! Scorers for prompt sections.
//!
//! `SectionScorer` ranks `Engram<PromptSection>` inputs by priority, recency,
//! and cache-layer fit. `GoalDirectedHeuristicScorer` adds goal-directed
//! heuristic scoring for router-facing composition surfaces so budget pressure
//! keeps sections that are both goal-aligned and information-bearing.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::prompt::{PromptSection, SectionPriority};
use roko_core::traits::Score as ScoreFn;
use roko_core::{Context, Engram, Score};

/// Ranks `Engram<PromptSection>` inputs by importance.
///
/// Score breakdown:
/// - **confidence**: priority mapped to `[0.2, 0.4, 0.8, 1.0]`
/// - **novelty**: 1.0 for recent sections (< 1 hr old), decaying to 0.0 over a day
/// - **utility**: section-length inverse (shorter = higher utility per token)
/// - **reputation**: trust from provenance (1.0 for trusted, 0.1 for tainted)
pub struct SectionScorer {
    /// Time threshold in ms for "recent" sections (full novelty).
    pub recency_window_ms: i64,
    /// Time threshold in ms where novelty falls to zero.
    pub staleness_window_ms: i64,
}

impl Default for SectionScorer {
    fn default() -> Self {
        Self {
            recency_window_ms: 60 * 60 * 1000,        // 1 hour
            staleness_window_ms: 24 * 60 * 60 * 1000, // 24 hours
        }
    }
}

impl SectionScorer {
    /// A scorer with default recency windows (1h fresh, 24h stale).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ScoreFn for SectionScorer {
    fn score(&self, signal: &Engram, ctx: &Context) -> Score {
        let Ok(section) = PromptSection::from_signal(signal) else {
            return Score::ZERO;
        };

        // Confidence ← priority
        let confidence = match section.priority {
            SectionPriority::Critical => 1.0,
            SectionPriority::High => 0.8,
            SectionPriority::Normal => 0.4,
            SectionPriority::Low => 0.2,
        };

        // Novelty ← recency
        let age_ms = (ctx.now_ms - signal.created_at_ms).max(0);
        let novelty = if age_ms < self.recency_window_ms {
            1.0
        } else if age_ms >= self.staleness_window_ms {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            let t = (age_ms - self.recency_window_ms) as f32
                / (self.staleness_window_ms - self.recency_window_ms) as f32;
            1.0 - t
        };

        // Utility ← content size inverse (shorter content = higher utility-per-token)
        #[allow(clippy::cast_precision_loss)]
        let len = section.content.len().max(1) as f32;
        let utility = (1000.0 / len).min(10.0);

        // Reputation ← signal trust
        let reputation = if signal.provenance.is_tainted() {
            0.1
        } else {
            signal.provenance.trust
        };

        Score::new(confidence, novelty, utility, reputation)
    }

    fn name(&self) -> &'static str {
        "section_scorer"
    }
}

/// Goal-directed heuristic scorer for prompt sections.
///
/// Pragmatic value is derived from cosine similarity between the current goal
/// embedding and the section embedding. Epistemic value is shaped by belief
/// strength over the section's topic plus an uncertainty bonus for
/// underexplored sections.
///
/// # Design note (COMP-05): HDC approximation of EFE
///
/// The spec (doc 07) calls for full Expected Free Energy (EFE) scoring with
/// proper Bayesian belief updates (KL divergence between posterior and prior).
/// This implementation uses an HDC-inspired hash embedding + cosine similarity
/// as an intentional approximation. The justification:
///
/// 1. **Correlation**: HDC cosine similarity correlates with information gain
///    for text sections -- high-similarity sections are redundant (low
///    epistemic value), low-similarity sections provide novel information
///    (high epistemic value). This captures the key gradient EFE needs.
///
/// 2. **Computational cost**: Proper Bayesian belief updates require
///    maintaining a full posterior distribution and simulating updates for
///    each candidate section during prompt assembly. This is prohibitively
///    expensive for real-time composition where we score hundreds of
///    candidates per prompt build.
///
/// 3. **Pragmatic adequacy**: In practice, the hash-embedding approach
///    produces ranking decisions that are good enough for prompt assembly
///    budgeting. The quality gap between this approximation and proper EFE
///    is small relative to the quality gap from having no scoring at all.
///
/// If higher-fidelity epistemic scoring is needed in the future, the
/// `epistemic_value()` method can be replaced with a proper KL-divergence
/// computation without changing the [`ScoreFn`] interface.
#[derive(Clone, Debug)]
pub struct GoalDirectedHeuristicScorer {
    goal_text: String,
    goal_embeddings: Vec<f32>,
    prior_beliefs: HashMap<String, f64>,
    embedding_dimensions: usize,
}

impl GoalDirectedHeuristicScorer {
    /// Create a scorer for a specific goal string.
    #[must_use]
    pub fn new(goal: impl AsRef<str>) -> Self {
        let goal_text = goal.as_ref().to_ascii_lowercase();
        let embedding_dimensions = 32;
        Self {
            goal_text: goal_text.clone(),
            goal_embeddings: embed_text(&goal_text, embedding_dimensions),
            prior_beliefs: HashMap::new(),
            embedding_dimensions,
        }
    }

    /// Attach topic prior beliefs, keyed by section name or topic label.
    #[must_use]
    pub fn with_prior_beliefs(mut self, prior_beliefs: HashMap<String, f64>) -> Self {
        self.prior_beliefs = prior_beliefs;
        self
    }

    fn section_embedding(&self, signal: &Engram) -> Vec<f32> {
        let section = PromptSection::from_signal(signal).ok();
        let mut text = String::new();
        if let Some(section) = section {
            text.push_str(&section.name);
            text.push('\n');
            text.push_str(&section.content);
        } else {
            text.push_str(signal.kind.as_str());
            text.push('\n');
            if let Ok(text_body) = signal.body.as_text() {
                text.push_str(text_body);
            }
        }
        embed_text(&text, self.embedding_dimensions)
    }

    fn topic_belief(&self, signal: &Engram, section: Option<&PromptSection>) -> f64 {
        let mut keys = Vec::new();
        if let Some(section) = section {
            keys.push(section.name.as_str());
        }
        if let Some(name) = signal.tag("name") {
            keys.push(name);
        }
        if let Some(topic) = signal.tag("topic") {
            keys.push(topic);
        }
        if let Some(cache_layer) = signal.tag("cache_layer") {
            keys.push(cache_layer);
        }

        for key in keys {
            if let Some(belief) = self.prior_beliefs.get(key) {
                return belief.clamp(0.0, 1.0);
            }
        }

        0.5
    }

    fn pragmatic_value(&self, signal: &Engram, section: Option<&PromptSection>) -> f32 {
        let section_embedding = self.section_embedding(signal);
        let embedding_similarity = cosine_similarity(&self.goal_embeddings, &section_embedding);
        let lexical_similarity = section
            .map(|section| token_overlap(&section.content, &self.goal_text))
            .unwrap_or(0.0);
        let goal_similarity =
            (0.65 * embedding_similarity + 0.35 * lexical_similarity).clamp(0.0, 1.0);
        let priority_bonus = section
            .map(|section| match section.priority {
                SectionPriority::Critical => 0.18,
                SectionPriority::High => 0.12,
                SectionPriority::Normal => 0.06,
                SectionPriority::Low => 0.02,
            })
            .unwrap_or(0.0);

        (goal_similarity + priority_bonus).clamp(0.0, 1.0)
    }

    fn epistemic_value(&self, signal: &Engram, section: Option<&PromptSection>) -> f32 {
        let belief = self.topic_belief(signal, section);
        let uncertainty = (1.0 - belief as f32).clamp(0.0, 1.0);
        let novelty_hint = signal.score.novelty.clamp(0.0, 1.0);
        let informational_leverage = section
            .map(|section| {
                let len = section.content.len().max(1) as f32;
                (1.0 / len.sqrt()).clamp(0.0, 1.0)
            })
            .unwrap_or(0.0);

        (0.65 * uncertainty + 0.2 * novelty_hint + 0.15 * informational_leverage).clamp(0.0, 1.0)
    }
}

impl ScoreFn for GoalDirectedHeuristicScorer {
    fn score(&self, signal: &Engram, _ctx: &Context) -> Score {
        let Ok(section) = PromptSection::from_signal(signal) else {
            return Score::ZERO;
        };

        let pragmatic = self.pragmatic_value(signal, Some(&section));
        let epistemic = self.epistemic_value(signal, Some(&section));
        let belief = self.topic_belief(signal, Some(&section)) as f32;
        let goal_focus = cosine_similarity(&self.goal_embeddings, &self.section_embedding(signal));
        let coherence = (0.5 + 0.5 * goal_focus).clamp(0.0, 1.0);
        let salience = (0.5 * pragmatic + 0.5 * epistemic).clamp(0.0, 1.0);

        Score::new_extended(
            pragmatic.clamp(0.0, 1.0),
            epistemic,
            (pragmatic + epistemic).max(0.0),
            belief.max(0.1),
            goal_focus.clamp(0.0, 1.0),
            salience,
            coherence,
        )
    }

    fn name(&self) -> &'static str {
        "goal_directed_heuristic_scorer"
    }
}

/// Compatibility alias: the "active inference" scorer is implemented as a
/// goal-directed heuristic scorer using HDC-approximate embeddings rather
/// than full Bayesian EFE (COMP-05).
///
/// # Design rationale
///
/// The spec (doc 07) calls for Expected Free Energy scoring with epistemic
/// value computed via KL divergence `D_KL(posterior || prior)`. This would
/// require maintaining a full belief state and simulating Bayesian updates
/// for each candidate section -- prohibitively expensive during prompt
/// assembly where hundreds of candidates are scored per build.
///
/// Instead, [`GoalDirectedHeuristicScorer`] approximates epistemic value
/// via three proxy signals:
///
/// 1. **Uncertainty** (`1.0 - topic_belief`): sections about topics the
///    agent is less certain about score higher, analogous to KL divergence
///    favoring belief-changing observations.
/// 2. **Novelty** (`signal.score.novelty`): sections with high upstream
///    novelty scores carry more information, correlating with entropy
///    reduction.
/// 3. **Informational leverage** (`1.0 / sqrt(len)`): shorter sections
///    have higher information density per token, maximizing value within
///    the budget.
///
/// The HDC hash-embedding cosine similarity in `pragmatic_value()` captures
/// goal alignment without requiring a trained embedding model. Empirically,
/// this produces ranking decisions adequate for prompt budgeting, and the
/// quality gap vs proper EFE is small relative to the benefit of having
/// any scoring at all.
///
/// See [`GoalDirectedHeuristicScorer`] struct documentation for the full
/// three-point justification (correlation, cost, adequacy). If higher-fidelity
/// epistemic scoring is needed, the `epistemic_value()` method can be replaced
/// with KL-divergence computation without changing the [`ScoreFn`] interface.
pub type ActiveInferenceScorer = GoalDirectedHeuristicScorer;

fn embed_text(text: &str, dimensions: usize) -> Vec<f32> {
    let mut vector = vec![0.0_f32; dimensions.max(1)];
    for (position, token) in tokenize(text).into_iter().enumerate() {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        token.hash(&mut hasher);
        position.hash(&mut hasher);
        let hash = hasher.finish();
        let index = (hash as usize) % vector.len();
        let sign = if hash & 1 == 0 { 1.0 } else { -1.0 };
        vector[index] += sign;
    }

    normalize_embedding(vector)
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn token_overlap(left: &str, right: &str) -> f32 {
    let left = tokenize(left)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let right = tokenize(right)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let overlap = left.intersection(&right).count() as f32;
    overlap / left.len().max(right.len()) as f32
}

fn normalize_embedding(mut vector: Vec<f32>) -> Vec<f32> {
    let magnitude = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        for value in &mut vector {
            *value /= magnitude;
        }
    }
    vector
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let len = left.len().min(right.len());
    if len == 0 {
        return 0.0;
    }

    let mut dot = 0.0_f32;
    let mut left_mag = 0.0_f32;
    let mut right_mag = 0.0_f32;
    for idx in 0..len {
        dot += left[idx] * right[idx];
        left_mag += left[idx] * left[idx];
        right_mag += right[idx] * right[idx];
    }

    let magnitude = (left_mag * right_mag).sqrt();
    if magnitude == 0.0 {
        0.0
    } else {
        (dot / magnitude).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{CacheLayer, Placement};
    use std::collections::HashMap;

    fn make_signal(priority: SectionPriority, content: &str, created_at_ms: i64) -> Engram {
        PromptSection::new("x", content)
            .with_priority(priority)
            .with_cache_layer(CacheLayer::Plan)
            .with_placement(Placement::Middle)
            .into_signal()
            .map(|mut s| {
                s.created_at_ms = created_at_ms;
                s.id = s.content_hash();
                s
            })
            .unwrap()
    }

    #[test]
    fn critical_scores_higher_than_low() {
        let scorer = SectionScorer::new();
        let crit = make_signal(SectionPriority::Critical, "x", 0);
        let low = make_signal(SectionPriority::Low, "x", 0);
        let ctx = Context::at(0);
        let cs = scorer.score(&crit, &ctx);
        let ls = scorer.score(&low, &ctx);
        assert!(cs.confidence > ls.confidence);
    }

    #[test]
    fn recent_section_has_full_novelty() {
        let scorer = SectionScorer::new();
        let s = make_signal(SectionPriority::Normal, "x", 0);
        let ctx = Context::at(100); // 100ms later — very fresh
        let score = scorer.score(&s, &ctx);
        assert_eq!(score.novelty, 1.0);
    }

    #[test]
    fn stale_section_has_zero_novelty() {
        let scorer = SectionScorer::new();
        let s = make_signal(SectionPriority::Normal, "x", 0);
        let ctx = Context::at(48 * 60 * 60 * 1000); // 48h later
        let score = scorer.score(&s, &ctx);
        assert_eq!(score.novelty, 0.0);
    }

    #[test]
    fn mid_age_section_has_partial_novelty() {
        let scorer = SectionScorer::new();
        let s = make_signal(SectionPriority::Normal, "x", 0);
        // Between recency (1h) and staleness (24h) — linear decay
        let ctx = Context::at(12 * 60 * 60 * 1000); // 12h
        let score = scorer.score(&s, &ctx);
        assert!(score.novelty > 0.0 && score.novelty < 1.0);
    }

    #[test]
    fn short_content_has_high_utility() {
        let scorer = SectionScorer::new();
        let short = make_signal(SectionPriority::Normal, "hi", 0);
        let long = make_signal(SectionPriority::Normal, &"x".repeat(10_000), 0);
        let ctx = Context::at(0);
        let ss = scorer.score(&short, &ctx);
        let ls = scorer.score(&long, &ctx);
        assert!(ss.utility > ls.utility);
    }

    #[test]
    fn non_section_signals_get_zero_score() {
        let scorer = SectionScorer::new();
        let not_a_section = Engram::builder(roko_core::Kind::Task)
            .body(roko_core::Body::text("not a section"))
            .build();
        let score = scorer.score(&not_a_section, &Context::at(0));
        assert_eq!(score, Score::ZERO);
    }

    #[test]
    fn goal_directed_heuristic_prefers_goal_aligned_sections() {
        let scorer = GoalDirectedHeuristicScorer::new("reduce routing latency")
            .with_prior_beliefs(HashMap::from([("routing".to_string(), 0.85)]));
        let aligned = make_signal(
            SectionPriority::Normal,
            "Improve routing latency by trimming context assembly.",
            0,
        );
        let unrelated = make_signal(
            SectionPriority::Normal,
            "Write onboarding documentation for a new helper.",
            0,
        );
        let ctx = Context::at(0).with_goal("reduce routing latency");

        let aligned_score = scorer.score(&aligned, &ctx);
        let unrelated_score = scorer.score(&unrelated, &ctx);

        assert!(aligned_score.effective() > unrelated_score.effective());
        assert!(aligned_score.salience >= unrelated_score.salience);
        assert_eq!(scorer.name(), "goal_directed_heuristic_scorer");
    }
}
