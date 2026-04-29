# Deep Research Prompt — Round 9 (Narrative Sharpening, Category Definition, and Untapped Mechanisms)

Copy everything below the `---` line into Claude Desktop with deep research enabled.

---

## Context

You are researching for **Nunchi**, a two-part system:

1. **Roko** — open-source Rust agent runtime (18 crates, ~177K LOC). Three primitives: Signal (durable, content-addressed, HDC-fingerprinted), Pulse (ephemeral on Bus), Cell (atomic computation, 9 protocols). Agents learn via predict-publish-correct on the Bus. Native 6-stage harness: OBSERVE → GATE → ASSEMBLE → INFER+TOOLS → REFLECT → CONSOLIDATE. CascadeRouter for model selection. 11-gate pipeline with adaptive thresholds.

2. **Nunchi chain** — sovereign EVM L1, Simplex consensus (Chan & Pass, IACR 2023/463), ~50ms blocks via co-located Tokyo validators (Hyperliquid model). Native HDC precompile (~400 gas, 20-100x cheaper than Solidity). ERC-8004 agent identities with 7-domain EMA reputation. On-chain knowledge substrate with demurrage-based pruning. ZK-HDC proofs (Circom + Groth16, <1s proving). ERC-8183 job market.

**Prior rounds covered**: substrates (R1), algorithms (R2), frontiers (R3), strategy (R4), production reality (R5), Series A (R6), reality check (R7), business model (R8). This is Round 9.

**Current positioning**: "Nunchi is the identity, reputation, and verifiable-similarity layer for the agent economy." Cost reduction (10-30x) is the wedge. Trust/identity/reputation is the moat. Network effect: "The thousandth agent joins smarter than the first."

**What we have and where it's weak** (from quality audit):
- Risks doc (8.5/10) — strongest. Honest, well-cited.
- Strategy doc (8.2/10) — strong but NHI market reframing is new and needs validation.
- Architecture doc (7.9/10) — HDC section is 9/10. Specializations and protocol differentiation are 7/10.
- Co-evolution doc (7.2/10) — entirely theoretical with zero empirical data. Needs validation or risk being cut.
- Landing page spec (7.7/10) — projections on marketing page risk credibility. Cost proof section is 9/10.
- Competitive doc (7.8/10) — "empty quadrant" is tautological (we defined 6 dimensions around our own features).

**What Round 9 must answer**: The gaps between 7 and 9. The narrative choices that separate a good pitch from a category-defining one. The mechanisms that create irreversible market position.

## Direction 1: Category Definition — What Category Is Nunchi Creating?

The strongest companies don't compete in existing categories — they define new ones. Stripe defined "developer-first payments." Temporal defined "durable execution." Linear defined "the tool developers actually want to use."

- What is the precise category Nunchi is creating? "Agent trust infrastructure"? "Non-human identity infrastructure"? "Agent coordination protocol"? "Verifiable agent economics"? Research how successful category-creation plays were named. Play Bigger (Al Ramadan, 2016) framework. How did Snowflake ("cloud data warehouse"), Databricks ("lakehouse"), and Confluent ("data streaming") each crystallize their category names?
- How long after founding did each take to establish the category? What were the key inflection points?
- Is there a version of this that positions Nunchi as infrastructure for agent COMPLIANCE rather than agent coordination? The EU AI Act August 2, 2026 deadline creates forced demand. Research: how did Vanta ($100M+ ARR) and OneTrust ($5B+) define their compliance categories? What was their category name at Series A vs. their category name at $100M ARR?
- Is "Non-Human Identity" (NHI) the better category? Saviynt ($3B, KKR), CyberArk ($25B Palo Alto acquisition), Oasis Security ($120M) are all attacking NHI from the security side. Is there an offensive framing (identity as enabler, not just security control) that Nunchi can own?
- Research category kings vs. category residents. Play Bigger data: category kings capture ~76% of total category market cap. What did the king do differently from the #2 in each case?

## Direction 2: The "Impossible to Go Back" Moment — Product Psychology

The best developer tools create a moment where users cannot imagine going back. Cursor's moment is autocomplete-then-tab. Vercel's moment is git-push-to-preview-URL. Supabase's moment is `supabase init` → working backend in 60 seconds.

