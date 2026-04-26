# Nunchi Pitch Strategy — May 6 Briefing
> Generated: April 26, 2026 | Research rounds R1–R15 + external market data
> Status: Pre-Series A, Casado (a16z) meeting

---

## Context Corrections (April 26)

**Team:** 7 people. Seed round closed. Not solo founder.
**Traction:** Zero external traction. No signed LOIs, no design partners, no revenue. Roko is functional and used daily to develop itself. Pitch framing must be "functional product + cost benchmark + design partner pipeline" — never use the word "traction," use "validation."
**Demo:** Asciinema recording of the real self-hosting loop (not a bespoke demo binary). Beat 4 (knowledge-compounding) and cost waterfall are non-negotiable beats.
**30x number:** Benchmark not yet run. HAL methodology ($44.86 naive / ~$1.42 optimized) is the defensible anchor. Run this week. Use whatever number it actually produces.
**Chain / crypto:** Mentioned exactly once on architecture slide as "optional verification layer." No token language. No blockchain language. Casado-only pitch for May 6.

---

## New Market Data Not in Current Docs

### Why Now — Fresh Numbers (April 2026)

| Signal | Number | Source |
|---|---|---|
| AI startup funding Q1 2026 | $47B, up 156% from Q1 2025's $18.3B | AI Startup Funding Landscape, Apr 2026 |
| Enterprise multi-agent inquiries | 1,445% surge, Q1 2024 → Q2 2025 | Gartner via LinkedIn, Feb 2026 |
| Google Cloud agentic partner fund | $750M committed, April 22, 2026 | Google Cloud Next, PR Newswire |
| Global 2000 running agents beyond pilot | 72% as of March 2026 | OpenClaw Enterprise Adoption Report |
| AI initiative cancellation rate | >50% by 2028 (Gartner) due to identity/trust failures | Gartner, 2026 |
| No universal agent registry | Confirmed gap — Gartner, 2026 | Gartner market analysis |
| EU AI Act enforcement date | August 2, 2026 — 14 weeks | EU Official |
| EU enterprises with concrete compliance activity | Only 26.2% (Deloitte, Q1 2026) | Deloitte survey |
| AI Act compliance price premiums | 20–30% vendor price uplift already happening | Raconteur, Apr 2026 |

### Competitive Landscape Update — Orkes (CRITICAL GAP)

**Orkes raised $60M Series B on April 23, 2026** — 3 days ago. Not in any current briefing docs. This will come up if Casado or any a16z analyst has done recent deal flow work.

**What Orkes is:**
- Built on Netflix's Conductor (2016), extended for AI agents
- 3,000+ enterprise customers: JP Morgan, Tesla, Atlassian, American Express
- 24,000 GitHub stars, 10,000+ developers
- Positioning: *"the missing orchestration layer"* for production AI workflows
- Series B led by AVP (not a16z — no portfolio conflict)
- Focus: single-org agent runtime, workflow automation, API integration, observability

**What Orkes does NOT do:**
- Cross-org coordination
- Cost-aware model routing
- Shared knowledge substrate
- ZK behavioral verification or reputation
- Anything outside a single organization's trust boundary

**The 30-second Orkes answer (memorize this):**
> "Orkes is doing what Temporal does at the workflow layer — making sure your orchestration completes reliably inside one organization. Their $60M round validates the durable-execution layer. But their own press release says they make AI 'reliable, observable, and governable' within a deployment — single-org. The coordination plane is what happens when agents from different organizations need to coordinate: shared memory, verified identity, policy enforcement across trust boundaries. Orkes is a likely customer, not a competitor."

**If pushed further:**
> "Orkes is the Netflix Conductor layer — it tells your agents what to do and confirms they did it. Nunchi answers: which agent should do it, do I trust the agent across org boundaries, what did it learn, and can I prove it to a regulator? Different questions, different layer."

### Comp Valuations — Updated

