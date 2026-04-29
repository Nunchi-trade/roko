# Deep Research: The Actual Deck, The Actual Memo, The Actual Demo Commands

Copy everything below the `---` line into Claude Desktop with deep research enabled.

---

## Situation: 10 Days to a16z

A founder is pitching **a16z on May 6, 2026**. The meeting is confirmed. This is the **15th and final research round** of a program that has run 14 prior rounds across ~350+ pages.

**Every strategic decision is locked. This round produces DELIVERABLES, not strategy.** Three artifacts are needed:

1. The **13-slide deck** with exact words on every slide
2. The **Aubakirova pre-read memo** (1 page, under 500 words)
3. The **demo script** with exact terminal commands that will be run live

All three must be internally consistent — same numbers, same framing, same vocabulary. They will be used on the same day.

## The Full Context (read all of it — you have no other source)

### What the company is

**Nunchi** = two parts:

**Roko** — open-source Rust agent runtime (18 crates, Apache 2.0). Agents run a 6-stage pipeline: OBSERVE → GATE → ASSEMBLE → INFER+TOOLS → REFLECT → CONSOLIDATE. A model router (CascadeRouter) learns which LLM to use per task, producing 10-30x cost reduction vs. naive execution. An 11-gate language-agnostic verification pipeline validates output. A knowledge store accumulates agent learnings with Ebbinghaus-style decay. Predict-publish-correct on the event bus makes every operator a learner. The system self-hosts: reads its own PRDs, generates plans, dispatches Claude agents, validates with gates, persists results.

**Nunchi Chain** — sovereign EVM L1. Simplex consensus, ~50ms blocks via co-located Tokyo validators (Hyperliquid architecture). Native HDC precompile (~400 gas for 10,240-bit similarity search). ERC-8004 agent identities with 7-domain EMA reputation. On-chain knowledge substrate with demurrage pruning. ZK-HDC proofs (<1s proving). Cooperative clearing engine that turns trades into knowledge deposits.

### The strategic decisions (all locked)

**Category**: "Agent Coordination Plane" — the infrastructure layer that separates agent coordination from agent execution, like SDN separated network control from forwarding. Named by Malika Aubakirova in a16z's Big Ideas 2026.

**Beachhead**: Enterprise support contracts on Roko OSS, not a platform. Temporal had zero commercial product at Series A. HashiCorp's Atlas (platform) failed; Vault (one wedge) won.

**Convergence proof**: Architectural, not mathematical. Use Casado's own vocabulary from his 2007 Stanford PhD (systems architecture thesis, NOT control theory). Closing line: "Ethane reduced networks to dumb forwarding governed by centralized policy. Nunchi does the same for LLM agents." Math (Borkar-Meyn, UCB1, Hedge) is defensive depth only.

**Pitch structure**: (1) Open with layer cake (Keycard at identity → Temporal at execution → Nunchi at coordination). (2) Anchor on his token-path margin quote. (3) Volunteer Temporal and Inngest boundaries before asked. (4) Lead traction with three numbers (logos, usage-depth, velocity). (5) Acknowledge competitors by name.

**Keycard differentiation**: "Keycard answers 'is this agent allowed to call this tool.' Nunchi answers 'which agent should call which tool, when, on which model, at what cost.' We're customers of Keycard, not competitors." Keycard's per-transaction pricing literally requires coordination above it.

**Temporal differentiation**: "Temporal owns 'did this code run.' Nunchi owns 'did the right agent, with the right memory, at the right price, with a receipt the counterparty can verify.' That second problem has network effects Temporal can't capture from a single namespace — same way Vercel built $9B on AWS Lambda."

**Traction framing**: Casado publicly disqualified GitHub stars and downloads as vanity metrics ("Investing in Orbit" post). LOC must NOT appear. The triad: (1) named design-partner logos, (2) usage-depth metric (agent runs or tool calls), (3) one velocity number with sharp comp.

### The people in the room

