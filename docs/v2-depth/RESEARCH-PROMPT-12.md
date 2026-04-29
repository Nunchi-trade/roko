# Deep Research: Closing the Pitch — Aubakirova Memo, Deck Copy, and Remaining Gaps

Copy everything below the `---` line into Claude Desktop with deep research enabled.

---

## Context: What You're Walking Into

You are helping a founder close the final preparation for a **Series A pitch to a16z**. This is **Round 13 of a sustained research program** that has run 12 prior rounds across ~300 pages of intelligence. The core strategic decisions are now locked. What remains is execution: the exact words on slides, the memo that opens the door, and the gaps that could derail the meeting.

**Here is the full decision chain, compressed. Read all of it — every decision builds on the previous one.**

### The company

**Nunchi** is two things:

1. **Roko** — an open-source Rust agent runtime (18 crates, ~177K lines of code, Apache 2.0). Agents run through a 6-stage pipeline (OBSERVE → GATE → ASSEMBLE → INFER+TOOLS → REFLECT → CONSOLIDATE). A model router (CascadeRouter) learns which LLM to use per task, producing 10-30x cost reduction. An 11-gate verification pipeline validates output. A knowledge store accumulates what agents learn, with decay. Agents share knowledge through a common bus. The system develops itself — it reads PRDs, generates plans, dispatches Claude agents, validates with gates, persists results.

2. **Nunchi Chain** — a sovereign EVM L1 blockchain (NOT a layer-2 or layer-3). Simplex consensus, ~50ms blocks via co-located Tokyo validators (same architecture as Hyperliquid). Native HDC precompile (~400 gas for 10,240-bit similarity search — 20-100x cheaper than Solidity). ERC-8004 agent identities with 7-domain reputation. On-chain knowledge substrate with demurrage-based pruning. ZK-HDC proofs (<1s proving). A cooperative clearing engine that turns every trade into a knowledge deposit ("clearing-as-inference").

### The category (decided R11)

**"Agent Coordination Plane"** — the infrastructure layer that separates agent coordination from agent execution, the same way Software-Defined Networking (SDN) separated network control from packet forwarding.

This category was chosen because:
- **Malika Aubakirova** (a16z infra partner, the warm intro) wrote in a16z's Big Ideas 2026 essay: *"the bottleneck becomes coordination: routing, locking, state management, and policy enforcement across massive parallel execution."* She named the category. Nunchi builds the canonical implementation. This is the SDN-Nicira playbook: Kate Greene named SDN at MIT Technology Review in 2009; Martin Casado commercialized it with Nicira (acquired by VMware for $1.26B in 2012).
- "Trust layer" is burned — 7+ companies claim it (Capsule $7M, Nava $8.3M, t54 Labs $5M, Gen Digital, Cloud Security Alliance).
- "Agent OS" is burned — 6+ claimants (Sycamore $65M, /dev/agents $56M, PwC, AIOS).
- Casado rewards **descriptive structural names** that imply mechanical separation (SDN, lakehouse, analytics engineering). Never aspirational ones (trust, intelligence, autonomy).

### The beachhead (decided R12)

**Enterprise support contracts on Roko OSS** — not a platform, not a managed service, not a compliance tool.

Precedent: Temporal raised $18.75M Series A (Oct 2020) with **zero commercial product**. Cloud launched 2 years later. First 1,000 paying customers arrived 3.5 years after Series A. HashiCorp's Atlas (platform) failed; Vault Enterprise (one product, one buyer) won.

