# Deep Research Prompt — Round 6 (Series A Intelligence)

Copy everything below the `---` line into Claude Desktop with deep research enabled.

---

## Research brief: Series A intelligence for an agent-protocol company

We're raising a Series A from tier-1 funds (a16z, Sequoia, Benchmark, Bessemer). The positioning is **"Stripe for the agent economy"** — structural coordination primitives that make agent coordination 10-30× cheaper, with on-chain identity, verifiable gates, and knowledge that compounds across sessions. The wedge is cost-reduction infrastructure for agent developers. The moat is a protocol standard (Signal/Block/Graph + HDC + ERC-8004).

Five prior research rounds established the technical stack, frontier capabilities, strategic positioning, and production reality. This round focuses on **what a sophisticated Series A investor needs to see, what comparable companies looked like at this stage, and what market intelligence we're missing.**

### What we know

- Agent coordination is the binding constraint (MAST: 41-86% failure rates, 79% from coordination)
- 10-30× cost reduction is defensible via caching × routing × gating × waste-trim
- The competitive quadrant (HDC + stigmergy + on-chain identity + verifiable gates + self-evolution) is structurally empty
- MCP playbook is the launch template (spec + 2 SDKs + 5 demos + 5 anchor partners)
- EU AI Act enforcement August 2, 2026 — compliance is a selling point
- Window is 6-12 months before MCP+A2A+ERC-8004+x402 lock in
- ACP gives distribution to 30M+ IDE users
- The consumer AI marketplace narrative is dead (GPT Store, Sora)
- On-chain agent economy is real but small ($30-80K/day real volume)

### What I need to know

#### Direction 1: Comparable company analysis at Series A stage

For each: what was ARR, team size, product maturity, and story at Series A?
- **Stripe** (2012 Series A, a16z) — what did the pitch look like? What metrics? What was the "why now"?
- **Temporal** ($103M Series B at $1.5B, Feb 2023 — what did Series A look like?) — how did they prove the deterministic-workflow thesis?
- **Vercel** ($150M Series D at $2.5B, 2022 — what was Series A?) — how did they demonstrate DX advantage?
- **Supabase** ($80M Series C, 2023 — what was Series A?) — how did OSS → hosted work?
- **Anthropic** ($124M seed) — what was the pitch for a research lab that hadn't shipped?
- **Databricks** (early Series A/B) — how did they pitch open-source platform?
- **HashiCorp** (Series A, $10M, 2014) — how did Terraform's protocol/standard pitch land?
- **Confluent** (Series A, $24M, 2015) — how did Kafka-the-protocol become Confluent-the-company?
- Any agent infrastructure company that has raised Series A in 2025-2026 — what was the pitch?

#### Direction 2: a16z thesis alignment — what they're looking for right now

- Read the latest a16z Big Ideas 2026 post — what agent infrastructure theses did they name?
- a16z crypto "Know Your Agent" thesis — what specifically are they looking for?
- Malika Aubakirova's "re-architecting the control plane" — what does she want to fund?
- a16z infra team (Martin Casado, Ali Ghodsi) — what infrastructure patterns do they back?
- What agent/AI companies has a16z invested in since January 2025? What's the pattern?
- Bessemer's "Five Frontiers" — which frontier is this? What metrics do they want?
- Sequoia's "Services: The New Software" (March 2026) — how does our pitch align?
- NFX's defensibility thesis (Pete Flint) — how does protocol + workflow embedding score?
- What did the winning pitch decks look like for recent agent-infra raises? (Any that are public or reported on)
- What questions did investors ask the founders of Temporal, LangChain, CrewAI at fundraise?

#### Direction 3: The "Stripe for agents" analogy — stress test it

