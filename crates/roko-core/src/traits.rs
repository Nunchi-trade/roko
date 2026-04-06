//! The six universal traits of the Roko architecture.
//!
//! These traits define the entire operational surface. Every capability in
//! the Roko design corpus — agent spawning, gate verification, prompt
//! assembly, model routing, memory retrieval, pheromone reaction, chain
//! participation — is an implementation of one of these traits.
//!
//! See [crate docs](crate) for the universal loop that composes them.

use crate::{
    error::Result, Budget, Context, ContentHash, Outcome, Query, Score, Selection, Signal, Verdict,
};
use async_trait::async_trait;

// ─── Substrate ─────────────────────────────────────────────────────────────

/// Stores and queries [`Signal`]s.
///
/// All storage backends implement this trait: `MemorySubstrate` (testing),
/// `FileSubstrate` (.roko/ persistence), `HdcSubstrate` (semantic search),
/// `ChainSubstrate` (shared on-chain state). They are API-identical from a
/// caller's perspective — pick the substrate that matches your durability,
/// visibility, and latency needs.
///
/// # Idempotence
///
/// `put` is idempotent for signals with identical content hashes. Re-putting
/// the same signal is a no-op.
///
/// # Concurrency
///
/// Substrates are `Send + Sync`. Impls must handle concurrent access internally.
#[async_trait]
pub trait Substrate: Send + Sync {
    /// Store a signal. Returns its content hash. Idempotent on content.
    async fn put(&self, signal: Signal) -> Result<ContentHash>;

    /// Retrieve a signal by content hash. Does not apply decay.
    async fn get(&self, id: &ContentHash) -> Result<Option<Signal>>;

    /// Query for signals matching the given filter. Impls may apply decay
    /// when evaluating `min_weight` and when ordering results.
    async fn query(&self, q: &Query, ctx: &Context) -> Result<Vec<Signal>>;

    /// Remove signals whose effective weight (score × decay) has fallen
    /// below `threshold` at `ctx.now_ms`. Returns count of pruned signals.
    async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize>;

    /// Optional: total count of stored signals (for metrics/health checks).
    async fn len(&self) -> Result<usize> {
        Ok(0)
    }

    /// Optional: is the substrate empty?
    async fn is_empty(&self) -> Result<bool> {
        Ok(self.len().await? == 0)
    }

    /// Human-readable name for logging/debugging.
    fn name(&self) -> &'static str {
        "unnamed_substrate"
    }
}

// ─── Scorer ────────────────────────────────────────────────────────────────

/// Rates a signal along multi-dimensional axes.
///
/// Scorers are pure functions of `(signal, context)`. They compose freely:
/// use `CompositeScorer` to combine several scorers via +/× operations.
///
/// # Examples of Scorers
///
/// - `RelevanceScorer`: how well does this signal match the current goal?
/// - `RecencyScorer`: how recent is this signal?
/// - `ReputationScorer`: how trustworthy is its author?
/// - `CatalyticScorer`: how many downstream signals does this enable?
pub trait Scorer: Send + Sync {
    /// Score a signal in the given context.
    fn score(&self, signal: &Signal, ctx: &Context) -> Score;

    /// Human-readable name.
    fn name(&self) -> &'static str {
        "unnamed_scorer"
    }
}

// ─── Gate ──────────────────────────────────────────────────────────────────

/// Verifies a signal against ground truth, producing a [`Verdict`].
///
/// Gates are the bridge to external reality: compile, run tests, simulate
/// transactions, check balances, validate schemas. A gate that returns
/// `passed = true` is a claim that the signal is correct in some domain.
///
/// # Async by default
///
/// Gates typically invoke subprocesses, HTTP calls, or chain RPCs, so the
/// trait is async. For pure/synchronous verification, implementors can return
/// a ready future.
#[async_trait]
pub trait Gate: Send + Sync {
    /// Verify the signal and return a verdict.
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict;

    /// Human-readable name (appears in verdicts).
    fn name(&self) -> &str;
}

// ─── Router ────────────────────────────────────────────────────────────────

/// Selects one signal from many candidates.
///
/// Routers are the decision-making layer: which model to call, which backend
/// to use, which gate to run next, which bounty to claim. They learn via
/// [`Router::feedback`] so they improve with experience.
///
/// # Implementations
///
/// - `StaticRouter` — deterministic choice (config-driven)
/// - `LinUCBRouter` — contextual bandit
/// - `CascadeRouter` — multi-stage confidence → UCB
/// - `WeightedRouter` — softmax over scorers
pub trait Router: Send + Sync {
    /// Select one signal from the candidates. None = no selection made.
    fn select(&self, candidates: &[Signal], ctx: &Context) -> Option<Selection>;

    /// Learn from a selection's actual outcome.
    fn feedback(&self, outcome: &Outcome);

    /// Human-readable name (appears in selections).
    fn name(&self) -> &str;
}

// ─── Composer ──────────────────────────────────────────────────────────────

/// Combines multiple signals into one new signal under a [`Budget`].
///
/// Composers are the assembly layer: prompts from sections, context packs
/// from fragments, transactions from operations, plans from tasks, bounties
/// from sub-bounties. Output respects budget constraints (tokens, bytes,
/// signal count, wall time).
pub trait Composer: Send + Sync {
    /// Combine input signals into a new composed signal.
    /// The composer may use the scorer to rank/select inputs under budget.
    fn compose(
        &self,
        signals: &[Signal],
        budget: &Budget,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<Signal>;

    /// Human-readable name.
    fn name(&self) -> &str;
}

// ─── Policy ────────────────────────────────────────────────────────────────

/// Watches a stream of signals and emits new signals in response.
///
/// Policies are the reactive/behavioral layer: conductor watchers, circuit
/// breakers, episode logging, pheromone reactions, heartbeat emission,
/// promotion to chain, sentinel detection. They run continuously over the
/// signal stream and may produce zero, one, or many output signals per tick.
pub trait Policy: Send + Sync {
    /// Examine the recent signal stream and produce new signals (interventions).
    fn decide(&self, stream: &[Signal], ctx: &Context) -> Vec<Signal>;

    /// Human-readable name.
    fn name(&self) -> &str;
}
