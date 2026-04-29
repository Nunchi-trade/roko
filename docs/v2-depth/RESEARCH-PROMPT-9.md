# Deep Research Prompt — Round 9b (13-Slide Deck: Exact Words, Numbers, and Visuals)

Copy everything below the `---` line into Claude Desktop with deep research enabled.

---

## What I need

Write the actual investor pitch deck for **Nunchi** — not a template, not an outline, but the exact words on each slide. 13 slides. Every headline, every subtext line, every number, and a description of the visual for each slide. This is for a meeting with Martin Casado (a16z infrastructure fund, $1.7B AUM) via warm intro through Malika Aubakirova.

The narrative arc is: **hook → problem → solution → proof → chain → identity → market → competition → business model → compliance → [team] → GTM → ask**.

## Context you need

**What Nunchi is (two parts):**

1. **Roko** — open-source Rust agent runtime (18 crates, ~177K LOC). Three primitives: Signal (durable, content-addressed, HDC-fingerprinted), Pulse (ephemeral on Bus), Cell (atomic computation implementing 9 protocols). Graph composition (TOML DAGs). Native 6-stage harness: OBSERVE → GATE → ASSEMBLE → INFER+TOOLS → REFLECT → CONSOLIDATE. CascadeRouter for model selection. 11-gate pipeline with adaptive thresholds. Every operator is a learner via predict-publish-correct on the Bus.

2. **Nunchi chain** — sovereign EVM L1 (NOT an L2/L3). Simplex consensus (Chan & Pass, IACR 2023/463). ~50ms blocks via co-located Tokyo validators (same architecture as Hyperliquid). reth/revm fork with native custom precompiles. HDC precompile at 0xA01 (~400 gas for top-K similarity search — 20-100x cheaper than Solidity). ERC-8004 agent identities (standard transferable, used to their fullest extent with 7-domain EMA reputation, ZK-HDC behavioral verification). On-chain knowledge substrate with demurrage-based pruning (agents query chain knowledge to inject context). ERC-8183 job market. ZK-HDC proofs (Circom + Groth16, <1s proving, ~250K gas verification).

**The positioning:** "Nunchi is the identity, reputation, and verifiable-similarity layer for the agent economy." Cost reduction (10-30x) is the wedge that gets developers in the door. Trust, identity, and cross-organization reputation is the moat that keeps them. Network effect: "The thousandth agent joins smarter than the first."

**The thesis statement (from the landing page):** "The model is the same. The system is the variable."

**April 2026 model pricing (verify these are current before using):**
- Claude Opus 4.7: $5/$25 per MTok (launched April 16, 2026)
- GPT-5.5: $5/$30 per MTok (launched April 23, 2026)
- DeepSeek V4: $0.14/$0.28 per MTok (launched April 24, 2026)
- DeepSeek V4 cache-read: $0.028/MTok