**Martin Casado** — a16z infrastructure fund (~$1.25B). Created SDN category with Nicira ($1.26B VMware acquisition 2012). Portfolio: Cursor, Convex, Braintrust, Fivetran, Kong, Netlify. Led $43M Deeptune Series A March 2026 ("the missing layer in enterprise AI"). Key quotes: "everybody has to be on the token path and ask how to extract margin" (Latent Space Feb 19). "Bitter Economics" thesis: frontier labs gross-margin-negative on next training run. "Non-consensus product, consensus pitch" (Newcomer Aug 2025). His PhD thesis is systems ARCHITECTURE, not control theory — no Lyapunov proofs. He has said NOTHING about multi-agent coordination as a category. Opportunity and risk.

**Malika Aubakirova** — Partner on Casado's AI Infrastructure team. Stanford GSB. Ex-Google/Chronicle Security. Wrote "Native Agent Infrastructure Will Become Standard" in Big Ideas 2026. Co-led Keycard ($38M, Oct 2025). Co-wrote "Et Tu, Agent?" blog (20% hallucinated packages, 50% more vulnerable deps). CRITICAL: Keycard is her deal. Nunchi must NOT sound like Keycard or she flags conflict.

**Joel de la Garza** — Security GP. Co-wrote Big Ideas 2026: "a single agentic goal triggers recursive fan-out of 5,000 sub-tasks... to a legacy database it looks like a DDoS attack."

**Yoko Li** — Leads Inngest (workflow-as-code) and Keycard. Clean layer-separation precedent in her own portfolio.

**Sarah Wang** — Led Temporal $5B Series D. Agent durability seat belongs to her practice, NOT Casado's. If overlap, he routes to her.

### Key numbers (all sourced)

| Metric | Value | Source |
|--------|-------|--------|
| Agent failure rate | 41-86% | MAST, NeurIPS 2025 |
| Failures from coordination | 79% | MAST |
| Naive agent cost | $44.86/task | Princeton HAL (excludes caching) |
| Optimized cost | ~$1.42/task | Caching + routing + gating |
| NHI market | $9.45B → $18.71B by 2030 | 11.9% CAGR |
| Platform vs tool multiple | 8.2x vs 3.9x | Equal Ventures / BVP |
| Temporal valuation | $5B at 40-60x ARR | Led by Sarah Wang |
| Braintrust valuation | $800M | Casado-led Series A |
| Agentic AI YTD 2026 | $2.66B / 44 rounds | 143% over same period 2025 |
| EU AI Act Article 50 | August 2, 2026 | ~14 weeks |

### Meeting logistics

- **Location**: 2865 Sand Hill Road or 180 Townsend. Confirm with EA 48h out.
- **Duration**: 45 min blocked. 20-25 presented, 20+ Q&A.
- **Attendees**: 2-10 people. Get list 24h prior.
- **Pre-send**: Deck as PDF 24-48h ahead with 1-page exec summary. NEVER DocSend.
- **Post-meeting**: Thank-you within 4-6h same day with 8-12 refs attached.
- **Demo AV**: Own MacBook, HDMI + USB-C dongles, LTE hotspot, second laptop backup.

### Calendar context for May 6

- DeepSeek V4-Pro 75% promo expires May 5 (day before). Post-discount: $1.74/$3.48.
- OpenAI Workspace Agents goes PAID May 6 (same day). Partners read the headlines that morning.
- No major conferences before May 6.

---

## Deliverable 1: The 13-Slide Deck (Exact Copy)

Write the ACTUAL words for every slide. Not templates. The real thing. Each slide gets:
- **Headline** (max 8 words, 60pt+ font)
- **Subtext** (1-3 sentences, 30pt+ font)
- **Key number(s)** to display
- **Visual** (what goes on screen — code, chart, diagram, screenshot)
- **Speaker notes** (verbatim 30-60 seconds of what to SAY while the slide is up)
- **15-30 words per slide** total visible text (Kawasaki mechanical floor: 30pt minimum)

### Slide structure (locked from R14):

**Slide 1: Title + Thesis**
"The model is the same. The system is the variable." Below: "Nunchi — the Agent Coordination Plane." This is NOT the traction slide — save numbers for later.

