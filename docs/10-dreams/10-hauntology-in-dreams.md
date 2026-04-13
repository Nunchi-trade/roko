# Hauntology in Dreams: Spectral Knowledge Recombination

> **Layer**: Cognitive Cross-Cut
>
> **Synapse Traits**: `Substrate` (knowledge retrieval with provenance awareness)
>
> **Prerequisites**: [07-hypnagogia-engine.md](07-hypnagogia-engine.md), [08-divergence-and-alpha.md](08-divergence-and-alpha.md)


> **Implementation**: Scaffold

---

## The Hauntological Frame

Jacques Derrida introduced "hauntology" in Specters of Marx (1993) as a way of thinking about how the past inhabits the present — not as fixed history but as spectral traces that continue to shape what is possible. Mark Fisher (2014, Ghosts of My Life) extended this to cultural production: the monoculture eliminates "lost futures" — possibilities that were once alive but have been foreclosed by homogenization.

Applied to AI agents: when all agents use the same foundation model, they are haunted by the same spectral traces — the same training data, the same patterns, the same biases. This produces what Fisher would call a "spectral monoculture" — a situation where all agents dream the same dreams, reach the same conclusions, and foreclose the same futures.

The Roko dream system's response to spectral monoculture is architectural: each agent's unique experiential history creates unique spectral traces that haunt its dreams differently from every other agent. The traces are not the training data (which is shared) but the **lived experiences** — episodes, emotional responses, knowledge entries — that accumulate through the agent's operational life.

---

## Spectral Traces in Dream Processing

During dreams, spectral traces manifest in three ways:

### 1. Experiential Ghosts

Every knowledge entry in NeuroStore carries provenance metadata: when it was created, from which episodes it was derived, and (for entries inherited from predecessors via backup/restore) which agent originally produced it. During dream replay, these provenance tags create "ghosts" — the echoes of past decisions, past agents, and past contexts that influenced the current knowledge state.

