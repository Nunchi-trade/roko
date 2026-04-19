//! The universal loop — one function that composes the six verbs.
//!
//! Every operation in Roko reduces to calling [`loop_tick`] with different
//! trait impls. Training the scaffold optimizer, picking a model, running a
//! gate, assembling a prompt, claiming a bounty — all are [`loop_tick`]
//! invocations with different substrates, scorers, gates, routers, composers,
//! and policies.
//!
//! ```text
//!   candidates = substrate.query(q, ctx)
//!       ↓
//!   selection = router.select(candidates, ctx)
//!       ↓
//!   composed  = composer.compose([selection], budget, scorer, ctx)
//!       ↓
//!   verdict   = gate.verify(composed, ctx)
//!       ↓
//!   if passed: substrate.put(composed) + policy.decide(stream, ctx)
//! ```

use serde::{Deserialize, Serialize};

use crate::{
    Budget, Composer, Context, Engram, Gate, Policy, Query, Router, Scorer, Substrate, Verdict,
    error::Result,
};

/// Configuration for a single tick of the universal loop (IF-04).
///
/// Controls limits and verbosity without changing the core loop logic.
/// Use `TickConfig::default()` for unlimited execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickConfig {
    /// Maximum number of turns (candidates examined) before stopping.
    /// `None` means unlimited.
    pub max_turns: Option<u64>,
    /// Timeout in seconds for the entire tick. `None` means no timeout.
    pub timeout_secs: Option<u64>,
    /// Budget ceiling in USD. `None` means no budget limit.
    pub budget_usd: Option<f64>,
    /// Whether to emit verbose tracing for this tick.
    pub verbose: bool,
}

impl Default for TickConfig {
    fn default() -> Self {
        Self {
            max_turns: None,
            timeout_secs: None,
            budget_usd: None,
            verbose: false,
        }
    }
}

impl TickConfig {
    /// Create a config with no limits (equivalent to `Default`).
    #[must_use]
    pub fn unlimited() -> Self {
        Self::default()
    }

    /// Set the maximum number of turns.
    #[must_use]
    pub const fn with_max_turns(mut self, max: u64) -> Self {
        self.max_turns = Some(max);
        self
    }

    /// Set the timeout in seconds.
    #[must_use]
    pub const fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Set the budget ceiling in USD.
    #[must_use]
    pub fn with_budget_usd(mut self, usd: f64) -> Self {
        self.budget_usd = Some(usd);
        self
    }

    /// Enable verbose tracing.
    #[must_use]
    pub const fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }
}

/// What happened during one tick of the universal loop.
#[derive(Debug)]
pub struct TickOutcome {
    /// How many candidates the substrate returned.
    pub candidates_examined: usize,
    /// The composed signal (if one was produced).
    pub composed: Option<Engram>,
    /// The gate's verdict (if composition happened).
    pub verdict: Option<Verdict>,
    /// Signals emitted by the policy.
    pub emitted: Vec<Engram>,
    /// Content hashes of signals written back to substrate.
    pub written: Vec<crate::ContentHash>,
}

impl TickOutcome {
    /// Did this tick's work pass its gate?
    #[must_use]
    pub fn passed(&self) -> bool {
        self.verdict.as_ref().is_some_and(|v| v.passed)
    }

    /// Did this tick do any work (query returned candidates)?
    #[must_use]
    pub const fn did_work(&self) -> bool {
        self.candidates_examined > 0
    }
}

fn ensure_lineage(mut signal: Engram, parent: crate::ContentHash) -> Engram {
    if !signal.lineage.contains(&parent) {
        signal.lineage.push(parent);
    }
    signal
}

/// Run one tick of the universal loop.
///
/// # Steps
///
/// 1. Query the substrate for candidates matching `query`.
/// 2. Ask the router to select one (returns early if none selected).
/// 3. Ask the composer to build a new signal from the selection.
/// 4. Ask the gate to verify the composed signal.
/// 5. If it passes: write it back to the substrate and run the policy.
///
/// # Errors
///
/// Propagates errors from the substrate and composer. Gate failures are
/// *not* errors — they return a failing [`Verdict`] in the outcome.
#[allow(clippy::similar_names, clippy::too_many_arguments)]
pub async fn loop_tick(
    substrate: &dyn Substrate,
    scorer: &dyn Scorer,
    router: &dyn Router,
    composer: &dyn Composer,
    gate: &dyn Gate,
    policy: &dyn Policy,
    query: &Query,
    budget: &Budget,
    ctx: &Context,
) -> Result<TickOutcome> {
    loop_tick_with_config(
        substrate, scorer, router, composer, gate, policy, query, budget, ctx,
        &TickConfig::default(),
    )
    .await
}

