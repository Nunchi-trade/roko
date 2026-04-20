//! The six universal traits of the Roko architecture.
//!
//! These traits define the entire operational surface. Every capability in
//! the Roko design corpus — agent spawning, gate verification, prompt
//! assembly, model routing, memory retrieval, pheromone reaction, chain
//! participation — is an implementation of one of these traits.
//!
//! See [crate docs](crate) for the universal loop that composes them.

use crate::{
    Budget, ContentHash, Context, Datum, Engram, Outcome, PolicyOutputs, Pulse, Query, Score,
    Selection, TopicFilter, Verdict, error::Result,
};
use async_trait::async_trait;
use roko_primitives::HdcVector;

// ─── Substrate ─────────────────────────────────────────────────────────────

/// Stores and queries [`Engram`]s.
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
    /// Store an engram. Returns its content hash. Idempotent on content.
    async fn put(&self, engram: Engram) -> Result<ContentHash>;

    /// Retrieve an engram by content hash. Does not apply decay.
    async fn get(&self, id: &ContentHash) -> Result<Option<Engram>>;

    /// Query for engrams matching the given filter. Impls may apply decay
    /// when evaluating `min_weight` and when ordering results.
    async fn query(&self, q: &Query, ctx: &Context) -> Result<Vec<Engram>>;

    /// Query by HDC similarity against a fingerprint, returning ranked matches.
    ///
    /// The default implementation reports no indexed matches, which keeps
    /// existing substrates source-compatible until they add native support.
    async fn query_similar(
        &self,
        _fp: &HdcVector,
        _radius: f32,
        _limit: usize,
        _ctx: &Context,
    ) -> Result<Vec<(ContentHash, f32)>> {
        Ok(Vec::new())
    }

    /// Remove engrams whose effective weight (score × decay) has fallen
    /// below `threshold` at `ctx.now_ms`. Returns count of pruned engrams.
    async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize>;

    /// Optional: total count of stored engrams (for metrics/health checks).
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

/// Rates an engram along multi-dimensional axes.
///
/// Scorers are pure functions of `(engram, context)`. They compose freely:
/// use `CompositeScorer` to combine several scorers via +/× operations.
///
/// # Examples of Scorers
///
/// - `RelevanceScorer`: how well does this engram match the current goal?
/// - `RecencyScorer`: how recent is this engram?
/// - `ReputationScorer`: how trustworthy is its author?
/// - `CatalyticScorer`: how many downstream engrams does this enable?
pub trait Scorer: Send + Sync {
    /// Score an engram in the given context.
    ///
    /// This is the implementor hook — override this in your scorer impl.
    fn score(&self, engram: &Engram, ctx: &Context) -> Score;

    /// Alias for [`score`](Self::score) — score a persisted engram.
    ///
    /// Provided so callers can be explicit about the input type.
    fn score_engram(&self, engram: &Engram, ctx: &Context) -> Score {
        self.score(engram, ctx)
    }

    /// Score an ephemeral pulse by promoting it to a synthetic engram.
    fn score_pulse(&self, p: &Pulse, ctx: &Context) -> Score {
        let synthetic = Engram::from_pulse_synthetic(p);
        self.score(&synthetic, ctx)
    }

    /// Score either an engram or a pulse via [`Datum`] dispatch.
    fn score_datum(&self, datum: Datum<'_>, ctx: &Context) -> Score {
        match datum {
            Datum::Engram(e) => self.score(e, ctx),
            Datum::Pulse(p) => self.score_pulse(p, ctx),
        }
    }

    /// Human-readable name.
    fn name(&self) -> &'static str {
        "unnamed_scorer"
    }
}

// ─── Gate ──────────────────────────────────────────────────────────────────

/// Verifies an engram against ground truth, producing a [`Verdict`].
///
/// Gates are the bridge to external reality: compile, run tests, simulate
/// transactions, check balances, validate schemas. A gate that returns
/// `passed = true` is a claim that the engram is correct in some domain.
///
/// # Async by default
///
/// Gates typically invoke subprocesses, HTTP calls, or chain RPCs, so the
/// trait is async. For pure/synchronous verification, implementors can return
/// a ready future.
#[async_trait]
pub trait Gate: Send + Sync {
    /// Verify the engram and return a verdict.
    async fn verify(&self, engram: &Engram, ctx: &Context) -> Verdict;

    /// Verify a batch of ephemeral pulses by promoting them to a synthetic engram.
    async fn verify_stream(&self, pulses: &[Pulse], ctx: &Context) -> Verdict {
        let synthetic = Engram::from_pulses(pulses);
        self.verify(&synthetic, ctx).await
    }

    /// Human-readable name (appears in verdicts).
    fn name(&self) -> &str;
}

// ─── Router ────────────────────────────────────────────────────────────────

/// Selects one engram from many candidates.
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
    /// Select one engram from the candidates. None = no selection made.
    ///
    /// This is the implementor hook — override this in your router impl.
    fn select(&self, candidates: &[Engram], ctx: &Context) -> Option<Selection>;

    /// Alias for [`select`](Self::select) — select from persisted engrams.
    ///
    /// Provided so callers can be explicit about the input type.
    fn select_engram(&self, candidates: &[Engram], ctx: &Context) -> Option<Selection> {
        self.select(candidates, ctx)
    }

    /// Select one pulse from a set of ephemeral candidates.
    ///
    /// Returns `None` by default; override for pulse-aware routing.
    fn select_pulse(&self, _candidates: &[Pulse], _ctx: &Context) -> Option<Selection> {
        None
    }

    /// Learn from a selection's actual outcome.
    fn feedback(&self, outcome: &Outcome);

    /// Human-readable name (appears in selections).
    fn name(&self) -> &str;
}

