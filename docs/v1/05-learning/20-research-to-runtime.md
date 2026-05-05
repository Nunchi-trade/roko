# Research-to-Runtime Pipeline

> **REF16 source:** `../../tmp/refinements/16-research-to-runtime.md`
> **Glossary:** [Naming and Glossary](../00-architecture/01-naming-and-glossary.md)
> **Cross-references:** [18-self-learning-cybernetic-loops](18-self-learning-cybernetic-loops.md), [19-heuristics-worldviews-and-falsifiers](19-heuristics-worldviews-and-falsifiers.md), [16-predictive-foraging](16-predictive-foraging.md), [25-research-to-runtime](../21-references/25-research-to-runtime.md), [12-4-tier-distillation-pipeline](../06-neuro/12-4-tier-distillation-pipeline.md)
>
> **Implementation status**: Target-state concept. No `Claim`, `Paper`, or replication-ledger code exists. The provenance-backed heuristic idea is valuable; the full paper economy (claims, replication trials, ledger) is deferred.

---

## Purpose

REF16 describes a target-state way to make research part of the learning loop rather than a one-time influence. In that design, the system would ingest papers continuously, extract testable claims, lift validated claims into heuristics, and keep the resulting evidence live through replication-ledger feedback. Today, the most practical slice is narrower: keep provenance visible when papers inform heuristics or parameters.

This chapter is the learning-layer bridge between academic provenance and runtime behavior. It explains how paper-backed ideas become parameter choices, heuristics, and calibration records that can be checked, revised, or retired when the live system disagrees.

## The Pipeline

The proposed flow is:

1. **Paper** - ingest the source as an Engram with authorship, venue, and provenance.
2. **Claim** - extract or author a testable hypothesis with an explicit falsifier.
3. **Heuristic** - lift the claim into a reusable prior once local structure is stable enough.
4. **Trial** - run the heuristic against real episodes, gates, and outcome Pulses.
5. **Calibration** - update confidence, confidence bounds, and trust based on what actually happened.

The key point is that the same evidence can move through the stack more than once. A claim may begin as a paper-backed prior, then become a heuristic, then be revised by trial results, then be demoted or promoted by its replication ledger. That full lifecycle is deferred for now.

## Paper As Engram (Target-State)

Papers live in the same durable substrate as other long-lived records. That means the source itself stays addressable, citeable, and comparable over time.

```rust
pub struct Paper {
    pub title: String,
    pub authors: Vec<String>,
    pub venue: Option<String>,
    pub year: u16,
    pub provenance: PaperProvenance,
    pub claims: Vec<ClaimId>,
}
```

The important behavior is not the exact schema. It is that the source paper remains available for later review, so the system can distinguish "this was a paper-backed idea" from "this worked in our stack."

## Claim As Hypothesis (Target-State)

A claim is the paper's runtime-facing unit of meaning. It should be small enough to test and explicit enough to fail.

```rust
pub struct Claim {
    pub paper: PaperId,
    pub hypothesis: Hypothesis,
    pub falsifier: Predicate,
    pub context: Vec<Predicate>,
    pub calibration: Calibration,
}
```

The falsifier is the load-bearing part. If the claim cannot be disproved by runtime signals, it is not yet a learning object. In practice, the falsifier should point at observable outcomes already present in the Bus or Episode streams.

## Heuristic Lifting

Claims that survive repeated trials can lift into `Heuristic` Engrams. That lifting preserves lineage rather than flattening the research source into a generic rule.

The practical rule is:

- paper → provenance
- claim → testable hypothesis with a falsifier
- heuristic → reusable belief with calibration and receipts

That is the same REF14 machinery, but with a research-specific origin story.

## Replication Ledger

In the target-state design, the replication ledger is the bridge between external research and local calibration. It records how the paper's reported effect compares with the effect observed in Roko's actual deployment.

```rust
pub struct ReplicationLedger {
    pub claim: ClaimId,
    pub paper_effect: f64,
    pub our_effect: f64,
    pub our_n: u32,
    pub divergence_ci: (f64, f64),
    pub status: ReplicationStatus,
}
```

The ledger is not just a report. It is an input to calibration. If a paper-derived claim keeps replicating, the associated heuristic stays warm. If it diverges, the claim should lose weight even if the original citation is strong.

That makes evidence cumulative instead of ceremonial. The runtime cares about the paper, but it trusts the paper only through the behavior it continues to see.

## Claim-Resolved Parameters

Some runtime defaults should be resolved from claims rather than literals. The docs may express this as a `claim!` macro, a resolver function, or equivalent lookup, but the behavior should be the same: the parameter is bound to a claim ID, not just a comment.

```rust
let epsilon = claim!("auer2002", "epsilon_greedy", default = 0.1)?;
```

If the claim's replication ledger weakens or local calibration drifts too far, the target-state resolver should fall back to a safe default or a lower-trust value. That keeps paper-backed parameters auditable without making them sticky.

## Calibration Feedback

In the target-state design, calibration should consume both local trials and replication-ledger updates.

- Local trials say whether the heuristic works in this deployment.
- Replication-ledger entries say whether the paper's claim still matches the deployed reality.
- The combined signal updates confidence, trust, and promotion or retirement decisions.

This is the main operational consequence of REF16: the system does not merely cite research. It tests it, tracks divergence, and lets the result change runtime behavior.

## Relationship To Other Docs

- [19-heuristics-worldviews-and-falsifiers](19-heuristics-worldviews-and-falsifiers.md) defines the heuristic layer that paper claims lift into.
- [18-self-learning-cybernetic-loops](18-self-learning-cybernetic-loops.md) defines the Bus-backed calibration loop that carries the trial outcomes.
- [16-predictive-foraging](16-predictive-foraging.md) covers task-level prediction and calibration, which this chapter reuses at research level.
- [25-research-to-runtime](../21-references/25-research-to-runtime.md) collects the source-facing paper, claim, starter-kit, and replication-contract framing.
- [12-4-tier-distillation-pipeline](../06-neuro/12-4-tier-distillation-pipeline.md) covers the durable memory tier that keeps research lineage available.
- See also `../../tmp/refinements/16-research-to-runtime.md` for the full proposal.