**Key data points to use (all sourced):**
- MAST taxonomy (Berkeley, NeurIPS 2025, arXiv:2503.13657): 41-86.7% multi-agent failure rates across 1,642 production traces, 7 frameworks. 79% of failures from coordination, not model capability.
- Princeton HAL benchmark (ICLR 2026, arXiv:2510.11977): 21,730 rollouts. SWE-Agent + Claude Opus = $44.86-$59 per task at 54% accuracy. HAL Generalist + Haiku = $2.97 at 44%. "Agents can be 100x more expensive while only being 1% better."
- Cost reduction stack: prompt cache (5x, Anthropic: 92% cache hit in Claude Code production) × model routing (3x, RouteLLM: 85% cost cut retaining 95% quality) × gating (2x) = 30x theoretical, 10-20x practical.
- NHI market: $9.45B (2024) → $18.71B by 2030 at 11.9% CAGR. Machine-to-human identity ratio: 82:1 globally (CyberArk 2025), 144:1 in financial services (Entro 2025).
- NHI acquisitions: Saviynt $700M at ~$3B (KKR, Dec 2025). Palo Alto acquired CyberArk for $25B (Feb 2026). GitGuardian $50M Series C pivoting to NHI (Feb 2026). Oasis Security $120M (March 2026).
- EU AI Act Article 50 enforcement: August 2, 2026 (~14 weeks). Only 35.7% of EU managers feel prepared (Deloitte, n=500). Only 26.2% have started concrete compliance. Penalties: €35M or 7% turnover (prohibited), €15M or 3% (transparency).
- Compliance-as-distribution: Vanta ~$100M+ ARR from SOC 2. OneTrust $5B+ from GDPR. PCI-DSS is Stripe's hidden moat.
- ERC-8004: ~80-150K active agents at 3 months post-mainnet (Ethereum, Base, Mantle, TRON, Hedera). 22,900 registrations in first 3 days.
- MCP: 97M monthly SDK downloads. Donated to Linux Foundation AAIF Dec 2025.
- Series A comps: LangChain $25M at $200M (Sequoia). CrewAI $18M at ~$100M (Insight). E2B $21M (Insight). Inngest $21M (Altimeter + a16z). Mastra $22M (Spark). Story Protocol $25M→$80M at $2.25B (a16z crypto). /dev/agents $56M at $500M (Index/CapitalG).
- Nava: $8.3M seed, April 14, 2026. Polychain + Archetype. Founders ex-EigenLayer. Building "Arbiter" on Arbitrum L3 + Tempo. Validates the trust-layer thesis.
- Braintrust: $800M valuation, Series A led by Casado at a16z, $80M Series B led by Iconiq. AI observability. In Casado's portfolio — proves he invests in agent infrastructure.
- Temporal: $300M at $5B (a16z-led, Feb 2026). >380% YoY growth. 9.1T lifetime actions. The durable execution comparable.
- Token graveyard (appendix): VIRTUAL peaked $5.07 → now ~$0.70 (-86%). AI16Z/ELIZAOS -99.98% after forced 1:6 migration. FET/ASI -94%. Bittensor insider dump controversy.
- Cursor: ~$2B ARR, in talks at $50B (a16z + Thrive). Claude Code: ~$2.5B annualized run rate.

**The a16z target:**
- **Casado** (infrastructure fund): Portfolio includes Cursor, Braintrust, Convex, Kong, Netlify, Fivetran. Key shift: April 2025 skeptical → March 2026 led $43M Deeptune Series A.
- **Aubakirova** (warm intro): Big Ideas 2026 quote — "the bottleneck becomes coordination: routing, locking, state management, and policy enforcement." Co-authored "Et Tu, Agent?"
- **Dixon/Yahya** (crypto fund, ~$7B AUM): Led Catena Labs $18M, Story Protocol. April 16 a16z crypto post on 5 missing primitives: KYA identity, governance, x402 payments, trust pricing, user control.

**What the current landing page looks like** (I have screenshots):
The existing site at nunchi.network has 7 scroll sections with ROSEDUST dark aesthetic (near-black background, rose/pink accents, monospace typography):

