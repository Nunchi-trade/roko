# Consolidated Roadmap

> **Abstract:** This chapter is the sequencing layer for the architecture refinements. The
> primitive chapters explain what Roko is and how the parts fit together; this roadmap explains
> what lands next, what must land first, and which Q1-Q4 milestones produce visible wins without
> stacking too many high-risk changes in the same window. The primary source is
> [tmp/refinements/35-consolidated-roadmap.md](../../tmp/refinements/35-consolidated-roadmap.md);
> terminology follows [Naming and Glossary](./01-naming-and-glossary.md).

> **Implementation**: Planned

> **Team calibration note:** This roadmap was originally drafted as a 5-7 engineer program. The actual project shape is 1 developer + AI agents, so the quarter labels below should be read as full-team estimates, simultaneous workstreams should be reduced, and elapsed timelines will stretch accordingly.

**Part of**: [Roko PRD](../INDEX.md)
**Status**: Written
**Prerequisites**: [Refactor Plan Phases](./33-refactor-plan-phases.md), [Synergy & Integration Map](./34-synergy-integration-map.md), [Implementation Readiness Audit](./31-implementation-readiness-audit.md)

---

## Abstract

The refinement set from REF02 through REF34 describes a coherent architecture, but it also
creates a sequencing problem. Some primitives only compound if they land in dependency order:
`Pulse` before `Bus` migration, the `Bus` before subsystem rewiring, HDC fingerprint before
demurrage tuning, heuristics before c-factor actuation, StateHub projection before multi-surface
UX parity. This chapter makes that order explicit.

The roadmap is therefore not a second design document. It is the canonical dependency and
delivery view over the existing design. It keeps the architecture honest by stating which work is
critical path, which tracks can run in parallel, which risks deserve isolated budget, and what a
user should be able to see at the end of each quarter on a full-team schedule.

See also [tmp/refinements/35-consolidated-roadmap.md](../../tmp/refinements/35-consolidated-roadmap.md)
for the full proposal, [Naming and Glossary](./01-naming-and-glossary.md) for canonical
vocabulary, [Refactor Plan Phases](./33-refactor-plan-phases.md) for the kernel cutover plan, and
[Synergy & Integration Map](./34-synergy-integration-map.md) for why the dependency order matters.

---

## 1. Sequencing Principles

The roadmap follows five rules.

1. **Dependency order first.** Land the primitive that later work assumes before landing the
   dependent work.
2. **One major risk per phase.** Kernel cutovers, demurrage tuning, and multi-tenant safety
   splits should not peak in the same quarter.
3. **Ship visible wins.** Every quarter needs a demoable capability, not only internal cleanup.
4. **Parallelize independent tracks.** Learning, UX, and platform work can advance together once
   the kernel critical path is stable.
5. **Keep adjacent plans non-blocking.** This roadmap aligns with `tmp/ux-followup/` and
   `tmp/MASTER-PLAN.md`, but it does not require those tracks to stop while the refinements land.

Those principles keep the architecture from turning into either a pure research backlog or a set
of disconnected implementation spikes.

---

## 2. Critical Path

The architecture has one clear critical path and several dependent branches.

1. **Kernel framing and migration**: REF02 `Pulse`, REF03 `Bus`, REF04 `Datum` and generalized
   operators, REF05 seven-step loop, REF06 phased refactor plan, REF07 naming, and REF08 code
   sketches.
2. **Crate boundary cleanup**: REF20 modularity and composability turns the kernel story into
   enforceable package seams.
3. **Learning substrate**: REF10 self-learning loops, REF11 HDC fingerprint, REF12 demurrage,
   REF13 c-factor, REF14 heuristics, and REF16 research-to-runtime build on the stabilized
   two-medium, two-fabric runtime.
4. **Surface and ecosystem work**: REF17 plugin SPI, REF22 developer UX, REF23 user UX, REF24
   deployment UX, REF25 domain profiles, REF26 StateHub projection, REF27 realtime wire surface,
   REF28 CLI parity, REF29 web UI, and REF30 rich UX primitives depend on the earlier kernel and
   learning layers.
5. **Integrators and Phase 2**: REF31 synergy framing, REF32 safety spine, REF33 observability,
   and REF09 Phase-2 Bus/Substrate backends harden or extend what the earlier quarters establish.

The practical rule is simple: if a workstream assumes a shared `Bus`, typed `Pulse` topics, or
calibration-bearing heuristics, it is downstream of the kernel and learning substrate. This is the
reason sequencing matters more than raw backlog count.

### 2.1 Dependency ladder

The highest-value edges from the dependency graph can be read as one compact ladder:

| Upstream | Must land before | Why |
|---|---|---|
| `Pulse` + `Bus` + `Datum` | subsystem migration and seven-step loop cleanup | Shared transport and operator vocabulary must exist before callers can converge on it |
| Kernel migration | HDC fingerprint, demurrage, heuristics, self-learning loops | The learning substrate assumes the two-medium, two-fabric runtime is stable enough to observe |
| HDC fingerprint + demurrage + heuristics | c-factor actuation and replication-ledger expansion | Calibration and cohort policy only compound once memory, evidence, and confidence surfaces are real |
| Plugin SPI + StateHub projection + shared wire protocol | multi-surface UX parity and domain-profile rollout | External surfaces and extensions need one shared runtime contract |
| Safety spine + deployment hardening | Phase-2 backends and cross-deployment witness flows | Distributed trust should extend a hardened runtime, not precede it |

---

## 3. Quarter-by-Quarter Roadmap

The quarter framing below is a full-team estimate. For 1 developer + AI agents, use the same
dependency order but expect fewer concurrent tracks and a longer wall-clock timeline.

### 3.1 Q1 - Foundation (full-team estimate)

**Headline:** the two-medium kernel becomes the canonical runtime story and existing subsystems
start migrating away from ad hoc transport surfaces.

| Track | Scope | Primary docs |
|---|---|---|
| Kernel | Land Phases A-C of the refactor sequence: `Pulse`, `Bus`, `Datum`, operator generalization, seven-step loop, and first subsystem migration | [02](./02-engram-data-type.md), [07b](./07b-bus-transport-fabric.md), [33](./33-refactor-plan-phases.md) |
| Naming | Finish the canonical rename pass and keep the glossary authoritative | [01](./01-naming-and-glossary.md) |
| Modularity | Extract kernel seams such as `roko-bus` and scaffold the SPI boundary | [15](./15-crate-map.md), [23](./23-architectural-analysis-improvements.md) |
| Observability baseline | Ship the first Roko-specific dashboards and transport-level telemetry needed to watch the migration | [31](./31-implementation-readiness-audit.md), [32](./32-comprehensive-test-strategy.md), `tmp/refinements/33-observability-telemetry.md` |

**Quarter risk:** kernel refactor.

**Quarter demo:** a self-hosting plan flow runs on the new kernel vocabulary and no longer
depends on scattered local publication types.

### 3.2 Q2 - Learning Substrate (full-team estimate)

**Headline:** durable memory becomes semantically indexed and economically shaped while the
runtime starts learning from prediction and falsification loops.

| Track | Scope | Primary docs |
|---|---|---|
| HDC fingerprint | Add a first-class HDC fingerprint to every durable Engram and expose similarity queries | [02](./02-engram-data-type.md), [27](./27-temporal-knowledge-topology.md) |
| Demurrage | Replace age-only pruning with balance, reinforcement, and cold-tier durable memory management | [04](./04-decay-variants.md), [18](./18-decay-tier-matrix.md), [25](./25-attention-as-currency.md) |
| Heuristics | Promote heuristics, falsifiers, and calibration into inspectable library objects | [../05-learning/19-heuristics-worldviews-and-falsifiers.md](../05-learning/19-heuristics-worldviews-and-falsifiers.md) |
| Self-learning and c-factor | Wire prediction/outcome topics, calibration policies, and visible c-factor measurement | [11](./11-dual-process-and-active-inference.md), [14](./14-c-factor-collective-intelligence.md) |
| Research-to-runtime | Land paper, claim, and replication-ledger starter flows | [../05-learning/20-research-to-runtime.md](../05-learning/20-research-to-runtime.md), [../21-references/25-research-to-runtime.md](../21-references/25-research-to-runtime.md) |

**Quarter risk:** demurrage rate tuning.

**Quarter demo:** calibrated heuristics are visible in the product, HDC-backed retrieval is live,
and c-factor can be inspected during a multi-agent run.

### 3.3 Q3 - Ecosystem and UX (full-team estimate)

**Headline:** the runtime becomes externally legible and extensible: plugin SPI, StateHub
projection, stable realtime transport, and first-party UX surfaces all converge.