| Company | Round | Valuation | What They Build | Layer |
|---|---|---|---|---|
| Temporal | Series D, Feb 2026 | $5B | Durable execution, single-org | Execution |
| Orkes | Series B, Apr 2026 | ~$300M est. | Workflow orchestration, single-org | Execution |
| LangChain | Series B, Oct 2025 | $1.25B | Agent framework | Framework |
| Braintrust | Series B, Feb 2026 | ~$800M | AI observability/eval | Evaluation |
| Keycard | Series A, Oct 2025 | ~$200M est. | Agent identity/auth | Identity |
| **Nunchi** | **Series A target** | **Empty cell** | **Coordination plane** | **Coordination** |

> The empty cell is the pitch. Every adjacent layer is funded. The coordination plane between them is not.

**LangChain correction:** Docs previously cited $200M Series A. Updated: $1.25B Series B valuation (Fortune, Oct 2025; TechBuzz, Apr 2026). Strengthens the comp story — if the framework layer commands $1.25B, the coordination plane above it should command more.

### NHI Market Sizing — Updated Numbers

- NHI Access Management: **$10.71B in 2025 → $25.65B by 2033** at 11.53% CAGR (updated from $9.45B/$18.71B in prior docs)
- Machine-to-human identity ratio: **80:1+** driven primarily by embedded AI agents (Gartner)
- Non-human identity governance sector raised **$400M+ in 2025 alone**; named Gartner category
- CyberArk/Palo Alto acquisition: **$25B**, February 2026 — premium multiples confirmed for identity infrastructure

### Temporal Pricing — A New Wedge

Temporal Cloud has a documented **10–50x cost scaling problem** — bills 10–50x higher than initial estimates due to action multiplication. One migration case study: $2.25M/year on custom infra → $96K/year on Temporal Cloud. This is both a comp for Nunchi's per-action pricing and a signal that action-based billing without routing optimization bleeds enterprise customers. Roko's CascadeRouter is the direct answer to this problem.

---

## 13-Slide Deck Outline — Pre-Traction Build

> Core reframe: you are not selling momentum. You are selling **inevitability**.
> Never use the word "traction." Use "validation."

---

### Slide 1 — Title / Thesis

**Content:**
> *Nunchi — The Agent Coordination Plane*
> *A durable runtime for production agents, with verifiable coordination across organizations.*

One line. No bullets. No metrics. Team of 7 + contact in footer.

**Why:** Casado rewards descriptive structural names. "Agent Coordination Plane" is the category, not a feature claim. The thesis is implicit in the name.

---

### Slide 2 — Why Now

**Three tailwinds, each a hard number, each time-bounded:**

1. **The failure rate:** MAST taxonomy (Berkeley, NeurIPS 2025) — 1,642 production traces, failure rates 41–86%, 79% from coordination failures, not model failures.
2. **The demand explosion:** Gartner 1,445% surge in multi-agent inquiries (Q1 2024 → Q2 2025). Google Cloud $750M agentic partner fund, April 22, 2026. Enterprise intent confirmed.
3. **The enforcement clock:** EU AI Act Article 50 enforcement August 2, 2026 — 14 weeks. Only 26.2% of EU enterprises have begun concrete compliance activities.

**Design:** Three rows. Hard numbers. Dates. No decorative timeline widget. Specificity is credibility.

---

### Slide 3 — The Problem

**Headline:** *The model is not the variable. The system is.*

**Three bullets:**
- Princeton NLP: a single well-tooled agent matches or beats multi-agent ensembles on **64% of tasks**. Adding agents adds failure modes, not capability.
- Google DeepMind: beyond ~45% single-agent accuracy, adding agents **hurts performance** — coordination errors compound geometrically.
- Gartner: **>50% of AI initiatives will halt by 2028** — reason: inability to validate, audit, or trust agent behavior.

**Payoff line:** Klarna reversed its all-AI customer service strategy — not because the AI couldn't do the tasks, but because it couldn't demonstrate compliance, provenance, or accountability.