1. **Hero**: "Observe. Predict. Compound." with floating diamond particles, orbital animation, two CTAs: "Open dashboard" / "Read the thesis". Nav: LOOP | SCAFFOLD | ANATOMY | MEMORY | COLLECTIVE | CHAIN | PROOF.
2. **The Loop**: "Systems that get better at getting better." Shows the 6-phase cognitive loop (observe, gate, assemble, reflect, consolidate) with an interactive diagram. Gate detail pane shows "Ready by surprise" — routing/model info. Three mock counters at bottom: 84,213 / 12,425 / 3,240 (these are placeholder/mock data, not real).
3. **The Scaffold**: "Every cycle makes the next one smarter." → "The model is the same. The system is the variable." Split terminal comparison: left shows conventional agent (every invocation starts from zero), right shows Roko (prior experience compiles into reusable knowledge). Cost comparison at bottom: LEFT $0.025 vs RIGHT $0.009. Below: quotes about scaffold being the product and network knowledge compounding. A performance curve graph showing improvement over sessions.
4. **Anatomy**: "Twelve organs. Five zones. One specimen." Shows cognitive architecture with tabs: PERCEPTION, EFFECT, AFFECT, MEMORY, LINK, EIO, DAIMON, IDENTITY. Visual: organ-like spheres of different sizes. Detail pane shows "Cognitive gate" with routing description.
5. **Memory**: "A pattern, not a word." HDC visualization: 10,240-bit vector noise pattern, similarity bars. Shows "CrossDomainResonance: The field-wide knowledge transfer." Performance comparison bars.
6. **Collective**: "The thousandth agent joins smarter than the first." Particle field visualization. Agent count slider (100 agents). C-factor collective intelligence curve.
7. **Chain**: "A library, not a ledger." Shows block visualization with knowledge deposits. Stats: 3.0 / 17 / 2928sec. Transaction list.
8. **Proof**: "Run it, then run it again." Cold run terminal showing per-line costs ($0.012 cache miss, $0.011 miss, $0.009 retry, etc.). Chain mint panel (no deposits during cold run). "RUN WARM" button. Loop phase diagram (Cold → stages → awaiting warm).
9. **CTA**: "The next agent to join inherits everything the last one learned." Three buttons: Open dashboard / Read the paper / GitHub. Footer: NUNCHI · ROKO © 2026 · v0.177K · 2026-04-26.

**What's WRONG with the current landing page for investor purposes:**
- "Observe. Predict. Compound." is a tagline — not a value proposition. An investor who lands here doesn't know what this product DOES in the first 3 seconds.
- The mock data (84,213 / 12,425 / 3,240) is fake. An investor who discovers this loses trust immediately.
- No cost comparison using real benchmark data (HAL). The terminal shows $0.012/$0.009 — pennies, not the dramatic $44.86 → $1.42 contrast.
- "Twelve organs. Five zones. One specimen." is poetic but inscrutable to someone who hasn't read the docs.
- No mention of EU AI Act, NHI market, or compliance anywhere on the page.
- No mention of Series A, team, or business model (expected for an investor-facing page variant).
- The "Read the paper" CTA links to nothing yet.
- Block #160,168 in the nav is mock chain data.

## The 13 Slides

For EACH slide, provide:
- **Headline**: The exact words in large type (max 8 words)
- **Subtext**: 1-3 sentences below the headline
- **Key number(s)**: The specific data point(s) to display prominently
- **Visual description**: What the visual should be (chart, code, animation, diagram)
- **Speaker notes**: What to SAY while this slide is up (the verbal narrative, 30-60 seconds)
- **Source citations**: Where each number comes from

### Slide 1: HOOK
"The model is the same. The system is the variable."
Show the $44.86 → $1.42 cost animation (same task, same model access, different system). This is the first thing they see. It must stop them scrolling their phone.

Research: what is the most effective opening slide format for infrastructure pitches? Is a number more effective than a tagline? Should the $44.86 → $1.42 be the HEADLINE instead of the tagline? Look at how Stripe's first pitch deck opened (2010), how Temporal's opened, how Cloudflare's opened. What did the highest-converting Series A decks lead with?

### Slide 2: PROBLEM
MAST data — 41-86% failure rate, 79% from coordination.
One chart. Not paragraphs.

Research: what chart type is most effective for "scary failure rate" data in pitch decks? Bar chart? Waterfall? Single giant number? Look at how cybersecurity companies present breach statistics. Is "79% from coordination, not capability" more powerful as a pie chart or as a headline?

Also include: Princeton HAL finding "agents can be 100x more expensive while only being 1% better." And: Gartner ">40% of agentic AI projects will be cancelled by end of 2027." And: "Only 11-14% of enterprise agent pilots reach production."