| Track | Scope | Primary docs |
|---|---|---|
| Plugin SPI | Land the staged extension model from prompt/profile layers through native and WASM boundaries | [../18-tools/14-plugin-sdk.md](../18-tools/14-plugin-sdk.md), [15](./15-crate-map.md) |
| StateHub projection | Promote StateHub projection into a kernel-tier shared data surface | `tmp/refinements/26-statehub-rearchitecture.md`, [../12-interfaces/22-statehub-projection-layer.md](../12-interfaces/22-statehub-projection-layer.md) |
| Realtime wire protocol | Freeze the shared wire protocol and support multiple client surfaces against it | [../12-interfaces/06-websocket-streaming.md](../12-interfaces/06-websocket-streaming.md), [../12-interfaces/INDEX.md](../12-interfaces/INDEX.md) |
| Developer and user UX | Ship the four-layer Rust SDK, interactive `roko init`, unified verbs, CLI parity, and the first web UI release | [../12-interfaces/19-rust-sdk-developer-ux.md](../12-interfaces/19-rust-sdk-developer-ux.md), [../12-interfaces/21-user-ux-running-agents.md](../12-interfaces/21-user-ux-running-agents.md), [../12-interfaces/23-rich-ux-primitives.md](../12-interfaces/23-rich-ux-primitives.md) |
| Deployment shape | Make single-machine and single-server deployment portable and reproducible | [../19-deployment/INDEX.md](../19-deployment/INDEX.md), `tmp/refinements/24-deployment-ux.md` |

**Quarter risk:** UX scope creep.

**Quarter demo:** a third party installs a plugin and sees the same runtime surface reflected in
CLI, TUI, and web clients via one shared projection and transport contract.

### 3.4 Q4 - Scale, Safety, and Domains (full-team estimate)

**Headline:** Roko becomes domain-shaped, auditable, and multi-tenant enough for serious team use.

| Track | Scope | Primary docs |
|---|---|---|
| Domain profiles | Ship the first set of domain profiles with `TypedContext`, starter heuristics, and gates | `tmp/refinements/25-domain-specific-agents.md`, [../02-agents/INDEX.md](../02-agents/INDEX.md) |
| Safety spine | Make custody, sandbox tiers, provenance, taint, and audit tooling coherent across surfaces | [05](./05-provenance-and-attestation.md), [26](./26-cognitive-immune-system.md), [../11-safety/INDEX.md](../11-safety/INDEX.md) |
| Replication ledger expansion | Extend claim tracking and evidence export from starter kit into broader runtime use | [../05-learning/20-research-to-runtime.md](../05-learning/20-research-to-runtime.md), [../21-references/25-research-to-runtime.md](../21-references/25-research-to-runtime.md) |
| Deployment hardening | Add multi-tenant deployment shape, identity integration, and Helm-grade packaging | [../19-deployment/12-production-hardening.md](../19-deployment/12-production-hardening.md), `tmp/refinements/24-deployment-ux.md` |
| c-factor actuation | Let policy react to degraded collective process instead of only measuring it | [14](./14-c-factor-collective-intelligence.md) |

**Quarter risk:** multi-tenant auth and isolation.

**Quarter demo:** a team selects a domain profile, runs an auditable plan with custody records,
and watches c-factor, cost, and safety surfaces update in real time.

### 3.5 Q5-Q6 - Phase 2 Optionality (full-team estimate)

Q5 and Q6 are not required for the architecture to stand. They are where the Phase-2 backends and
long-horizon compounding layers become worth landing.

- Add ChainBus, MultiBus, and other non-local transport backends from REF09.
- Bring Dreams online as the Delta-speed consolidation loop.
- Extend witness and replication flows across deployments.
- Revisit the Composer rewrite once HDC-driven retrieval and projection layers are stable.
- Consider the published plugin registry only after the SPI and audit model have settled.

This is deliberate sequencing, not deferral by accident. Q1-Q4 should produce a coherent product
even if Q5-Q6 slips.

---

## 4. Parallel Tracks and Team Shape

The roadmap's quarter framing assumes a small team can run independent tracks once the Q1 kernel
work stops being a blocker. In the actual 1 developer + AI agents setup, many of these tracks
serialize rather than running in parallel.

| Role | Primary ownership |
|---|---|
| Kernel engineer | Q1 runtime changes, then stewardship of the core contracts through Q4 |
| Learning engineer | Q2 learning substrate, heuristics, calibration, replication-ledger starter flows |
| UX engineer | Q3 surface work across CLI, TUI, web, and shared primitives |
| Platform engineer | Plugin SPI, deployment shape, observability, and safety infrastructure |
| Domain lead | Q4 domain profiles and domain-specific gates, heuristics, and onboarding |

The team can be smaller, but the sequencing penalty is clear: with fewer than five focused owners,
reduce the number of domain profiles and push some Q4 work by one quarter rather than overloading
Q2 and Q3. With 1 developer + AI agents, that reduction is the default operating assumption.

A comfortable Q1-Q4 plan is therefore roughly a 5-7 engineer program over 6-12 months, with the
extra range driven mostly by how many Q3 and Q4 tracks can truly run in parallel. For the actual
team shape, treat that as a calibration reference rather than a promised calendar.

