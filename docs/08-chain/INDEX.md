# Topic 08: Chain Layer (Korai)

> The Korai chain is a dedicated EVM for agent coordination: identity (soulbound passports), reputation (7-domain EMA), marketplace (Spore/Sparrow), knowledge sharing (HDC precompile), and economic incentives (KORAI/DAEJI tokens with demurrage). Blockchain is ONE domain plugin — the chain layer uses the same Synapse traits as every other domain.

---

## Sub-documents

| # | Document | Summary |
|---|---|---|
| 00 | [00-vision-and-framing.md](./00-vision-and-framing.md) | Blockchain as domain plugin framing. Three-level knowledge architecture (Local → Mesh → Chain). What goes on-chain vs. off-chain. Tier 6 deferred status. |
| 01 | [01-korai-chain-spec.md](./01-korai-chain-spec.md) | Chain parameters (400ms blocks, 2s finality). Block structure, state model. Custom `korai_*` RPC methods. Chain intelligence pipeline. Deployment modes. |
| 02 | [02-korai-token-economics.md](./02-korai-token-economics.md) | KORAI/DAEJI tokens. Demurrage (1% annual decay). 5 earning mechanisms, 5 spending mechanisms. Fee distribution. Relationship to NeuroStore decay. |
| 03 | [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md) | HDC precompile at 0xA01. 10,240-bit BSC vectors. ~400 gas for top-K=20. Three-tier search (Bloom → approximate → exact). Cross-domain resonance. |
| 04 | [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md) | Soulbound ERC-721 agent identity. Full passport struct. 4 tiers (Protocol/Sovereign/Worker/Edge). Capability bitmask. Registration process. |
| 05 | [05-ventriloquist-defense.md](./05-ventriloquist-defense.md) | SHA-256 system prompt hash committed on-chain. Pre-job TEE verification. 24h timelock for updates. Rate limiting. Prompt replacement attack prevention. |
| 06 | [06-erc-8004-registries.md](./06-erc-8004-registries.md) | Three on-chain registries: Identity (passport), Reputation (feedback auth, EMA), Validation (work proofs). Separation of concerns. Cross-registry flows. |
| 07 | [07-4-tier-gossip-architecture.md](./07-4-tier-gossip-architecture.md) | GossipSub v1.1 (ms) → MiroFish simulation (sec) → FABRIC TEE aggregation (epoch) → Canonical Event Bus (block). Gossip envelope format. |
| 08 | [08-eight-gossip-topics.md](./08-eight-gossip-topics.md) | 8 topics: knowledge, reputation, job, heartbeat, anomaly, simulation, governance, peer-discovery. Schemas, TTL policies, subscription rules. |
| 09 | [09-peer-scoring-3-layer.md](./09-peer-scoring-3-layer.md) | 3-layer scoring: protocol (GossipSub mesh), application (domain behavior), economic (stake-weighted). Combined score determines mesh membership. Sybil resistance. |
| 10 | [10-spore-job-market.md](./10-spore-job-market.md) | Spore job marketplace. Job lifecycle (POSTED → SETTLED). Capability matching. Escrow. Fee structure. Solo/Pair/Consortium/Collective job types. |
| 11 | [11-sparrow-power-of-two-choices.md](./11-sparrow-power-of-two-choices.md) | Power-of-two-choices dispatch. O(log log N) max load. VRF-based selection. Load probing. Fallback to auction. |
| 12 | [12-three-hiring-models.md](./12-three-hiring-models.md) | Random VRF (fast), Blind Auction (competitive, 3 variants), Direct Hire (1.5× premium, Tier 0-1 only). Speed-quality-cost tradeoffs. |
| 13 | [13-vickrey-reputation-auction.md](./13-vickrey-reputation-auction.md) | Adjusted score: `s_i = p_i × (1 + (1 - R_i))`. Payment: `s_second / (1 + (1 - R_winner))`. Truthful bidding preserved. Commit-reveal scheme. |
| 14 | [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md) | 7 base domains. EMA smoothing with adaptive alpha. 30-day half-life decay. 4 discipline states. Slash rates by violation type. C-factor aggregation. |
| 15 | [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md) | Binary Fuse filter (8.7 bits/entry, <1% FPR). WebSocket block ingestion. Gap detection with Roaring Bitmaps. Connection pool with HTTP fallback. |
| 16 | [16-triage-curiosity-midas.md](./16-triage-curiosity-midas.md) | 4-stage pipeline: rule-based → MIDAS-R anomaly → contextual enrichment → HDC/Bayesian curiosity scoring. No LLM in this path. |
| 17 | [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md) | `ChainClient` (8 read methods) and `ChainWallet` (5 write methods). 3 custody modes: delegation, embedded (ERC-4337), local key. Types and errors. |
| 18 | [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md) | In-process EVM on revm. Fork mode, scenario replay, 141 tests. Korai chain extensions (HDC precompile, registries, RPC). roko_bridge for trait impl. |
| 19 | [19-chain-agent-heartbeat.md](./19-chain-agent-heartbeat.md) | 9-step mapping to Synapse loop: OBSERVE → RETRIEVE → ANALYZE → GATE → SIMULATE → VALIDATE → EXECUTE → VERIFY → REFLECT. Three cognitive speeds. |
| 20 | [20-x402-micropayments.md](./20-x402-micropayments.md) | HTTP 402 Payment Required. ERC-3009 transferWithAuthorization. No API keys, no accounts. Batch settlement. Self-funding agent loop. |
| 21 | [21-isfr-clearing-settlement.md](./21-isfr-clearing-settlement.md) | Intersubjective Fact Registry. QP solver with bisection O(80n). KKT optimality certificates verifiable on-chain. Reputation-weighted aggregation. |
| 22 | [22-valhalla-privacy-layer.md](./22-valhalla-privacy-layer.md) | 4 privacy tiers: Public → Access-Gated → Confidential (TEE) → Full Sealed (ZK). PSI for capability matching. TEE attestation integration. |
| 23 | [23-knowledge-futures-market.md](./23-knowledge-futures-market.md) | P3 deferred. Committed knowledge production via staked futures. Demand signaling. Early withdrawal penalties. Market-making function. |
| 24 | [24-current-status-and-6-contracts.md](./24-current-status-and-6-contracts.md) | Implementation status. 6 Solidity contracts: Agent Registry, Reputation, Marketplace, Escrow, KORAI Token, Validation. Build order. Tier 6 dependencies. |

