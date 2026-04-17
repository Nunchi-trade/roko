# The Universal Cognitive Loop

> **Abstract:** Every agent in Roko runs the same seven-step cognitive loop at its own timescale. This revision replaces the older nine-step framing with a version that treats `Pulse` and `Bus` as first-class, makes `PERSIST` and `BROADCAST` co-equal in step 6, and moves Neuro, Daimon, and Dreams out of the loop sequence and into the operators they actually influence. See also [tmp/refinements/05-loop-retold.md](../../tmp/refinements/05-loop-retold.md) and [Naming Map and Glossary](01-naming-and-glossary.md).

> **Implementation status:** Shipping

---

## 1. The Seven-Step Loop

```
1. SENSE      → Substrate.query | Bus.subscribe | external I/O
2. ASSESS     → Scorer + Router jointly rank and select
3. COMPOSE    → Composer assembles a prompt Engram under budget
4. ACT        → execute LLM / tool / chain work, producing Pulses + Engrams
5. VERIFY     → Engram-gates plus stream-gates produce Verdict Engrams
6. PERSIST    → Substrate.put(Engrams)
   BROADCAST  → Bus.publish(Pulses)
7. REACT      → Policy outputs more Pulses and Engrams
```

The universal loop is the same across coding, research, coordination, and consolidation. What changes is the scope of what is sensed, the budget available to compose, and the persistence cadence at each cognitive speed.

## 2. Step-by-Step Specification

### Step 1: SENSE

`SENSE` has three sources:

* `Substrate.query()` for durable Engrams such as plans, episodes, heuristics, verdicts, and stored context.
* `Bus.subscribe()` for live Pulses such as turn output, approval requests, cancellation, and timing signals.
* External I/O for inputs that have not yet been normalized into either fabric, such as LLM streams, subprocess output, filesystem watches, or inbound HTTP requests.

These sources may arrive together. A single tick can merge all three into a unified working set before the next step.

### Step 2: ASSESS

`ASSESS` is a combined Scorer + Router operation. The Scorer assigns relevance and confidence across candidates, and the Router chooses what should be acted on now.

The important part is not just ranking. The combined step answers two questions together: what matters, and why this item wins over the alternatives.

### Step 3: COMPOSE

The Composer turns the selected material into a prompt Engram or other execution bundle under a budget. This is where the current context window is shaped, trimmed, and ordered.

`COMPOSE` can include durable knowledge from the Substrate, live Pulses from the Bus, and task-specific constraints from the current runtime context.

### Step 4: ACT

`ACT` executes the selected work. In the common case this is an LLM turn, but the same step also covers tool calls and chain actions.

The output is typically twofold:

* A stream of Pulses for live observers.
* A final Engram that captures the action result for downstream verification and storage.

### Step 5: VERIFY

`VERIFY` is not a single check. It is a gate pipeline made of Engram-gates plus stream-gates.

* Engram-gates verify durable outputs, producing Verdict Engrams.
* Stream-gates watch live Pulses during execution and can halt or downgrade the step before the final result is accepted.

This keeps verification tied to the actual action path instead of treating it as a post-hoc afterthought.

### Step 6: PERSIST and BROADCAST

Step 6 is intentionally split into two co-equal operations that happen together.

* `PERSIST` writes Engrams into the Substrate with lineage intact.
* `BROADCAST` publishes Pulses onto the Bus for live consumers.

The point is not sequencing. Durable records and ephemeral delivery are different jobs, and the architecture treats them as peers.

### Step 7: REACT

Policies consume the new Pulses and emit further outputs. Those outputs can be additional Pulses, new Engrams, or both.

Common reactions include episode consolidation, circuit-breaking, routing feedback, and task-specific follow-up actions. This is the step where the loop becomes self-modifying over time without pretending that policy is a separate cognitive phase.

## 3. Cross-Cuts Are Not Loop Steps

Neuro, Daimon, and Dreams are cross-cuts. They inject into operators and phases; they do not occupy numbered positions in the loop.