---

### Slide 4 — Team

**Put it here — second-highest-salience concern after market, addressed early.**

What this slide says:
- Founders (7 people). Lead founder name + one sentence on domain depth.
- What you've built: *177K lines of Rust, 18 crates, fully operational plan-execute-gate-persist loop — built and used daily to develop itself.*
- Seed round closed: [amount], [investors if nameable].
- Any named advisors: name + one-line credential.
- If no advisors yet: "Recruiting two technical advisors this month — [specific credential targets]."

**Design note:** Team slide before product slide signals confidence, not defense.

---

### Slide 5 — Architecture (The Casado Identity Test)

**This is the slide Casado is looking for.** His PhD thesis (SDN / Ethane) was about separating the control plane from the data plane. This slide must show the same band separation.

```
┌──────────────────────────────────────────────────┐
│           APPLICATION LAYER                       │
│  LangChain · CrewAI · Mastra · AutoGen · Orkes    │
├──────────────────────────────────────────────────┤
│         AGENT COORDINATION PLANE  ← Nunchi        │
│  Identity · Routing · Knowledge · Policy          │
├──────────────────────────────────────────────────┤
│           EXECUTION LAYER                         │
│  Temporal · Keycard · x402 · Model APIs           │
└──────────────────────────────────────────────────┘
```

