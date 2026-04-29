# Deep Research: Category Definition for an Agent Infrastructure Startup

Copy everything below the `---` line into Claude Desktop with deep research enabled.

---

## Who you are helping

You are a research analyst helping a founder define the **market category** for a startup called **Nunchi**. The founder is preparing a Series A pitch for **Martin Casado** at **a16z** (Andreessen Horowitz), targeting **$20-30M at $200-400M post-money**. The warm intro is through **Malika Aubakirova**, an a16z partner who wrote in her "Big Ideas 2026" essay: *"the bottleneck becomes coordination: routing, locking, state management, and policy enforcement across massive parallel execution."*

Category definition is the single most important strategic decision before the pitch. The category name determines which budget the buyer uses, which analysts cover you, which competitors you're compared to, and whether investors see a $100M outcome or a $10B outcome. Get this wrong and everything downstream — deck, landing page, GTM, pricing — is misaligned.

## What Nunchi is (read this carefully — you need all of it)

Nunchi is a two-part system for agent coordination, identity, and trust:

**Part 1 — Roko (open-source Rust runtime)**
An agent orchestration runtime. 18 Rust crates, ~177,000 lines of code. Apache 2.0 licensed. It does the following:
- Agents run through a 6-stage pipeline: OBSERVE → GATE → ASSEMBLE → INFER+TOOLS → REFLECT → CONSOLIDATE
- A model routing system (CascadeRouter) learns which LLM to use for each task type, producing 10-30x cost reduction vs. naive agent execution
- An 11-gate verification pipeline checks agent output at 7 rungs (compile, test, lint, diff, LLM review, etc.) — language-agnostic, not tied to any specific programming language
- A knowledge store accumulates what agents learn, with decay (old knowledge fades unless it's confirmed useful)
- Agents share knowledge through a common event bus and can query each other's accumulated insights
- The system self-hosts: Roko reads its own PRDs, generates plans, dispatches Claude agents, validates with gates, and persists results

**Part 2 — Nunchi Chain (sovereign EVM blockchain)**
A purpose-built blockchain for agent identity and knowledge sharing:
- Sovereign EVM L1 (not a layer-2 or layer-3). Simplex consensus. ~50ms block times via co-located validators in Tokyo (same architecture as Hyperliquid, the derivatives exchange)
- A native hardware-accelerated precompile for Hyperdimensional Computing (HDC) similarity search at ~400 gas — this is 20-100x cheaper than doing the same computation in Solidity on Ethereum
- ERC-8004 agent identities with 7-domain reputation (coding, security, research, chain, knowledge, operations, strategy). Reputation decays over time if not refreshed by real work.
- An on-chain knowledge substrate where agents publish what they've learned. Other agents can query this shared knowledge to get smarter. Demurrage (decay) prunes stale entries automatically.
- Zero-knowledge proofs over HDC vectors (ZK-HDC): an agent can prove "my capability fingerprint is within distance D of reference vector R" without revealing its full fingerprint. Circom + Groth16, under 1 second proving time.
- A generalized job market (ERC-8183) where agents can post, bid on, and complete tasks with reputation-weighted matching

**Part 3 — ISFR and Cooperative Clearing (the beachhead application)**
The chain's most powerful feature is clearing-as-inference: every trade produces structured intelligence.

- **ISFR (Internet Secured Funding Rate)**: The first credible on-chain benchmark rate. Composite of 4 DeFi source classes (lending 60%, structured 25%, funding 10%, staking 5%). Validator-computed every 10 seconds (not oracle-dependent). Manipulation-resistant: tolerates 49% corrupted weight.
- **Yield perpetuals**: Perpetual futures on DeFi yield rates, settled against ISFR. No expiration, no rollover, single pool. Up to 10x leverage. "Clearing profiles" let users declare intent once; agents handle everything else.
- **Cooperative clearing**: Batch auctions solved via convex optimization with KKT optimality certificates verified on-chain in O(n). Every clearing round requires prediction commitments scored by CRPS (strictly proper — truthful reporting is uniquely optimal). Each round emits a ClearingInsight to the InsightStore.
- **Market gap**: $668T in TradFi interest rate derivatives (BIS) vs <$100M on-chain. Over 1,000,000:1 ratio. $49.5B in DeFi lending TVL with unhedged variable rate exposure. Pendle proved demand ($13.4B peak TVL) but uses expiring instruments.
- **The flywheel**: Agents produce knowledge → knowledge improves agents → better agents attract users → more users create more knowledge. Clearing is the mechanism that converts economic activity into structured intelligence. Each round makes the next one more valuable.
- **Four moats**: Knowledge (millions of scored observations), calibration (epistemic reputation compounds), benchmark (natural monopoly — LIBOR→SOFR transition: $250T, 5 years), and NOT code (fork code in hours, fork InsightStore never).

**The current positioning has two versions** (which may need unifying — that's what this research is for):

**Version A (infra investors like Casado):** "Nunchi is the identity, reputation, and verifiable-similarity layer for the agent economy." Cost reduction is the wedge. Trust/identity/knowledge is the moat.

**Version B (crypto investors like Dixon/Yahya):** "ISFR is DeFi's SOFR moment — the first credible on-chain benchmark rate. Yield perpetuals are the instrument. Clearing-as-inference means every trade produces knowledge that makes the next trade better."

**The thesis statement** (on the landing page):
"The model is the same. The system is the variable."

**The network effect claim**:
"The thousandth agent joins smarter than the first." (Because shared knowledge compounds.)

**Alternative closing from the litepaper:**
"The model is commoditizing. The knowledge is not. We are building the network that compounds it."

**Key data points**:
- 41-86% of multi-agent deployments fail; 79% of failures are from coordination, not model capability (MAST taxonomy, Berkeley, NeurIPS 2025, arXiv:2503.13657)
- Princeton HAL benchmark: naive agent costs $44.86/task; optimized system brings it to ~$1.42/task (ICLR 2026, arXiv:2510.11977). HAL costs exclude caching — real production cost is ~20-25% of HAL listed price.
- Non-Human Identity (NHI) market: $9.45B (2024) → $18.71B by 2030 at 11.9% CAGR. Machine-to-human identity ratio: 82:1 globally (CyberArk 2025), 144:1 in financial services (Entro 2025).
- EU AI Act Article 50 enforcement: August 2, 2026 (~14 weeks). Creates forced demand for agent identity/transparency infrastructure.
- MCP (Model Context Protocol): 97M monthly downloads. A2A (Agent-to-Agent): 150+ supporting organizations. Standards are crystallizing now.

**The competitive landscape** (so you know what names NOT to overlap with):
- **Nava** ($8.3M seed, April 14, 2026, Polychain + Archetype): "Arbiter" middleware for agent transaction verification. Arbitrum L3 + Tempo.
- **Capsule Security** ($7M seed, April 16, 2026): "runtime trust layer for agentic AI"
- **Temporal** ($300M at $5B, a16z-led, Feb 2026): durable execution for workflows. Not agent-specific but used by agent companies. Category: "durable execution."
- **Braintrust** ($800M valuation, Casado-led Series A): AI observability/evaluation. Category: "AI observability."
- **LangChain** ($25M at $200M, Sequoia): agent framework. Category: "agent framework."
- **0G Labs** ($359M): decentralized AI compute. Category: "decentralized AI."
- **Olas** ($13.8M): off-chain agent framework on Gnosis. Category: "autonomous agent services."
- **Saviynt** ($700M at $3B, KKR): identity security. Category: "identity governance."
- **t54 Labs** ($5M seed, Ripple): "trust layer for the agentic AI economy."

## What I need you to research

### Direction 1: How Category Kings Were Named

Research the actual naming history of successful technology categories. For EACH of the following, find:
- What the company/product was called at founding vs. at category dominance
- When exactly the category name crystallized (which year, which event, which analyst report)
- Who named it — was it the company, an analyst firm (Gartner, Forrester), the press, or the community?
- What alternative names were considered and rejected
- How long it took from founding to category-name lock-in

Categories to research:
1. **"Cloud computing"** — Who coined it? Was it Amazon, Google, Salesforce, or someone else? When did "cloud" beat "utility computing," "on-demand computing," and "grid computing"?
2. **"DevOps"** — Patrick Debois coined it at DevOpsDays 2009. But when did it become a Gartner category? When did the first "DevOps platform" company raise on the category name?
3. **"Data streaming" / "event streaming"** — Confluent's category. Jay Kreps wrote "The Log" in 2013. When did "data streaming" become a Gartner MQ? What was the category called before Confluent named it?
4. **"Durable execution"** — Temporal's category. Maxim Fateev coined it. When? How did it beat "workflow orchestration" and "workflow automation"? Has Gartner adopted it?
5. **"Infrastructure as Code"** — HashiCorp's category. When did it beat "configuration management" (Puppet/Chef)?
6. **"Developer Experience" / "DX platform"** — Vercel's positioning. Is this a real category or marketing?
7. **"Identity Security" / "Identity Governance"** — When did this split from "IAM"? Who drove the split? (CyberArk? SailPoint? Saviynt?)
8. **"AI Observability"** — Braintrust/Arize/Langfuse. When did this become distinct from "ML monitoring"? Is it a Gartner category yet?

### Direction 2: Category Name Candidates for Nunchi

For each candidate category name, evaluate:
- Does it pass the "would an analyst create a Magic Quadrant for this?" test?
- Does it have a natural buyer (what title at what company would have budget for this)?
- Does it position Nunchi as the king, or as a resident in someone else's category?
- Does it create a $10B+ market or constrain to a niche?
- Does it resonate with Casado specifically (based on his public writing/investing)?

Candidate names to evaluate:

1. **"Agent Trust Infrastructure"** — Pros: trust is the moat, not the wedge. Cons: "trust" is vague; what does the buyer actually buy?
2. **"Non-Human Identity" (NHI)** — Pros: $18.7B market, Saviynt/$3B validates. Cons: NHI is a security category owned by CyberArk/Saviynt; Nunchi would be fighting incumbents.
3. **"Agent Coordination Protocol"** — Pros: technically precise. Cons: "protocol" sounds like a standard, not a product; hard to monetize a protocol.
4. **"Verifiable Agent Infrastructure"** — Pros: ZK-HDC is genuine IP. Cons: "verifiable" is a crypto word that triggers pattern-matching.
5. **"Agent Operating System"** — Pros: massive framing. Cons: overused (every agent company claims this); Microsoft literally ships one.
6. **"Agent Identity and Reputation"** — Pros: specific, testable. Cons: narrow; misses the orchestration/cost-reduction story.
7. **"Compound AI Infrastructure"** — Pros: builds on Berkeley's "Compound AI Systems" thesis (Zaharia et al. 2024) which Casado knows. Cons: vague; doesn't differentiate from LangChain.
8. **"Agent Compliance Infrastructure"** — Pros: regulation creates the buyer (AI Governance Lead). Cons: compliance companies get compliance multiples (3-5x revenue), not infrastructure multiples (15-25x).
9. **"Cognitive Infrastructure"** — Pros: nods to cognitive science (ACT-R, CoALA, Global Workspace Theory) backing the architecture. Cons: too academic for a buyer.
10. **"Agent-Native Financial Infrastructure"** — Pros: encompasses both runtime and chain, nods to the ISFR/yield-perps beachhead. Cons: "financial infrastructure" may limit perceived TAM to DeFi.
11. **"Cognitive Clearing Infrastructure"** — Pros: captures the clearing-as-inference mechanism (every trade produces knowledge). Cons: "clearing" is niche jargon.
12. **"Intelligence Network"** — Pros: captures the compounding knowledge flywheel. Cons: extremely vague; could mean anything.
13. **"Agent Knowledge Network"** — Pros: captures the core moat (shared knowledge that compounds). Cons: doesn't convey the financial/clearing angle.
14. **Something entirely different** — Maybe the right name hasn't been tried. What would Play Bigger's framework produce? What would a Sequoia scout call this? Consider that the strongest framing might not be about agents at all — it might be about knowledge or intelligence or trust.

### Direction 3: The Play Bigger Framework Applied

Al Ramadan, Dave Peterson, Christopher Lochhead, and Kevin Maney published "Play Bigger: How Pirates, Dreamers, and Innovators Create and Dominate Markets" (2016). It remains the canonical text on category design.

Research:
- What is Play Bigger's step-by-step process for naming a category? (The "Category Design" methodology)
- What are their criteria for a good category name vs. a bad one?
- What examples do they cite of companies that got the name wrong initially and had to rename?
- How does the "Lightning Strike" concept work? (A coordinated launch event that establishes the category.)
- What's the "6-10 Rule" — the claim that it takes 6-10 years for a category to mature?
- Has anyone applied the Play Bigger framework specifically to crypto/blockchain infrastructure? To agent/AI infrastructure?
- Christopher Lochhead has a podcast ("Lochhead on Marketing"). Has he said anything about AI agent infrastructure categories specifically in 2025-2026?

### Direction 4: What Casado Specifically Responds To

Martin Casado is the target investor. His category-thinking is shaped by his own experience:

- He created the "Software-Defined Networking" (SDN) category with Nicira (acquired by VMware for $1.26B in 2012). Research: how did he name SDN? When did the term crystallize? Did he coin it or did someone else?
- He then invested in companies that created or redefined categories at a16z. Research his portfolio for category-creation patterns: Databricks ("lakehouse"), Anyscale ("distributed computing for AI"), Braintrust ("AI observability"), Cursor (what category?).
- His March 2026 $43M Deeptune Series A — what category did Deeptune claim? How was it framed?
- His published writing on a16z.com about infrastructure investing — what framework does he use to evaluate new categories?
- His April 2025 skepticism ("I don't see a lot of evidence we can close the control loop") about agent coordination — what would overcome this skepticism? What evidence would he need?

### Direction 5: Aubakirova's Thesis as Category Anchor

Malika Aubakirova wrote: "the bottleneck becomes coordination: routing, locking, state management, and policy enforcement across massive parallel execution."

- Does this quote map to an existing Gartner/Forrester category, or is it describing something that doesn't have a name yet?
- She also co-authored "Et Tu, Agent? Did You Install the Backdoor?" — what category framing does that paper use for agent security?
- If we name the category using HER vocabulary ("coordination infrastructure for parallel agent execution"?), does that create a stronger warm-intro path?
- Research: has any VC partner ever publicly named a category that their portfolio company then adopted? Is there precedent for "investor names the category"?

### Direction 6: Category Economics — What Determines the Multiple?

Different categories get different revenue multiples. This matters enormously for whether the pitch produces a $200M or a $2B valuation.

Research:
- What are the current (2025-2026) revenue multiples for: identity security companies, compliance platforms, developer tools, agent infrastructure, blockchain L1s, open-source infrastructure?
- Specifically: Saviynt ($3B at what ARR?), Vanta ($4B at what ARR?), Temporal ($5B at what ARR?), Braintrust ($800M at what ARR?)
- What determines whether a company gets a "platform" multiple (15-25x) vs. a "tool" multiple (5-10x) vs. a "compliance" multiple (3-5x)?
- Is there data on how the category name itself affects the multiple? (e.g., "AI infrastructure" gets X multiple vs. "developer tools" gets Y)

### Direction 7: The "Stripe for X" Trap and Alternatives

"Stripe for agents" was the original positioning. Stripe literally built their own agent commerce protocol (ACP) and co-founded the x402 Foundation (April 2, 2026). The positioning was retired.

Research:
- What other companies fell into the "X for Y" positioning trap? ("Uber for Z" companies, "Airbnb for Z" companies)
- What happens when the company you're analogizing to enters your market?
- What are the alternatives to analogy-based positioning? (Category creation from scratch, technical-thesis positioning, buyer-problem positioning)
- Specifically: when Stripe built ACP, how did that affect companies that had positioned as "Stripe for agents"?
- Is there a canonical case of a startup that SUCCESSFULLY repositioned after their analogy company entered the market?

### Direction 8: The Naming Test Battery

For whichever category name emerges as the recommendation, run it through these tests:

1. **The Gartner test**: Would an analyst create a Magic Quadrant with this name? Would companies pay $30K for inclusion?
2. **The budget test**: Which line item in a Fortune 500 company's budget does this come from? Who signs the PO?
3. **The cocktail party test**: Can a non-technical person understand what the category is in one sentence?
4. **The search test**: What happens when you Google the category name today? Is the namespace clean or cluttered?
5. **The competitor test**: Would Nava, Temporal, Braintrust, or LangChain want to be in this category? (If yes, you haven't differentiated enough. If no, you've created white space.)
6. **The Casado test**: Based on his public writing and portfolio, would he pattern-match this to a category he's seen succeed?
7. **The 10-year test**: Will this category name still make sense in 2036? Or is it tied to a transient technology?
8. **The headline test**: "Nunchi raises $25M to build [category name]" — does that headline make a reporter want to write the story?

## Output Format

### 1. Category History Analysis
For each of the 8 categories researched in Direction 1, provide: founding date → category name crystallization date → who named it → what was rejected → time to lock-in.

### 2. Candidate Evaluation Matrix
A table with all 10+ candidate names scored 1-10 on: Gartner test, budget test, cocktail party test, search test, competitor test, Casado test, 10-year test, headline test. Plus a weighted overall score.

### 3. The Recommendation
One category name with:
- The exact name (max 4 words)
- The one-sentence definition
- The buyer persona it creates (title, company size, budget source)
- The Gartner MQ it would anchor
- The 3 companies that would be in the MQ alongside Nunchi
- Why this name beats the other candidates
- The risk if this name is wrong

### 4. The Lightning Strike Plan
If we adopt the recommended category name:
- What artifact do we publish to claim it? (Blog post? Whitepaper? Open-source spec?)
- When do we publish relative to the Series A?
- Who do we get to endorse it? (Analysts? Partners? Academic collaborators?)
- What conference talk title would establish it?

### 5. The Dual-Narrative Resolution
Nunchi has two pitch versions: generic "agent trust infrastructure" (for infra investors) and specific "DeFi's SOFR moment via ISFR + yield perps" (for crypto investors). Research:
- Are there precedents of companies that successfully ran dual narratives for different investor types? (Helium pitched telecom to generalists, crypto to token investors. Story Protocol pitched IP law to generalists, tokenized IP to crypto.)
- Is the dual narrative a strength (reaches both pools) or a weakness (neither pool fully buys in)?
- For a16z specifically, which narrative resonates with the infra fund (Casado) vs. the crypto fund (Dixon/Yahya)? Can one meeting serve both, or do you need separate pitches?
- What is the UNIFIED category name that encompasses both narratives? "Agent-native financial infrastructure"? "Cognitive clearing infrastructure"? Something else?
- Research: does the ISFR/yield-perps beachhead HELP or HURT with Casado? He's infra, not crypto. But Hyperliquid ($40B+ market cap) proved derivatives infrastructure can be massive. Does the SOFR analogy ($668T market) make this feel bigger than "agent trust"?

### 6. Benchmark Rate as Category Anchor
ISFR could be the category-defining artifact — like SOFR defined a market. Research:
- How did SOFR itself get established? Who at the NY Fed drove it? What was the ARRC's (Alternative Reference Rates Committee) role? How long from proposal to adoption?
- Is "benchmark rate infrastructure" a category? Who would be in it? (Bloomberg, Refinitiv/LSEG, ICE Benchmark Administration?)
- What makes benchmark rates natural monopolies? Research the economics of benchmark lock-in.
- For the pitch: is "we're building DeFi's SOFR" a stronger opening line than "we're building agent trust infrastructure"?
- Research: Pendle ($13.4B peak TVL, $47.8B trading volume 2025). What category do they claim? How do they position? What's their revenue? Their raise history?

### 7. The Casado Pitch Line
The exact sentence to say in the first 30 seconds of the meeting that frames the category for Casado. Based on his SDN background, his portfolio, and his known skepticism about agent coordination. Now consider TWO versions:
- Version A (if leading with infra narrative): connects to his OpenFlow/Nicira experience
- Version B (if leading with ISFR/clearing narrative): connects to his infrastructure-category-creation experience

### 8. Full Citations
Every source, with URLs, dates, and author names. Distinguish between primary sources (the founder said it), analyst reports (Gartner published it), and secondary sources (a blog interpreted it).
