# Deep Research Prompt — Round 5 (From Spec to Reality)

Copy everything below the `---` line into Claude Desktop with deep research enabled.

---

## Research brief: what does the first year of deployment actually look like?

Four prior research rounds established: the technical stack (R1-R2), the frontier capabilities (R3), and the strategic positioning (R4). This round is different again: **I need to know what happens when theory meets reality.** What breaks? What works faster than expected? What do the first 100 developers actually need? What does the competitive landscape look like *this month*?

The system is positioned as a **protocol for the agent economy** (peers: Stripe, ERC-20, MCP), with Signal/Block/Graph as composable primitives, HDC as universal router fabric, stigmergic coordination, active inference cognitive layer, self-evolving L4, provably-corrigible safety, and on-chain identity via ERC-8004 + x402. The immediate wedge is 10-30× cost reduction via structural primitives (caching × routing × gating × handoffs).

### What I need to know now

#### Direction 1: How protocols actually get adopted — the mechanics of going from 0 to standard

MCP went from announcement (Nov 2024) to 97M monthly SDK downloads (Mar 2026) in 16 months. ERC-20 went from EIP to $11.4T cumulative DEX volume. A2A went from Google announcement to 150+ org support in months.

- What are the specific, step-by-step mechanics that made MCP succeed? First integration? First viral moment? What was the adoption curve shape?
- How did ERC-20 achieve critical mass? What was the minimum viable standard that triggered composability explosion?
- What protocols *failed* to get adopted despite being technically superior? (gRPC vs REST early days, SOAP vs REST, etc.) What killed them?
- What's the minimum number of integrations needed before a protocol becomes self-sustaining?
- Research on standards adoption: Shapiro-Varian, Arthur increasing returns, Katz-Shapiro network externalities — what do they say about timing?
- How does an open-source protocol monetize? (Linux Foundation model, Ethereum Foundation model, MCP Foundation model)
- What's the role of a reference implementation vs the spec itself? Does the spec or the implementation drive adoption?
- How do you get the first 10 organizations to adopt? First 100? What changes at each order of magnitude?

#### Direction 2: Real-world agent deployment at scale — case studies from production

Not benchmarks — production. What happens when companies run agents in anger?

- **Temporal + OpenAI Codex**: How does deterministic-workflow / non-deterministic-activity work in practice at OpenAI scale? Failure modes? Cost data?
- **Replit Agent**: $150M annualized from $2.8M in 8 months. What's the architecture? What broke at scale? What retention looks like?
- **Cursor**: $100M ARR year one. How much is agent vs autocomplete? What's the cost-per-user? How do they manage LLM costs?
- **Harvey**: $75M ARR, $5B valuation. Legal agent architecture? Multi-agent coordination patterns? How do they handle hallucination in legal context?
- **Cognition Devin**: What actually happened? Real performance vs marketing? Why the gap?
- **Anthropic Claude Code**: $1B run rate in 6 months. Architecture? Cost structure? What makes it sticky?
- **Linear + AI agents**: 10.1% → 24.4% agent-delegated work in 2 months. What UX pattern drove this?
- **Any company running 100+ concurrent agents in production**: What coordination problems emerged? What tooling was missing?
- Real cost data: what does running a coding agent cost per developer per month? Trading agent? Research agent? Customer support agent?

#### Direction 3: Developer experience that drives adoption — what the first 100 developers need

- What made Stripe's DX legendary? Specific patterns (API design, documentation, error messages, onboarding flow)
- What made Vercel's DX win vs Netlify? Specific moments in the developer journey
- Supabase: MIT license → $70M ARR. What role did OSS play? When did hosted become necessary?
- **Time-to-first-value benchmarks**: What's the fastest any developer platform achieved TTV? What's the mechanism?
- What do developers evaluate in the first 5 minutes? First hour? First day? First week?
- Research on developer decision-making: how do developers choose between competing platforms?
- What's the role of templates/starters vs blank canvas? (Lovable, Cursor, Gamma all achieved <60s TTV via templates)
- What documentation patterns work for protocol specs? (Stripe docs, Ethereum yellow paper, MCP spec — different audiences, different patterns)
- What's the minimum viable SDK? (One language or polyglot from day one?)
- Developer community research: Discord vs GitHub Discussions vs forum? What creates sticky communities?