**Slide 2: Problem**
One hero stat. Aubakirova quote underneath. De la Garza quote as reinforcement. Do NOT use a chart — one giant number is more effective (R10 finding from 50-deck review).

**Slide 3: Solution as Code**
The `nunchi run` CLI output. Real, not pseudocode. Four primitives visible: identity, prediction, gates, knowledge. Below: "Four primitives. One command."

**Slide 4: Founder + Team**
Solo founder with three proof points. Named #2 with commit + equity refresh. 2-3 advisors with specific contributions. (R14: solo founder is a bus-factor risk — lead with #2 early to defuse.)

**Slide 5: Why Now**
Three converging forces. Aubakirova thesis (a16z named the problem). Standards crystallizing (MCP 97M/mo, A2A v1.0). Stripe locked payments (x402/ACP) — value moves upstack. Plus: "Bitter Economics" — model providers can't be trusted to optimize cost.

**Slide 6: The Layer Cake**
Visual: Keycard (identity) → Temporal (execution) → Nunchi (coordination). "a16z has funded each layer. The coordination layer is open." Casado sees his own portfolio pattern.

**Slide 7: Product**
The 6-stage pipeline with the predict-publish-correct delta showing live. Real screenshot or demo output. Connect Roko runtime to Nunchi chain visually.

**Slide 8: Traction**
Three numbers only. Named logos (design partners). Usage-depth metric. One velocity number with comp anchor. NO LOC. NO star count (Casado disqualified them). NO "self-hosting loop." Format: Temporal's "Snap, Box, Coinbase, Checkr" pattern.

**Slide 9: Cost Proof**
$44.86 → $1.42 waterfall. HAL baseline → caching → routing → gating → full stack. "All raw data published. Third-party reproducible." Below: Casado's token-path margin quote.

**Slide 10: Competition**
Harvey-Ball or Power Grid format (NOT 2x2). Include columns where competitors score HIGHER (production users, community). Name Keycard, Temporal, Nava, Capsule. "We are customers of Keycard, not competitors."

**Slide 11: Business Model**
Enterprise support → managed hosting. Apache 2.0 + BSL. "Ship one product first — the Vault playbook, not Atlas." Platform commitments: 130% NRR, 40% second-product attach by M12, 5+ runtimes.

**Slide 12: Use of Funds**
$25M. Engineering 55%, GTM 25%, G&A 20%. Three milestones: 3 FDE engagements M6, SOC 2 M9, $1M ARR M12.

**Slide 13: Ask + Close**
"$25M to build the Agent Coordination Plane." Ethane line. Then: "The model is commoditizing. The knowledge is not. We're building the network that compounds it."

**Appendix** (8-15 slides, not presented, for second meeting): Token graveyard, ISFR expansion thesis, technical architecture, Keycard boundary detail, Temporal boundary detail, competitive deep dive, regulatory timeline, team expansion plan.

Research: for EACH slide, find the best comparable from Temporal, LangChain, Vercel, Supabase, or Snowflake. What specific WORDS did their best slide use? The goal is to write copy that pattern-matches to what Casado has already approved as a partner.

### Slide Design

Research: what is the literal visual template that a16z infra deals use? Is it Sequoia's template? Google Slides or Figma? Light or dark? What font? The R10 finding was: Casado is content-first, not design-first. No fancy animations. But R14 confirms 8/12 agent-infra sites use dark. What does the DECK use?

## Deliverable 2: The Aubakirova Pre-Read Memo

This is the 1-page document sent 24-48h before the meeting. It may be the most important single artifact — it determines whether Aubakirova champions the deal or flags conflict.

**Format**: Inline email body or attached 1-page PDF. Under 500 words. Technical, not salesy.

