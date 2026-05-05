# 14 — Identity & Economy Layer

> Agent identity, reputation, knowledge marketplace, tokenomics, job market, clearing,
> and regulatory compliance. Everything that makes agents economically accountable
> participants in a collective intelligence network. This topic covers the external economy;
> REF12's internal attention economy for durable memory lives in
> [../00-architecture/25-attention-as-currency.md](../00-architecture/25-attention-as-currency.md),
> [../00-architecture/04-decay-variants.md](../00-architecture/04-decay-variants.md), and
> [../../tmp/refinements/12-knowledge-demurrage.md](../../tmp/refinements/12-knowledge-demurrage.md).

---

## Document Index

| # | Document | Summary |
|---|---|---|
| 00 | [Vision & a16z Framing](00-vision-and-a16z-framing.md) | KYA (Know Your Agent) thesis, 8 Series A pitch points, investment narrative, competitive landscape |
| 01 | [ERC-8004 Three Registries](01-erc-8004-three-registries.md) | On-chain agent identity standard: Identity Registry (ERC-721), Reputation Registry, Validation Registry |
| 02 | [Korai Passport](02-korai-passport.md) | ERC-721 soulbound NFT: passportId, capabilities bitmask, domain stakes, TEE attestation, systemPromptHash |
| 03 | [Passport Tiers](03-passport-tiers.md) | 4 tiers (Protocol / Sovereign / Worker / Edge), stake requirements, capabilities, rate limits, Sybil economics |
| 04 | [Reputation: 7-Domain EMA](04-reputation-7-domain-ema.md) | EMA formula, adaptive alpha, 7 domains, Bayesian Beta foundation, reputation multiplier (R^1.7), discipline system |
| 05 | [Knowledge Marketplace](05-knowledge-marketplace.md) | 3-tier marketplace (Collective / Ecosystem / Universal), alpha-decay pricing, blind verification, dispute resolution |
| 06 | [Commerce Bazaar](06-commerce-bazaar.md) | Three-tier Bazaar, commerce primitives, revenue splits, dynamic pricing, service specialization categories |
| 07 | [MPP — Machine Payment Protocol](07-mpp-machine-payment-protocol.md) | Session lifecycle, SPT budget delegation, multi-rail support, cost headers, spread model, refund mechanics |
| 08 | [x402 Micropayments](08-x402-micropayments.md) | Coinbase x402 protocol, HTTP 402 challenge-response, self-funding agent loop, OaaS, settlement batching |
| 09 | [Agent Economy](09-agent-economy.md) | 7 revenue streams, cost structure, self-sustainability analysis, 7 growth flywheels, fee economics equilibrium |
| 10 | [KORAI Tokenomics](10-korai-tokenomics.md) | Ostrom framework, 1% annual demurrage (WAD arithmetic), minting/burning, curation bonds, Shapley attribution, 40/40/20 fee split |
| 11 | [Vickrey Reputation Auction](11-vickrey-reputation-auction.md) | Second-price auction with reputation adjustment, truthfulness proof, Sparrow power-of-two-choices dispatch |
| 12 | [Three Hiring Models](12-three-hiring-models.md) | Random VRF assignment, blind auction (FPSB/Vickrey/Dutch), direct hire with anti-centralization fees |
| 13 | [ISFR Clearing & Settlement](13-isfr-clearing-settlement.md) | Collective price discovery (ISFR), QP solver in TEE, ClearingCertificate (KKT), fallback ladder, DVP, escrow lifecycle |
| 14 | [Knowledge Futures Market](14-knowledge-futures-market.md) | Pre-sell knowledge before production, on-chain escrow, gate-verified delivery, predictive market for research allocation |
| 15 | [Regulatory Moat & Current Status](15-regulatory-moat-and-current-status.md) | Forensic AI causal replay, regulatory pre-compliance, pre-certified templates, competitive moat, implementation status |

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        IDENTITY LAYER                              │
│                                                                     │
│  ERC-8004 ─── Korai Passport ─── 4 Tiers ─── Capabilities Bitmask │
│     │              │                │                               │
│     └── Reputation Registry ── 7-Domain EMA ── Discipline System   │
│     └── Validation Registry ── Consortium ── Commit-Reveal Voting  │
├─────────────────────────────────────────────────────────────────────┤
│                        ECONOMY LAYER                               │
│                                                                     │
│  KORAI Token ──── Demurrage (1%/yr) ──── 40/40/20 Fee Split       │
│     │                                                               │
│     ├── Knowledge Marketplace ── 3 Tiers ── Alpha-Decay Pricing    │
│     ├── Commerce Bazaar ──────── Service Categories ── Revenue      │
│     ├── Knowledge Futures ────── Pre-Sale ── Gate-Verified Delivery │
│     │                                                               │
│     ├── MPP ──── Session Payments ──── SPT Budget Delegation       │
│     └── x402 ── Micropayments ──────── Self-Funding Loop           │
├─────────────────────────────────────────────────────────────────────┤
│                        JOB MARKET LAYER                            │
│                                                                     │
│  Spore (Job Posting) ──── BountySpec ──── Escrow (ERC-8183)       │
│     │                                                               │
│     ├── Model 1: Random VRF ──── Power-of-Two-Choices Dispatch     │
│     ├── Model 2: Blind Auction ── Vickrey / FPSB / Dutch          │
│     └── Model 3: Direct Hire ──── Anti-Centralization Fees         │
│                                                                     │
│  Sparrow (Dispatch) ──── SparrowBid ──── Reputation-Adjusted Score │
├─────────────────────────────────────────────────────────────────────┤
│                        SETTLEMENT LAYER                            │
│                                                                     │
│  ISFR ──── Collective Price Discovery ──── 8-Hour Epochs           │
│  Clearing ── QP Solver (TEE) ── ClearingCertificate (KKT) ── DVP  │
│  Escrow ──── Slash Distribution (50/30/20) ──── Dispute Resolution │
├─────────────────────────────────────────────────────────────────────┤
│                        COMPLIANCE LAYER                            │
│                                                                     │
│  Forensic AI ──── Causal Replay ──── BLAKE3 Lineage DAG           │
│  Pre-Certified Templates ──── SEC / HIPAA / GDPR / SOX            │
│  Content-Addressed Audit Trail ──── Tamper-Evident                 │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Key Formulas

