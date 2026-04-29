# Deep Research: Final Pre-Meeting Intelligence — May 6 a16z Pitch

Copy everything below the `---` line into Claude Desktop with deep research enabled.

---

## Situation

A founder is pitching **a16z on May 6, 2026** — 10 days from now. The pitch is for a **Series A ($20-30M at $200-400M post)** for a startup called **Nunchi**. This is a **pitch + live demo** format. The meeting has been set.

This is **Round 14 of a sustained research program** (13 prior rounds, ~300+ pages). All major strategic decisions are locked. What remains is tactical execution intelligence — the kind of information that changes what you say in the first 60 seconds, what you show in the demo, and how you handle the five most likely objections.

## What has been decided (compressed from 13 prior rounds)

### The company
**Nunchi** = two parts:
1. **Roko** — open-source Rust agent runtime (18 crates, 177K LOC, Apache 2.0). 6-stage pipeline: OBSERVE → GATE → ASSEMBLE → INFER+TOOLS → REFLECT → CONSOLIDATE. CascadeRouter learns which LLM to use (10-30x cost reduction). 11-gate verification pipeline. Shared knowledge store with decay. Self-hosting: reads its own PRDs, generates plans, dispatches Claude agents, validates, persists.
2. **Nunchi Chain** — sovereign EVM L1, Simplex consensus, ~50ms blocks via co-located Tokyo validators (Hyperliquid architecture). Native HDC precompile (~400 gas). ERC-8004 agent identities, 7-domain reputation. On-chain knowledge substrate with demurrage. Cooperative clearing engine (clearing-as-inference).

### The category (decided R11)
**"Agent Coordination Plane"** — separates agent coordination from execution, like SDN separated network control from forwarding. Named by Malika Aubakirova in a16z's Big Ideas 2026: *"the bottleneck becomes coordination: routing, locking, state management, and policy enforcement across massive parallel execution."*

### The beachhead (decided R12)
**Enterprise support on Roko OSS**, not a platform. Temporal raised $18.75M Series A with zero commercial product. HashiCorp's Atlas (platform) failed; Vault (one wedge) won. First customers: Hebbia (a16z portfolio, >2% OpenAI volume), Harvey (~$5-15M/mo LLM spend), Decagon (a16z portfolio).

### The convergence proof (decided R12)
**Architectural, not mathematical.** Use Casado's own vocabulary from his 2007 Stanford PhD (systems architecture, NOT control theory): logically centralized control, control/data plane separation, default-off, flow-level granularity. Closing line: "Ethane reduced networks to dumb forwarding governed by centralized policy. Nunchi does the same for LLM agents." Math (Borkar-Meyn, UCB1, Hedge) is defensive only — one slide, one equation.

### The pitch structure (decided R12-R13)
1. Open with the **layer cake**: Keycard at identity, Temporal at execution, Nunchi at coordination — making Casado repeat his own portfolio pattern (Pinecone+LangChain, Clerk+Keycard)
2. Anchor on **his token-path margin quote**: "Everybody has to be on the token path and everybody has to ask how do I extract margin on the tokens going through" — cost-aware routing IS that mechanism
3. **Volunteer boundaries** with Yoko Li's Inngest (workflow-as-code, not agent coordination) and Sarah Wang's Temporal (durable execution, not multi-agent coordination) BEFORE they're asked
4. Lead traction with **three numbers** (one logo, one OSS, one revenue/design-partner metric)
5. Have **Stripe data-room access** ready (post-Cluely default for ARR verification)
6. **Acknowledge Capsule and Nava by name** — pretending they don't exist looks unaware; positioning above them looks confident
7. Run **parallel funds** on same timeline — a16z is conviction-driven but allergic to being single bidder

### The demo (decided R12)
Local CLI binary against cached LLM proxy. Three minutes. Four primitives per output line (identity, prediction, gates, knowledge). Kill-the-controller durability moment. Hand Casado the laptop. Three backups (asciicast, Docker, Loom).