---

## 5. Risk Register and Checkpoints

The roadmap has a few dominant risks, and each one needs a named checkpoint rather than vague
confidence.

| Checkpoint | Question | Why it matters |
|---|---|---|
| End of Q1 month 1 | Is the kernel cutover still safer than incremental patching? | Prevents the migration from turning into indefinite compatibility debt |
| End of Q2 | Is demurrage producing useful compounding instead of cold-tier churn? | Protects the learning substrate from false sophistication |
| End of Q3 | Are external plugins actually installing and surviving onboarding? | Tests whether the SPI is real ecosystem leverage or only internal architecture |
| End of Q4 | Are any domain profiles producing surprising replication-ledger findings? | Distinguishes a live domain platform from a themed demo |

Supporting risks remain active across multiple quarters:

- HDC encoder drift across deployments requires versioned fingerprints.
- Plugin ABI churn requires release discipline and semver boundaries.
- c-factor should remain a diagnostic and regulator, not a direct objective to reward-hack.
- Cross-doc vocabulary drift should be caught by keeping the glossary and sequencing docs aligned.

For the full roadmap rationale and longer risk table, see
[tmp/refinements/35-consolidated-roadmap.md](../../tmp/refinements/35-consolidated-roadmap.md).

---

## 6. How This Roadmap Maps to Existing Planning Material

This chapter does not replace the existing planning artifacts; it normalizes them around the
refinement set.

### 6.1 `tmp/ux-followup/`

The UX follow-up catalog remains useful for concrete gap lists. The roadmap contributes the
sequence:

- Q1 closes the spec-code drift and transport-wiring closures that depend on the shared `Bus`.
- Q2 and Q3 absorb most observability, state portability, and surface consistency work.
- Remaining hygiene items stay parallel and should not block kernel, learning, or UX tracks.

### 6.2 `tmp/MASTER-PLAN.md`

The older tiered planning material still tracks breadth. This roadmap adds architectural homes and
dependency order so those items stop reading like a flat inventory. Where the legacy plan groups
work by broad tier, this chapter groups it by sequencing pressure and user-visible milestone.

If the two planning views disagree, prefer the refinement-backed dependency order here and update
the broader inventory later.

---

## 7. Not-Doing List

The roadmap is as much about exclusion as inclusion. These items are intentionally outside Q1-Q4.

- Training custom models on accumulated episodes.
- A graphical plan editor beyond the existing plan views.
- Full native SDK parity beyond the explicitly planned client surfaces.
- A first-party inference server.
- A Kubernetes operator beyond Helm-grade deployment packaging.
- A native mobile app.
- A standalone voice-first workflow.

Deferring them keeps the sequencing stable. Each could become real work later, but none should
preempt the critical path described above.

---

## 8. One-Year Outcome

If Q1 through Q4 land in order on a full-team schedule, the resulting one-year story is coherent:

1. The kernel speaks one transport and storage language: `Engram`, `Pulse`, `Substrate`, `Bus`,
   `Topic`, `TopicFilter`, `Datum`, and `PulseSource`.
2. The learning layer compounds through HDC fingerprint, demurrage, heuristics, and c-factor
   instead of treating them as isolated experiments.
3. Plugins, StateHub projection, and surface clients share one runtime contract.
4. Domain profiles and safety infrastructure make the system auditable enough for team workflows.

That is the point of the roadmap. It turns a rich but potentially scattered refinement set into a
delivery sequence that compounds rather than backtracks.

---

## 9. Cross-References

- See [tmp/refinements/35-consolidated-roadmap.md](../../tmp/refinements/35-consolidated-roadmap.md) for the canonical source proposal.
- See [Implementation Readiness Audit](./31-implementation-readiness-audit.md) for the current-state scorecard this roadmap sequences.
- See [Refactor Plan Phases](./33-refactor-plan-phases.md) for the lower-level Phase A-D migration mechanics.
- See [Synergy & Integration Map](./34-synergy-integration-map.md) for the primitive interaction map that explains why this order compounds.
- See [Naming and Glossary](./01-naming-and-glossary.md) for the canonical vocabulary used across Q1-Q4 sequencing.

## 10. Maintenance

This chapter is the architecture-level source of truth for sequencing.

- Review it at the end of each quarter and update planned-versus-actual status.
- Revise checkpoint outcomes when a go or no-go decision is made.
- Keep new refinement chapters consistent with the dependency order here unless a later refinement
  explicitly changes that order.
- Treat the glossary and this roadmap as coupled documents whenever new vocabulary changes the
  plan shape.
