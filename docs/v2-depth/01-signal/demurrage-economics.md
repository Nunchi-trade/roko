# Demurrage Economics

> Depth for [01-SIGNAL.md](../../unified/01-SIGNAL.md) &sect;6. Derives the demurrage rate law from first principles, unifies decay, tiers, and the attention economy into a single economic model with identifiable equilibria and phase transitions.

---

## 1. First Principles: Why Idle Knowledge Has a Cost

Silvio Gesell (1916) observed that physical goods depreciate -- grain rots, iron rusts -- but money does not. This asymmetry gives money holders an unfair advantage: they can wait indefinitely while goods holders cannot. His solution: charge a *demurrage fee* on idle money to match the depreciation rate of real goods.

The same asymmetry exists in knowledge systems. A live insight that just helped an agent complete a task is *valuable*. The same insight sitting untouched in Store for 30 days is *costly*: it occupies index space, it appears in similarity queries where it may be stale, and it crowds out fresher knowledge in context windows. Yet unlike physical goods, knowledge does not rot on its own. Without demurrage, Store becomes a hoarder's attic.

From information theory, the cost of storing a message of length L bits for time T is at least `L * T * r` where `r` is the marginal cost of one bit-second of storage. The demurrage rate is the *economic* expression of this information-theoretic floor: knowledge must earn its keep or be archived.

### 1.1 The Gesell-Shannon Derivation

Start from two premises:

**P1 (Gesell)**: Idle assets should bear a holding cost proportional to their value and the duration of idleness.

**P2 (Shannon)**: The information content of a message is `H = -log2(p)`. Redundant messages (high p) carry less information. Novel messages (low p) carry more.

Combining: the holding cost of a Signal should be inversely related to its informational contribution. A Signal that is highly redundant with existing Store contents (low novelty) should pay *more* to stay warm, because its marginal information value is lower. A Signal that is unique (high novelty) should pay *less*, because losing it is costlier.

This gives the novelty-weighted demurrage rate:

```
effective_rate(signal) = base_rate / (1 + novelty(signal))
```

Where `novelty(signal) = 1 - max_similarity(signal, top_K_neighbors)` using HDC Hamming distance. A perfectly novel Signal (no similar neighbors) pays `base_rate / 2`. A perfectly redundant Signal (identical to a neighbor) pays the full `base_rate`.

This is the anti-hoarding mechanism described in [01-SIGNAL.md](../../unified/01-SIGNAL.md) &sect;6. It emerges from first principles, not from ad hoc tuning.

---

## 2. The Rate Law

The demurrage balance evolves according to a differential equation with three terms:

```
d(balance)/dt = -r - beta * balance + reinforcement(t)
```

Discretized per tick:

```rust
/// Demurrage tick: charge holding cost and apply reinforcement.
///
/// The rate law has three terms:
///   1. Flat tax:     -r * dt           (constant drain, floor-aware)
///   2. Proportional: -beta * b * dt    (wealthy Signals pay more)
///   3. Reinforcement: +bonus * novelty (earned by use)
pub fn demurrage_tick(
    signal: &mut Signal,
    dt_days: f64,
    config: &DemurrageConfig,
    novelty: f64,
    reinforcement: Option<ReinforceKind>,
) {
    let tier = signal.tier;

    // 1. Charge: flat + proportional, scaled by tier
    let charge_mult = tier.charge_multiplier() as f64;
    let flat_charge = config.flat_tax_per_day * dt_days * charge_mult;
    let prop_charge = config.exp_decay_per_day * signal.balance * dt_days * charge_mult;
    let total_charge = flat_charge + prop_charge;

    signal.balance -= total_charge;
    signal.demurrage_paid += total_charge;

    // 2. Reinforce if earned
    if let Some(kind) = reinforcement {
        let base_bonus = config.bonus_for(kind);
        let novelty_weight = novelty;  // 0..1, anti-hoarding
        let reinforce_mult = tier.reinforcement_multiplier() as f64;
        let earned = base_bonus * novelty_weight * reinforce_mult;
        signal.balance += earned;
    }

    // 3. Floor enforcement
    let floor = tier.cold_floor() as f64;
    if signal.balance < floor {
        signal.balance = floor;
        // Signal is candidate for cold storage
    }

    signal.last_touched_at = Utc::now();
}
```