### Key Keycard differentiation (decided R13)
Keycard = identity-bound task-scoped JWTs (Auth0 for agents). Nunchi = coordination, routing, gates, shared knowledge ABOVE identity. "Keycard answers 'is this agent allowed to call this tool.' Nunchi answers 'which agent should call which tool, when, on which model, with what context, at what cost.'" Keycard's per-transaction pricing requires a coordination layer above it.

### Key Temporal differentiation (decided R13)
Temporal = single-tenant durable execution. Their blog: "The agent framework handles the AI. Temporal handles the infrastructure." Nunchi is the layer above. "Temporal owns 'did this code run.' Nunchi owns 'did the right agent, with the right memory, at the right price, with a receipt the counterparty can verify.' Network effects Temporal can't capture from a single namespace — same way Vercel built $9B on AWS Lambda."

### Critical risks to manage
- **Aubakirova/Keycard conflict**: She co-led Keycard ($38M). If Nunchi sounds like Keycard, deal dies. Must differentiate sharply on slide 1.
- **Sarah Wang routing**: Temporal is her practice. If Casado feels overlap, he routes to her. Volunteer the boundary early.
- **Solo founder bus factor**: Name senior #2 with commit and equity refresh on slide 1.
- **Post-Cluely rigor**: Stripe data-room access for ARR verification is default. Have it ready.
- **Dual-entity structure**: C-corp + Foundation is unusual for non-crypto infra. May need to collapse to single C-corp.

## What I need you to research NOW (10 days to meeting)

### Direction 1: The Exact Deck Copy

Write the actual words for each of the 13 slides. Every headline (max 8 words), every subtext (1-3 sentences), every speaker note (30-60 seconds of what to SAY). This is the deliverable that matters most.

**Slide 1**: Thesis + category. "The model is the same. The system is the variable." Subtext: "Nunchi — the Agent Coordination Plane."

**Slide 2**: Problem. One hero stat: "41-86% of multi-agent deployments fail. 79% from coordination." Below: Aubakirova quote. Below: de la Garza quote ("recursive fan-out of 5,000 sub-tasks looks like a DDoS attack").

**Slide 3**: Solution as code. The `nunchi run` CLI output showing all four primitives.

**Slide 4**: Founder + team. Three proof points + named #2 + 2-3 advisors.

**Slide 5**: Why now. Three forces: standards crystallizing (MCP/A2A), Aubakirova thesis (a16z named it), Stripe locked payments (value moves upstack). Plus Casado's "Bitter Economics" — model providers can't be trusted to optimize cost.

**Slide 6**: The layer cake. Visual showing Keycard (identity) → Temporal (execution) → Nunchi (coordination). Casado sees his own portfolio pattern.

**Slide 7**: Product. Real screenshots or the demo CLI output. The 6-stage pipeline. Predict-publish-correct with actual deltas.

**Slide 8**: Traction. Three numbers in Temporal's format. "177K lines of Rust. Self-hosting loop operational. [Design partner conversations / GitHub stat]." Acknowledge: no revenue yet — Temporal had none at Series A either.

**Slide 9**: Cost proof. $44.86 → $1.42 waterfall. "All raw data published. Third-party reproducible." Casado's token-path margin quote underneath.

**Slide 10**: Competition. Harvey-Ball table. Columns where competitors score HIGHER (production users, community). Name Keycard, Temporal, Nava, Capsule explicitly. "We are customers of Keycard, not competitors."

**Slide 11**: Business model. Enterprise support → managed hosting. Apache 2.0 + BSL. "Ship one product first — the Vault playbook, not Atlas." Platform multiple commitments: 130% NRR, 40% second-product attach by M12.

**Slide 12**: Use of funds. Engineering 55%, GTM 25%, G&A 20%. Milestones: 3 FDE engagements by M6, SOC 2 by M9, $1M ARR by M12.

**Slide 13**: Ask. "$25M to build the Agent Coordination Plane." The Ethane line. Close: "The model is commoditizing. The knowledge is not."

**Appendix slides** (pulled if asked): Token graveyard (VIRTUAL -86%, ELIZAOS -99.98%), technical deep dive, Keycard boundary, Temporal boundary, ISFR expansion thesis.

