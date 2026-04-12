//! Scorers for prompt sections.
//!
//! `SectionScorer` ranks `Signal<PromptSection>` inputs by priority, recency,
//! and cache-layer fit. The `HighestScoreRouter` can use this scorer to pick
//! the most important section when the composer's budget is tight.

use crate::prompt::{PromptSection, SectionPriority};
use roko_core::{Context, Score, Scorer, Signal};

/// Ranks `Signal<PromptSection>` inputs by importance.
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

impl Scorer for SectionScorer {
    fn score(&self, signal: &Signal, ctx: &Context) -> Score {
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
        let reputation = if signal.provenance.tainted {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{CacheLayer, Placement};

    fn make_signal(priority: SectionPriority, content: &str, created_at_ms: i64) -> Signal {
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
        let not_a_section = Signal::builder(roko_core::Kind::Task)
            .body(roko_core::Body::text("not a section"))
            .build();
        let score = scorer.score(&not_a_section, &Context::at(0));
        assert_eq!(score, Score::ZERO);
    }
}