### 2.1 Steady-State Analysis

At steady state, `d(balance)/dt = 0`, so:

```
0 = -r - beta * b_ss + reinforcement_rate
b_ss = (reinforcement_rate - r) / beta
```

If `reinforcement_rate > r`, the Signal has a positive steady-state balance and stays warm. If `reinforcement_rate < r`, the balance declines until it hits the cold floor.

This means there is a *critical reinforcement rate*:

```
reinforcement_critical = r
```

Any Signal whose reinforcement rate exceeds the flat tax survives indefinitely. Any Signal below that rate decays. The exponential term `beta * b` prevents balance from growing without bound -- even a heavily-cited Signal has a finite steady-state balance.

### 2.2 The Half-Life Correspondence

For a Signal receiving no reinforcement at all, the balance decays as:

```
b(t) = (b_0 + r/beta) * exp(-beta * t) - r/beta
```

The effective half-life is:

```
t_half = ln(2) / beta
```

For the default `beta = 0.02/day`, the half-life is `ln(2) / 0.02 = 34.7 days`. This is the connection to the legacy `Decay::HalfLife` model: demurrage with zero flat tax and zero reinforcement reduces to exponential decay, which is exactly HalfLife.

```
Demurrage(r=0, beta, reinforcement=0)  =  HalfLife(half_life = ln(2)/beta)
```

Similarly, Ebbinghaus decay is the special case where `beta` varies with the number of retrievals (spaced repetition). Demurrage subsumes both legacy models.

---

## 3. The Phase Space

The state of a Signal in the demurrage system is described by three variables:

- **balance** (b): current attention credit, [0, unbounded)
- **tier** (T): Transient | Working | Consolidated | Persistent
- **novelty** (n): HDC dissimilarity to nearest neighbors, [0, 1]

These define a 3D phase space where the dynamics play out:

```
        balance
        ^
        |     * Persistent equilibrium (b > 1.2)
   1.2  |---- - - - - - - - - - - - - - - - -
        |     * Consolidated equilibrium (0.8 < b < 1.2)
   0.8  |---- - - - - - - - - - - - - - - - -
        |     * Working equilibrium (0.35 < b < 0.8)
   0.35 |---- - - - - - - - - - - - - - - - -
        |     * Transient zone (b < 0.35)
   0.0  +-----------------------------------> novelty
        0.0                                 1.0
```

### 3.1 Fixed Points (Equilibria)

Each tier has a **basin of attraction** defined by its balance band:

| Tier | Balance band | Charge multiplier | Reinforcement multiplier | Equilibrium character |
|---|---|---|---|---|
| Transient | < 0.35 | 2.0x | 1.5x | Unstable: high charge pushes toward cold floor or Working |
| Working | 0.35 - 0.80 | 1.0x | 1.0x | Metastable: survives with moderate reinforcement |
| Consolidated | 0.80 - 1.20 | 0.5x | 0.75x | Stable: low charge, broad reinforcement keeps it here |
| Persistent | > 1.20 | 0.1x | 0.5x | Deeply stable: almost pinned, requires sustained contradiction to dislodge |

The key insight: **Transient is an unstable equilibrium**. A Signal in Transient either gets reinforced and climbs to Working, or fails to earn reinforcement and drops to cold storage. There is no comfortable resting state in Transient. This is by design -- new knowledge should either prove itself quickly or get out of the way.

### 3.2 Phase Transitions

Tier promotion and demotion are **phase transitions** in the dynamical system. They are not smooth -- they are discrete jumps that change the charge and reinforcement multipliers, altering the dynamics.

