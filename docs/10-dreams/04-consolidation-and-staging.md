# Consolidation and the Staging Buffer

> **Layer**: Cognitive Cross-Cut (L2 Scaffold → L3 Harness gate validation)
>
> **Synapse Traits**: `Substrate` (NeuroStore write), `Gate` (confidence threshold validation), `Composer` (Engram assembly)
>
> **Crate**: `roko-dreams` — Integration logic within `cycle.rs`
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md), [02-nrem-replay.md](02-nrem-replay.md), [03-rem-imagination.md](03-rem-imagination.md)


> **Implementation**: Scaffold

---

## What Consolidation Does

Consolidation is the third and final phase of each dream cycle. It is a pure computation phase — no LLM call required. Its purpose is to evaluate the outputs from NREM replay (insights, cross-episode patterns) and REM imagination (counterfactual hypotheses, creative recombinations), stage them for future validation, and promote validated entries to permanent knowledge in NeuroStore.

The biological basis is the synaptic homeostasis hypothesis (Tononi & Cirelli 2006, Sleep Medicine Reviews): during waking, synaptic connections accumulate. During sleep, a global renormalization occurs — important connections are strengthened while unimportant ones are pruned. Consolidation in Roko performs the equivalent: it selectively strengthens high-value knowledge and lets low-value knowledge decay.

---

## The Staging Buffer

### Design

Dream-generated hypotheses do not go directly into permanent knowledge. They enter a **staging buffer** — a holding area where hypotheses wait for waking validation. This is the "dream → reality check" pipeline.

The staging buffer is implemented as a SQLite table within the NeuroStore:

```sql
CREATE TABLE staged_hypotheses (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    confidence REAL DEFAULT 0.25,
    source_phase TEXT NOT NULL,          -- 'nrem_replay' | 'rem_counterfactual' | 'rem_combinational' | etc.
    source_episodes TEXT,                -- JSON array of episode IDs that contributed
    generation_mode TEXT,                -- e.g., 'pearl_scm_l3', 'boden_transformational'
    created_at INTEGER NOT NULL,         -- Unix timestamp in milliseconds
    last_validated_at INTEGER,           -- Updated when waking evidence is found
    validation_count INTEGER DEFAULT 0,  -- Number of independent waking confirmations
    status TEXT DEFAULT 'staged',        -- staged | partially_validated | validated | promoted | expired | refuted
    hdc_vector BLOB,                     -- 10,240-bit BSC vector for similarity queries
    contradicts TEXT,                    -- ID of existing knowledge entry this contradicts (if any)
    novelty_score REAL DEFAULT 0.0       -- HDC distance from existing knowledge centroid
);

CREATE INDEX idx_staged_status ON staged_hypotheses(status);
CREATE INDEX idx_staged_confidence ON staged_hypotheses(confidence);
CREATE INDEX idx_staged_created ON staged_hypotheses(created_at);
```

### Why a Staging Buffer?

Dreams are creative but unreliable. The REM phase deliberately suppresses executive control to enable novel combinations — but this means many dream outputs are speculative, contradictory, or simply wrong. The staging buffer provides a "trial period" where hypotheses can be tested against reality before being trusted.

This mirrors the biological process: not all dream content is adaptive. Much of it is noise. The brain's consolidation mechanisms selectively strengthen useful associations and let useless ones decay. The staging buffer implements the same selection pressure.

---

## The Confidence Ladder

Hypotheses climb through a five-stage confidence ladder. Each stage represents a different level of trust:

| Stage | Confidence Range | Status | Description | Action Taken |
|-------|-----------------|--------|-------------|-------------|
| 1 | 0.20–0.30 | `staged` | Just entered from a dream. No waking evidence yet. | Stored in staging buffer. Not used for decision-making. |
| 2 | 0.30–0.50 | `partially_validated` | Some waking evidence supports the hypothesis. | Agent may reference it in context but does not rely on it. |
| 3 | 0.50–0.70 | `validated` | Multiple independent confirmations. | Agent begins to act on this hypothesis tentatively. |
| 4 | ≥ 0.70 | `promoted` | Hypothesis is promoted to permanent NeuroStore entry. | Written to NeuroStore as Insight or Heuristic. May be compiled into PLAYBOOK.md. |
| 5 | (special) | `refuted` | Waking evidence directly contradicts the hypothesis. | Marked as refuted. If it contradicts existing knowledge, the existing entry's confidence is increased. |

### Confidence Boost Mechanics

Each independent waking confirmation boosts a hypothesis's confidence by a fixed delta:

```
new_confidence = old_confidence + confirmation_boost × (1.0 - old_confidence)
```

Where `confirmation_boost` is configurable (default: 0.15). The `(1.0 - old_confidence)` factor ensures diminishing returns — each successive confirmation adds less. This prevents runaway confidence accumulation from repeated similar events.

