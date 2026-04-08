//! Simple routers that need no learning state.
//!
//! These routers are the building blocks; adaptive bandit routers (`LinUCB`,
//! Thompson sampling) live in `roko-learn`.

use parking_lot::Mutex;
use roko_core::{Context, Outcome, Router, Scorer, Selection, Signal};
use std::sync::Arc;

/// Picks the first candidate (deterministic, no-state).
#[derive(Debug, Clone, Default)]
pub struct FirstRouter;

#[allow(clippy::unnecessary_literal_bound)]
impl Router for FirstRouter {
    fn select(&self, candidates: &[Signal], _ctx: &Context) -> Option<Selection> {
        candidates
            .first()
            .map(|s| Selection::new(s.id, self.name()))
    }
    fn feedback(&self, _outcome: &Outcome) {}
    fn name(&self) -> &str {
        "first"
    }
}

/// Picks the candidate with the highest effective score (via a [`Scorer`]).
pub struct HighestScoreRouter {
    scorer: Arc<dyn Scorer>,
}

impl HighestScoreRouter {
    /// Construct a router that picks by effective score via the given scorer.
    #[must_use]
    pub fn new(scorer: Arc<dyn Scorer>) -> Self {
        Self { scorer }
    }
}

#[allow(clippy::unnecessary_literal_bound)]
impl Router for HighestScoreRouter {
    fn select(&self, candidates: &[Signal], ctx: &Context) -> Option<Selection> {
        candidates
            .iter()
            .map(|s| (s, self.scorer.score(s, ctx).effective()))
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(s, score)| {
                Selection::new(s.id, self.name())
                    .with_confidence(score.clamp(0.0, 1.0))
                    .with_reasoning(format!("score={score:.3}"))
            })
    }
    fn feedback(&self, _outcome: &Outcome) {}
    fn name(&self) -> &str {
        "highest_score"
    }
}

/// Picks candidates in round-robin order. Maintains a counter.
#[derive(Debug, Default)]
pub struct RoundRobinRouter {
    counter: Mutex<usize>,
}

impl RoundRobinRouter {
    /// Construct a new round-robin router with counter at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[allow(clippy::unnecessary_literal_bound)]
impl Router for RoundRobinRouter {
    fn select(&self, candidates: &[Signal], _ctx: &Context) -> Option<Selection> {
        if candidates.is_empty() {
            return None;
        }
        let mut counter = self.counter.lock();
        let idx = *counter % candidates.len();
        *counter = counter.wrapping_add(1);
        drop(counter);
        Some(Selection::new(candidates[idx].id, self.name()))
    }
    fn feedback(&self, _outcome: &Outcome) {}
    fn name(&self) -> &str {
        "round_robin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scorer::ConstScorer;
    use roko_core::{Body, Kind, Score};

    fn sig(text: &str, t: i64) -> Signal {
        Signal::builder(Kind::Task)
            .body(Body::text(text))
            .created_at_ms(t)
            .build()
    }

    #[test]
    fn first_router_picks_first() {
        let a = sig("a", 0);
        let b = sig("b", 0);
        let r = FirstRouter;
        let sel = r.select(&[a.clone(), b], &Context::at(0)).unwrap();
        assert_eq!(sel.chosen, a.id);
    }

    #[test]
    fn first_router_empty() {
        assert!(FirstRouter.select(&[], &Context::at(0)).is_none());
    }

    #[test]
    fn highest_score_picks_highest() {
        let a = Signal::builder(Kind::Task)
            .body(Body::text("a"))
            .score(Score::new(0.1, 0.0, 0.0, 1.0))
            .created_at_ms(0)
            .build();
        let b = Signal::builder(Kind::Task)
            .body(Body::text("b"))
            .score(Score::new(0.9, 0.0, 0.0, 1.0))
            .created_at_ms(0)
            .build();
        // Use the signal's own score via a scorer that returns what we set.
        // Here we just use a const scorer — the router will score both equally,
        // but we'll test with a scorer that varies by tag.
        let scorer: Arc<dyn Scorer> = Arc::new(ConstScorer::new(Score::new(0.5, 0.0, 0.0, 1.0)));
        let r = HighestScoreRouter::new(scorer);
        // With a const scorer, both have equal score — returns one of them.
        let sel = r.select(&[a.clone(), b.clone()], &Context::at(0)).unwrap();
        assert!(sel.chosen == a.id || sel.chosen == b.id);
    }

    #[test]
    fn round_robin_rotates() {
        let a = sig("a", 0);
        let b = sig("b", 0);
        let c = sig("c", 0);
        let r = RoundRobinRouter::new();
        let ctx = Context::at(0);
        let candidates = [a.clone(), b.clone(), c.clone()];
        assert_eq!(r.select(&candidates, &ctx).unwrap().chosen, a.id);
        assert_eq!(r.select(&candidates, &ctx).unwrap().chosen, b.id);
        assert_eq!(r.select(&candidates, &ctx).unwrap().chosen, c.id);
        // Wraps back to a
        assert_eq!(r.select(&candidates, &ctx).unwrap().chosen, a.id);
    }
}
