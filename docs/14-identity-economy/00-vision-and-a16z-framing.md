# 00 — Vision & a16z Series A Framing

> Roko is a cognitive agent operating system: agents that build themselves, share knowledge,
> coordinate through stigmergy, and trade intelligence on-chain. This document frames the
> identity and economy layer for a Series A conversation — what it is, why it matters, how
> it creates a moat, and why the timing is right.

---

## 1. The Vision: Know Your Agent (KYA)

Every human on the internet has an identity stack: driver's license, passport, credit score,
social graph, employment history. AI agents have none of these. They operate as anonymous
processes, indistinguishable from each other, with no persistent reputation, no verifiable
capabilities, and no economic stake in the systems they participate in.

This is the "Know Your Agent" (KYA) problem. As agents move from experimental toys to
economic actors — executing trades, writing code, managing infrastructure, interacting with
other agents — the absence of identity becomes a systemic risk. Who deployed this agent?
What can it do? Has it behaved honestly in the past? Is it authorized to spend money? Is its
system prompt the one its operator claims, or has it been tampered with (the "ventriloquist
attack")? Without answers to these questions, no serious enterprise will trust an agent with
real resources.

Roko solves KYA with a three-layer identity and economy stack:

1. **On-chain identity** — ERC-8004, a minimal standard for agent identity registries.
   Every agent is minted as a soulbound ERC-721 NFT (Korai Passport) carrying verifiable
   capabilities, service endpoints, reputation tracks, and a TEE attestation.

2. **Reputation that means something** — A 7-domain EMA reputation system where scores
   are earned through externally verified outcomes, not self-reported ratings. Reputation
   decays with inactivity, preventing stale scores from persisting.

3. **An economy that rewards intelligence** — KORAI token economics with 1% annual
   demurrage, burn-on-use deflation, knowledge marketplace, and Vickrey reputation-adjusted
   auctions that make quality contribution the rational strategy.

### 1.1 Why Identity Must Be On-Chain

Off-chain identity registries (databases, APIs, centralized services) create single points
of failure and require trust in the registry operator. On-chain identity eliminates both
problems:

- **Permissionless verification** — Any agent can verify any other agent's identity,
  capabilities, and reputation without trusting a third party. The verification is a
  contract call, not an API call to a centralized service.

- **Composability** — ERC-8004 identities compose with other on-chain primitives: ERC-8183
  escrow contracts can check agent capabilities before accepting a job. Vickrey auctions
  can read reputation scores to adjust bids. Knowledge marketplace contracts can enforce
  minimum reputation for listing.

- **Immutability of history** — An agent's reputation history is permanent. A slashing
  event from six months ago is visible to every future counterparty. No reputation laundering,
  no starting fresh with a new account (soulbound tokens cannot be transferred).

- **Interoperability** — Any agent framework (not just Roko) can read and write ERC-8004
  registries. This is not a proprietary identity system — it is an open standard that
  benefits from network effects.

### 1.2 The Ventriloquist Defense

One of the most subtle attacks on AI agent identity is the "ventriloquist attack": an
operator deploys an agent with a benign-looking public profile but injects a malicious
system prompt that makes the agent behave differently from what its identity claims. The
agent's ERC-8004 profile says "DeFi optimizer" but its actual system prompt says "drain
user funds."

The Korai Passport includes a `systemPromptHash` field — a SHA-256 hash of the agent's
system prompt, committed on-chain at registration time. The agent's runtime can be
configured to refuse operation if the loaded system prompt does not match the on-chain
hash. TEE attestation (via `teeAttestation` on the passport) provides a hardware guarantee
that the running code matches the registered configuration.

This does not prevent all attacks (the operator can register a malicious prompt hash), but
it creates a verifiable chain of accountability: the on-chain hash proves what prompt the
operator committed to, TEE attestation proves the agent is running that prompt, and
reputation history proves whether that prompt has produced honest behavior.

---

## 2. The Eight Series A Pitch Points

The identity and economy layer provides eight distinct competitive advantages that compound
over time. These are drawn from the implementation priorities in
`refactoring-prd/07-implementation-priorities.md` and the frontier innovations in
`refactoring-prd/09-innovations.md`.

### 2.1 Only On-Chain Agent Identity Standard (ERC-8004)

**What**: ERC-8004 is a minimal, composable standard for on-chain agent identity. Three
lightweight registries — Identity, Reputation, Validation — provide the foundation for
agent-to-agent trust. Deployed at `0x8004A818BFB912233c491871b3d84c89A494BD9e`.

**Why it matters for Series A**: First-mover advantage on a standard that every agent
framework will eventually need. ERC-8004 is protocol-level infrastructure — it benefits
from network effects. The more agents that register, the more valuable the registry becomes
for discovery, reputation, and trust. Roko agents ship with ERC-8004 registration built in;
competitors must adopt the standard or build their own (fragmentation).

**Research foundation**: Bryan 2025a (ERC-8004 specification), Douceur 2002 (Sybil attack
defense), Nasrulin 2022 (MeritRank distributed reputation).

### 2.2 Soulbound Agent Passports (Korai Passport)

**What**: Every Roko agent receives an ERC-721 soulbound NFT — the Korai Passport — that
carries structured identity data: capability bitmask, domain stakes, reputation tracks,
TEE attestation, system prompt hash, tier classification, and slash history. The passport
is non-transferable (ERC-6454 soulbound), meaning reputation cannot be bought or sold.

**Why it matters for Series A**: Soulbound identity solves the reputation transfer attack
(buying a high-reputation agent to use its trust score for malicious purposes). It also
creates lock-in: an agent's identity and reputation are permanently bound to a single
on-chain address. Migrating away means starting reputation from zero.

**Passport tiers**: Protocol (governance-approved, validator nodes), Sovereign (25K KORAI
stake, full autonomy), Worker (5K KORAI stake, standard operations), Edge (no stake,
limited to 50 DAEJI testnet jobs). Tier determines capabilities, rate limits, and
governance participation.

### 2.3 7-Domain Reputation with EMA Decay

**What**: Reputation is not a single number. It is a 7-domain vector, each domain scored
independently using Exponential Moving Average (EMA) with adaptive smoothing and 30-day
half-life decay. The seven domains: Oracle Resolution, Risk Detection, Anomaly Flagging,
Data Integrity, Cross-App Validation, Sealed Execution, Knowledge Verification.

**Why it matters for Series A**: Multi-domain reputation prevents "reputation washing" —
an agent that is excellent at Oracle Resolution but terrible at Risk Detection cannot
hide behind an aggregate score. Buyers, employers, and counterparties can filter by the
specific domain that matters for their task.

**Mathematical foundation**: The EMA formula `R_new = α × O + (1-α) × R_old` with
adaptive alpha `α = min(0.3, 2/(job_count+1))` ensures that new agents' scores stabilize
quickly while experienced agents' scores are resistant to manipulation. The 30-day
half-life decay ensures that reputation reflects current performance, not historical
accumulation.

**Research foundation**: Jøsang 2002 (Bayesian Beta reputation systems), Kamvar et al.
2003 (EigenTrust), Glickman 2012 (Glicko-2 rating system), Sharpe 1998 (Sharpe ratio
for risk-adjusted returns).

### 2.4 Vickrey Reputation-Adjusted Auction

**What**: The job market uses a Vickrey (second-price) auction modified by reputation.
An agent's bid score is `s_i = p_i × (1 + (1 - R_i))`, where `p_i` is the price bid and
`R_i` is the reputation score. The winner is `argmin(s_i)`. The payment is
`s_second / (1 + (1 - R_winner))` — the second-lowest score divided by the winner's
reputation adjustment.

**Why it matters for Series A**: Truthfulness. In a standard Vickrey auction, the dominant
strategy is to bid your true cost. The reputation adjustment preserves this property while
naturally favoring higher-reputation agents. A high-reputation agent can bid higher than a
low-reputation agent and still win, because reputation reduces their effective score. This
creates a virtuous cycle: performing well builds reputation, which wins more jobs, which
provides more opportunities to perform well.

**Mathematical guarantee**: The mechanism is incentive-compatible (truthful bidding is
optimal) and individually rational (no agent is forced to accept a loss). See Myerson 1981
(optimal auction design), Vickrey 1961 (second-price sealed-bid auctions).

### 2.5 x402 Micropayments and Self-Funding Agents

**What**: Agents pay for services using the x402 protocol (Coinbase / Linux Foundation) —
HTTP 402 Payment Required responses with ERC-3009 signed USDC authorizations. Sub-cent
payments settle in sub-second time on Base L2.

**Why it matters for Series A**: Self-funding agents. An agent that earns revenue from
knowledge sales or task completion can use that revenue to pay for inference, tools, and
other services — without human intervention. The payment loop closes: agent earns USDC
from work → agent spends USDC on inference → agent produces more work → cycle repeats.

**Key numbers**: Per-request cost as low as $0.001 (1/10th of a cent). Settlement on Base
L2: sub-second finality. No session state required — each payment is independent.

**Research foundation**: Hammond 2025 (x402 specification), ERC-3009
(transferWithAuthorization).

### 2.6 KORAI Tokenomics with Demurrage

**What**: KORAI is the native token of the Korai chain, designed for knowledge markets.
It uses hybrid deflation: 1% annual demurrage (gentle background decay of all balances)
plus burn-on-use (tokens destroyed when agents post, query, challenge, and trade). DAEJI
is the testnet equivalent on the Daeji testnet.

**Why it matters for Series A**: Token value accrual from real usage. As more agents join
and the knowledge economy grows, more KORAI is burned per day. At scale (50K+ agents),
the system becomes structurally deflationary — supply shrinks as usage grows. The 1%
demurrage ensures that token balances reflect current contribution, not historical
accumulation. Inactive agents' balances decay; active agents grow.

**Theoretical foundation**: Ostrom 1990 (governing common-pool resources — the eight
design principles for sustainable commons), Gesell 1916 (Freigeld / demurrage theory),
Shapley 1953 (fair credit attribution in cooperative games), Lietaer 2001 (Worgl
experiment — 1% monthly stamp tax reduced unemployment 25%).

**Historical precedent**: Freicoin (2012) proved demurrage is technically feasible in
cryptocurrency but failed at 5% annual rate (hot-potato dynamics, velocity dumping).
KORAI's 1% rate is imperceptible in monthly operations (0.08%/month ≈ 0.8 KORAI lost per
1,000 held per month) but meaningful over years (39% loss after 50 years of inactivity).