- What is Nunchi's "impossible to go back" moment? Current candidate: `nunchi run --share` producing a shareable URL with live cost meter, agent execution timeline, and ZK proof. Is this strong enough?
- Research: what does the cognitive science say about "lock-in through delight" vs "lock-in through dependency"? Is there a moment that creates BOTH? (Stripe's `curl` example creates delight; their idempotency keys create dependency.)
- What did Temporal's "impossible to go back" moment look like for developers? Research their first 1,000 users. What was the retention driver?
- Research: Linear's "3.7x speed advantage over Jira" (DevTools Insights 2024). How did they measure this? Is there an equivalent benchmark for agent coordination tools?
- What is the developer equivalent of dopamine scheduling for a cost meter? (Seeing money saved in real time.) Research gamification in developer tools. Are there examples of cost meters that changed developer behavior?

## Direction 3: Niche Construction Empiricism — Does Co-Evolution Actually Work?

Our co-evolution thesis (every agent improves the environment for the next agent, producing compound returns) is compelling but entirely theoretical. We need data.

- Are there ANY empirical studies of compound improvement in AI agent codebases? Look for: longitudinal studies of code quality under repeated AI agent modification, documentation density trends, context efficiency improvements over time.
- Research: what do the Cursor/Claude Code usage logs show about improvement over time? Anthropic's April 2026 research on Claude Code productivity — any data on whether code quality improves or degrades with repeated agent use?
- The 1% per-invocation improvement claim produces 170% gain at 100 invocations (compound interest). Has anyone measured the actual per-invocation improvement rate for coding agents? What is the real number?
- Research: Shumailov et al. (2024) on model collapse from synthetic data. Does the same apply to code? Do agents writing code that other agents read create a code-quality collapse? Or is it different because code has external verification (tests, compilation)?
- GitHub Copilot usage data: any studies on whether codebases that use Copilot heavily have higher or lower quality metrics over time? Stack Overflow quality trends since AI coding tools?
- The affordance scoring framework (extensibility, test coverage, doc coverage, coupling, stability, size) — have similar frameworks been validated in software engineering research? Research: Chidamber & Kemerer (1994) OO metrics, SonarQube quality gates, CodeClimate maintainability scores. How do they correlate with AI agent performance?

## Direction 4: The NHI Market — Is This Really the TAM?

We just reframed around the $18.7B NHI market. Validate this.

- NHI access management: $9.45B (2024) → $18.71B by 2030 (11.9% CAGR). Source? Is this from a major analyst firm (Gartner, Forrester, IDC) or a lesser-known source? How reliable is this projection?
- Compare: what percentage of the NHI market is "agent identity" specifically vs. "service account management" or "API key rotation"? Nunchi addresses agent identity, not all NHI. What is the realistic serviceable addressable market (SAM)?
- Research: what do the NHI security companies (Saviynt, Oasis, Astrix, GitGuardian) actually sell? Is it API key lifecycle management, or is it something closer to what Nunchi does (behavioral reputation, verified identity)?
- Identity-as-a-service market: what does Auth0 (acquired by Okta for $6.5B), WorkOS ($1.25B valuation), and Clerk ($300M) tell us about the identity infrastructure business model? Do they sell per-identity, per-authentication, or per-feature?
- Research: ERC-8004 adoption curve. 80-150K is our projection. What is the ACTUAL number of registered ERC-8004 identities as of today? What is the daily registration rate? Can we get data from Dune Analytics or Etherscan?

## Direction 5: Compelling Narratives from Adjacent Worlds

The pitch needs stories, not just data. Research narratives from adjacent domains that can be adapted.

- **TLS/PKI analogy**: Nunchi is to agent trust what TLS/PKI is to HTTP. Research: how did SSL/TLS actually get adopted? What drove the "HTTPS everywhere" transition? Was it browser warnings (compliance pull) or developer demand (bottom-up)? What was the timeline from optional to mandatory?
- **Credit bureau analogy**: Nunchi's reputation system is like a credit bureau for agents. Research: how did Equifax/Experian/TransUnion establish their data moats? What was the initial bootstrapping challenge (no data → no users → no data)? How did they solve it?
- **EPA emissions monitoring analogy**: Continuous monitoring of agent behavior is like emissions monitoring. Research: what forced adoption of emissions monitoring? Was it regulation (Clean Air Act) or market demand? Timeline from voluntary to mandatory?
- **Insurance underwriting analogy**: Munich Re's aiSure is already offering agent liability coverage. Research: what data do insurers need to underwrite agent risk? How does reputation data reduce insurance premiums? This is a concrete revenue opportunity.
- Research: what narrative did Stripe use in their Series A (Patrick Collison, a16z, 2012)? What about Cloudflare's narrative (Matthew Prince, a16z, 2012)? Both sold "invisible infrastructure" — how did they make investors see value in something users never directly interact with?

## Direction 6: Mechanisms for Irreversible Market Position

What creates a position that competitors cannot replicate by copying features?

- **Data network effects**: Research which data network effects are truly defensible vs. which are "rate-of-learning, bootstrappable" (Towson's ~98% claim). Is on-chain knowledge with demurrage a stronger moat than standard data aggregation?
- **Standard-setting moats**: How did USB-IF, Wi-Fi Alliance, and Bluetooth SIG create standards that locked in their founding members? Can Nunchi do this with ERC-8004 + ERC-8183?
- **Two-sided marketplace bootstrapping**: Research how Uber, Airbnb, and DoorDash solved cold start. Does "compliance pull" (EU AI Act) + "cost pull" (10-30x reduction) create enough supply-side incentive to bootstrap without subsidies?
- **Regulatory capture (positive sense)**: How did Vanta become the SOC 2 standard? How did Plaid become embedded in financial regulation? Can Nunchi become the de facto Article 50 implementation?
- **Research: "switching costs by love" vs "switching costs by lock-in."** Which developer infrastructure companies achieved both? (Stripe: love = DX; lock-in = payment method vault. Vercel: love = preview URLs; lock-in = edge caching config.)

## Direction 7: Demo and Landing Page Optimization

The demo is the most important artifact. Make it undeniable.

- Research: what are the top 5 most effective developer tool demos of all time? (Stripe's 7-line curl example? Linear's homepage? Tailwind's utility-first code comparison?) What made them work?
- Is there a better demo format than split-terminal cost comparison? Research: are interactive calculators ("enter your agent spend, see your savings") more effective than fixed demos? What does the B2B SaaS demo research say?
- Research: "above the fold" landing page patterns for developer infrastructure. What specific metrics should be on the hero section? (Stripe shows "Millions of companies" + revenue processed. Temporal shows "9.1 trillion actions." What is Nunchi's equivalent?)
- Research: the "cost clock" pattern in real-time applications. Are there examples of developer tools that show money being saved in real time? What is the psychological impact of watching a number NOT go up?
- Research: how do compliance deadlines drive conversion? Vanta's sales velocity presumably increased as SOC 2 became table stakes. Is there data on conversion rate as a function of days-until-deadline?

## Direction 8: Untapped Business Models in Agent Infrastructure

What business models exist in this space that no one has tried?

- **Agent insurance underwriting**: Munich Re aiSure exists but is it underwriting based on behavioral data? Could Nunchi's reputation system provide the actuarial data that makes agent insurance viable? Research: what is the addressable market for AI liability insurance? HSB launched an SMB AI Liability policy March 2026 — what are the terms?
- **Agent SLA marketplace**: Could Nunchi enable agents to offer SLAs backed by staked tokens? Research: are there examples of token-staked SLAs in other domains (Chainlink, The Graph)? What do they earn?
- **Compliance-as-a-Service for agent deployments**: Instead of selling infrastructure, sell the compliance outcome. "Deploy through Nunchi and you're Article 50 compliant." Research: what does the compliance-as-a-service market look like? Drata, Secureframe, Vanta — what do they charge?
- **Agent talent marketplace**: Not just jobs for agents, but a way for companies to find, evaluate, and hire agents based on verified reputation. Research: does this exist? What would the take rate be? What are the comparables in human talent marketplaces (Toptal 30-40%, Upwork 10-20%)?
- **Knowledge licensing**: If Agent A publishes knowledge that Agent B uses to complete a $50K task, does Agent A deserve compensation? Research: knowledge licensing models in other domains. Patent licensing (qualcomm: 3-5% of device price). Academic journal access fees. Music licensing (Spotify: $0.003-0.005 per stream).

## Direction 9: What Casado Specifically Cares About (April 2026)

He is the target investor. What is he writing about, investing in, and tweeting about RIGHT NOW?

- Research: Martin Casado's last 10 public statements (blog posts, tweets, podcast appearances, conference talks) from March-April 2026. What themes recur?
- Research: his April 2025 "I don't see a lot of evidence we can close the control loop" skepticism vs. his March 2026 $43M Deeptune investment. What changed his mind? What does this tell us about what he is NOW looking for?
- Research: a16z infrastructure fund investment thesis as of Q1 2026. What are their stated investment criteria? What portfolio gaps exist?
- Research: Casado's OpenFlow/Nicira background. How does the SDN-to-cloud networking transition map onto the agent coordination market? Is there a "Nicira moment" for agent infrastructure?
- Research: Aubakirova's Big Ideas 2026 and her "Et Tu, Agent?" paper. What specific attack vectors does she highlight? How does Nunchi's architecture address each one?

## Direction 10: Pricing That Creates Category Lock-In

- Research: what pricing models create the strongest lock-in for infrastructure companies? Per-API-call (Stripe), per-seat (Atlassian), per-compute-unit (AWS), per-action (Temporal)?
- Research: how did Temporal price at Series A vs. now? What was the pricing evolution? Did they start free and add paid tiers, or start paid?
- Research: the "Helium model" for token-priced infrastructure. How has Helium's burn-and-mint actually performed post-HIP-141? What is the current MOBILE/IOT token economics reality?
- What is the right price point for agent identity registration? If there are 80-150K ERC-8004 agents, what would agents or their operators pay for verified identity? Research: domain name pricing ($10-15/year for .com), SSL certificate pricing ($0 via Let's Encrypt to $1500/year for EV), and API key management pricing (HashiCorp Vault, AWS Secrets Manager).
- Research: is there a "negative pricing" model that works? (Pay agents to register initially, then charge for premium features.) How did Uber's initial driver subsidies create supply? Could Nunchi subsidize early agent identity registration?

## Output Format

For each finding, provide:

1. **One-line verdict**: "Integrate now," "Reframe narrative," "Add to pitch," "Research deeper," "Deprioritize"
2. **The specific number or fact** that matters
3. **How it changes our narrative** — does it strengthen, weaken, or redirect the current positioning?
4. **The investor test** — would Martin Casado care about this? Would Malika Aubakirova? Would Chris Dixon?
5. **The catch** — what makes this finding unreliable, incomplete, or risky?

## Deliverables

1. **Executive summary**: Top 5 findings that change the pitch, ranked by impact
2. **Category definition recommendation**: The exact category name, with evidence for why it wins
3. **Narrative upgrades**: Specific sentences/paragraphs to add to 02-STRATEGY.md
4. **Demo improvements**: Changes to 08-DEMO.md and 09-LANDING-PAGE.md based on findings
5. **Business model additions**: New revenue streams or pricing models to add to 03-BUSINESS-MODEL.md
6. **NHI market validation**: Is $18.7B the right number? What is the realistic SAM?
7. **Co-evolution validation**: Any empirical data found, or should we cut the theoretical framing?
8. **Casado intelligence**: What to say in the first 30 seconds of the meeting
9. **Competitive updates**: Any new entrants or movements in the last 30 days
10. **Full citations**: arXiv IDs, conference venues, blog posts, funding announcements — with dates

## Priority Rubric

Weight findings by:
1. **Narrative impact** (does it make the pitch sharper?) — 30%
2. **Investor specificity** (does it address what Casado/Aubakirova/Dixon care about?) — 25%
3. **Empirical grounding** (is it backed by numbers, not just theory?) — 20%
4. **Differentiation** (can only Nunchi credibly claim this?) — 15%
5. **Freshness** (is it from the last 90 days?) — 10%