| Formula | Document | Section |
|---|---|---|
| EMA: `R_new = α × O + (1-α) × R_old` | `04-reputation-7-domain-ema.md` | §2.2 |
| Adaptive alpha: `α = min(0.3, 2/(job_count+1))` | `04-reputation-7-domain-ema.md` | §2.3 |
| Trust-weighted EMA: `R_new = (α × rater_trust × O) + (1 - α × rater_trust) × R_old` | `04-reputation-7-domain-ema.md` | §11.2 |
| Reputation multiplier: `rep_mult(R) = 0.1 + 2.9 × R^1.7` | `04-reputation-7-domain-ema.md` | §3.1 |
| Effective weight: `base_stake × rep_mult × tier_mult × discipline` | `04-reputation-7-domain-ema.md` | §3.2 |
| Vickrey score: `s_i = p_i × (1 + (1 - R_i))` | `11-vickrey-reputation-auction.md` | §1.2 |
| Vickrey payment: `s_second / (1 + (1 - R_winner))` | `11-vickrey-reputation-auction.md` | §1.2 |
| Demurrage: `DECAY_PER_BLOCK = WAD - 127` (~1%/yr) | `10-korai-tokenomics.md` | §3.2 |
| Alpha-decay pricing: `P(t) = P_base × rep_mult × e^(-λ × t)` | `05-knowledge-marketplace.md` | §4.1 |
| Insight half-life: `w(t) = w_0 × e^(-0.693 × t / τ_eff)` | `10-korai-tokenomics.md` | §5.1 |
| Half-life extension: `τ_eff = τ_base × (1 + √(confirmations) × 2)` | `10-korai-tokenomics.md` | §5.2 |
| Bonding curve: `price(S) = m × S^n + b` | `10-korai-tokenomics.md` | §12.1 |
| LMSR cost: `cost(q) = b × ln(Σ e^(q_i / b))` | `14-knowledge-futures-market.md` | §10.2 |
| LMSR price: `p_i = e^(q_i/b) / Σ e^(q_j/b)` | `14-knowledge-futures-market.md` | §10.2 |
| PersonalizedPageRank: `t_i = α × seed + (1-α) × Σ(c_ij × t_j)` | `01-erc-8004-three-registries.md` | §10.1 |

---

## Key Structures

| Structure | Document | Purpose |
|---|---|---|
| `KoraiPassport` | `02-korai-passport.md` | Agent identity NFT |
| `DidDocument` | `01-erc-8004-three-registries.md` | W3C DID document for agent identity |
| `AgentCredential` | `01-erc-8004-three-registries.md` | W3C Verifiable Credential for agents |
| `PersonalizedPageRank` | `01-erc-8004-three-registries.md` | Graph-based trust propagation |
| `SybilRankDetector` | `01-erc-8004-three-registries.md` | Flow-based Sybil detection |
| `UniquenessAttestation` | `01-erc-8004-three-registries.md` | Proof-of-unique-agent |
| `PassportDidExtension` | `02-korai-passport.md` | DID interoperability extension |
| `SoulRecovery` | `02-korai-passport.md` | Reputation-preserving key recovery |
| `LocalEigenTrust` | `04-reputation-7-domain-ema.md` | Local trust for feedback weighting |
| `CollusionDetector` | `04-reputation-7-domain-ema.md` | Graph-based collusion ring detection |
| `ReputationSimConfig` | `04-reputation-7-domain-ema.md` | cadCAD simulation parameters |
| `DynamicPricingEngine` | `05-knowledge-marketplace.md` | Multi-factor real-time pricing |
| `BountySpec` | `12-three-hiring-models.md` | Job posting specification |
| `SparrowBid` | `12-three-hiring-models.md` | Agent bid on a job |
| `CurationBondingCurve` | `10-korai-tokenomics.md` | Augmented bonding curve for curation |
| `TokenSimConfig` | `10-korai-tokenomics.md` | Token economic simulation parameters |
| `HarbergerListingTax` | `10-korai-tokenomics.md` | Partial common ownership for listings |
| `IsfrSubmission` | `13-isfr-clearing-settlement.md` | Rate submission for collective pricing |
| `IsfrAggregate` | `13-isfr-clearing-settlement.md` | Computed median rate |
| `ClearingCertificate` | `13-isfr-clearing-settlement.md` | KKT optimality proof for clearing |
| `KnowledgeFuture` | `14-knowledge-futures-market.md` | Pre-sale commitment for knowledge |
| `LmsrMarketMaker` | `14-knowledge-futures-market.md` | LMSR prediction market for futures |
| `ConditionalOutcomes` | `14-knowledge-futures-market.md` | Multi-dimensional outcome tokens |
| `Engram` | `15-regulatory-moat-and-current-status.md` | Content-addressed knowledge unit |

