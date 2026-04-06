//! `NoOp` implementations of all six traits.
//!
//! These are the "do nothing sensibly" defaults. They let you build a full
//! `RokoAgent` with placeholder impls, then swap in real ones incrementally.
//!
//! Anti-pattern: never define a trait without a `NoOp` impl — tests and bring-up
//! code should never block on missing implementations.

use async_trait::async_trait;
use roko_core::{
    error::Result, Body, Budget, Composer, Context, Gate, Kind, Outcome, Policy, Router, Score,
    Scorer, Selection, Signal, Verdict,
};

/// A scorer that returns `Score::NEUTRAL` for every signal.
#[derive(Debug, Clone, Default)]
pub struct NoOpScorer;
impl Scorer for NoOpScorer {
    fn score(&self, _s: &Signal, _ctx: &Context) -> Score {
        Score::NEUTRAL
    }
    fn name(&self) -> &'static str {
        "noop_scorer"
    }
}

/// A gate that always passes.
#[derive(Debug, Clone, Default)]
pub struct NoOpGate;
#[async_trait]
#[allow(clippy::unnecessary_literal_bound)]
impl Gate for NoOpGate {
    async fn verify(&self, _s: &Signal, _ctx: &Context) -> Verdict {
        Verdict::pass("noop_gate")
    }
    fn name(&self) -> &str {
        "noop_gate"
    }
}

/// A router that always selects the first candidate (if any).
#[derive(Debug, Clone, Default)]
pub struct NoOpRouter;
#[allow(clippy::unnecessary_literal_bound)]
impl Router for NoOpRouter {
    fn select(&self, candidates: &[Signal], _ctx: &Context) -> Option<Selection> {
        candidates
            .first()
            .map(|s| Selection::new(s.id, "noop_router"))
    }
    fn feedback(&self, _outcome: &Outcome) {}
    fn name(&self) -> &str {
        "noop_router"
    }
}

/// A composer that returns its first input unchanged (identity).
/// If given no inputs, returns an empty signal.
#[derive(Debug, Clone, Default)]
pub struct NoOpComposer;
#[allow(clippy::unnecessary_literal_bound)]
impl Composer for NoOpComposer {
    fn compose(
        &self,
        signals: &[Signal],
        _budget: &Budget,
        _scorer: &dyn Scorer,
        _ctx: &Context,
    ) -> Result<Signal> {
        Ok(signals.first().cloned().unwrap_or_else(|| {
            Signal::builder(Kind::Custom("empty".into()))
                .body(Body::empty())
                .build()
        }))
    }
    fn name(&self) -> &str {
        "noop_composer"
    }
}

/// A policy that emits no signals.
#[derive(Debug, Clone, Default)]
pub struct NoOpPolicy;
#[allow(clippy::unnecessary_literal_bound)]
impl Policy for NoOpPolicy {
    fn decide(&self, _stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        Vec::new()
    }
    fn name(&self) -> &str {
        "noop_policy"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_signal() -> Signal {
        Signal::builder(Kind::Task).body(Body::text("x")).build()
    }

    #[test]
    fn noop_scorer_is_neutral() {
        let s = NoOpScorer;
        assert_eq!(
            s.score(&mk_signal(), &Context::at(0)),
            Score::NEUTRAL
        );
    }

    #[tokio::test]
    async fn noop_gate_always_passes() {
        let g = NoOpGate;
        let v = g.verify(&mk_signal(), &Context::at(0)).await;
        assert!(v.passed);
        assert_eq!(v.gate, "noop_gate");
    }

    #[test]
    fn noop_router_picks_first() {
        let r = NoOpRouter;
        let s1 = mk_signal();
        let s2 = Signal::builder(Kind::Task).body(Body::text("y")).build();
        let sel = r.select(&[s1.clone(), s2], &Context::at(0)).unwrap();
        assert_eq!(sel.chosen, s1.id);
    }

    #[test]
    fn noop_router_no_candidates() {
        let r = NoOpRouter;
        assert!(r.select(&[], &Context::at(0)).is_none());
    }

    #[test]
    fn noop_composer_returns_identity() {
        let c = NoOpComposer;
        let s = mk_signal();
        let scorer = NoOpScorer;
        let out = c
            .compose(&[s.clone()], &Budget::unlimited(), &scorer, &Context::at(0))
            .unwrap();
        assert_eq!(out.id, s.id);
    }

    #[test]
    fn noop_policy_emits_nothing() {
        let p = NoOpPolicy;
        assert!(p.decide(&[mk_signal()], &Context::at(0)).is_empty());
    }
}