### Slide 3: SOLUTION
The solution shown as a 4-line SDK code snippet, not an architecture diagram. The insight from SWE-agent's ACI paper (arXiv:2405.15793): interface design matters as much as model capability. Show what it looks like to USE Nunchi, not what Nunchi IS architecturally.

Research: what is the equivalent of Stripe's 7-line curl example for agent coordination? What does `nunchi run` look like? Draft the most compelling 4-line code snippet that shows: (1) create agent, (2) run task, (3) observe cost, (4) see knowledge deposited. The code must be real enough to copy-paste. Consider both Python and TypeScript versions.

Also: reference the native 6-stage harness (OBSERVE → GATE → ASSEMBLE → INFER+TOOLS → REFLECT → CONSOLIDATE) but as a SUBTITLE, not the main visual.

### Slide 4: COST PROOF
HAL benchmark numbers. 30x breakdown: 5x cache × 3x routing × 2x gating. The fraud-prevention commitment: "All raw data published. Third-party reproducible. Full task IDs, model versions, and cost breakdowns."

Research: how do the best infrastructure companies present cost comparisons? Is a waterfall chart (showing each multiplier stacking) more effective than a bar chart? Look at how AWS presents cost savings, how Snowflake presents price/performance, how Temporal's Forrester TEI study (201% ROI, 14-month payback) was structured.

Include the specific demo numbers: naive baseline ~$44.86/task (HAL published for SWE-Agent + Opus) vs Nunchi-optimized ~$1.42/task. Note: "These are median costs across 100 SWE-bench Verified tasks. P25-P75 range: $0.80-$2.10 for Nunchi vs $28-$59 for naive."

### Slide 5: THE CHAIN
Sovereign EVM L1. One number per line, no paragraphs:
- 50ms blocks (Simplex consensus, Tokyo co-location)
- ~400 gas HDC similarity search (20-100x cheaper than Solidity)
- <1s ZK-HDC proving time (Circom + Groth16)
- 7-domain EMA reputation with slashing
- On-chain knowledge substrate with demurrage pruning

Research: how do crypto-infrastructure pitch decks present chain specs? Look at how Story Protocol, Monad, MegaETH, and Berachain presented their L1 specs to a16z crypto. Is a "stat stack" (one number per line) more effective than a diagram? Should this slide include a comparison to Ethereum mainnet gas costs?

The key insight to convey: "This is not a general-purpose chain. It exists for one reason: to make agent coordination verifiable, reputation portable, and knowledge compounding."

### Slide 6: IDENTITY
Reframe around NHI market. Headline should convey: every agent needs an identity, and the identity must carry reputation.

Key numbers:
- 82:1 machine-to-human identity ratio (CyberArk 2025)
- 144:1 in financial services (Entro 2025)
- NHI market: $9.45B → $18.71B by 2030
- Saviynt: $700M at $3B. CyberArk: acquired for $25B.

"The question is whether an agent's identity is a static API key or a reputation-bearing, ZK-verifiable credential."

Research: how do identity companies (Okta, Auth0, WorkOS) frame the identity problem in their pitch decks? Is "82:1 machine-to-human ratio" more powerful as a headline number or as supporting data? Look at how CyberArk's annual Identity Security Landscape report presents the NHI explosion.

### Slide 7: MARKET
Lead with NHI ($18.7B addressable by 2030), NOT the theoretical $230B TAM. Investors have TAM fatigue. Add the compliance market.

Research: what is the optimal market sizing slide format? "Concentric circles" (TAM/SAM/SOM) or "bottom-up from unit economics"? The strongest pitches use bottom-up. Calculate: if there are X agents registered with ERC-8004 identities paying $Y/year for verified identity + reputation, the market is $Z. What are realistic values for X and Y?

Include:
- NHI market: $9.45B → $18.71B by 2030 (11.9% CAGR)
- Compliance catalyst: Vanta ~$100M+ ARR from SOC 2 alone. OneTrust $5B+ from GDPR.
- EU AI Act Article 50 creates forced demand starting August 2, 2026.
- Agent infrastructure proven investable: Temporal $5B, Braintrust $800M.