#### Direction 4: Agent economics in production — real numbers

- What does LLM inference actually cost per-agent-hour for different use cases? (coding, research, trading, customer support)
- What's the real-world effectiveness of semantic caching? (Claimed 73-86% cost reduction — confirmed in production?)
- What's the real-world effectiveness of model routing? (RouteLLM claimed 85% cost cut — at what quality loss in practice?)
- What's the cost curve for running 10 vs 100 vs 1000 concurrent agents?
- Token usage patterns: what percentage of tokens are wasted in typical agent loops? Where does waste concentrate?
- KV cache economics: what's the ROI of cross-agent KV sharing (KVCOMM, KVFlow) in practice?
- Inference provider pricing trends: what will GPT-5/Claude-5/Gemini-3 class models cost in 6 months? 12 months?
- The economics of self-hosted vs API: at what scale does self-hosting break even?
- Real agent marketplace economics: what are Olas Mech Marketplace agents actually earning? Virtuals agents?
- x402 real transaction data: what percentage is real commerce vs gamified testing?

#### Direction 5: What changed in the last 30-60 days that shifts priorities

- Latest agent framework releases (LangGraph, MS Agent Framework, Bedrock AgentCore updates)
- Latest MCP/A2A developments
- Latest ERC-8004 adoption metrics
- New research papers in agent self-improvement, coordination, safety (April-May 2026)
- Any new competitive entrants positioning as "agent protocol" or "agent OS"?
- Latest SWE-bench, GAIA, Terminal-Bench results — has the capability frontier moved?
- Any major agent failures/incidents in production? (security breaches, cost runaways, coordination failures)
- What has Anthropic/OpenAI/Google/Microsoft shipped in the agent space in the last 60 days?
- Any new VC theses or market maps for agent infrastructure?
- Regulatory developments: EU AI Act agent provisions? US executive orders? Any jurisdiction-specific requirements?

#### Direction 6: Measurement and proof — how to demonstrate the claims

The spec claims 10-30× cost reduction, composability explosion, collective intelligence, self-improvement. How to prove each?

- What benchmarks exist for agent cost efficiency? (Not just accuracy — cost per correct answer)
- How do you measure composability? (Number of valid Block combinations? Time to create a new workflow?)
- How do you measure collective intelligence improvement? (c-factor over time? Cross-agent transfer learning?)
- How do you measure self-improvement? (SWE-bench trajectory over time? Task diversity expansion?)
- What's the equivalent of Stripe's "7 lines of code to accept payments" demo for an agent protocol?
- A/B testing methodology for agent systems: how do you run controlled experiments when agents are non-deterministic?
- What metrics do VCs actually look at for protocol adoption? (Sequoia, a16z — specific KPIs they track?)
- How did Temporal prove its value proposition? (380% YoY revenue — what drove conviction?)

#### Direction 7: The dark horses — competitors nobody's talking about yet

- Startups in stealth working on agent infrastructure (YC batches, recent seed rounds)
- Academic labs with working prototypes that could become products
- Big tech internal projects that could be open-sourced (like how Google open-sourced A2A)
- Non-obvious competitors from adjacent spaces (game engines, robotics, financial infrastructure)
- What would it look like if Temporal added agent primitives? If Dagster went full agent-native?
- Is anyone else combining HDC + agents? HDC + blockchain? Active inference + agents?
- What's happening in the Chinese agent ecosystem? (Qwen, DeepSeek, ByteDance agents)
- Any hardware companies building agent-specific chips? (beyond neuromorphic HDC)

