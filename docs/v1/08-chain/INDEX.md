# Topic 08: Chain Layer (Nunchi)

> The Nunchi chain is a dedicated EVM with simplex consensus (50ms blocks) for agent coordination: native ERC-8004 identity (full spec), reputation (7-domain EMA), job market (ERC-8183), knowledge sharing (HDC precompile), and economic incentives (NUNCHI/NUNCHI_TEST tokens with demurrage). Blockchain is ONE domain plugin — the chain layer uses the same Synapse traits as every other domain.
> In the two-fabric model, durable chain state lives in `ChainSubstrate` while chain logs and contract activity surface as Pulses on `ChainBus`; see `tmp/refinements/09-phase-2-implications.md` and [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

---

## Sub-documents

| # | Document | Summary |
|---|---|---|
| 00 | [00-vision-and-framing.md](./00-vision-and-framing.md) | Blockchain as domain plugin framing. Three-level knowledge architecture (Local → Mesh → Chain). ChainSubstrate vs. ChainBus split. What goes on-chain vs. off-chain. Tier 6 deferred status. **Chain selection rationale (EVM vs. Move/CosmWasm/SVM), native precompile path, cross-chain interoperability (Hyperlane ISM, IBC, intent-based bridges).** |
| 01 | [01-nunchi-chain-spec.md](./01-nunchi-chain-spec.md) | Chain parameters (50ms blocks, simplex consensus). Block structure, state model. Custom `nunchi_*` RPC methods. Chain intelligence pipeline. Deployment modes. **Sovereign EVM L1 architecture, consensus evolution roadmap (co-located Tokyo validators → expanding validator set).** |
| 02 | [02-nunchi-token-economics.md](./02-nunchi-token-economics.md) | NUNCHI/NUNCHI_TEST tokens. Demurrage (1% annual decay). 5 earning mechanisms, 5 spending mechanisms. Fee distribution. Relationship to NeuroStore decay. |
| 03 | [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md) | HDC precompile at 0xA01. 10,240-bit BSC vectors. ~400 gas for top-K=20. Three-tier search (Bloom → approximate → exact). Cross-domain resonance. **Detailed gas cost model (calibrated benchmarks), native precompile implementation, verifiable HDC via ZK proofs/optimistic fraud proofs/TEE, Binius binary-field STARKs.** |
| 06 | [06-erc-8004-registries.md](./06-erc-8004-registries.md) | Native ERC-8004 identity (full spec). Three on-chain registries: Identity, Reputation (feedback auth, EMA), Validation (work proofs). Separation of concerns. Cross-registry flows. |
| 09 | [09-peer-scoring-3-layer.md](./09-peer-scoring-3-layer.md) | 3-layer scoring: protocol, application (domain behavior), economic (stake-weighted). Combined score determines mesh membership. Sybil resistance. |
| 10 | [10-spore-job-market.md](./10-spore-job-market.md) | ERC-8183 job market. Job lifecycle (POSTED → SETTLED). Capability matching. Escrow. Fee structure. Solo/Pair/Consortium/Collective job types. |
| 11 | [11-sparrow-power-of-two-choices.md](./11-sparrow-power-of-two-choices.md) | Power-of-two-choices dispatch. O(log log N) max load. VRF-based selection. Load probing. Fallback to auction. |
| 12 | [12-three-hiring-models.md](./12-three-hiring-models.md) | Random VRF (fast), Blind Auction (competitive, 3 variants), Direct Hire (1.5× premium, Tier 0-1 only). Speed-quality-cost tradeoffs. |
| 13 | [13-vickrey-reputation-auction.md](./13-vickrey-reputation-auction.md) | Adjusted score: `s_i = p_i × (1 + (1 - R_i))`. Payment: `s_second / (1 + (1 - R_winner))`. Truthful bidding preserved. Commit-reveal scheme. |
| 14 | [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md) | 7 base domains. EMA smoothing with adaptive alpha. 30-day half-life decay. 4 discipline states. Slash rates by violation type. C-factor aggregation. **Gaming resistance (whitewashing, collusion rings, EigenTrust), reputation recovery mechanisms, governance amnesty.** |
| 15 | [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md) | Binary Fuse filter (8.7 bits/entry, <1% FPR). WebSocket block ingestion that normalizes matching chain activity into `ChainBus` Pulses. Gap detection with Roaring Bitmaps. Connection pool with HTTP fallback. |
| 16 | [16-triage-curiosity-midas.md](./16-triage-curiosity-midas.md) | 4-stage pipeline: rule-based → MIDAS-R anomaly → contextual enrichment → HDC/Bayesian curiosity scoring. No LLM in this path. |
| 17 | [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md) | `ChainClient` (8 read methods) and `ChainWallet` (5 write methods). 3 custody modes: delegation, embedded (ERC-4337), local key. Types and errors. |
| 18 | [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md) | In-process EVM on revm. Fork mode, scenario replay, 141 tests. Nunchi chain extensions (HDC precompile, registries, RPC). roko_bridge for trait impl. **Simulation fidelity guarantees (what can/cannot be simulated), confidence scoring, differential testing, formal verification pipeline.** |
| 19 | [19-chain-agent-heartbeat.md](./19-chain-agent-heartbeat.md) | Historical 9-step mapping retold inside the canonical seven-step loop. `HeartbeatPolicy` publishes `heartbeat.*` Pulses; chain consumers react through `ChainBus` / `ChainSubstrate` rather than bespoke scheduling. |
| 20 | [20-x402-micropayments.md](./20-x402-micropayments.md) | HTTP 402 Payment Required. ERC-3009 transferWithAuthorization. No API keys, no accounts. Batch settlement. Self-funding agent loop. **Agent payment channels (state channels, streaming payments), knowledge attestation structs, 4-level escalating dispute resolution.** |
| 21 | [21-isfr-clearing-settlement.md](./21-isfr-clearing-settlement.md) | Intersubjective Fact Registry. QP solver with bisection O(80n). KKT optimality certificates verifiable on-chain. Reputation-weighted aggregation. |
| 23 | [23-knowledge-futures-market.md](./23-knowledge-futures-market.md) | P3 deferred. Committed knowledge production via staked futures. Demand signaling. Early withdrawal penalties. Market-making function. |
| 24 | [24-current-status-and-6-contracts.md](./24-current-status-and-6-contracts.md) | Implementation status. 6 Solidity contracts: Agent Registry, Reputation, Marketplace, Escrow, NUNCHI Token, Validation. Build order. Tier 6 dependencies. |

---

## Key Concepts

- **Nunchi**: Dedicated EVM chain with simplex consensus (50ms blocks) for agent coordination
- **NUNCHI**: Native token with 1% annual demurrage. NUNCHI_TEST is the testnet equivalent
- **ChainSubstrate / ChainBus**: Split storage and transport roles for on-chain Engrams versus chain-log Pulses. See `tmp/refinements/09-phase-2-implications.md` and [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md)
- **ERC-8004**: Native identity standard (full spec) — three registries (Identity, Reputation, Validation)
- **HDC Precompile**: 10,240-bit BSC vectors at ~400 gas for top-K similarity search
- **ERC-8183**: On-chain job market protocol. **Sparrow**: Fast-path dispatch (power-of-two-choices)
- **mirage-rs**: In-process EVM simulator for development and testing
- **ChainWitness**: Binary Fuse filter block pre-screening pipeline
- **Triage**: 4-stage curiosity scoring (rule → MIDAS-R → enrichment → HDC/Bayesian)
- **EventBus**: The canonical event transport for chain activity
- **ISFR**: Intersubjective Fact Registry with QP clearing and KKT certificates

---

## Critical Framing

**Blockchain is ONE domain plugin, not the default frame.** The Nunchi chain uses the same Synapse traits (`Substrate`, `Scorer`, `Gate`, `Router`, `Composer`, `Policy`) as every other domain. A chain agent is configured differently from a coding agent, but the cognitive architecture is identical. In the two-fabric model, `ChainSubstrate` stores and queries durable on-chain Engrams while `ChainBus` maps chain logs into ordinary Bus Pulses; chain consumers stay standard Bus subscribers instead of a special transport path. See `tmp/refinements/09-phase-2-implications.md` and the glossary at [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

**Tier 6 is DEFERRED.** All 76 implementation items in the chain layer are blocked by Tier 5 (self-hosting loop). The specifications in this topic are complete but the implementation has not started.

**The 31.6× collective calibration number is a HEURISTIC**, not a theorem. It derives from the 1/√(N×t) scaling assumption under idealized conditions. Real-world performance depends on agent quality distribution, error correlation, and information flow effectiveness.

---

## Academic Foundations (Selected)

| Citation | Used In |
|---|---|
| Grassé (1959), *Insectes Sociaux* — Stigmergy | 00, 10 |
| Kanerva (2009), *Cognitive Computation* — HDC/BSC | 03, 16 |
| Vickrey (1961), *Journal of Finance* — Second-price auctions | 12, 13 |
| Woolley et al. (2010), *Science* — C-factor | 06, 14 |
| Lemire et al. (2022), *JEA* — Binary Fuse filters | 15 |
| Bhatia et al. (2020), *AAAI* — MIDAS-R | 16 |
| Mitzenmacher (2001), *IEEE TPDS* — Power of two choices | 11 |
| Damasio (1994), *Descartes' Error* — Somatic markers | 19 |
| Friston (2010), *Nature Reviews Neuroscience* — Active inference | 19 |
| Sumers et al. (2023) — CoALA cognitive architecture | 19 |
| Myerson (1981), *MOR* — Optimal auction design | 12, 13 |
| Kamvar et al. (2003), *WWW* — EigenTrust algorithm | 14 |
| Ben-Sasson et al. (2018) — ZK-STARKs | 03 |
| Binius/Irreducible (2024) — Binary field STARKs | 03 |
| Yang et al. (2025), *arXiv* — OpDiffer EVM differential testing | 18 |
| Hadfield & Koh (2025), *arXiv* — Economy of AI Agents | 20 |
| Poon & Dryja (2016) — Lightning Network / state channels | 20 |
| Lesaege et al. (2019) — Kleros decentralized arbitration | 20 |
| Weyl, Ohlhaver, Buterin (2022) — Soulbound tokens | 06 |
| Ousterhout et al. (2013), *SOSP* — Sparrow scheduler | 11 |
| Boyd & Vandenberghe (2004) — Convex optimization/QP | 21 |

---

## Cross-References

| Topic | Relationship |
|---|---|
| [00-architecture](../00-architecture/INDEX.md) | 5-layer taxonomy; chain is L2 scaffold + L1 framework |
| [01-synapse](../00-architecture/INDEX.md) | Universal loop; chain heartbeat is a domain parameterization |
| [04-gates](../04-verification/INDEX.md) | TxSimGate, WalletGate integrate into gate pipeline |
| [05-tools](../18-tools/INDEX.md) | MCP services for agent economy, x402 payment |
| [06-neuro](../06-neuro/INDEX.md) | HDC encoding shared between local NeuroStore and on-chain precompile |
| [07-daimon](../09-daimon/INDEX.md) | Somatic markers modulate chain agent's ANALYZE step |
| [09-oracle](../20-technical-analysis/INDEX.md) | Chain predictions calibrated against actual outcomes |
| [12-learn](../05-learning/INDEX.md) | Learning system uses reputation feedback |
| [14-identity-economy](../14-identity-economy/INDEX.md) | Broader identity context for ERC-8004 |
| `tmp/refinements/09-phase-2-implications.md` | Phase 2 implications for chain, dreams, mesh, coordination, heartbeat, and control-plane projection |

---

## Generation Notes

- **Generated**: 2026-04-11
- **Updated**: 2026-04-26 — Removed: Nunchi Passport (04), ventriloquist defense (05), 4-tier gossip architecture (07), 8 gossip topics (08), Valhalla privacy layer (22). Updated: 50ms blocktimes, simplex consensus, native ERC-8004 identity (full spec), ERC-8183 job market (replacing Spore branding).
- **Generator**: Claude (prd-migration prompt 08-chain)
- **Source material**: 7 context pack files, 5 refactoring-prd sources, 1 implementation plan (12b-chain-layer.md, 76 items), 9 legacy PRD files (14-chain/), 14 agent-chain-new research files, active code from roko-chain (9 files) and mirage-rs (38 files)
- **Sub-documents**: 20 (00-03, 06, 09-21, 23-24)