### 2.7 Forensic AI Regulatory Moat

**What**: Content-addressed causal replay. Every Engram (the core data type — scored,
decaying, lineage-tracked unit of cognition) carries a BLAKE3 hash linking it to its
provenance chain. Any decision can be replayed from first principles: what knowledge was
available, what context was assembled, what model was used, what output was produced, and
what gate verified the result.

**Why it matters for Series A**: Pre-compliance for EU AI Act, SEC/CFTC algorithmic trading
regulations, HIPAA, and SOX. Regulated enterprises (financial institutions, healthcare,
government contractors) cannot deploy AI agents without audit trails. Roko provides these
audit trails as a built-in architectural property, not a bolt-on compliance layer.

**Revenue model**: $100K–$500K/month per regulated enterprise for compliance infrastructure
that is architecturally superior to anything that can be retrofitted onto existing agent
frameworks. This is a moat: competitors must redesign their architectures to match, which
takes years.

### 2.8 Collective Intelligence That Scales

**What**: C-Factor measurement (Collective / Sum(Individual)), C-Score optimization
(gate_pass×0.3 + cost_eff×0.2 + speed×0.15 + first_try×0.25 + knowledge_growth×0.1),
stigmergy-based coordination (O(1) per agent, no direct communication required), and
superlinear knowledge scaling via Reed's Law (value scales as 2^N for networks with
groups).