For each slide: research what the BEST version of that slide looks like from comparable companies. What did Temporal's slide 2 say? What did LangChain's slide 8 say? What did Vercel's slide 13 say?

### Direction 2: The Aubakirova Memo (Final Draft)

Write the complete 1-page memo (under 500 words) that gets forwarded to Casado. If the meeting is already set, this may serve as the pre-read or the follow-up "why we're excited" artifact. Format: inline email body or PDF attachment.

Must include: her quote, MAST data, cost proof, layer-cake positioning (above Keycard), the Ethane analogy, three traction numbers.

Research: do a16z partners prefer a pre-read before the meeting, or do they prefer to come in cold? What does their scheduling process imply about prep?

### Direction 3: Last 48 Hours Before the Meeting

Research what happens in the last 48 hours before a successful a16z pitch:
- What do founders who closed a16z rounds report doing the night before / morning of?
- What does the a16z office layout look like? (2 partners in the room? 5? Is there a specific conference room?)
- What devices should the demo run on? (Bring your own laptop? They have a TV? Projector?)
- What is the typical meeting length? (30 min? 45? 60?)
- Should the founder send anything the morning of? (A one-pager? Nothing? "Looking forward to meeting"?)
- What does the post-meeting follow-up look like? (Same day thank-you? Wait for them?)

### Direction 4: The Three Numbers

Every successful infrastructure Series A at a16z leads traction with three numbers. Research what three numbers Nunchi should use:

- **Logo metric**: What counts? Design partner LOIs? "In conversation with"? Do they need to be paying?
- **OSS metric**: GitHub stars? Monthly downloads? Contributor count? Lines of code? What do a16z infra partners actually look at?
- **Revenue/engagement metric**: If zero revenue, what substitutes? Weekly active developers? Self-hosting milestones?

Research: what three numbers did Temporal use at Series A (Oct 2020)? LangChain at Series A (Feb 2024)? Inngest at Series A (Sept 2025)? What was the format: "X metric, Y metric, Z metric"?

### Direction 5: What Drops Between Now and May 6

Research what is likely to happen in the agent infrastructure space in the next 10 days (April 26 - May 6, 2026) that could affect the pitch:
- Any conferences or events? (AI Engineer Summit? Devconnect? Google I/O?)
- Any expected product launches from competitors?
- Any a16z announcements?
- DeepSeek V4-Pro promotional pricing expires May 5 — the day before the meeting. Does this affect the cost comparison numbers?
- Any regulatory developments (EU AI Act, Digital Omnibus)?

### Direction 6: The "What If They Say No" Playbook

Research:
- What does an a16z "no" look like? Do they ghost, or do they give explicit feedback?
- If passed, what is the optimal next move? (Sequoia? Founders Fund? Lightspeed? Index?)
- Which other firms have active agent infrastructure theses in April 2026?
- What does the "parallel process" look like — should other funds already be in conversation?
- How do you take a16z "no" and use the diligence work to accelerate the next conversation?

### Direction 7: The Demo Failure Playbook

The demo is the centerpiece. Research what happens when demos fail at a16z:
- Are there public accounts of failed demos at a16z that still resulted in investment?
- What is the recovery move if the cached LLM proxy fails and real API calls are slow?
- Should there be a "guided walkthrough" version (talking through pre-recorded output) as a fourth backup?
- How do you handle "can you show me X instead?" — what are the 5 most likely ad-hoc requests from an infrastructure investor watching an agent demo?

## Output Format

### 1. 13-Slide Deck (Complete Copy)
For each: headline, subtext, key number, visual, speaker notes (verbatim what to say), comparable reference.

### 2. Aubakirova Memo (Ready to Send)
Under 500 words. PDF-formatted.

### 3. 48-Hour Tactical Brief
Checklist for May 4-6.

### 4. Three-Number Recommendation
The exact three numbers, with format and source.

### 5. 10-Day Horizon Scan
Events, launches, risks between now and May 6.

### 6. Plan B Playbook
If a16z passes: next 3 target funds, timeline, what carries forward.

### 7. Demo Failure Recovery
Five failure scenarios with recovery moves.

### 8. Full Citations
URLs and dates for everything.