// ─── Composer ──────────────────────────────────────────────────────────────

/// Combines multiple engrams into one new engram under a [`Budget`].
///
/// Composers are the assembly layer: prompts from sections, context packs
/// from fragments, transactions from operations, plans from tasks, bounties
/// from sub-bounties. Output respects budget constraints (tokens, bytes,
/// engram count, wall time).
///
/// # Datum-polymorphic input (TM-05)
///
/// The [`compose_datums`](Self::compose_datums) method accepts `&[Datum<'_>]`
/// so callers can mix persisted engrams and ephemeral pulses in a single
/// compose call.  The default implementation filters for engrams and
/// delegates to [`compose`](Self::compose), so existing implementations
/// get the new entry point for free.
pub trait Composer: Send + Sync {
    /// Combine input engrams into a new composed engram.
    /// The composer may use the scorer to rank/select inputs under budget.
    fn compose(
        &self,
        engrams: &[Engram],
        budget: &Budget,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<Engram>;

    /// Compose from a polymorphic mix of engrams and pulses.
    ///
    /// The default implementation extracts engrams (converting pulses via
    /// [`Engram::from_pulse_synthetic`]) and delegates to [`compose`](Self::compose).
    /// Override for pulse-aware composition that treats the two media differently.
    fn compose_datums(
        &self,
        datums: &[Datum<'_>],
        budget: &Budget,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<Engram> {
        let engrams: Vec<Engram> = datums
            .iter()
            .map(|d| match d {
                Datum::Engram(e) => (*e).clone(),
                Datum::Pulse(p) => Engram::from_pulse_synthetic(p),
            })
            .collect();
        self.compose(&engrams, budget, scorer, ctx)
    }

    /// Human-readable name.
    fn name(&self) -> &str;
}

// ─── Policy ────────────────────────────────────────────────────────────────

/// Watches a stream of engrams and emits new engrams in response.
///
/// Policies are the reactive/behavioral layer: conductor watchers, circuit
/// breakers, episode logging, pheromone reactions, heartbeat emission,
/// promotion to chain, sentinel detection. They run continuously over the
/// engram stream and may produce zero, one, or many output engrams per tick.
///
/// # Pulse-aware decisions (TM-06)
///
/// The [`decide_with_pulses`](Self::decide_with_pulses) method accepts both
/// the persisted engram stream **and** the ephemeral pulse stream, returning
/// a [`PolicyOutputs`] that can contain both engrams (to persist) and pulses
/// (to publish on the Bus).  The default implementation ignores pulses and
/// wraps the existing [`decide`](Self::decide) output in `PolicyOutputs`,
/// so existing implementations get the new entry point for free.
pub trait Policy: Send + Sync {
    /// Examine the recent engram stream and produce new engrams (interventions).
    fn decide(&self, stream: &[Engram], ctx: &Context) -> Vec<Engram>;

    /// Examine both persisted engrams and ephemeral pulses, returning
    /// [`PolicyOutputs`] that may contain both engrams and pulses.
    ///
    /// The default implementation ignores `pulses`, delegates to
    /// [`decide`](Self::decide), and wraps the resulting engrams in
    /// `PolicyOutputs` (with an empty pulse list).  Override to produce
    /// pulses or to incorporate the pulse stream into the decision.
    fn decide_with_pulses(
        &self,
        engrams: &[Engram],
        _pulses: &[Pulse],
        ctx: &Context,
    ) -> PolicyOutputs {
        let out_engrams = self.decide(engrams, ctx);
        PolicyOutputs {
            engrams: out_engrams,
            pulses: Vec::new(),
        }
    }

    /// Human-readable name.
    fn name(&self) -> &str;
}

// ─── Bus ─────────────────────────────────────────────────────────────────

/// Publish/subscribe transport for ephemeral [`Pulse`]s.
///
/// The Bus is the real-time transport layer that complements the durable
/// [`Substrate`]. Pulses flow through the Bus for immediate downstream
/// reactions; only those worth persisting get promoted to [`Engram`]s and
/// stored in a Substrate.
///
/// # Sequence numbers
///
/// Every published pulse receives a monotonically increasing sequence number
/// (bus-scoped). Subscribers can use sequence numbers for gap detection and
/// ordered replay.
///
/// # Implementations
///
/// - `PulseBus` (roko-core) — wraps `EventBus<Pulse>` with topic filtering
pub trait Bus: Send + Sync {
    /// The receiver type returned by [`subscribe`](Self::subscribe).
    type Receiver: Send;

    /// Publish a pulse to all matching subscribers. Returns the assigned
    /// sequence number.
    fn publish(&self, pulse: Pulse) -> Result<u64>;

    /// Subscribe to pulses matching the given topic filter.
    fn subscribe(&self, filter: TopicFilter) -> Result<Self::Receiver>;
}
