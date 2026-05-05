# 01-signal -- Depth Index

Depth for [01-SIGNAL.md](../../unified/01-SIGNAL.md)

---

## Source docs (8)

### Signal struct internals

| Source doc | Status |
|---|---|
| `docs/00-architecture/02-engram-data-type.md` | Redesigned |
| `docs/00-architecture/02b-pulse-ephemeral-event.md` | Redesigned |

### Scoring and appraisal

| Source doc | Status |
|---|---|
| `docs/00-architecture/03-score-7-axis-appraisal.md` | Redesigned |
| `docs/00-architecture/25-attention-as-currency.md` | Redesigned |

### Decay algorithms

| Source doc | Status |
|---|---|
| `docs/00-architecture/04-decay-variants.md` | Redesigned |
| `docs/00-architecture/18-decay-tier-matrix.md` | Redesigned |

### Provenance and kinds

| Source doc | Status |
|---|---|
| `docs/00-architecture/05-provenance-and-attestation.md` | Redesigned |
| `docs/00-architecture/19-compositional-kinds.md` | Redesigned |

---

## Depth docs (4)

| Doc | What it covers | Source docs redesigned |
|---|---|---|
| [signal-algebra.md](signal-algebra.md) | Signal + Pulse semiring (bind/bundle/permute), Compound kinds as lattice join, graduation/projection functors, lineage DAG as free category, scaling analysis | 02-engram, 02b-pulse, 19-compositional-kinds |
| [demurrage-economics.md](demurrage-economics.md) | Gesell-Shannon rate law derivation, phase space (balance x tier x novelty), fixed points, Markov chain tier progression, VCG attention auction as live-economy dual, per-kind rate tables, cold storage/thaw | 04-decay-variants, 18-decay-tier-matrix, 25-attention-as-currency |
| [scoring-and-calibration.md](scoring-and-calibration.md) | Score-Verify-Score feedback loop, temperature scaling as Score protocol Cell, Beta-Binomial confidence updating, precision-weighted aggregation, Pareto front, meta-calibration, isotonic regression fallback | 03-score-7-axis-appraisal |
| [provenance-and-taint.md](provenance-and-taint.md) | Taint as lattice-based IFC, join propagation, declassification, Custody as dependent-type witness, custody-gated Store, attestation layers, cross-Space trust boundaries, action-time taint gates | 05-provenance-and-attestation |
