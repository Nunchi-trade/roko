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

use crate::{
    Budget, Composer, Context, Gate, Policy, Query, Router, Scorer, Signal, Substrate, Verdict,
    error::Result,
};

/// What happened during one tick of the universal loop.
#[derive(Debug)]
pub struct TickOutcome {
    /// How many candidates the substrate returned.
    pub candidates_examined: usize,
    /// The composed signal (if one was produced).
    pub composed: Option<Signal>,
    /// The gate's verdict (if composition happened).
    pub verdict: Option<Verdict>,
    /// Signals emitted by the policy.
    pub emitted: Vec<Signal>,
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
    let composed = composer.compose(&[chosen], budget, scorer, ctx)?;

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