An "independent confirmation" is defined as a waking episode that:
1. Has HDC similarity > 0.60 to the hypothesis content
2. Occurred after the hypothesis was created
3. Has a successful outcome (passed all gates)
4. Was not itself generated during a dream cycle

### Refutation Mechanics

A hypothesis is refuted when waking evidence directly contradicts it:
1. A waking episode with HDC similarity > 0.60 to the hypothesis content
2. Has a failed outcome (or the opposite of what the hypothesis predicted)
3. The contradiction is flagged with the contradicting episode's ID

Refuted hypotheses are not deleted — they remain in the staging buffer with `status: 'refuted'` for future reference. The refutation itself becomes a useful data point: "I dreamed X, but reality showed not-X."

If the hypothesis originally contradicted an existing knowledge entry (marked in the `contradicts` field), that existing entry receives a confidence boost of 0.10 — the existing knowledge was right and the dream hypothesis was wrong.

---

## Promotion to Permanent Knowledge

When a hypothesis reaches confidence ≥ 0.70, the integration phase promotes it:

### Step 1: Knowledge Type Assignment

The promoted hypothesis is classified into a knowledge type based on its content and generation mode:

| Generation Mode | Typical Knowledge Type | Description |
|----------------|----------------------|-------------|
| NREM standard/reverse replay | **Insight** | Pattern extracted from experience replay |
| NREM cross-episode pattern | **Insight** | Structural similarity across episodes |
| REM Pearl SCM (any level) | **Heuristic** | Causal relationship with actionable prediction |
| REM Boden combinational | **Insight** | Novel connection between domains |
| REM Boden exploratory | **Heuristic** | Boundary condition of an existing strategy |
| REM Boden transformational | **Strategy** | Novel approach based on assumption violation |
| Threat simulation | **Warning** | Anticipated threat with early warning signs |

### Step 2: NeuroStore Write

The hypothesis is written to NeuroStore as a `KnowledgeEntry`:

```rust
let entry = KnowledgeEntry {
    id: generate_id(),
    content: hypothesis.content.clone(),
    kind: assigned_knowledge_type,
    confidence: hypothesis.confidence,
    source: "dream".to_string(),
    source_episodes: hypothesis.source_episodes.clone(),
    hdc_vector: hypothesis.hdc_vector.clone(),
    created_at: Utc::now(),
    half_life: default_half_life_for_type(assigned_knowledge_type),
    provenance: Provenance::Dream {
        dream_cycle_id: current_cycle_id.clone(),
        generation_mode: hypothesis.generation_mode.clone(),
    },
};
knowledge_store.write(entry).await?;
```

### Step 3: PLAYBOOK.md Update (for Heuristics)

If the promoted entry is a Heuristic (an actionable rule), it is also compiled into `PLAYBOOK.md`:

```markdown
## Heuristic: {heuristic_title}

**Confidence**: {confidence}
**Source**: Dream cycle {cycle_id}, {generation_mode}
**Validated by**: {validation_episodes}

{heuristic_content}

**When to apply**: {conditions}
**When NOT to apply**: {boundary_conditions}
```

PLAYBOOK.md is the agent's executable strategy document — a human-readable, machine-parseable list of rules the agent follows during waking operation. Dream-promoted heuristics join rules extracted from direct experience.

### Step 4: Staging Buffer Update

The staging buffer entry is updated:

```sql
UPDATE staged_hypotheses
SET status = 'promoted',
    last_validated_at = {now}
WHERE id = {hypothesis_id};
```

### Step 5: Event Emission

A `DreamOutcomeEvent` is emitted for downstream listeners:

```rust
pub struct DreamOutcomeEvent {
    pub hypothesis_id: String,
    pub validated: bool,
    pub confidence: f64,
    pub dream_cycle_origin: String,
    pub validation_episodes: Vec<String>,
}
```

---

## Temporal Decay and Expiration

### Unvalidated Hypothesis Expiration