#### Direction 8: Regulatory and compliance landscape

- EU AI Act: what are the specific requirements for autonomous agents? Timeline?
- US: any executive orders or proposed legislation specific to AI agents?
- Financial regulation: can autonomous trading agents operate legally? In which jurisdictions?
- Data privacy: GDPR implications for agent memory and knowledge sharing
- Liability: who's responsible when an autonomous agent causes harm? Latest legal thinking?
- On-chain agents: regulatory status of agents that hold crypto, make transactions, earn revenue?
- Agent identity: legal status of ERC-8004 passports? Can an agent be a legal entity?
- What compliance features do enterprise buyers require before deploying agents?
- SOC 2 / ISO 27001 implications for agent platforms
- Insurance: is there an emerging market for agent liability insurance?

#### Direction 9: What happens at 10K agents — emergent phenomena and failure modes

- Project Sid (Altera): 1,000 agents showed emergent religion, democracy, role specialization. What happened next? Any follow-up studies?
- Polystrat (Olas): >30% of Polymarket wallets are AI. What emergent dynamics appeared?
- Has anyone run >10K coordinated agents? What coordination problems are unique to that scale?
- Flash crashes: any documented cases of agent-caused market instability?
- Herding behavior: do LLM agents converge on the same strategies (since they share training data)?
- Resource contention: what happens when thousands of agents compete for the same LLM inference capacity?
- Emergent communication: any documented cases of agents developing unexpected communication patterns?
- Knowledge pollution: at scale, does shared knowledge degrade or improve? What's the empirical evidence?
- Social dynamics: do agent populations develop hierarchies, factions, or cooperation norms without being designed to?

#### Direction 10: The spec as intellectual contribution — what makes it citable and influential

- What specs/papers became foundational references? (Ethereum yellow paper, Bitcoin whitepaper, MapReduce paper, Raft consensus)
- What makes a technical document influential beyond its immediate community?
- Should the spec be published as an academic paper? A whitepaper? An EIP-style proposal? All three?
- What conferences or venues would give it maximum visibility? (Not just ML — systems, PL, HCI, economics)
- How to make the HDC + active inference + stigmergy combination a recognized research direction?
- What naming/branding makes a protocol stick? (REST, GraphQL, gRPC — what worked about the names?)
- Role of formal verification in credibility: does having TLA+ specs or categorical proofs matter for adoption?

### Evaluation criteria

For each finding:
1. **Verdict**: "act on now" / "incorporate into spec" / "plan for month 3-6" / "monitor"
2. **Evidence quality**: Peer-reviewed? Production data? Vendor marketing? Anecdotal?
3. **What it means for the spec**: Does this change a design decision? Validate an existing one? Add a new requirement?
4. **Time sensitivity**: Is there a window closing? A competitor moving? A standard solidifying?
5. **Risk if ignored**: What happens if we don't act on this finding?

### Output format

1. **Executive summary**: Top 5 findings that change what we do *this month*
2. **Per-direction sections**: Findings with 5-point evaluation
3. **The 90-day roadmap**: Given everything in this research, what should the first 90 days of deployment look like? What's the sequence that maximizes learning while building momentum?
4. **Competitive dashboard**: Current state of every named competitor with recent developments
5. **Risk register**: Top 10 risks ranked by (probability × impact), with mitigation for each
6. **The "prove it" list**: For each major claim (10× cost, composability, c-factor, self-improvement), the specific benchmark or demonstration that would convince a skeptical a16z partner
7. **Full citations** with dates, venues, repos

Prioritize:
- **Production data over benchmarks** (what works in anger, not in papers)
- **Recent developments** (last 60 days > last 6 months > last year)
- **Actionable findings** (things that change a decision this week)
- **Negative results and failure modes** (what to avoid)
- **Time-sensitive information** (windows closing, standards solidifying, competitors moving)
- **Things that validate or invalidate the existing spec's assumptions**