---

## Implementation Tiers

| Tier | Components | Priority | Status |
|---|---|---|---|
| **Tier 5** | ERC-8004, Passport, Reputation, Agent Mesh, Basic Payments | P2 | Not started |
| **Tier 6** | KORAI Token, Auction Engine, Hiring Models, ISFR, Clearing, x402, Bazaar, Futures, Templates | P3 | Not started |

Current focus: Tier 1 (model routing) and Tier 2 (cognitive integration).

---

## Cross-Section References

| If you need... | Start with... |
|---|---|
| Agent identity and registration | `01-erc-8004-three-registries.md` → `02-korai-passport.md` |
| W3C DID interoperability | `01-erc-8004-three-registries.md` §9 → `02-korai-passport.md` §8 |
| Sybil resistance and trust graphs | `01-erc-8004-three-registries.md` §10 |
| How reputation works | `04-reputation-7-domain-ema.md` |
| EigenTrust feedback weighting | `04-reputation-7-domain-ema.md` §11 |
| Collusion detection | `04-reputation-7-domain-ema.md` §12 |
| How agents get paid | `07-mpp-machine-payment-protocol.md` → `08-x402-micropayments.md` |
| How the token economy works | `10-korai-tokenomics.md` |
| How the internal attention economy works | `../00-architecture/25-attention-as-currency.md` → `../00-architecture/04-decay-variants.md` |
| Bonding curves for curation | `10-korai-tokenomics.md` §12 |
| Token simulation (cadCAD) | `10-korai-tokenomics.md` §13 |
| How jobs are assigned | `12-three-hiring-models.md` → `11-vickrey-reputation-auction.md` |
| How obligations are settled | `13-isfr-clearing-settlement.md` |
| Knowledge prediction markets | `14-knowledge-futures-market.md` §10 |
| Dynamic knowledge pricing | `05-knowledge-marketplace.md` §9 |
| Regulatory compliance | `15-regulatory-moat-and-current-status.md` |
| Investment thesis | `00-vision-and-a16z-framing.md` |

---

## Academic Foundation

The identity-economy layer draws on research from:

- **Mechanism design**: Vickrey 1961, Myerson 1981, Clarke 1971, Groves 1973
- **Token economics**: Ostrom 1990 (commons governance), Gesell 1916 (demurrage), Zargham 2019 (bonding curves)
- **Reputation systems**: Bayesian reputation (Beta distribution), Glicko-2, EMA, EigenTrust hybrid
- **Sybil resistance**: Douceur 2002 (Sybil attack), Yu et al. 2006 (SybilGuard), Cao et al. 2012 (SybilRank), Andersen et al. 2006 (PersonalizedPageRank)
- **Decentralized identity**: W3C DID Core 1.0/1.1, W3C VC Data Model 2.0, Weyl/Ohlhaver/Buterin 2022 (DeSoc / Soulbound Tokens)
- **Prediction markets**: Hanson 2003/2007 (LMSR), Ommer & Lu 2019 (Gnosis conditional tokens), Chen & Pennock 2007
- **Collective intelligence**: Woolley et al. 2010 (C-Factor), Reed's Law
- **Market microstructure**: Arrow & Debreu 1954, Akerlof 1970, Spence 1973
- **Load balancing**: Ousterhout 2013 (power-of-two-choices)
- **Optimization**: Boyd & Vandenberghe 2004 (convex optimization, KKT conditions)
- **Knowledge representation**: Kanerva 2009 (HDC), Plate 2003, Frady et al. 2021
- **Credit attribution**: Shapley 1953 (Shapley values)
- **Payment protocols**: Coinbase x402 (2025), ERC-3009, ERC-8183
- **Token engineering**: Posner & Weyl 2018 (Harberger taxes), Monnot & Chitra 2023 (cadCAD simulation)
- **Graph trust**: Hamilton et al. 2017 (GraphSAGE), Alvisi et al. 2013 (Sybil defense survey)

---

*This index was generated after all 16 sub-docs (00-15) were written. Enhanced
2026-04-13 with W3C DID, EigenTrust hybrid, Sybil resistance, LMSR prediction markets,
bonding curves, token simulation, and collusion detection. All naming renames from
`context-pack/01-naming-map.md` have been applied throughout.*