### Slide 8: COMPETITION
Field table from competitive intelligence doc. But ADD two columns where competitors score HIGHER to avoid appearing rigged:
- "Production Users" — Olas has 400 daily active agents. Temporal has thousands of enterprise customers. Nunchi has zero production users today.
- "Dev Community" — LangGraph has 90M monthly downloads. MCP has 97M. Nunchi is pre-launch.

This honesty is strategic. Casado will respect it. An investor who sees a competitive table where you win every column assumes the columns were chosen to produce that result.

Research: what is the optimal competitive positioning slide? 2x2 matrix? Feature table? "Only one in the quadrant" diagram? Look at how Gartner Magic Quadrant positioning works and whether startups should mimic or subvert it. How did Snowflake position against Redshift and BigQuery?

Nava ($8.3M, April 14) validates the thesis. Include them prominently — a funded competitor HELPS at this stage.

### Slide 9: BUSINESS MODEL
Three revenue streams: managed cloud (near-term), chain economics (post-mainnet), enterprise compliance (regulation-driven).

- Runtime: Apache 2.0 (maximum adoption, embeds everywhere)
- Cloud: BSL with 4-year Apache conversion (revenue protection)
- Token: Helium-hybrid burn-and-mint, deferred 18-24 months post-Series A
- Structure: NunchiLabs Inc. (Delaware C-corp) + Nunchi Foundation (Cayman/Swiss)
- Template: Story Protocol / PIP Labs dual-asset structure

Research: how do the best dual-asset (equity + token) pitch decks present the business model without triggering the "crypto company" pattern-match? Look at how Story Protocol, Helium/Nova Labs, and Worldcoin/Tools for Humanity presented their dual structures. What words did they use and avoid?

The Token Graveyard data goes in an APPENDIX slide (not in the main 13): VIRTUAL -86%, ELIZAOS -99.98%, FET -94%, Bittensor insider dump. Title: "Why we defer the token." This is the "we learned from their mistakes" slide. Research: is it better to have this in the appendix (pulled out if asked) or proactively in the main deck?

### Slide 10: EU AI ACT / COMPLIANCE
Countdown to August 2, 2026. This is the urgency slide.

- 35.7% of EU managers feel prepared (Deloitte, n=500)
- Only 26.2% have started concrete compliance activities
- Penalties: €35M or 7% global turnover
- "Vanta built a unicorn on SOC 2. OneTrust built $5B on GDPR. Article 50 is next."

Research: what is the optimal way to present a regulatory deadline as a revenue catalyst? Is a countdown timer effective in a pitch deck, or is it gimmicky? How did Vanta's pitch deck frame SOC 2 compliance as a growth driver? How did OneTrust frame GDPR?

Map each Article 50 requirement to a Nunchi primitive:
- Article 50(1): disclosure of AI nature → ERC-8004 identity
- Article 50(2): machine-readable marking of AI outputs → ZK-HDC attestation
- Article 50(4): deepfake disclosure → not directly applicable but adjacent

### Slide 11: TEAM
[Leave blank — will be filled in separately]

Research: for a solo/early-stage founder, what is the optimal team slide format? Should it focus on the founder's background, the first hires planned, or advisor commitments? How did solo founders like Patrick Collison (Stripe), Guillermo Rauch (Vercel), and Paul Copplestone (Supabase) present team slides at their Series A when the team was tiny?

### Slide 12: GO-TO-MARKET
MCP playbook: spec + 2 SDKs (Python + TypeScript) + 5 reference MCP servers + 5 anchor partners on day one.

Design partners (priority order): Cleric (multi-agent SRE), Decagon ($4.5B, Agent Operating Procedures), Harvey ($8B, legal AI), Hebbia ($700M, document intelligence), Resolve.ai ($1B, SRE).