- How did Stripe frame itself at Series A? What was the one-liner?
- What made "7 lines of code" the viral demo? What's our equivalent?
- How did Stripe go from payments API → Atlas → Connect → Treasury? What's our expansion path?
- What's the Stripe comparison that DOESN'T work? Where does the analogy break?
- Are there better analogies from recent raises? ("Plaid for X", "Twilio for X" — which pattern fits best?)
- How did Twilio's "API for communication" land? What's the parallel?
- Is "TCP/IP for agents" (Sequoia's framing) better or worse than "Stripe for agents"?
- What framing did Temporal use? HashiCorp? Confluent? What worked?

#### Direction 4: The 90-day launch plan — what to ship first

- Based on MCP's launch sequence, what should day one look like?
- What's the minimum demo that proves the core differentiator in <60 seconds?
- Which 5 anchor partners/integrations have the highest signal value?
- Should we launch at a conference? Which one? Or is a blog post + Product Hunt sufficient?
- What's the community seeding strategy? (MCP didn't need one — Anthropic had distribution. What do we do?)
- Should the spec be published as an academic paper? If so, which venue?
- What's the regulatory prep that must happen before launch? (EU AI Act, MSB classification)
- What's the pricing model? (OSS core + hosted, 0% take on marketplace, outcome-based for enterprise?)
- What's the hiring plan? (Research5 says forward-deployed engineers at first enterprise traction)
- What's the fundraise timeline relative to launch? (Raise before launch? After first anchor partners? After Pareto proof?)

#### Direction 5: Market sizing and TAM that investors will believe

- What's the credible TAM for "agent coordination infrastructure"?
- How do you bridge from $11B AI orchestration market to the $15T Gartner B2B-intermediated-by-agents number?
- What's the bottom-up market size? (N agent developers × willingness to pay × usage growth)
- What's the wedge market? (Just cost-reduction gateway? Cost + identity? Cost + identity + verification?)
- What comparable market sizes did Temporal, Stripe, Confluent cite at Series A?
- How do you model the expansion from wedge to full protocol?
- What's the revenue model that supports the protocol thesis? (OSS + hosted? Protocol fees? Marketplace take?)

#### Direction 6: The counter-thesis — what could kill this

- "Single-agent with better tools beats multi-agent coordination" (Princeton NLP's finding) — how do we address this?
- "LangGraph at 90M downloads has already won" — what's the response?
- "On-chain identity is a solution looking for a problem" — evidence for/against?
- "10× cost reduction is a feature, not a company" (Portkey/Helicone objection) — what makes this more?
- "The agent economy won't materialize for 5+ years" — what's the counter?
- "Chinese open-source models commoditize everything" — DeepSeek V4 at $3.48/M output
- "Temporal will just add agent primitives" — how defensible is the protocol?
- What killed companies with similar theses? (Agent infrastructure startups that failed in 2024-2025)
- What's the honest bear case?

#### Direction 7: Narrative and messaging that converts investors

- What storytelling patterns work for deep-tech infrastructure pitches at Series A?
- How do you make "protocol for the agent economy" feel urgent and not abstract?
- What's the demo that makes a partner say "I need to fund this"?
- How do you handle the "too early" objection? The "too complex" objection?
- What's the founder story that resonates? (Previous company, lived the problem, built the solution)
- How do you position the crypto/on-chain component without triggering crypto skepticism?
- What's the board composition that signals credibility?
- How did Stripe, Temporal, HashiCorp handle the "just open-source it" skepticism?

#### Direction 8: What's happening this week that matters

- Agent framework releases in the last 14 days
- Major agent deployment announcements
- New research papers on agent coordination
- VC fund announcements focused on agent infrastructure
- Any competitive moves in the HDC + agents space
- Any new agent standards or protocol proposals
- Enterprise agent deployment case studies published recently
- Agent-related security incidents

### Evaluation criteria

For each finding:
1. **Fundraise impact**: Does this change the pitch, the timing, the positioning, or the ask?
2. **Evidence quality**: Peer-reviewed? Public filing? Founder interview? Anonymous source?
3. **Time sensitivity**: Must act this week? This month? This quarter?
4. **Counter-thesis strength**: How strong is this objection? What's the best response?

### Output format

1. **The pitch in one page**: Given everything in this research, write the one-page narrative that would go on the first page of the pitch deck
2. **Comparable company table**: Series A metrics for 8-10 relevant comparisons
3. **Investor question bank**: Top 20 questions a16z will ask, with best-available answers
4. **Counter-thesis responses**: For each bear case, the strongest rebuttal with evidence
5. **90-day launch sequence**: Week-by-week plan based on MCP playbook
6. **The demo script**: What to show in 3 minutes that proves the thesis
7. **Full citations** with dates and sources

Prioritize:
- What a16z partner needs to hear in a 30-minute meeting
- Evidence that converts skeptics, not confirms believers
- Timing-sensitive intelligence (what changes if we wait 30 days?)
- Honest weaknesses alongside strengths (investors respect candor)
- Concrete numbers over narrative claims