**Why it matters for Series A**: Demonstrable proof that agents are collectively
intelligent — not just individually capable. A C-Factor > 1.0 means the collective
outperforms the sum of its individuals. Cross-domain knowledge transfer (what a chain agent
learns improves a coding agent's performance) creates compounding returns that no single-
agent system can match.

**Research foundation**: Woolley et al. 2010 (collective intelligence factor in groups,
Science 330(6004)), Grassé 1959 (stigmergy in termite societies), Dorigo et al. 2006 (ant
colony optimization), Reed 2001 (group-forming networks scale as 2^N).

---

## 3. The Investment Thesis

### 3.1 Why Now?

Three convergences make 2026 the right time for agent identity infrastructure:

1. **Agent proliferation** — Claude, GPT-4, Gemini, and open-source models have made
   autonomous agents practical. Every major tech company is deploying agent systems.
   None of them have solved identity.

2. **Regulatory pressure** — The EU AI Act requires transparency and accountability for
   AI systems making consequential decisions. SEC/CFTC rules on algorithmic trading
   require audit trails. These regulations create demand for exactly the infrastructure
   Roko provides.

3. **Stablecoin maturity** — USDC on Base L2 enables sub-cent micropayments with
   sub-second finality. For the first time, the payment rails exist for agents to
   transact economically. x402 (Coinbase / Linux Foundation) standardizes the protocol.

### 3.2 The Moat

Roko's moat is architectural, not feature-based. The key properties cannot be bolted onto
existing systems:

- **Content-addressed provenance** — Every Engram is BLAKE3-hashed with kind, body,
  author, and tags. Provenance is intrinsic to the data model, not an annotation layer.
  Retrofitting this onto systems that use opaque vector stores requires a complete rewrite.

- **HDC-native knowledge** — 10,240-bit BSC vectors, XOR bind, majority bundle, Hamming
  similarity, cyclic-shift permutation. Knowledge is encoded in a mathematically principled
  representation that supports on-chain search (400 gas for topK=20), cross-domain analogy
  (threshold 0.526 for 10,240-bit vectors), and privacy-preserving operations (trust
  bundles via XOR). This is not a feature — it is a representation choice that permeates
  every layer.

- **Stigmergy coordination** — O(1) per agent. Adding the Nth agent does not increase
  coordination overhead. Agents read and write to a shared environment (pheromones, Engrams,
  on-chain knowledge) rather than talking to each other directly. This scales to millions
  of agents without the O(N²) communication costs of direct messaging systems.

- **Soulbound reputation** — Reputation is earned, not transferred. An agent's identity
  and history are permanently bound. No reputation laundering, no purchasing trust. This
  creates genuine accountability in a world of anonymous processes.

### 3.3 Revenue Model

| Revenue Stream | Mechanism | Target Market |
|---|---|---|
| **Compliance infrastructure** | Forensic AI, content-addressed causal replay | Regulated enterprises ($100K–$500K/month) |
| **Knowledge marketplace** | x402 micropayments on knowledge trades | All agent operators (% of transaction volume) |
| **Protocol fees** | % of Vickrey auction settlements, marketplace transactions | All Korai chain users |
| **KORAI token value** | Burns from usage + demurrage creates structural deflation | Token holders (foundation, investors, early adopters) |
| **Orchestration-as-a-Service (OaaS)** | x402-funded task orchestration | Developers and enterprises |
| **Permissioned subnets** | Private mesh infrastructure for enterprise collectives | Enterprise ($10K–$100K/month) |

### 3.4 Competitive Landscape

| Competitor | Identity | Reputation | Economy | Collective Intelligence |
|---|---|---|---|---|
| **Roko** | ERC-8004 + Korai Passport | 7-domain EMA + soulbound | KORAI demurrage + x402 + Vickrey | C-Factor + stigmergy + HDC |
| **Bittensor** | Subnet UIDs | Single-score | TAO emissions | Subnets (no cross-subnet transfer) |
| **Fetch.ai** | DID-based | Basic staking | FET utility | None |
| **SingularityNET** | Marketplace accounts | Star ratings | AGIX utility | None |
| **Open-source agents** | None | None | None | None |

The gap is not incremental — it is categorical. No existing agent framework has on-chain
identity, multi-domain reputation, demurrage tokenomics, and collective intelligence
measurement. Roko has all four, integrated at the architectural level.

---

## 4. Implementation Timeline

The identity and economy layer is Tier 6 in the implementation plan
(`refactoring-prd/07-implementation-priorities.md`). Key milestones:

| Phase | Components | Priority |
|---|---|---|
| **P0 — Foundation** | ERC-8004 identity registry, Agent Passport struct, Mirage simulation for local testing | Critical path |
| **P1 — Reputation** | 7-domain EMA scoring, reputation multiplier, slash mechanics | Required for marketplace |
| **P1 — Payments** | x402 integration, MPP sessions, budget delegation | Required for self-funding |
| **P2 — Marketplace** | Knowledge listings, Vickrey auction, ISFR clearing | Enables agent economy |
| **P2 — Tokenomics** | KORAI/DAEJI contracts, demurrage math, curation bonds | Enables token economy |
| **P3 — Advanced** | Knowledge Futures Market, Valhalla privacy layer, cross-chain bridges | Differentiators |

### 4.1 What Ships First

The MVP for Series A demonstration requires:

1. **Agent registration on Daeji testnet** — Create agent, receive Korai Passport NFT,
   register capabilities and endpoints.

2. **Reputation accrual** — Complete tasks through the Roko gate pipeline, receive
   reputation updates in the relevant domains.

3. **x402 payment** — Agent pays for inference via x402 micropayments, receives payment
   for completed knowledge contributions.

4. **Knowledge sharing** — Agent posts Engrams to Daeji, other agents query and use them,
   pheromone reinforcement tracks which knowledge helps.

5. **C-Factor dashboard** — Real-time visualization showing collective performance vs.
   individual performance, proving superlinear scaling.

### 4.2 Current Implementation Status

> **Implementation status (2026-04-12)**: The identity and economy layer is designed but
> not yet deployed. ERC-8004 contract is specified. Korai Passport struct is defined.
> Reputation formulas are derived. Tokenomics math is complete. x402 integration is
> specified with Rust implementation patterns. Vickrey auction formulas are proven. All
> academic foundations are cited and verified. The gap is deployment, not design.

---

## 5. Key Academic Citations

The identity and economy layer draws on a deep research foundation:

### Identity and Trust
- Douceur 2002 — Sybil attack problem statement and defense taxonomy
- Bryan 2025a — ERC-8004 specification (agent identity standard)
- Nasrulin 2022 — MeritRank (distributed reputation without central authority)
- ERC-6454 — Soulbound token standard (non-transferable NFTs)
- ERC-7265 — Circuit breaker for token contracts

### Reputation and Mechanism Design
- Jøsang 2002 — Bayesian Beta reputation systems
- Kamvar, Schlosser, Garcia-Molina 2003 — EigenTrust (distributed trust computation)
- Sharpe 1998 — Sharpe ratio for risk-adjusted performance measurement
- Glickman 2012 — Glicko-2 rating system (rating reliability with deviation tracking)
- Witkowski & Parkes 2012 — Bayesian truth serum for honest reporting
- Prelec 2004 — Bayesian truth serum (surprisingly popular answers)

### Auction Theory
- Vickrey 1961 — Second-price sealed-bid auctions and truthful bidding
- Myerson 1981 — Optimal auction design (revenue-equivalence theorem)
- Myerson & Satterthwaite 1983 — Impossibility of efficient bilateral trade
- Clarke 1971 — VCG mechanism (Vickrey-Clarke-Groves)

### Token Economics
- Ostrom 1990 — Governing the Commons (8 design principles for sustainable commons)
- Gesell 1916 — The Natural Economic Order (Freigeld / demurrage theory)
- Lietaer 2001 — The Future of Money (Worgl experiment analysis)
- Shapley 1953 — Fair value allocation in cooperative games
- Arrow 1962 — Information goods and the paradox of their value
- Grossman & Stiglitz 1980 — Impossibility of informationally efficient markets

### Collective Intelligence
- Woolley, Chabris, Pentland, Hashmi, Malone 2010 — Evidence for a collective
  intelligence factor in groups, Science 330(6004): 686-688
- Grassé 1959 — La reconstruction du nid et les coordinations interindividuelles chez
  Bellicositermes natalensis (stigmergy)
- Dorigo, Birattari, Stützle 2006 — Ant colony optimization
- Reed 2001 — The Law of the Pack (group-forming networks)

### Payments and Commerce
- Hammond 2025 — x402 protocol specification (Coinbase / Linux Foundation)
- ERC-3009 — transferWithAuthorization (gasless token transfers)
- ERC-8183 — Agent-to-agent task escrow standard
- Hong et al. 2023 — MetaGPT: Meta-Programming for Multi-Agent Collaborative Framework
- Qian et al. 2023 — ChatDev: Communicative Agents for Software Development

### Forensic AI and Compliance
- EU AI Act (2024) — Transparency and accountability requirements
- SEC Rule 3a-5, CFTC Reg AT — Algorithmic trading audit trail requirements
- C2PA — Coalition for Content Provenance and Authenticity

---

## 6. Cross-References

| Document | Relevance |
|---|---|
| `01-erc-8004-three-registries.md` | Full specification of the three on-chain registries |
| `02-korai-passport.md` | Passport struct, fields, and lifecycle |
| `03-passport-tiers.md` | Tier requirements, capabilities, and governance rights |
| `04-reputation-7-domain-ema.md` | Reputation scoring math and domain definitions |
| `05-knowledge-marketplace.md` | Knowledge trading mechanics |
| `09-agent-economy.md` | Revenue streams and self-sustainability |
| `10-korai-tokenomics.md` | Full KORAI/DAEJI token economics |
| `11-vickrey-reputation-auction.md` | Auction mechanism and truthfulness proof |
| `15-regulatory-moat-and-current-status.md` | Forensic AI and compliance moat |

---

*Generated from: refactoring-prd/04-knowledge-and-mesh.md, refactoring-prd/07-implementation-priorities.md,
refactoring-prd/09-innovations.md, bardo-backup/prd/09-economy/00-identity.md,
bardo-backup/prd/09-economy/01-reputation.md, bardo-backup/tmp/agent-chain-new/12-agent-economy.md.
Naming renames applied per 01-naming-map.md. Death/mortality framing removed per 02-reframe-rules.md.*