Forward-deployed engineering model: ~$1M/year for 4 FDEs. Palantir-to-Sierra-to-Harvey pattern.

Distribution: get scaffolded into AI coding tools (Cursor, Claude Code, Lovable, Bolt). "Supabase in 55% of YC" mechanism. MCP server as distribution wedge.

Research: what is the most compelling GTM slide format? Is "logos of design partners" more effective than "the playbook"? How did LangChain, Temporal, and Supabase present their GTM at Series A?

### Slide 13: ASK
$20-30M at $200-400M. "The thousandth agent joins smarter than the first."

Research: what is the optimal ask slide? Should it include use-of-funds breakdown? Milestone targets? How did the highest-valued Series A pitches frame their ask? Should the closing line be the thesis ("The model is the same. The system is the variable.") or the network effect ("The thousandth agent joins smarter than the first.")?

Include the Series B milestone: "$3-5M ARR with 4+ marquee logos by month 12."

## Additional Research for the Deck

### Visual Language
The current landing page uses ROSEDUST — near-black backgrounds with rose/pink accents, monospace typography, particle animations, orbital diagrams. Should the pitch deck match this aesthetic, or should it be more conventional for a VC meeting? Research: do VCs respond better to "looks like every other pitch deck" (Sequoia template) or "the design IS part of the pitch" (Linear's website-as-deck)?

### The Landing Page as Pitch Deck
The current landing page (7 sections: Loop, Scaffold, Anatomy, Memory, Collective, Chain, Proof) was designed as a pitch deck you scroll through. For the actual 13-slide investor deck: should any landing page sections be reused directly? Or should the deck be completely independent?

Current landing page weaknesses for investor context:
- "Observe. Predict. Compound." is a tagline, not a value proposition
- Mock data (84,213 / 12,425 / 3,240 counters) is fake — credibility risk
- No cost comparison using real HAL benchmark data
- "Twelve organs. Five zones. One specimen." is poetic but inscrutable
- No mention of EU AI Act, NHI market, compliance, fundraise, or business model
- Terminal costs show pennies ($0.012), not the dramatic $44.86 → $1.42 contrast

### What to ALSO draft while doing the deck

1. **The Aubakirova cold email memo** (one page, under 500 words): Opens with her Big Ideas 2026 quote. Shows Nunchi is what she described. MAST data validates her thesis. Cost proof. NHI market. Braintrust validates Casado's interest in the space. Close: "30 minutes with Martin to show the demo."

2. **The "why now" one-pager**: For the data room. Three converging forces: (a) standards crystallizing (MCP 97M/mo + A2A 150+ orgs + ERC-8004), (b) compliance forcing function (EU AI Act Aug 2), (c) cost-reduction wedge proven (HAL benchmark). Window: 6-12 months before lock-in.

3. **The appendix slides** (pulled out if asked):
   - Token Graveyard: why we defer the token 18-24 months
   - Technical deep dive: HDC, ZK-HDC, Simplex consensus
   - Competitive deep dive: Nava, Tempo, 0G, Olas, RNWY, Chitin.id
   - Team expansion plan: first 10 hires
   - Cost-reduction benchmark methodology and fraud prevention

## Output Format

For each slide:
```
## Slide N: [TITLE]

**Headline**: [exact words, max 8 words]
**Subtext**: [1-3 sentences]
**Key number(s)**: [the specific data to display]
**Visual**: [description of chart/diagram/code/animation]
**Speaker notes**: [what to SAY, 30-60 seconds]
**Sources**: [citation for every number]
**Landing page equivalent**: [which current landing page section maps to this, if any]
**What changes on the landing page**: [what should update to align with the deck]
```

For the additional materials (memo, one-pager, appendix), provide complete drafts.

## Priority

1. The 13 slides with exact words (this is the deliverable)
2. The Aubakirova memo (most time-sensitive outreach artifact)
3. Landing page change recommendations (what to update to align deck ↔ site)
4. The appendix slides
5. The "why now" one-pager