```rust
/// Tier transition as a phase transition.
/// When balance crosses a tier boundary AND usage criteria are met,
/// the Signal's dynamics change discontinuously.
pub fn check_tier_transition(
    signal: &mut Signal,
    stats: &UsageStats,
) -> Option<TierTransition> {
    // Promotion: balance in higher band + usage criteria
    if let Some(target) = check_promotion(signal, stats) {
        let from = signal.tier;
        signal.tier = target;
        return Some(TierTransition {
            from,
            to: target,
            direction: Direction::Promotion,
            trigger: stats.last_event.clone(),
        });
    }
    // Demotion: balance in lower band OR contradiction
    if let Some(target) = check_demotion(signal, stats) {
        let from = signal.tier;
        signal.tier = target;
        return Some(TierTransition {
            from,
            to: target,
            direction: Direction::Demotion,
            trigger: stats.last_event.clone(),
        });
    }
    None
}
```

### 3.3 Conditions for Getting Stuck

A Signal can get stuck in a tier under these conditions:

**Stuck in Transient** (the graveyard orbit): Reinforcement exactly offsets charge. Balance hovers near 0.35 without crossing. The Signal is too useful to freeze but not useful enough to promote. *Fix*: the 2.0x charge multiplier makes this unstable -- any perturbation pushes it one way or the other.

**Stuck in Consolidated** (the comfortable middle): Low charge (0.5x) and broad reinforcement criteria make Consolidated very comfortable. A Signal can sit here for months without reaching Persistent because the promotion criteria are strict (10+ uses, 3+ sessions, no contradictions). *This is acceptable*: Consolidated is the intended home for most useful knowledge.

**Stuck in Persistent** (the ossification trap): Once a Signal reaches Persistent, the 0.1x charge makes it nearly free to hold. Even if the Signal becomes outdated, the very low charge means balance declines extremely slowly. *Fix*: contradiction-based demotion is the escape valve. Even one unresolved contradiction at the Persistent tier triggers a demotion to Consolidated.

---

## 4. The Tier Progression as a Markov Chain

Modeling tier transitions as a Markov chain makes the long-run distribution of knowledge across tiers predictable and tunable.

```
                  p_tw                p_wc                p_cp
  Transient ──────────► Working ──────────► Consolidated ──────────► Persistent
      ▲                    ▲                     ▲                      |
      |    p_wt            |    p_cw             |       p_pc           |
      ◄────────────────────◄─────────────────────◄──────────────────────
      |
      |  p_t_cold
      ▼
  Cold Storage
      |
      |  p_cold_t (thaw)
      ▼
  Transient (restart)
```

The transition matrix:

```rust
/// Markov transition probabilities per demurrage tick.
/// These are *conditional* probabilities given the current tier.
///
/// Values are illustrative defaults calibrated from the
/// charge multipliers and typical reinforcement rates.
pub struct TierTransitionMatrix {
    pub p_transient_to_working:     f64,  // 0.15 per day
    pub p_transient_to_cold:        f64,  // 0.10 per day
    pub p_working_to_transient:     f64,  // 0.05 per day
    pub p_working_to_consolidated:  f64,  // 0.08 per day
    pub p_consolidated_to_working:  f64,  // 0.02 per day
    pub p_consolidated_to_persistent: f64, // 0.03 per day
    pub p_persistent_to_consolidated: f64, // 0.005 per day
    pub p_cold_to_transient:        f64,  // 0.01 per day (thaw)
}
```

### 4.1 Stationary Distribution

At the stationary distribution (long-run equilibrium), the fraction of knowledge at each tier is determined by the balance between inflow and outflow rates.

For the default parameters:
- ~15% Transient (high turnover)
- ~35% Working (the active workspace)
- ~30% Consolidated (the reliable library)
- ~10% Persistent (the bedrock)
- ~10% Cold (the archive)

These fractions are controllable by tuning the charge and reinforcement multipliers. If the system hoards too much (>50% Persistent), increase `persistent_charge_multiplier`. If it forgets too fast (<10% Consolidated), decrease `consolidated_charge_multiplier` or increase reinforcement bonuses.

### 4.2 Ergodicity

The Markov chain is ergodic if every tier is reachable from every other tier. Cold storage -> Transient (thaw) ensures that even frozen knowledge can re-enter the warm path. The chain is therefore ergodic, which guarantees:

