# Research-to-Runtime Pipeline

> **Refinement source**: `../../tmp/refinements/16-research-to-runtime.md`
>
> This chapter translates research into a target-state runtime loop: `Paper -> Claim -> Heuristic -> Trial -> Calibration`.
> Papers would be stored as Engrams, claims would become testable hypotheses, heuristics would be the reusable runtime form, trials would be episodes, and calibration would record whether the claim actually holds in Roko's deployment.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture glossary](../00-architecture/01-naming-and-glossary.md), [Research-to-Runtime Pipeline](../05-learning/20-research-to-runtime.md), [Heuristics, Worldviews, and Falsifiers](../05-learning/19-heuristics-worldviews-and-falsifiers.md)
**See also**: `../../tmp/refinements/16-research-to-runtime.md`

> **Implementation**: Reference

---

## Overview

Roko already consumes research implicitly: papers inform architecture, bandits, memory, active inference, collective intelligence, and HDC. This chapter makes that flow explicit and auditable. The core change is simple: the paper itself would become a first-class Engram, a claim would become a structured hypothesis with a falsifier, a heuristic would become the runtime projection of that claim, and repeated trials would produce calibration data that can confirm, weaken, or retire the heuristic.

The result is a target-state living research loop rather than a one-time literature pass. The system does not merely cite research; it would test research against its own episodes, record the outcome, and keep a replication ledger for later review.

## Pipeline

### 1. Paper

A paper is a target-state Engram that captures bibliographic metadata, provenance, and the system's own notes about why the source matters. Papers would be content-addressed like other durable knowledge and can be linked through lineage when a claim or heuristic depends on them.

### 2. Claim

A claim is the smallest testable restatement of a paper result. It should include:

- a source paper
- a structured hypothesis
- an explicit falsifier
- the context in which the claim is supposed to apply
- the observed calibration so far

A claim is not a citation note. It is a target-state runtime hypothesis that can be checked against episodes and metrics.

### 3. Heuristic

When a claim is ready for use in the agent loop, it lifts into a Heuristic. This is the target-state operational form that can be injected into prompts, policies, routing, and calibration logic. The heuristic retains lineage back to the paper and carries its own calibration history.

### 4. Trial

A trial is a bounded episode or episode slice in which the claim's prediction is tested against observed outcomes. Trials should be scoped tightly enough that the falsifier can be evaluated from runtime signals, not from outside laboratory conditions.

### 5. Calibration

Calibration records how the claim performed in Roko's actual deployment. The important signal is not whether the paper sounded plausible, but whether the claim replicates, partially replicates, fails, or depends on context.

## Data Shapes

```rust
pub struct Paper {
    pub id: Uuid,
    pub doi: Option<String>,
    pub title: String,
    pub authors: Vec<String>,
    pub year: u16,
    pub abstract_: String,
    pub fingerprint: HdcVector,
    pub claims: Vec<ClaimId>,
    pub provenance: PaperProvenance,
}

pub struct Claim {
    pub id: Uuid,
    pub paper: PaperId,
    pub quote: String,
    pub hypothesis: Hypothesis,
    pub falsifier: Predicate,
    pub context: Vec<Predicate>,
    pub calibration: Calibration,
}

pub struct ReplicationLedger {
    pub claim: ClaimId,
    pub paper_effect: f64,
    pub our_effect: f64,
    pub our_n: u32,
    pub divergence_ci: (f64, f64),
    pub status: ReplicationStatus,
}
```

The exact field names can vary by implementation, but the information content should not: source, claim, falsifier, observed effect, and replication status must all be recoverable.

## Replication Ledger

The replication ledger is the target-state persistent record of whether a claim holds up in Roko's environment. It is most useful when it distinguishes several outcomes rather than collapsing everything into success or failure.

- `Untested` means the claim has not yet been exercised.
- `Insufficient` means there are too few trials to say anything useful.
- `Replicates` means the observed effect stays within the expected confidence band.
- `PartialReplicates` means the effect points in the right direction but is weaker.
- `FailsToReplicate` means the claim does not hold in Roko's deployment.
- `ContextDependent` means the claim holds only under some conditions.

The ledger should also preserve the paper's reported effect, the system's observed effect, the sample size, and the divergence between the two. That makes research-driven engineering reviewable instead of folkloric.

## Starter Kit

The launch set should include a small number of foundational papers and claims that anchor the major architectural primitives already in use:

- Kanerva on HDC capacity and near-orthogonality
- Friston on the Free Energy Principle and active inference
- Woolley on collective intelligence and c-factor
- Sutton on temporal-difference learning
- Auer on bandits and exploration
- Hanson on prediction markets
- Axelrod on cooperation and reciprocal strategies
- Janis on groupthink symptoms
- Clark on predictive processing
- Gesell on demurrage and holding cost
- Ostrom on commons governance
- Weick on sensemaking

Each source should arrive with one to three claims and an explicit falsifier. The point is not to maximize bibliography size. The point is to start with a curated library whose claims can actually be checked by the runtime.

## Ingestion Lanes

### Manual

A human or agent reads a paper, creates the Paper Engram, and drafts the Claims. This lane is the highest quality and should be used for foundational or high-stakes sources.

### Agent-Curated

A research role can crawl feeds, produce Paper and Claim drafts, and publish them to a `research.candidate` Bus topic for review. Approved items can then be promoted to `research.approved`; rejected items should remain traceable so the review history is auditable.

### Watchdog

A Watchdog subscribes to a claim's falsifier across episodes. When the falsifier matches an observed outcome, the watchdog publishes a `claim.violated` Pulse and triggers recalibration. This lane is the always-on backstop that keeps the ledger honest after the initial ingestion pass.

## Replication Contract

When a claim is exported for external review or sharing with another deployment, use a stable markdown contract so both humans and tools can compare results.

```markdown
---
claim_id: c.kanerva2009.orthogonality
paper_doi: 10.1007/s12559-009-9009-8
paper_effect: "Two random 10,000-bit vectors have cosine similarity ~0 ± 0.01"
our_effect: 0.0097 ± 0.004
our_n: 1000000
roko_version: "2.3.1"
context:
  vector_dim: 10240
  encoder: default_v1
  deployment_profile: coding
status: replicates
first_observed: 2026-03-01
last_observed: 2026-04-14
---

## Notes

We observe expected orthogonality within the paper's confidence band across a large random sample of vector pairs from production storage.
```

The contract should be easy to import into another system, diff against local outcomes, and archive alongside the replication ledger.

## Runtime Use

Research-derived heuristics should carry provenance when they are injected into prompts or routing decisions. That provenance is the bridge between academic citation and system behavior: the agent can show where a heuristic came from, how often it has been confirmed, and whether it has drifted.

When a parameter is sourced from a claim, prefer claim-resolved configuration or a `claim!`-style resolver over hard-coded constants so the runtime can explain why it chose a value and what evidence still supports it.

The practical rule is simple: if a claim cannot be falsified by runtime signals, it is not ready to become a heuristic. If a heuristic cannot be calibrated, it is not ready to remain in the hot path.

## Cross-References

- [01-naming-and-glossary](../00-architecture/01-naming-and-glossary.md)
- [20-research-to-runtime](../05-learning/20-research-to-runtime.md)
- [19-heuristics-worldviews-and-falsifiers](../05-learning/19-heuristics-worldviews-and-falsifiers.md)
- `../../tmp/refinements/16-research-to-runtime.md`