/// Run one tick of the universal loop with explicit configuration.
///
/// Like [`loop_tick`] but accepts a [`TickConfig`] for controlling limits
/// and verbosity. The `tick_config` parameter is consulted for verbose
/// logging; budget and turn limits are advisory and should be enforced
/// by the outer orchestration loop that calls this function repeatedly.
///
/// # Errors
///
/// Propagates errors from the substrate and composer.
#[allow(clippy::similar_names, clippy::too_many_arguments)]
pub async fn loop_tick_with_config(
    substrate: &dyn Substrate,
    scorer: &dyn Scorer,
    router: &dyn Router,
    composer: &dyn Composer,
    gate: &dyn Gate,
    policy: &dyn Policy,
    query: &Query,
    budget: &Budget,
    ctx: &Context,
    _tick_config: &TickConfig,
) -> Result<TickOutcome> {
    // 1. Query the substrate for candidates.
    let candidates = substrate.query(query, ctx).await?;
    let candidates_examined = candidates.len();

    if candidates.is_empty() {
        return Ok(TickOutcome {
            candidates_examined: 0,
            composed: None,
            verdict: None,
            emitted: Vec::new(),
            written: Vec::new(),
        });
    }

    // 2. Router selects one candidate (or bails).
    let Some(selection) = router.select(&candidates, ctx) else {
        return Ok(TickOutcome {
            candidates_examined,
            composed: None,
            verdict: None,
            emitted: Vec::new(),
            written: Vec::new(),
        });
    };

    // 3. Find the selected signal among candidates; feed it to the composer.
    let Some(chosen) = candidates
        .iter()
        .find(|s| s.id == selection.chosen)
        .cloned()
    else {
        return Ok(TickOutcome {
            candidates_examined,
            composed: None,
            verdict: None,
            emitted: Vec::new(),
            written: Vec::new(),
        });
    };
    let composed = ensure_lineage(
        composer.compose(&[chosen.clone()], budget, scorer, ctx)?,
        chosen.id,
    );

    // 4. Gate verifies the composition.
    let verdict = gate.verify(&composed, ctx).await;

    // 5. If passed, persist and run policy reaction.
    let mut written = Vec::new();
    let mut emitted = Vec::new();
    if verdict.passed {
        let id = substrate.put(composed.clone()).await?;
        written.push(id);

        // Policy sees the new signal and may produce reactions.
        let reactions = policy.decide(std::slice::from_ref(&composed), ctx);
        for r in reactions {
            let id = substrate.put(r.clone()).await?;
            written.push(id);
            emitted.push(r);
        }
    }

    Ok(TickOutcome {
        candidates_examined,
        composed: Some(composed),
        verdict: Some(verdict),
        emitted,
        written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Body, Budget, ContentHash, Context, Engram, Kind, Provenance, Query, Result, Score,
        Selection, verdict::Verdict,
    };
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::sync::Arc;

    struct TestSubstrate {
        candidate: Engram,
        written: Arc<Mutex<Vec<Engram>>>,
    }

    #[async_trait]
    impl Substrate for TestSubstrate {
        async fn put(&self, signal: Engram) -> Result<ContentHash> {
            self.written.lock().push(signal.clone());
            Ok(signal.id)
        }

        async fn get(&self, _id: &ContentHash) -> Result<Option<Engram>> {
            Ok(None)
        }

        async fn query(&self, _q: &Query, _ctx: &Context) -> Result<Vec<Engram>> {
            Ok(vec![self.candidate.clone()])
        }

        async fn prune(&self, _threshold: f32, _ctx: &Context) -> Result<usize> {
            Ok(0)
        }
    }

    struct TestRouter {
        choice: Selection,
    }

    impl Router for TestRouter {
        fn select(&self, _candidates: &[Engram], _ctx: &Context) -> Option<Selection> {
            Some(self.choice.clone())
        }

        fn feedback(&self, _outcome: &crate::Outcome) {}

        fn name(&self) -> &'static str {
            "test_router"
        }
    }

    struct PassthroughComposer;

    impl Composer for PassthroughComposer {
        fn compose(
            &self,
            signals: &[Engram],
            _budget: &Budget,
            _scorer: &dyn Scorer,
            _ctx: &Context,
        ) -> Result<Engram> {
            Ok(Engram::builder(Kind::Prompt)
                .body(Body::text("composed"))
                .provenance(Provenance::trusted("composer"))
                .score(Score::NEUTRAL)
                .created_at_ms(0)
                .lineage(
                    signals
                        .iter()
                        .flat_map(|signal| signal.lineage.iter().copied()),
                )
                .build())
        }

        fn name(&self) -> &'static str {
            "passthrough"
        }
    }

    struct PassGate;

    #[async_trait]
    impl Gate for PassGate {
        async fn verify(&self, _signal: &Engram, _ctx: &Context) -> Verdict {
            Verdict::pass("pass_gate")
        }

        fn name(&self) -> &'static str {
            "pass_gate"
        }
    }

    struct NoopPolicy;

    impl Policy for NoopPolicy {
        fn decide(&self, _stream: &[Engram], _ctx: &Context) -> Vec<Engram> {
            Vec::new()
        }

        fn name(&self) -> &'static str {
            "noop_policy"
        }
    }

    struct ZeroScorer;

    impl Scorer for ZeroScorer {
        fn score(&self, _signal: &Engram, _ctx: &Context) -> crate::Score {
            crate::Score::NEUTRAL
        }

        fn name(&self) -> &'static str {
            "zero_scorer"
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn loop_tick_adds_missing_upstream_lineage() {
        let candidate = Engram::builder(Kind::Task)
            .body(Body::text("task"))
            .provenance(Provenance::trusted("source"))
            .created_at_ms(0)
            .build();
        let substrate = TestSubstrate {
            candidate: candidate.clone(),
            written: Arc::new(Mutex::new(Vec::new())),
        };
        let router = TestRouter {
            choice: Selection::new(candidate.id, "test_router"),
        };
        let composer = PassthroughComposer;
        let gate = PassGate;
        let policy = NoopPolicy;
        let scorer = ZeroScorer;
        let budget = Budget::unlimited();
        let ctx = Context::now();

        let outcome = loop_tick(
            &substrate,
            &scorer,
            &router,
            &composer,
            &gate,
            &policy,
            &Query::all(),
            &budget,
            &ctx,
        )
        .await
        .unwrap();

        assert!(outcome.passed());
        let composed = outcome.composed.as_ref().expect("composed signal");
        assert!(composed.lineage.contains(&candidate.id));
        assert_eq!(substrate.written.lock().len(), 1);
        assert!(substrate.written.lock()[0].lineage.contains(&candidate.id));
    }
}