* **Neuro** contributes durable knowledge to `SENSE` and `COMPOSE`. It is the enrichment path for retrieval, prompt assembly, and tier progression.
* **Daimon** biases `ASSESS` and influences `ACT`. It modulates selection and action gating using affective state.
* **Dreams** runs on its own Delta-speed consolidation cycle. It consumes recent Engrams, synthesizes new ones, and feeds the results back into the Substrate.

This distinction matters because the loop is about execution order, while the cross-cuts are about where additional cognitive machinery hooks in.

## 4. The Loop at Three Speeds

The same seven-step loop runs at three cognitive speeds:

* **Gamma** is the fast turn loop. It handles token streams, quick gates, and live context.
* **Theta** is the reflective loop. It handles plan-level work, full verification, and episode consolidation.
* **Delta** is the background loop. It handles Dreams consolidation, deeper synthesis, and slower persistence cadence.

The speed changes the budget and scope, not the structure of the loop.

## 5. Shipping Implementation

The shipping `loop_tick` path should be read as the minimal kernel version of the seven-step model, with Bus integration added alongside the existing Substrate path.

```rust
pub struct TickConfig<'a> {
    pub substrate: &'a dyn Substrate,
    pub bus: Option<&'a dyn Bus>,
    pub scorer: &'a dyn Scorer,
    pub router: &'a dyn Router,
    pub composer: &'a dyn Composer,
    pub policies: &'a [Box<dyn Policy>],
    pub ctx: &'a Context,
    pub budget: &'a Budget,
}

pub async fn loop_tick<'a>(cfg: TickConfig<'a>) -> Result<TickOutcome> {
    // 1. SENSE
    let engrams = cfg.substrate.query(cfg.ctx.sense_predicate()).await?;
    let pulses = match cfg.bus {
        Some(bus) => drain_bus_since(bus, cfg.ctx.last_seq).await?,
        None => Vec::new(),
    };

    // 2. ASSESS
    let data: Vec<Datum> = engrams
        .iter()
        .map(Into::into)
        .chain(pulses.iter().map(Into::into))
        .collect();
    let selection = cfg.router.select(&data, cfg.ctx)?;

    // 3. COMPOSE
    let composed = cfg.composer.compose(&data, cfg.budget, cfg.scorer, cfg.ctx)?;

    // 4. ACT
    let outcome = execute(&composed, cfg.bus, cfg.ctx).await?;

    // 5. VERIFY
    let verdict = cfg.ctx.gate_pipeline().verify(&outcome.engram, cfg.ctx).await?;

    // 6. PERSIST + BROADCAST
    cfg.substrate.put(outcome.engram.clone()).await?;
    cfg.substrate.put(verdict.engram.clone()).await?;
    if let Some(bus) = cfg.bus {
        bus.publish(verdict.to_pulse(Topic::new("gate.verdict.emitted"), 0, cfg.ctx.source())).await?;
    }

    // 7. REACT
    for policy in cfg.policies {
        let out = policy.decide(&pulses, cfg.ctx);
        for pulse in out.pulses {
            if let Some(bus) = cfg.bus {
                bus.publish(pulse).await?;
            }
        }
        for engram in out.engrams {
            cfg.substrate.put(engram).await?;
        }
    }

    Ok(TickOutcome { selection, composed, outcome, verdict })
}
```

The important architectural point is that Bus support is additive, not a replacement for Substrate. The current implementation can keep the substrate-only path as the default while callers migrate to live Pulse handling.

## 6. Why This Is the Right Shape

The revised loop matches the system the runtime actually runs:

* It separates durable knowledge from live transport.
* It collapses score-and-route into one decision point.
* It keeps verification tied to both durable outputs and live streams.
* It treats persistence and publication as co-equal operations.
* It keeps cross-cuts explicit instead of smuggling them into the loop as a ninth step.

That is the load-bearing change in [tmp/refinements/05-loop-retold.md](../../tmp/refinements/05-loop-retold.md), and this doc is now aligned to it.

## Cross-References

* [Naming Map and Glossary](01-naming-and-glossary.md)
* [tmp/refinements/05-loop-retold.md](../../tmp/refinements/05-loop-retold.md)