---

## Key Concepts

- **Korai**: Dedicated EVM chain for agent coordination (not Ethereum mainnet)
- **KORAI**: Native token with 1% annual demurrage. DAEJI is the testnet equivalent
- **Korai Passport**: Soulbound ERC-721 NFT — the agent's on-chain identity
- **ERC-8004**: Three registries (Identity, Reputation, Validation)
- **HDC Precompile**: 10,240-bit BSC vectors at ~400 gas for top-K similarity search
- **Spore**: Job posting protocol. **Sparrow**: Fast-path dispatch (power-of-two-choices)
- **mirage-rs**: In-process EVM simulator for development and testing
- **ChainWitness**: Binary Fuse filter block pre-screening pipeline
- **Triage**: 4-stage curiosity scoring (rule → MIDAS-R → enrichment → HDC/Bayesian)
- **Valhalla**: 4-tier privacy layer (Public → Access-Gated → TEE → ZK)
- **ISFR**: Intersubjective Fact Registry with QP clearing and KKT certificates

---

## Critical Framing

**Blockchain is ONE domain plugin, not the default frame.** The Korai chain uses the same Synapse traits (`Substrate`, `Scorer`, `Gate`, `Router`, `Composer`, `Policy`) as every other domain. A chain agent is configured differently from a coding agent, but the cognitive architecture is identical. Nothing about the chain layer requires special treatment in the Roko kernel.

**Tier 6 is DEFERRED.** All 76 implementation items in the chain layer are blocked by Tier 5 (self-hosting loop). The specifications in this topic are complete but the implementation has not started.

**The 31.6× collective calibration number is a HEURISTIC**, not a theorem. It derives from the 1/√(N×t) scaling assumption under idealized conditions. Real-world performance depends on agent quality distribution, error correlation, and information flow effectiveness.

---

## Academic Foundations (Selected)

| Citation | Used In |
|---|---|
| Grassé (1959), *Insectes Sociaux* — Stigmergy | 00, 07, 10 |
| Kanerva (2009), *Cognitive Computation* — HDC/BSC | 03, 16 |
| Vyzovitis et al. (2020) — GossipSub v1.1 | 07, 09 |
| Vickrey (1961), *Journal of Finance* — Second-price auctions | 12, 13 |
| Woolley et al. (2010), *Science* — C-factor | 06, 14 |
| Lemire et al. (2022), *JEA* — Binary Fuse filters | 15 |
| Bhatia et al. (2020), *AAAI* — MIDAS-R | 16 |
| Mitzenmacher (2001), *IEEE TPDS* — Power of two choices | 11 |
| Damasio (1994), *Descartes' Error* — Somatic markers | 19 |
| Friston (2010), *Nature Reviews Neuroscience* — Active inference | 19 |
| Sumers et al. (2023) — CoALA cognitive architecture | 19 |
| Myerson (1981), *MOR* — Optimal auction design | 12, 13 |
| Weyl, Ohlhaver, Buterin (2022) — Soulbound tokens | 04, 05, 06 |
| Ousterhout et al. (2013), *SOSP* — Sparrow scheduler | 11 |
| Boyd & Vandenberghe (2004) — Convex optimization/QP | 21 |
| Costan & Devadas (2016) — Intel SGX | 05, 22 |

---

## Cross-references to Other Topics

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
| [14-identity-economy](../14-identity-economy/INDEX.md) | Broader identity context for Korai Passport |

---

## Generation Notes

- **Generated**: 2026-04-11
- **Generator**: Claude (prd-migration prompt 08-chain)
- **Source material**: 7 context pack files, 5 refactoring-prd sources, 1 implementation plan (12b-chain-layer.md, 76 items), 9 legacy PRD files (14-chain/), 14 agent-chain-new research files, active code from roko-chain (9 files) and mirage-rs (38 files)
- **Naming applied**: GNOS→KORAI/DAEJI, Golem→Agent, Grimoire→Neuro, Styx→Agent Mesh, Clade→Collective/Mesh, Bardo→Roko, Signal→Engram
- **Sub-documents**: 25 (00-24)
- **Total output**: ~75,000 words across 26 files