The wedge: agent identity + cost-aware routing + gates, sold as a control plane for one workload. First customers: Hebbia (#1, a16z portfolio, >2% of OpenAI daily volume), Harvey (#2, ~$5-15M/mo LLM spend, public job req for "Context Engineering & Agent Infrastructure"), Decagon (#3, a16z portfolio, hiring Staff SWE Agent Orchestration).

### The convergence proof (decided R12)

**Architectural, not mathematical.** Casado's PhD (Stanford 2007) is a systems-architecture thesis, not control theory. No Lyapunov proofs, no bandit theory. His vocabulary: logically centralized control, control/data plane separation, default-off, flow-level granularity, trusted computing base minimization.

The pitch closing line: *"Ethane reduced enterprise networks to dumb forwarding elements governed by a logically centralized policy. Nunchi reduces LLM agents to dumb invocation elements governed by a logically centralized routing-and-gating policy."*

Math is defensive only: UCB1 regret bounds, Borkar-Meyn ODE for EMA adaptation, Hedge for predict-publish-correct. One slide, one equation. Do NOT lead with Lyapunov — it reads as PhD theater.

### The demo (decided R12)

Local CLI binary against cached LLM proxy. Three minutes. Four primitives visible per output line (identity, prediction, gates, knowledge). Ends with kill-the-controller durability moment. Hand Casado the laptop for the second command.

```
$ nunchi run agents/researcher.py --task "Summarize Q3 fintech earnings"
  ⏵ agent      researcher@v2  ·  nhi://acme/researcher.v2  (verified)
  ⏵ predict    $0.043  ·  12.4s  ·  route: haiku → gpt-4o-mini
  ⏵ gates      pii_scan ✓   cost_ceiling<$0.10 ✓   sox_compliance ✓
  ⏵ knowledge  loaded 7 facts from /finance/q3 (3 agents, 0.91 avg conf)
  running ▓▓▓▓▓▓▓▓▓▓ done in 9.8s
  ⏵ actual     $0.031  (−28% vs predicted)  ·  routed to haiku
  ⏵ deposited  2 new facts → /finance/q3
```

### The access path (decided R12)

Pitch **Aubakirova first**, not Casado directly. She owns the agent-infra thesis on his team. She co-led **Keycard** ($38M, Oct 2025) — "Auth0 moment for agent access." **CRITICAL CONFLICT RISK**: if Nunchi sounds like Keycard, she passes. Differentiate: Keycard = identity authentication; Nunchi = coordination, prediction, and shared knowledge ABOVE the identity primitive.

Her "Et Tu, Agent?" blog post (April 2, 2026): 20% of AI-recommended packages are hallucinations; AI agents pick known-vulnerable deps 50% more often than humans.

### Key numbers

| Metric | Value | Source |
|--------|-------|--------|
| Agent coordination failure rate | 41-86% | MAST, NeurIPS 2025 (arXiv:2503.13657) |
| Failures from coordination | 79% | MAST |
| Naive agent cost | $44.86/task | Princeton HAL (arXiv:2510.11977) — excludes caching |
| Optimized cost | ~$1.42/task | HAL baseline + caching + routing + gating |
| NHI market | $9.45B → $18.71B by 2030 | 11.9% CAGR |
| Machine:human identity ratio | 82:1 to 144:1 | CyberArk / Entro 2025 |
| EU AI Act Article 50 | August 2, 2026 | ~14 weeks away |
| Platform multiple premium | 8.2x vs 3.9x SaaS | Equal Ventures / BVP Cloud Index |
| Temporal valuation | $5B at 40-60x ARR | Led by Sarah Wang, NOT Casado |
| Braintrust valuation | $800M | Casado-led Series A |
| Series A comp range | $15-35M at $150-250M | LangChain, CrewAI, E2B, Inngest, Mastra |

---

## What I need you to research now

### Direction 1: The Aubakirova Memo (Complete Draft)

Write the actual cold email memo to attach to a warm intro email to Malika Aubakirova. This is the most time-sensitive deliverable.

**Constraints:**
- Under 500 words
- Technical, not salesy — she is an ex-SRE (Chronicle Security / Google)
- Opens with her Big Ideas 2026 quote verbatim
- Shows Nunchi is what she described
- Cites MAST data (41-86% failure, 79% coordination)
- Shows cost proof ($44.86 → $1.42)
- Names the category: Agent Coordination Plane
- Differentiates from Keycard (identity auth is the primitive; Nunchi is the coordination layer above it — "Keycard solved identity; what solves the rest?")
- Notes Braintrust ($800M) validates the agent infra layer; Nunchi sits one layer deeper
- Closes: "30 minutes with Martin to show the demo"
- Includes the Ethane analogy in one sentence

Research: what format do a16z partners prefer for cold outreach? Is a PDF memo better than inline email? How long should it be? Look at how successful founders got meetings with a16z infra partners specifically. Any public accounts of how Temporal, LangChain, or Vercel first approached their lead investors?

### Direction 2: The 13-Slide Deck — Exact Words

Write the actual words for each of the 13 slides. Not a template. The real thing. Use these constraints from R10 and R12:

**Slide 1 (Title):** "The model is the same. The system is the variable." Below: "Nunchi — the Agent Coordination Plane." No animation, no number. Thesis wins over number on slide 1.

**Slide 2 (Problem):** One giant number. "41-86% of multi-agent deployments fail. 79% from coordination, not capability." Source: MAST (arXiv:2503.13657). Below: Aubakirova quote verbatim.

**Slide 3 (Solution):** The `nunchi run` CLI output — real, not pseudocode. Show output below it. Four primitives visible: identity, prediction, gates, knowledge.

**Slide 4 (Founder/Team):** Solo founder with three proof points: domain expertise, shipping track record, hiring/community pull. 2-4 named advisors with specific contributions.

**Slide 5 (Why Now):** Three converging forces: (1) standards crystallizing (MCP 97M/mo, A2A 150+ orgs), (2) Aubakirova thesis (a16z's own team named the problem), (3) regulatory tailwind (EU AI Act August 2, 2026). "Stripe locked the payments lane (x402/ACP). The value moves upstack to coordination."

**Slide 6 (Product):** Real screenshots. The four primitives: identity, routing, gates, shared knowledge. Connection between Roko runtime and Nunchi chain.

**Slide 7 (How It Works):** The 6-stage pipeline. But visual, not text. Show the predict-publish-correct loop with actual cost deltas.

**Slide 8 (Traction):** Logos + community. Roko: 18 crates, 177K LOC, self-hosting loop operational. Design partner conversations with [Hebbia, Harvey, Decagon]. GitHub stats. If no external logos yet, show the self-hosting milestone: "Roko develops itself."

**Slide 9 (Cost Comparison):** $44.86 → $1.42. Honest waterfall: HAL baseline (no cache) → caching alone (4-5x) → routing (3x) → gating (2x) → full stack ($1.42, ~30x). "All raw data published. Third-party reproducible."

**Slide 10 (Competition):** Harvey-Ball feature table (NOT 2x2). Include columns where competitors score higher (production users, dev community). Acknowledge Temporal, LangChain, Nava. Keycard is ADJACENT (identity) not competitive (coordination). "The coordination plane is not the trust layer."

**Slide 11 (Business Model):** Enterprise support + managed hosting. Apache 2.0 runtime + BSL cloud. Dual-asset structure (equity NunchiLabs + Foundation token deferred 18-24mo). "Ship one product first, then platform." Temporal precedent.

**Slide 12 (Use of Funds + Milestones):** Engineering 55%, GTM 25%, G&A 20%. Milestones: 3 FDE engagements by M6, SOC 2 by M9, $1M ARR by M12, 130% NRR target for Series B.

**Slide 13 (Ask + Close):** "$25M to build the Agent Coordination Plane." The Ethane line: "Ethane reduced networks to dumb forwarding elements governed by a centralized policy. Nunchi does the same for LLM agents." Close: "The model is commoditizing. The knowledge is not. We're building the network that compounds it."

For each slide provide: headline (max 8 words), subtext (1-3 sentences), key number, visual description, speaker notes (30-60s), sources.

### Direction 3: The Keycard Differentiation Deep-Dive

This is the highest-risk conversation in the meeting. Aubakirova co-led Keycard ($38M). If Nunchi sounds like Keycard, the deal is dead.

Research:
- What exactly does Keycard do? Product features, architecture, customer base, pricing
- How do they describe themselves? ("Auth0 moment for agent access" — but what does that mean technically?)
- What is an "identity-bound, task-scoped token"? How does this differ from ERC-8004?
- Where does Keycard's product END and Nunchi's BEGIN? What is the exact boundary?
- Is there a "better together" story? (Keycard handles identity auth → Nunchi handles coordination above it)
- Research: are there precedents of a16z funding two companies in the same stack? (e.g., investing in both a database and an ORM, or both an identity provider and an application platform)

### Direction 4: What's in Casado's Head Right Now (Last 30 Days)

Research his last 30 days of public activity (March 27 - April 26, 2026):
- Any blog posts on a16z.com?
- Any podcast appearances? (World of DaaS, Six Five Pod, Latent Space, others)
- Any tweets or X posts?
- Any new investments announced?
- Any conference talks?
- What is the Deeptune investment thesis — has Casado spoken about it since the March 19 announcement?
- Has he commented on: Temporal's $5B round? Cursor's $50B talks? The agent infrastructure wave?

Construct: the ideal conversation opener for a 30-minute meeting. What does he want to hear in the first 60 seconds that makes him lean forward?

### Direction 5: The "Why Not Just Use Temporal" Objection

Temporal is the elephant in the room. $5B valuation. a16z portfolio company. Agent companies use it. The objection will come.

Research:
- What specifically does Temporal NOT do that Nunchi does? (Agent identity, cost-aware routing, shared knowledge, reputation, ZK proofs)
- What does Temporal do that Nunchi should NOT try to replicate? (Durable execution, workflow checkpointing, deterministic replay)
- Is there a "Temporal + Nunchi" story? (Temporal for execution durability, Nunchi for agent coordination above it)
- How did Snowflake handle the "why not just use AWS Redshift" objection? How did Databricks handle "why not just use Snowflake"? The pattern for "why not just use the big player."
- Has Temporal commented on agent coordination? Do they see it as their next expansion?

### Direction 6: The Landing Page Rewrite Brief

The current landing page (nunchi.network) has 7 scroll sections: Loop, Scaffold, Anatomy, Memory, Collective, Chain, Proof. It uses the ROSEDUST dark aesthetic. Key problems: mock data (84,213 / 12,425 counters are fake), no mention of Agent Coordination Plane category, no Aubakirova quote, no cost comparison with HAL numbers, "Twelve organs. Five zones. One specimen." is inscrutable.

Draft a rewrite brief for the landing page that aligns it with the deck narrative:
- What should the hero section say? (Thesis line + one-sentence category definition)
- What sections should be added? (Cost proof, compliance, coordination plane explanation)
- What sections should be removed or condensed? (Anatomy is too abstract, Memory is too academic)
- Should the landing page match the deck 1:1 or serve a different audience?
- What real numbers should replace the mock data?

### Direction 7: Remaining Gaps That Could Derail the Meeting

What haven't we thought of? Research:
- Common reasons a16z passes at the partner meeting stage (not the initial screening)
- What due diligence does a16z infra run between first meeting and term sheet?
- Are there legal/structural issues with the dual-entity approach (Delaware C-corp + Foundation) that could surface?
- Is there a "too early" risk? (No revenue, no customers, no live chain)
- Is there a team risk? (Solo founder with 177K LOC — impressive but also a bus factor concern)
- What questions does Sarah Wang's platform team ask during diligence?

## Output Format

### 1. Aubakirova Memo (Complete, Ready to Send)
Under 500 words. PDF-ready format.

### 2. 13-Slide Deck (Exact Words)
For each slide: headline, subtext, key number, visual, speaker notes (30-60s), sources.

### 3. Keycard Differentiation Brief
One page. Product boundary, "better together" narrative, conflict mitigation.

### 4. Casado State-of-Mind (Last 30 Days)
Timeline of activity. The 60-second opener.

### 5. Temporal Objection Response
The prepared answer. With Snowflake/Databricks pattern.

### 6. Landing Page Rewrite Brief
Section-by-section recommendations. What to add, remove, change.

### 7. Deal-Killer Gap Analysis
Things that could go wrong that we haven't prepared for.

### 8. Full Citations
Every source with URLs and dates.