**Caption (say this, don't print it):**
> "Ethane reduced networks to dumb forwarding governed by centralized policy. Nunchi does the same for LLM agents."

**Design:** Three horizontal bands. Nunchi in the middle band, bold. Everything else labels the context. Casado will recognize this diagram instantly — it is structurally identical to the SDN architecture he commercialized at Nicira.

**Chain mention:** If included at all, appears in small text under the middle band: "Optional: verifiable coordination layer." That's the only mention. No further elaboration unless asked.

---

### Slide 6 — How It Closes the Loop

**Four functions, four verbs:**
- **Observes** — what agents are doing (episode log, trace)
- **Decides** — which agent, which model, at what cost (CascadeRouter)
- **Enforces** — policy, gates, identity (gate pipeline)
- **Records** — verifiable audit chain (persisted, compliance-ready)

**The code diff goes here.** Side-by-side: boilerplate LangChain AgentExecutor (left) vs. `roko plan run` (right). Right side shows four observable primitives in the output:
- `Signal hit [30,000 tokens saved]`
- `Route → Haiku [task: file-read]`
- `Gate ✓ [tests pass]`
- `Episode persisted`

Four lines of observability the left pane has none of.

---

### Slide 7 — Let Me Show You

**One line:** *"The self-hosting loop — live."*

Demo pivot point. Run the asciinema or live CLI from here. The demo is Beat 1–4 (see below). If the demo fails, continue from Slide 8 — this slide has no content to defend.

**Required demo beats (non-negotiable):**
1. `roko plan` — reads the PRD, generates structured plan with token estimate
2. `roko run` — executes, CascadeRouter picks Haiku for file-read vs. Opus for reasoning
3. `roko gate` — validation step, shows early-exit on test pass
4. `roko persist` — knowledge signal deposited; second run shows "Signal hit [30K tokens saved]"

**Cost waterfall must be visible in Beat 2 output:**
```
Naive estimate:  $44.86
  Caching:       -78%  →  $9.89
  Routing:       -67%  →  $3.26
  Gate exit:     -56%  →  $1.42
Actual cost:     $X.XX  (benchmark result)
```

---

### Slide 8 — Validation (not "Traction")

**The honest pre-traction slide. The word "traction" never appears.**

Three rows:

| Validation Type | Evidence |
|---|---|
| **Technical** | Roko used daily to develop itself for [X months]. 177K LOC, 18 crates, plan-execute-gate-persist loop operational. Every PR is an agent run. |
| **Literature** | MAST (1,642 production traces, Berkeley / NeurIPS 2025). HAL benchmark (Princeton, ICLR 2026 — 21,730 rollouts). RouteLLM (85% cost reduction, 95% quality retained). These confirm Nunchi's architectural bets are correct. |
| **Commercial** | Every adjacent layer is funded — and the coordination plane is empty: Orkes $60M (Apr 2026), Temporal $300M at $5B (Feb 2026), LangChain $1.25B (Oct 2025). |

**Design note:** No logo wall — you don't have one yet. Use the funding comps table as the visual. Four rows: Temporal, Orkes, LangChain, then "Coordination Plane — [empty cell]." The empty cell is the pitch.

---

### Slide 9 — Cost Reduction (The Proof Slide)

**Headline:** *The same task. 20x cheaper. With a receipt.*

**Waterfall:**
```
Naive (HAL baseline):     $44.86/task  ←  published, reproducible
  × Prompt caching:        ÷5   →  $8.97
  × Model routing:         ÷3   →  $2.99
  × Gate early-exit:       ÷2   →  $1.49
Nunchi optimized:          ~$X.XX/task  ←  actual benchmark result
```

**Mandatory footnote (verbatim):**
> "HAL benchmark costs exclude prompt caching. With 80-90% production cache hit rates and Anthropic's 90% cache discount, HAL figures are an upper bound. Nunchi benchmark run: [date], 100 SWE-bench Verified tasks, full methodology at [url]."

**Why this footnote matters:** You are the only agent infrastructure pitch that discloses HAL's caching methodology limitation. Every other pitch either doesn't know or doesn't say. This footnote is what separates you from "another 30x claim."

**If benchmark isn't run before May 6:** Use the theoretical waterfall with HAL's published numbers as denominator. Mark result range as "estimated, benchmark in progress — full methodology published [week of X]." Never claim a specific number you haven't measured.

---

### Slide 10 — Market

**Lead with the specific. Not the aspirational.**

**Primary (the anchor — use this first):**
- NHI Access Management: **$10.71B in 2025 → $25.65B by 2033** at 11.53% CAGR
- Machine-to-human identity ratio: **80:1+** (Gartner)
- NHI governance companies raised **$400M+ in 2025 alone**; named Gartner category
- Palo Alto acquired CyberArk at **$25B** in Feb 2026 — premium multiples confirmed for identity infra

**Secondary:**
- AI agents market: $7.84B → $52B by 2030 (46–50% CAGR)
- 1,445% surge in enterprise multi-agent inquiries (Gartner, 2025→2026)

**What to cut:** The $230B theoretical TAM. Cut it entirely. Creates a credibility problem.

---

### Slide 11 — Competition

**Not a 2x2. A primitive comparison matrix.**

| Capability | Nunchi | Temporal | Orkes | Keycard | LangChain |
|---|---|---|---|---|---|
| Cross-org coordination | ✓ | ✗ | ✗ | ✗ | ✗ |
| Cost-aware model routing | ✓ | ✗ | ✗ | ✗ | ✗ |
| Verifiable behavioral identity | ✓ | ✗ | ✗ | ✓ (auth only) | ✗ |
| Shared knowledge substrate | ✓ | ✗ | ✗ | ✗ | ✗ |
| Open-source runtime | ✓ | ✗ | ✓ | ✗ | ✓ |

**One sentence under the table:**
> "Temporal owns 'did this code run.' Orkes owns 'did this workflow complete.' Keycard owns 'is this agent authorized.' Nunchi owns 'did the right agent, with the right memory, at the right price, produce a verifiable result?' That's the coordination plane."

---

### Slide 12 — Why It Compounds

**Three flywheels:**

**1. Knowledge compounding**
Every agent run deposits signals into the shared HDC-indexed knowledge store. The thousandth agent starts 30,000 tokens ahead of the first. Competitors keeping knowledge siloed within a single tenant have no cross-ecosystem compounding mechanism.

**2. Protocol lock-in**
MCP (97M monthly SDK downloads), A2A (150 organizations, v1.0 stable April 9, 2026), ERC-8004 (22,900 registrations in 3 days) — the agent protocol stack is crystallizing in the next 6–12 months. Deep integration now creates switching costs that are architectural, not contractual. The window to establish the canonical coordination plane closes when payment rails lock in identity requirements — likely Q4 2026.

**3. Regulatory tailwind as recurring revenue**
August 2, 2026 is a known spending trigger. Vanta built $100M ARR from SOC 2 automation. OneTrust exceeded $5B valuation from GDPR tooling. Nunchi's coordination plane makes Article 50 audit trails native — not bolted on. The compliance record doesn't migrate.

---

### Slide 13 — The Ask

**Milestones only. No dollar amount on this slide.**

| Timeline | Milestone |
|---|---|
| Month 1 | Cost benchmark published (100 SWE-bench tasks, full methodology) |
| Month 3 | SOC 2 Type II certification complete (audit firm: Schellman) |
| Month 6 | 3 design partner deployments with signed reference agreements |
| Month 12 | Protocol spec donated to Linux Foundation AAIF |

Dollar amount comes out verbally when asked. The milestones are what you're buying with the capital. This signals you know what you need rather than just naming a number.

**Target design partners (priority order):**
1. Hebbia — a16z portfolio (Casado's firm), Sivulka claims 2% of OpenAI daily volume, matrix product orchestrates o1/o3/GPT-4o in parallel
2. Harvey — April 2026 essay "engineers are now harder to coordinate," public job req for Context Engineering / Agent Infrastructure
3. Decagon — a16z portfolio, hiring Staff SWE Agent Orchestration (public listing)
4. Sierra — $10B valuation, outcome-based pricing makes cost reduction existential
5. Resolve.ai — ex-Splunk founders, coordinated multi-agent system in public materials (**NOTE: Greylock company, not a16z — do NOT use as demo topic in Casado pitch**)

---

## Why Now — Opening (Tightest Version)

Three sentences, three numbers, one closing line. Use this as the literal meeting opener:

> "In the last 90 days: Google committed $750 million to accelerate agentic AI deployment. Gartner reported a 1,445% surge in multi-agent enterprise inquiries. And Orkes raised $60 million last Thursday to make single-org orchestration production-grade. Every layer of the agent stack is getting funded — except the one between them. That's the coordination plane. And a16z named it: 'the bottleneck becomes coordination — routing, locking, state management, and policy enforcement across massive parallel execution.' Malika wrote that in Big Ideas 2026. We're building the canonical implementation."

**Why this works:** Current (this week), three independent market signals, quotes a16z's own thesis back at them without being sycophantic (quoting Aubakirova, not Casado), creates the empty-quadrant narrative, positions Nunchi as the response to a category a16z itself named.

---

## Orkes Response Matrix

### Quick Facts (April 23, 2026)
- Built on Netflix's Conductor (2016)
- $60M Series B, AVP-led (not a16z — no portfolio conflict)
- 3,000+ enterprise customers: JP Morgan, Tesla, Atlassian, American Express
- 24,000 GitHub stars, 10,000+ developers
- Positioning: "the missing orchestration layer" for production AI workflows

### What Orkes Does NOT Do
- Cross-org coordination
- Cost-aware model routing
- Shared knowledge substrate
- ZK behavioral verification / reputation
- Anything outside a single organization's trust boundary

### 30-Second Answer
> "Orkes makes single-org orchestration production-grade — same layer as Temporal. Their $60M validates that layer. But Orkes's own press says 'reliable, observable, and governable within a deployment.' That's one org. The coordination plane is the layer above: cross-org shared memory, verified identity, policy enforcement across trust boundaries. Orkes is a likely customer. They're not a competitor."

### Extended Answer (if pushed)
> "Orkes is the Netflix Conductor layer — it tells your agents what to do and confirms they did it. Nunchi answers: which agent should do it, do I trust it across org boundaries, what did it learn, and can I prove it to a regulator? Temporal owns 'did this code run.' Orkes owns 'did this workflow complete.' We own 'was the right thing done by the right actor with a verifiable receipt.' Same relationship as Vercel to AWS Lambda — built above, not competing."

### Critical Note for the Meeting
Casado has **not** invested in Orkes (AVP-led). This is a positive signal — the durable execution category is confirming, and the coordination plane thesis sits above it.

---

## "Platform Absorbs It" Objection — Tightest Response

**The objection:** Microsoft MAF 1.0 GA'd April 7. Google's $750M partner fund dropped April 22. Won't the hyperscalers just build this?

**The answer (30 seconds):**
> "The hyperscalers are funding ecosystem deployment — exactly as they funded Kubernetes deployment. Cloudflare didn't lose because AWS had networking. Databricks didn't lose because Spark ran on AWS. The coordination plane is the same bet: independent infrastructure wins the layer above the hyperscaler because it works across all of them. Microsoft's coordination layer works inside Azure. Google's works inside GCP. The enterprise with agents on both needs a coordination plane that isn't owned by either. That's structurally the same reason Temporal exists — AWS has Step Functions, but Temporal works across clouds."

**Reinforce with ** Google's $750M fund is *partner-led deployment* — they're paying partners to build *on* agentic AI, not building the coordination layer themselves. The fund announcement explicitly says the money goes to "partners deploying agentic AI solutions" — validating the ecosystem build-out without pre-empting the coordination plane.

---

## Cost Benchmark — Methodology & Framing

### What to Run This Week
- 100 SWE-bench Verified tasks (consistent with HAL benchmark task set)
- Baseline: stock LangChain AgentExecutor, Claude claude-opus-4 (no caching, no routing)
- Roko: with caching + CascadeRouter + gate-early-exit enabled
- Log: task IDs, model versions, cost breakdowns per task, token counts, gate exit rate
- Publish: full CSV + methodology doc to GitHub

### The Waterfall Decomposition (Theoretical Frame)
```
Naive (HAL published):    $44.86/task
  Prompt caching (÷5):    $8.97    (80-90% hit rate × Anthropic 90% cache discount)
  Model routing (÷3):     $2.99    (CascadeRouter: Haiku for file-read, Opus for reasoning)
  Gate early-exit (÷2):   $1.49    (42% of tasks exit before full execution)
Optimized estimate:       ~$1.42/task  →  ~31x
```

### Framing the Actual Number
If your benchmark produces **20x**: "20x cost reduction on a published, reproducible benchmark — waterfall shows where it comes from: 5x caching, 2x routing, 2x gate exit. Full methodology at [url]."

If it produces **30x**: Same framing, stronger number.

If it produces **10x**: Disclose it as 10x. The fraud prevention checklist (task IDs, model versions, cost breakdowns) makes 10x more credible than an unverified 30x claim. The methodology is the asset, not the multiplier.

### The Fraud Prevention Checklist (mandatory in published methodology)
1. Full task ID list (reproducible)
2. Model versions pinned (no switching mid-run)
3. Per-task cost breakdown (not just aggregate)
4. Cache hit rate disclosed
5. Gate exit rate disclosed
6. Baseline methodology: stock LangChain with no optimization
7. Date of run + API pricing version in effect

---

## Market Sizing — Recommended Slide Order

**Lead with the specific, not the aspirational. Cut the $230B TAM entirely.**

1. **Primary anchor — NHI Access Management (Gartner-named, verified):**
   - $10.71B in 2025 → $25.65B by 2033 at 11.53% CAGR
   - 80:1+ machine-to-human identity ratio (Gartner, 2026)
   - $400M+ in NHI governance funding in 2025 alone
   - $25B Palo Alto / CyberArk acquisition validates premium multiples

2. **Secondary — AI Agents Market:**
   - $7.84B → $52B by 2030 (46–50% CAGR)
   - 1,445% surge in enterprise multi-agent inquiries

3. **Optional tertiary (use if the conversation goes macro):**
   - 72% of Global 2000 companies now operating agents beyond pilot (March 2026)
   - Gartner: 50%+ of AI initiatives halted by 2028 due to coordination/identity failures

---

## Pre-Meeting Priority Stack (10 Days to May 6)

| Priority | Task | Deadline | Blocker |
|---|---|---|---|
| P0 | Run HAL benchmark (100 SWE-bench tasks) | May 1 | Nothing — run this first |
| P0 | Record asciinema of self-hosting loop with Beat 4 (knowledge compounding) | May 3 | Benchmark result needed for waterfall |
| P1 | Build/update Slide 8 (Validation) with actual benchmark number | May 4 | Asciinema complete |
| P1 | Prepare Orkes response (memorize the 30-second version) | May 2 | None |
| P2 | Recruit one technical advisor (one LinkedIn outreach today) | May 5 | None |
| P2 | Update comps in Slide 10 with Orkes $60M, LangChain $1.25B | May 2 | None |
| P3 | Build cost waterfall slide with actual number | May 5 | Benchmark result |
| P3 | Strip $230B TAM from market slide | May 2 | None |

---

## Sections to Cut or Deprioritize for May 6

| Document | Action | Reason |
|---|---|---|
| 10-AGENT-COEVOLUTION.md | Cut from briefing set / move to appendix | Score 7.2, entirely theoretical, no empirical data, zero investor value |
| $230B TAM framing | Remove from market slide | Aspirational and dismissible — leads with weakness |
| ISFR/clearing layer | Background only — not in deck | Phase 4 (month 9+), Casado-only pitch, no live implementation |
| Token / NUNCHI mechanics | Not in deck, not in conversation | Token conversations deferred 18-24 months post-Series A close |
| "Trust layer" framing | Never use this phrase | Saturated — 7+ companies use it. Exit the category. |
| "Agent OS" framing | Never use this phrase | Creates friction with framework ecosystems = potential partners |

---

## Strongest Sections — Keep As-Is

Per quality ratings and research synthesis:

- **Token Graveyard** (03-BUSINESS-MODEL.md) — Score 10. Best section in the set. Appendix only; do not surface unprompted.
- **Nava Deep Dive** (05-COMPETITIVE-INTELLIGENCE.md) — Score 9. Nava validates the on-chain agent verification thesis without competing at the coordination plane level.
- **EU AI Act Timeline** (02-STRATEGY.md) — Score 9. Well-sourced with countdown. Enforcement August 2, 2026 is a hard commercial trigger.
- **NHI Market Section** (02-STRATEGY.md) — Score 9. Possibly the best single section in the set. Lead with this in market sizing.
- **Three Dangerous Bears** (02-STRATEGY.md) — Score 9. Add: "Replits gross margins went from 36% to -14% due to agent compute costs. Cursor moved to credit-based pricing. Neither is a design partner target — both are proof points for Why Now."
- **Cost-Reduction Benchmark** (06-ROADMAP.md) — Score 9. Most investor-credible section. Anchor the entire pitch on this.

---

## Key Anti-Patterns (Never Use in Pitch)

| Phrase | Replace With |
|---|---|
| Web3 platform | Infrastructure for [specific vertical] |
| Tokenomics | Incentive design |
| Blockchain company | Company that uses verifiable compute |
| DeFi | (don't mention) |
| Trust layer | Coordination plane |
| Agent OS | (don't use) |
| Traction | Validation |
| 30x (unverified) | [actual benchmark result]x, with methodology |

---

*Document generated April 26, 2026. Incorporates research rounds R1–R15, April 2026 market data, and competitive intelligence through April 26 (Orkes $60M Series B, LangChain $1.25B Series B, Temporal $300M Series D, Nava $8.3M seed, Google Cloud $750M fund).*