Hypotheses that are not validated within a configurable window (default: 5,000 ticks / ~3.5 days for the legacy system, or 14 days in Roko's time-based scheduling) expire:

```sql
UPDATE staged_hypotheses
SET status = 'expired'
WHERE status IN ('staged', 'partially_validated')
AND created_at < {now} - {expiration_window};
```

Expired hypotheses are not deleted. They remain queryable for research purposes but are no longer candidates for promotion.

### Knowledge Demurrage

Promoted knowledge entries in NeuroStore are subject to temporal decay (demurrage). Each knowledge type has a default half-life:

| Knowledge Type | Default Half-Life | Rationale |
|---------------|------------------|-----------|
| **Insight** | 30 days | Insights from patterns are moderately durable |
| **Heuristic** | 90 days | Actionable rules should persist longer |
| **Strategy** | 60 days | Strategies need regular revalidation |
| **Warning** | 14 days | Threats are time-sensitive |
| **Fact** | 365 days | Verified facts decay slowly |

Confidence decays exponentially:

```
current_confidence = initial_confidence × 2^(-age / half_life)
```

Each independent waking confirmation resets the decay clock and applies a 1.5× confirmation boost to the confidence score. This implements the stigmergic principle from Grassé (1959, Insectes Sociaux 6(1)): knowledge entries are like pheromone deposits that decay over time. Confirmed entries are reinforced; unconfirmed entries evaporate.

Dream-validated entries receive a special "dream_validated" tag. Entries with this tag are exempt from decay for the first 7 days after validation, giving the waking agent time to encounter situations where the entry can be independently confirmed.

---

## Dream-Specific Quality Calibration

Not all dream outputs are equal. The integration phase applies quality calibration based on the generation mode and source:

| Quality Signal | Effect on Confidence | Rationale |
|---------------|---------------------|-----------|
| Hypothesis contradicts no existing knowledge | No modification | Neutral signal |
| Hypothesis confirms existing knowledge | +0.05 confidence bonus | Reinforcement |
| Hypothesis contradicts existing knowledge | -0.05 confidence penalty | Requires stronger evidence |
| Generated by transformational creativity | -0.05 confidence penalty | Most speculative mode |
| Generated by Pearl SCM Level 3 | No modification | Deep reasoning, neither bonus nor penalty |
| Multiple source episodes | +0.02 per source episode (max +0.10) | More evidence = more confidence |
| Source episode had high prediction error | +0.03 confidence bonus | High-surprise episodes yield more valuable insights |

---

## The Dream Journal

Every completed consolidation cycle produces a `DreamCycleReport` that is persisted as the agent's "dream journal":

```rust
pub struct DreamCycleReport {
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub processed_through: Option<DateTime<Utc>>,
    pub episodes_replayed: usize,
    pub counterfactuals_generated: usize,
    pub insights: Vec<InsightRecord>,
    pub patterns: Vec<PatternRecord>,
    pub staged_hypotheses: usize,
    pub promoted_hypotheses: usize,
    pub confidence_updates: usize,
    pub depotentiation: DepotentiationSummary,
}
```

Reports are stored in `.roko/dreams/dream-{unix_timestamp_ms}.json`. The `DreamRunner::latest_report()` method retrieves the most recent report. This enables:

- Human reviewers to inspect what the agent learned during its dream cycles
- Other agents to query dream journal entries for cross-agent knowledge sharing
- Post-hoc analysis of dream quality and consolidation effectiveness

---

## Safety Constraints

Dream outputs are subject to the same safety constraints as waking actions:

1. **No direct action from dreams**: Dream hypotheses cannot trigger tools, modify files, or take any external action. They can only be written to the staging buffer and, upon promotion, to NeuroStore.
2. **Budget limits**: The number of staging buffer entries is capped (default: 1,000). When the cap is reached, the oldest expired entries are garbage collected first, then the lowest-confidence staged entries.
3. **Contradiction limits**: A single dream cycle can produce at most 3 entries that contradict existing knowledge. Additional contradictions are discarded to prevent dream cycles from destabilizing the agent's knowledge base.
4. **Confidence ceiling**: Dream-generated entries cannot have initial confidence above 0.30. All higher confidence must come from waking validation.

---

## Academic Citations

| Paper | How It Informs Consolidation |
|-------|------------------------------|
| Tononi & Cirelli (2006), Sleep Medicine Reviews, "Synaptic homeostasis hypothesis" | Global renormalization during sleep: strengthen important, prune unimportant |
| Stickgold & Walker (2013), "Sleep-dependent memory triage" | Selective memory consolidation during sleep transitions |
| McClelland et al. (1995), Psychological Review, CLS theory | Fast episodic → slow semantic transfer via sleep replay |
| Grassé (1959), Insectes Sociaux 6(1) | Stigmergic knowledge: pheromone deposits that decay without reinforcement |
| Park et al. (2023), UIST, arXiv:2304.03442, "Generative Agents" | Memory synthesis and reflection cycle architecture |
| WSCL (2024), "Wake-sleep continual learning" | 38% reduction in catastrophic forgetting via interleaved wake-sleep training |

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [02-nrem-replay.md](02-nrem-replay.md) | NREM outputs that enter the staging buffer |
| [03-rem-imagination.md](03-rem-imagination.md) | REM outputs that enter the staging buffer |
| [05-dream-evolution.md](05-dream-evolution.md) | EVOLUTION phase that operates on promoted knowledge |
| [../03-neuro/INDEX.md](../06-neuro/INDEX.md) | NeuroStore where promoted entries are persisted |