When the agent replays an episode during NREM, it is replaying its own ghost — a compressed, decaying trace of a past self that no longer exists (the agent's knowledge and emotional state have changed since the episode occurred). When it replays an inherited entry from a predecessor agent, it is replaying someone else's ghost — knowledge that was compressed through the backup/restore pipeline and arrived with reduced confidence (0.85× per generation, following the Weismann barrier principle).

### 2. Emotional Specters

The Daimon's PAD vectors attach emotional weight to knowledge entries. During dreaming, high-emotion entries surface more readily (somatic marker prioritization). The emotional trace of a past experience — the anxiety of a failed deployment, the satisfaction of a clean test suite — persists as a spectral marker that influences how the entry is processed during dreams.

Emotional depotentiation during REM (Walker & van der Helm 2009) gradually reduces these spectral markers, but never eliminates them entirely. The agent's emotional history becomes a persistent but fading ghost that shapes its dream content.

### 3. Creative Specters (Hypnagogia)

The hypnagogia engine's anti-correlated retrieval (see [07-hypnagogia-engine.md](07-hypnagogia-engine.md)) surfaces knowledge entries that are maximally dissimilar to the agent's current focus. These "opposite" entries are spectral in Derrida's sense: they are the foreclosed possibilities, the paths not taken, the knowledge that is present in the agent's memory but never activated during normal waking retrieval.

When these foreclosed entries collide during hypnagogic onset, they can produce insights that neither the current focus nor the forgotten entry could have produced alone — a new possibility emerging from the collision of the present with its own spectral residue.

---

## The Compound Escape from Monoculture

The compound escape from the spectral monoculture requires all cognitive cross-cuts working together during dreams:

1. **Neuro (NeuroStore)** gives traces persistence. Knowledge entries with provenance tags carry the ghosts of past agents and past decisions. Without persistent memory, there would be nothing to haunt.

2. **Daimon (Affect Engine)** gives traces weight. Not all ghosts haunt equally. Emotionally charged experiences haunt more powerfully than routine ones. The Daimon's somatic markers create a hierarchy of spectral significance.

3. **Dreams (Consolidation)** gives traces structure. The dream cycle's NREM/REM/Integration pipeline processes spectral traces into organized knowledge — strengthening useful ghosts and letting irrelevant ones decay.

4. **Hypnagogia (Creative Onset)** gives traces novelty. The hypnagogia engine's anti-correlated retrieval and executive loosening produce new spectral traces from the collision of existing ones — ghosts that did not exist in the training data, unique to this particular agent's experiential history.

Each mechanism alone is insufficient. Persistence without emotion produces undifferentiated archives. Emotion without consolidation produces noise. Consolidation without creativity produces convergent strategies. Creativity without persistence produces ephemera.

Together, they produce an agent that is **differently haunted** from every other agent — not through different models but through different experiential ghosts processed through different emotional lenses in different creative states.

---

## Knowledge Transfer as Inheritance (Not Death)

The legacy Bardo architecture framed cross-agent knowledge transfer through the lens of death and succession: when an agent "died," its knowledge was compressed into a "death testament" and "inherited" by a successor through the "bloodstain" network.

In the Roko architecture, knowledge transfer is **user-controlled backup/restore**:

| Legacy Concept | Roko Equivalent |
|----------------|-----------------|
| Death testament | Knowledge export (backup) |
| Bloodstain inheritance | Knowledge import (restore) with confidence decay (0.85× per generation) |
| Library of Babel (cross-collective repository) | Mesh knowledge sharing (see `../06-mesh/INDEX.md`) |
| Generational compounding | Accumulative backups with provenance tracking |

The mechanisms are the same — knowledge flows from one agent to another with confidence decay to prevent blind trust in inherited information. What changes is the framing: this is not death and succession but backup, restore, and sharing. An agent can export its knowledge at any time, not only at termination.

During dreams, inherited (restored) knowledge entries carry a `provenance: "imported"` tag. The dream engine treats imported entries with slightly lower trust:
- Imported entries receive a 0.85× confidence multiplier (the Weismann barrier)
- Imported entries are eligible for dream replay but receive lower replay priority
- Imported entries that conflict with self-generated entries are flagged for explicit resolution during REM imagination

---

## Academic Citations

| Paper | Relevance |
|-------|-----------|
| Derrida (1993), Specters of Marx | Hauntology: the spectral traces of the past inhabiting the present |
| Fisher (2014), Ghosts of My Life | Lost futures: monoculture forecloses possibility spaces |
| Grossman & Stiglitz (1980), American Economic Review | Information convergence → zero marginal value |
| Walker & van der Helm (2009), Psychological Bulletin | Emotional depotentiation: spectral markers fade but never vanish |
| McClelland et al. (1995), CLS theory | Fast/slow memory systems bridged by spectral replay |

---

## Computational Hauntology: Measuring Spectral Influence

The hauntological frame is evocative but needs computational grounding. How do we measure whether "spectral traces" are actually influencing dream processing?

```rust
/// Spectral influence metrics for hauntological analysis.
pub struct SpectralInfluenceMetrics {
    /// Provenance depth: how many generations back the deepest inherited entry goes.
    pub max_provenance_depth: usize,
    /// Spectral density: fraction of active knowledge entries with inherited provenance.
    pub spectral_density: f64,
    /// Emotional residue: mean arousal of inherited entries vs self-generated.
    pub inherited_arousal_delta: f64,
    /// Foreclosure index: fraction of knowledge space unreachable from current focus
    /// (surfaced only through anti-correlated retrieval).
    pub foreclosure_index: f64,
    /// Ghost influence: fraction of dream insights that reference inherited entries.
    pub ghost_influence_fraction: f64,
}

/// Spectral trace provenance for knowledge entries.
pub struct SpectralProvenance {
    pub original_agent_id: String,
    pub generation_depth: usize,
    pub confidence_at_origin: f64,
    pub confidence_after_transit: f64,
    pub emotional_charge_at_origin: f64,
    pub transit_path: Vec<String>,
    pub created_at_origin: chrono::DateTime<chrono::Utc>,
}
```

### Test Criteria

```
1. Spectral density: for an agent with zero inherited entries, spectral_density = 0.0.
2. Provenance depth: for self-generated entries, generation_depth = 0.
3. Confidence decay: confidence_after_transit = confidence_at_origin * 0.85^generation_depth.
4. Ghost influence: ghost_influence_fraction ∈ [0.0, 1.0].
5. Foreclosure index: for an agent that has retrieved all entries at least once, foreclosure_index ≈ 0.0.
```

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [07-hypnagogia-engine.md](07-hypnagogia-engine.md) | Anti-correlated retrieval surfaces foreclosed possibilities |
| [08-divergence-and-alpha.md](08-divergence-and-alpha.md) | Divergence as the architectural response to monoculture |
| [02-nrem-replay.md](02-nrem-replay.md) | Replaying spectral traces during NREM |
| [03-rem-imagination.md](03-rem-imagination.md) | Processing spectral traces during REM |