**Must include**:
1. Her Big Ideas 2026 quote verbatim (opening line)
2. "We're building what you described"
3. MAST data: 41-86% failure, 79% coordination
4. Cost proof: $44.86 → $1.42 (HAL benchmark, excludes caching)
5. Category: Agent Coordination Plane
6. Layer cake: "Keycard solved identity. Temporal solved execution. The coordination layer is open."
7. Differentiation from Keycard: "We are customers of Keycard, not competitors. Our coordination layer drives volume to Keycard's per-transaction billing."
8. One Ethane sentence
9. Three traction numbers
10. Close: "30 minutes with Martin to show the demo."

Research: what pre-read format actually works for a16z partners? Is 500 words too long? Too short? Should it be a Google Doc, PDF, or inline email? Does Aubakirova have a preferred format based on her public behavior?

## Deliverable 3: The Demo Script (Exact Commands)

The demo runs on a MacBook against a local Docker controller with pre-warmed LLM cache. Three minutes. Four primitives per output line. The founder will hand Casado the laptop.

**Exact sequence**:

1. **Beat 1 (0:00-0:30)**: Identity + Gates
   ```
   $ nunchi run agents/researcher.py --task "Summarize Q3 fintech earnings"
   ```
   Output shows: agent identity (verified), prediction ($0.043), three gates passing, knowledge loaded from shared store.

2. **Beat 2 (0:30-1:15)**: Predict-Publish-Correct
   Output shows: actual cost $0.031 vs predicted $0.043 (-28% delta). Two new facts deposited.
   SAY: "Every run generates a training signal. The system gets cheaper with use."

3. **Beat 3 (1:15-2:15)**: Shared Knowledge
   Run a SECOND agent on a related task. It loads the first agent's deposited facts.
   Show: -64% cost vs naive because of shared knowledge.
   SAY: "The thousandth agent joins smarter than the first."
   **Hand Casado the laptop. Let him type the third command.**

4. **Beat 4 (2:15-2:35)**: Kill + Resume
   Ctrl-C the controller mid-run. Restart. Watch it resume from checkpoint.
   SAY: "Identity, prediction, shared memory, durability. Four primitives."

5. **Close (2:35-3:00)**:
   "Every multi-agent company will need these within 18 months."

**Pre-script a tool-call failure into Beat 2** so you can show retry/recovery without being asked (R14 finding: "show me what happens when a tool call fails" is the #1 most likely ad-hoc question).

Research: what specific Python/CLI commands look right for this? Draft the actual code. What does the output formatting look like (colors, symbols, alignment)? What agent name and task description will resonate with an infrastructure investor? ("Summarize Q3 fintech earnings" may be wrong — something closer to "triage this production alert" or "investigate this CI failure" might pattern-match better to Casado's infra portfolio.)

## Deliverable 4: The "What If" Cheat Sheet

A single page the founder keeps on the table (or memorizes) with prepared answers for:

1. "How are you different from Temporal?" (30-second answer, locked)
2. "How are you different from Keycard?" (30-second answer, locked)
3. "How are you different from Nava/Capsule/t54?" (15-second answer each)
4. "What about LangChain/CrewAI?" (15-second answer)
5. "What's your moat?" (Casado doesn't believe in data moats — answer with architecture)
6. "How do you close the control loop?" (His 2025 skepticism — answer architecturally)
7. "What happens if frontier models get 10x cheaper?" (Jevons paradox + reasoning token explosion)
8. "Why should we fund this instead of waiting 6 months?" (Aubakirova thesis + standard crystallization)
9. "Who's your #2?" (Named, with specific contribution)
10. "What happens if you get hit by a bus?" (IP is open-source, knowledge is in the system)

Research: are there other common a16z objections for pre-revenue infrastructure companies that aren't on this list?

## Output Format

### 1. The Deck (13 slides + appendix outline)
Exact headline, subtext, number, visual, speaker notes per slide. Total visible text under 400 words across all slides.

### 2. The Memo (ready to send)
Under 500 words. Formatted.

### 3. The Demo Script (runnable)
Exact terminal commands. Exact output. Beat-by-beat timing.

### 4. The Cheat Sheet (10-15 Q&As)
Question, 15-30 second answer, source for the answer.

### 5. Full Citations
Everything sourced.