1. A unique stationary distribution exists
2. The system converges to it regardless of initial conditions
3. Time averages equal ensemble averages (useful for observability)

---

## 5. The VCG Attention Auction as Live-Economy Dual

See [01-SIGNAL.md](../../unified/01-SIGNAL.md) &sect;6 for demurrage basics. The attention auction (from `25-attention-as-currency`) is the *live-economy dual* of the demurrage ledger:

| Property | Demurrage (memory ledger) | VCG Auction (loop ledger) |
|---|---|---|
| **What is spent** | Balance (holding cost) | Attention tokens (compute cost) |
| **When charged** | Between loops (idle tax) | During loops (active spend) |
| **Who pays** | The Signal (for existing) | The loop (for using) |
| **Incentive** | Signals must earn reinf. to survive | Signals must bid high to get selected |
| **Failure mode** | Hoarding (too much survives) | Starvation (too little selected) |

The duality: demurrage governs *who stays in memory*; the auction governs *who enters the context window*. A Signal with high balance but low bid score survives in Store but never gets used. A Signal with high bid score but declining balance gets used now but may not be available later.

```rust
/// The two ledgers interact at composition time.
///
/// VCG auction selects which Signals enter the context window.
/// Winning the auction counts as a `Retrieved` reinforcement event
/// on the demurrage ledger.
pub fn auction_and_reinforce(
    candidates: &mut [Signal],
    auction: &AttentionAuction,
    budget: &mut AttentionToken,
) -> AuctionOutcome {
    // 1. Run auction: select winners based on Score
    let mut bids: Vec<AttentionBid> = candidates.iter()
        .map(|s| AttentionBid {
            signal_ref: s.ref_(),
            bid_value: s.score.effective(),
            estimated_cost: estimate_context_cost(s),
            priority: classify_priority(s),
        })
        .collect();

    let outcome = auction.run(&mut bids, budget);

    // 2. Winners get a Retrieved reinforcement on the memory ledger
    for winner in &outcome.winners {
        if let Some(signal) = candidates.iter_mut()
            .find(|s| s.ref_() == winner.signal_ref)
        {
            let novelty = compute_novelty(signal);
            demurrage_reinforce(
                signal,
                ReinforceKind::Retrieved,
                novelty,
            );
        }
    }

    outcome
}
```

The reinforcement loop closes: Signals that win auctions get reinforced, which keeps their balance high, which keeps them available for future auctions. This is a **positive feedback loop** that concentrates attention on useful knowledge -- and it is checked by the novelty weighting, which prevents highly-cited but redundant Signals from monopolizing reinforcement.

---

## 6. Per-Kind Rate Tables

Different kinds of knowledge have different natural lifespans. The base demurrage rates are tuned per kind.

```toml
[demurrage.rates]
# Kind               flat_tax   exp_decay   rationale
text              = { r = 0.001, beta = 0.001 }  # Data artifacts: inherently stable
code              = { r = 0.001, beta = 0.001 }  # Source code: stable until refactored
insight           = { r = 0.010, beta = 0.020 }  # Observations: need ongoing confirmation
heuristic         = { r = 0.005, beta = 0.010 }  # Behavioral rules: durable once proven
warning           = { r = 0.100, beta = 0.200 }  # Danger flags: deliberately short-lived
causal_link       = { r = 0.005, beta = 0.008 }  # Cause-effect: survives longer than episode
strategy_fragment = { r = 0.020, beta = 0.030 }  # Strategies: go stale in evolving codebases
anti_knowledge    = { r = 0.010, beta = 0.020 }  # What-not-to-do: stays relevant
episode           = { r = 0.005, beta = 0.010 }  # Agent turns: feed learning loops
verdict           = { r = 0.002, beta = 0.003 }  # Gate verdicts: audit evidence, long-lived
```

### 6.1 Derived Half-Lives

From the rate law, the unreinforced half-life per kind:

| Kind | beta | Half-life (days) | Interpretation |
|---|---|---|---|
| Warning | 0.200 | 3.5 | Danger signals expire in under a week |
| StrategyFragment | 0.030 | 23 | Strategies go stale in about a month |
| Insight | 0.020 | 35 | Observations need fresh confirmation monthly |
| AntiKnowledge | 0.020 | 35 | Old mistakes stop blocking after a month |
| Heuristic | 0.010 | 69 | Proven rules persist for two months |
| Episode | 0.010 | 69 | Learning data persists for two months |
| CausalLink | 0.008 | 87 | Causal knowledge persists nearly three months |
| Verdict | 0.003 | 231 | Audit evidence persists for most of a year |
| Text/Code | 0.001 | 693 | Data artifacts persist for nearly two years |

These are the *unreinforced* half-lives. Any reinforcement extends them. A heavily-cited heuristic can persist indefinitely despite having a 69-day base half-life.

---

## 7. Cold Storage and Thaw

When balance drops below the tier's cold floor, the Signal enters cold storage. This is not deletion -- it is an economic tier shift.

```rust
/// Cold storage: archive the Signal body, keep the hash and lineage.
///
/// The Signal's hdc_fingerprint stays in the warm index for thaw
/// discovery, but its payload moves to slower storage.
pub async fn freeze(
    store: &dyn Store,
    cold_store: &dyn ColdStore,
    signal: &mut Signal,
) -> Result<()> {
    // 1. Move payload to cold storage
    cold_store.archive(signal.content_hash, &signal.payload).await?;

    // 2. Mark as frozen in warm index
    signal.tier = Tier::Frozen;
    signal.balance = 0.0;
    store.update_metadata(signal).await?;

    // 3. Publish thaw-eligible notification
    let pulse = Pulse::new(
        Topic::parse("knowledge.frozen"),
        Kind::Presence { event: PresenceEvent::Frozen },
        json!({ "hash": signal.content_hash.to_hex() }),
    );
    bus.publish(pulse).await?;

    Ok(())
}

/// Thaw: restore a frozen Signal to active duty.
///
/// Balance restarts at thaw_start_balance (default 0.3).
/// Tier restarts at Transient -- the Signal must re-earn its place.
pub async fn thaw(
    store: &dyn Store,
    cold_store: &dyn ColdStore,
    content_hash: ContentHash,
    config: &DemurrageConfig,
) -> Result<Signal> {
    // 1. Retrieve from cold storage
    let payload = cold_store.retrieve(content_hash).await?;

    // 2. Restore with conservative balance
    let mut signal = store.get_metadata(content_hash).await?;
    signal.payload = payload;
    signal.balance = config.thaw_start_balance;
    signal.tier = Tier::Transient;
    signal.last_touched_at = Utc::now();
    store.update(signal.clone()).await?;

    // 3. Publish thaw event (graduated Pulse)
    let pulse = Pulse::new(
        Topic::parse("knowledge.thawed"),
        Kind::Presence { event: PresenceEvent::Thawed },
        json!({
            "hash": content_hash.to_hex(),
            "restart_balance": config.thaw_start_balance,
        }),
    );
    bus.publish(pulse).await?;

    Ok(signal)
}
```

### 7.1 Thaw Triggers

A frozen Signal is thawed when:

1. **Query hit**: A Store similarity query returns a frozen Signal above the relevance threshold. The Store automatically thaws it before returning it to the caller.
2. **Explicit request**: An agent or operator explicitly requests a Signal by content hash.
3. **Cross-reference**: A new Signal's lineage references a frozen Signal. The lineage walker thaws it to maintain DAG integrity.
4. **Consolidation discovery**: During delta-speed consolidation, the Dreams engine discovers that a frozen Signal fills a gap in the current knowledge graph.

---

## 8. Observability

The demurrage system produces telemetry that answers the operational questions:

```rust
/// Demurrage telemetry emitted per consolidation cycle.
pub struct DemurrageTelemetry {
    pub timestamp: DateTime<Utc>,

    // Distribution
    pub balance_histogram: Vec<(f64, usize)>,  // (bin_edge, count)
    pub tier_counts: BTreeMap<Tier, usize>,

    // Flow rates
    pub total_charged: f64,          // sum of all demurrage charges this cycle
    pub total_reinforced: f64,       // sum of all reinforcement credits this cycle
    pub net_flow: f64,               // reinforced - charged (positive = healthy)

    // Transitions
    pub promotions: Vec<TierTransition>,
    pub demotions: Vec<TierTransition>,
    pub freezes: usize,
    pub thaws: usize,

    // Health indicators
    pub hoarding_index: f64,         // fraction of Signals with balance > 2.0
    pub starvation_index: f64,       // fraction of Signals with balance < 0.1
    pub reinforcement_by_kind: BTreeMap<String, f64>,

    // Top Signals
    pub attention_leaderboard: Vec<(SignalRef, f64)>,  // top 10 by balance
}
```

### 8.1 Dashboard Tiles

| Tile | What it shows | Why it matters |
|---|---|---|
| Balance histogram | Distribution of balance across all warm Signals | Detects hoarding (right-skewed) or starvation (left-skewed) |
| Net flow gauge | reinforced - charged per cycle | Negative = system is forgetting faster than learning |
| Tier pie chart | Fraction of Signals per tier | Healthy: most in Working/Consolidated |
| Thaw rate | Freezes and thaws per day | High thaw rate = cold floor too aggressive |
| Reinforcement breakdown | Which ReinforceKind drives the most credit | Tells you *how* knowledge stays alive |
| Attention leaderboard | Top 10 Signals by balance | Spots over-consolidated knowledge |

---

## 9. Feedback Loops

1. **Use -> Reinforce -> Survive -> Use** (virtuous cycle): Useful Signals get reinforced, which keeps them warm, which makes them available for future use. This is the intended steady state for valuable knowledge.

2. **Novelty -> Bonus -> Survive -> Reduce novelty** (self-regulating): Novel Signals get larger reinforcement bonuses, but as similar Signals accumulate, novelty decreases, bonuses shrink, and eventually only the most distinctive Signal in a cluster survives. This is the anti-hoarding mechanism.

3. **Contradiction -> Demote -> Low balance -> Freeze** (immune response): When a Signal is contradicted (by a gate failure or a newer Signal with opposing evidence), it gets demoted. Demotion increases the charge multiplier, accelerating balance decline. This is the knowledge immune system -- bad knowledge is actively expelled.

4. **Thaw -> Transient -> Prove again -> Promote** (second chances): Frozen knowledge that gets thawed restarts at Transient. It must re-earn its place through fresh reinforcement. This prevents "zombie knowledge" that was frozen for good reason from immediately returning to Consolidated.

5. **Delta dividend -> Better heuristics -> Cheaper ticks -> More budget -> More reinforcement**: Consolidation produces better heuristics, which reduce future attention spend per tick, which leaves more budget for reinforcement events, which keeps more Signals warm. The demurrage economy and the attention economy are coupled through the consolidation cycle.

---

## 10. Open Questions

1. **Rate auto-tuning**: Should the demurrage rates be fixed or should the system learn them from the stationary distribution? If the Persistent tier grows beyond 20%, should `persistent_charge_multiplier` automatically increase? This is a meta-economic question: who sets the tax rates?

2. **Inter-Space demurrage**: When Signals are shared across Spaces (workspaces), should they carry their balance with them or start fresh? If carried, a Signal rich in Space A gets a free ride in Space B. If reset, useful cross-Space knowledge must re-earn its place.

3. **Demurrage and consensus**: In a multi-agent setting, different agents may reinforce different Signals. Should reinforcement from multiple agents compound (rewarding consensus) or should it be capped (preventing popularity bias)?

4. **AntiKnowledge demurrage floor**: Should AntiKnowledge Signals have a guaranteed minimum balance that prevents them from ever freezing? The current spec says they "remain retrievable after freeze," but the mechanism for this is unclear. A pinned floor balance is one option; exemption from demurrage is another.

5. **Real-money grounding**: If Roko operates in an economic context (on-chain, marketplace), should demurrage rates be denominated in real tokens? The current model uses dimensionless balance units. Grounding in real cost would make the economics auditable but also introduces exchange rate risk.
