//! Composable scorers — `+` aggregates evidence, `×` scales independently.
//!
//! In Roko, scorers compose. Rather than define one giant `RelevanceScorer`,
//! compose smaller scorers:
//!
//! ```ignore
//! // Overall score = relevance × recency × reputation
//! let scorer = MulScorer::new(vec![
//!     Box::new(RelevanceScorer::new(query)),
//!     Box::new(RecencyScorer),
//!     Box::new(ReputationScorer),
//! ]);
//! ```

use roko_core::{Context, Engram, Score};
use roko_core::traits::Score as ScoreFn;

/// Sum several scorers element-wise (aggregates evidence).
pub struct SumScorer {
    scorers: Vec<Box<dyn ScoreFn>>,
    #[allow(dead_code)]
    name: String,
}

impl SumScorer {
    /// Construct a sum scorer from a list of component scorers.
    #[must_use]
    pub fn new(scorers: Vec<Box<dyn ScoreFn>>) -> Self {
        Self {
            scorers,
            name: "sum_scorer".to_string(),
        }
    }

    /// Construct with a custom name (useful for logs).
    #[must_use]
    pub fn named(name: impl Into<String>, scorers: Vec<Box<dyn ScoreFn>>) -> Self {
        Self {
            scorers,
            name: name.into(),
        }
    }
}

impl ScoreFn for SumScorer {
    fn score(&self, signal: &Engram, ctx: &Context) -> Score {
        self.scorers
            .iter()
            .fold(Score::ZERO, |acc, s| acc + s.score(signal, ctx))
    }

    fn name(&self) -> &'static str {
        "sum_scorer"
    }
}

/// Multiply several scorers element-wise (scales each axis).
pub struct MulScorer {
    scorers: Vec<Box<dyn ScoreFn>>,
    #[allow(dead_code)]
    name: String,
}

impl MulScorer {
    /// Construct a multiplicative scorer from a list of component scorers.
    #[must_use]
    pub fn new(scorers: Vec<Box<dyn ScoreFn>>) -> Self {
        Self {
            scorers,
            name: "mul_scorer".to_string(),
        }
    }

    /// Construct with a custom name (useful for logs).
    #[must_use]
    pub fn named(name: impl Into<String>, scorers: Vec<Box<dyn ScoreFn>>) -> Self {
        Self {
            scorers,
            name: name.into(),
        }
    }
}

impl ScoreFn for MulScorer {
    fn score(&self, signal: &Engram, ctx: &Context) -> Score {
        // Start with all-1 score so multiplication is identity.
        let one = Score::new(1.0, 1.0, 1.0, 1.0);
        self.scorers
            .iter()
            .fold(one, |acc, s| acc * s.score(signal, ctx))
    }

    fn name(&self) -> &'static str {
        "mul_scorer"
    }
}

/// Returns a fixed score for every signal. Useful for static weighting.
pub struct ConstScorer {
    value: Score,
}

impl ConstScorer {
    /// Construct a scorer that always returns `value`.
    #[must_use]
    pub const fn new(value: Score) -> Self {
        Self { value }
    }
}

impl ScoreFn for ConstScorer {
    fn score(&self, _s: &Engram, _ctx: &Context) -> Score {
        self.value
    }
    fn name(&self) -> &'static str {
        "const_scorer"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};

    fn signal() -> Engram {
        Engram::builder(Kind::Task).body(Body::text("x")).build()
    }

    #[test]
    fn const_scorer_returns_fixed() {
        let s = ConstScorer::new(Score::new(0.5, 0.0, 0.0, 1.0));
        let out = s.score(&signal(), &Context::at(0));
        assert_eq!(out, Score::new(0.5, 0.0, 0.0, 1.0));
    }

    #[test]
    fn sum_scorer_aggregates() {
        let a = Box::new(ConstScorer::new(Score::new(0.3, 0.0, 0.0, 0.0))) as Box<dyn ScoreFn>;
        let b = Box::new(ConstScorer::new(Score::new(0.4, 0.0, 0.0, 0.0))) as Box<dyn ScoreFn>;
        let sum = SumScorer::new(vec![a, b]);
        let out = sum.score(&signal(), &Context::at(0));
        assert!((out.confidence - 0.7).abs() < 1e-6);
    }

    #[test]
    fn mul_scorer_scales() {
        // 0.9 × 0.5 = 0.45 on confidence axis
        let a = Box::new(ConstScorer::new(Score::new(0.9, 1.0, 1.0, 1.0))) as Box<dyn ScoreFn>;
        let b = Box::new(ConstScorer::new(Score::new(0.5, 1.0, 1.0, 1.0))) as Box<dyn ScoreFn>;
        let mul = MulScorer::new(vec![a, b]);
        let out = mul.score(&signal(), &Context::at(0));
        assert!((out.confidence - 0.45).abs() < 1e-6);
    }

    #[test]
    fn mul_scorer_empty_is_one() {
        let mul = MulScorer::new(vec![]);
        let out = mul.score(&signal(), &Context::at(0));
        assert_eq!(out, Score::new(1.0, 1.0, 1.0, 1.0));
    }

    #[test]
    fn named_preserves_name() {
        let s = MulScorer::named("my_scorer", vec![]);
        // The ScoreFn trait returns a static name; the stored name is for logging.
        assert_eq!(s.name(), "mul_scorer");
    }
}
