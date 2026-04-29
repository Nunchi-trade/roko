# Deep Research Prompt — Round 7 (Execution Intelligence)

Copy everything below the `---` line into Claude Desktop with deep research enabled.

---

## Research brief: execution-specific intelligence for an agent-protocol company with a purpose-built blockchain

Six prior rounds established the technical stack (R1-R2), frontier capabilities (R3), strategic positioning (R4), production reality (R5), and Series A intelligence for a16z (R6). We now know the pitch ("Stripe for the agent economy"), the partner (Casado via Aubakirova), the demo (3-minute cost comparison with kill-the-worker), the comp ($15-35M at $150-250M post), and the window (6-12 months).

**This round focuses on execution specifics we need to act on in the next 30-60 days.**

### What's decided (treat as given)

- **Protocol**: Signal/Block/Graph with 9 protocols, 10 specializations. Two mediums (Signal durable, Pulse ephemeral). Every operator is a learner via predict-publish-correct. Demurrage replaces Ebbinghaus decay. HDC 10,240-bit fingerprints as universal router fabric.

- **Blockchain**: The **Nunchi blockchain** — purpose-built EVM chain with Simplex consensus and 50ms block times for AI agent coordination. Native HDC precompile at ~400 gas. Soulbound agent passports (ERC-721) with ventriloquist defense. Demurrage token economics (1% annual decay). 7-domain EMA reputation. Spore job marketplace. ISFR clearing with KKT certificates. ZK-HDC proofs (Bionetta/UltraGroth) for trustless capability attestation.

- **Positioning**: "Stripe for the agent economy." Cost-reduction wedge (10-30×), chain as main differentiator, protocol-not-framework framing. Lead with cost, expand to identity + knowledge + self-evolution.

- **Fundraise**: Series A targeting $20-30M at $200-400M post. Casado (a16z infra) as lead. Aubakirova as entry. Dixon/Yahya for on-chain. MCP playbook for launch.

- **Exoskeleton**: MCP (tools) + ACP (editors) + A2A (agent discovery) + ERC-8004 (identity) + x402 (payments).

### What I need to know now

#### Direction 1: Purpose-built blockchain for AI agents — competitive landscape and positioning

The Nunchi chain is a major differentiator but also a major complexity. I need the full competitive picture.

- What agent-specific blockchains exist or are being built? (ChaosChain, Theoriq, any others?)
- How is Olas positioning its on-chain agent coordination? What's their architecture?
- What's Bittensor's current state? Is dTAO working? Real subnet economics?
- What's VERSES doing with on-chain active inference? Any shipping product?
- **Simplex consensus** — what is it? Who developed it? What are the performance characteristics? How does it compare to Tendermint/HotStuff/Narwhal-Tusk? Any production deployments?
- **50ms block times** — what chains achieve sub-100ms finality? (Solana ~400ms, Monad targets 1s, Sei ~390ms) What are the engineering challenges at 50ms? Is this credible?
- **Custom EVM chains** — what's the state of reth/revm forks for custom EVM chains? Developer experience? Tooling maturity?
- How do you pitch a purpose-built blockchain to non-crypto investors (Casado)? What framing worked for Story Protocol?
- What's the regulatory posture for an agent-specific chain? Different from general-purpose L1?
- **HDC precompile** — has anyone else built custom EVM precompiles for AI/ML operations? What's the precedent?
- What token models have worked for infrastructure protocols? (Not speculation — utility tokens that retained value)
- **Demurrage tokens** — historical examples? Freicoin? Gesell's stamp money? Any modern implementations? Did they work?
- What's the current thinking on dual-structure fundraises (equity + token warrant)?

#### Direction 2: The 3-minute demo — how to build a killer cost comparison

Research6 prescribed a specific demo: side-by-side terminal panes, naive LangChain loop ($4.18) vs protocol ($0.14), kill-the-worker recovery. I need to know how to build this credibly.

- What does a realistic LangChain cost benchmark look like? What's the actual cost-per-task for common agent workloads?
- **SWE-bench Pro cost data** — what do current agents actually spend per resolved issue? (Cursor, Claude Code, Devin, OpenHands)
- How did Temporal build their "kill the worker" demo? What made it memorable?
- What benchmarks does Princeton HAL use for dual-axis (cost × accuracy) plotting?
- How do you make a live demo reliable? (Pre-record fallback? Staged environment? Both?)
- What agent cost comparison frameworks exist? (syftr, HAL, SWE-rebench)
- **Model pricing as of today** — exact pricing for Claude Opus 4.7, Sonnet 4.6, Haiku 4.5, GPT-5.5, DeepSeek V4, Gemini 3 Flash/Pro, Grok 4.1
- How much does prompt caching actually save in real agent loops? (Not FAQ chatbots — agent tool-calling loops)
- What's the cheapest way to run the "naive baseline" that's still credible? (Can't cherry-pick a bad config)

#### Direction 3: First 5 enterprise design partners — who and how

Research6 says "5 anchor partners with public quotes before launch" and "forward-deployed engineering at first enterprise traction."

- What enterprise companies are actively deploying multi-agent systems and hitting coordination failures?
- Which companies have publicly discussed agent cost problems? (Blog posts, conference talks, social media)
- What industries are furthest along in agent adoption? (Legal: Harvey. Support: Sierra. Coding: Cursor. What else?)
- How did Temporal get Snap, Netflix, JPMorgan as early customers? What was the sales motion?
- How did Stripe get its first 7 beta users? What made them willing to use pre-launch infrastructure?
- What's the "forward-deployed engineering" model? (Sierra embeds engineers. Harvey dedicates 10% to ex-lawyer CS. What's the pattern?)
- For an agent-protocol company, what's the right first customer profile? (Big tech? YC startup? Enterprise? Open-source project?)
- What integration effort does an enterprise design partner expect? (Days? Weeks? Dedicated engineer?)

#### Direction 4: SDK strategy — TypeScript and Python on day one

The MCP playbook requires two SDKs. What should they look like?

- What made MCP's TypeScript and Python SDKs successful? (API design, documentation, examples)
- What's the minimum viable SDK for an agent protocol? (Client only? Client + server? Full framework?)
- How do Temporal, LangGraph, and CrewAI structure their SDKs? What do developers praise and complain about?
- **TypeScript vs Python adoption** — which should be higher-quality at launch? (Octoverse 2025: TS is #1 language)
- What testing infrastructure do SDK launches need? (Conformance tests? Integration tests? Example repos?)
- How do you version a protocol SDK before the protocol is stable? (Pre-1.0 semantics)
- What's the documentation pattern? (Stripe three-column? Temporal tutorials? Both?)

#### Direction 5: Academic publication and credibility

Research6 recommends arXiv → NeurIPS workshop → OSDI main track → CACM. I need specifics.

- What's the right framing for an academic paper about Signal/Block/Graph? (Systems? PL? AI? HCI?)
- Which NeurIPS 2026 workshops accept agent infrastructure papers? Submission deadlines?
- **OSDI 2027** — submission deadline? What kind of paper gets in? (Needs eval section?)
- Is there precedent for a protocol spec being published as an academic paper? (Raft? MapReduce? Ethereum yellow paper was not peer-reviewed initially)
- What academic collaborators would strengthen the paper? (Which labs work on agent coordination, HDC, active inference?)
- How did the Raft paper's "user study" approach work for credibility? Could we do something similar?
- Should the HDC + active inference + stigmergy combination be its own paper? Or part of the systems paper?

#### Direction 6: Agent identity as a market (KYA thesis deep dive)

The a16z crypto April 16 post named KYA (Know Your Agent) as a missing primitive. The Nunchi passport IS KYA.

- What's the current state of agent identity standards beyond ERC-8004?
- How is Catena Labs (a16z-funded) approaching agent identity? Architecture? Product?
- What's Keycard (a16z-funded) doing? How does it relate?
- **Non-human identity in financial services** — the "96-to-1" ratio. What specific problems does this create?
- How do merchants currently block or rate-limit agents? What would KYA solve?
- Is there an ISO or W3C standard emerging for machine identity?
- How does agent identity interact with EU AI Act Article 50 transparency requirements?
- What's the market size for "agent identity infrastructure"?
- How does Nunchi's soulbound passport + ventriloquist defense compare to alternatives?

#### Direction 7: ZK-HDC and trustless capability attestation

ZK proofs over HDC vectors (Bionetta/UltraGroth: 320-byte proof, ~250K gas verification, <2 min proving on smartphone) let agents prove capability without revealing knowledge. This is the cryptographic primitive that makes the Nunchi passport economically meaningful.

- What's the current state of ZK-ML and ZK proofs over binary operations? (Bionetta, EZKL, Risc0, SP1, Jolt)
- **ZK-HDC precompile**: has anyone built a custom ZK verifier precompile in an EVM chain? What's the gas cost?
- How would a `hdc_xor_popcnt` SP1 precompile work? Estimated 1-2 engineer-months — is this realistic?
- What's the proving time for 10,240-bit Hamming distance in current ZK systems? Can it hit <10ms?
- **Enterprise demand for verifiable agent credentials** — what specific use cases require "prove you can do X without showing how"?
- How does ZK-HDC compare to alternatives for agent capability attestation? (TEE attestation, credential networks, reputation alone)
- What's the market for verifiable AI credentials? (ISO 42001, EU AI Act conformity assessment — could ZK-HDC serve compliance?)
- **Fuzzy PSI over HDC vectors** — for private stigmergy (agents discover similar knowledge without revealing it). Current state of VOLE-based Fuzzy PSI?

#### Direction 8: Demurrage economics — will tokens that decay actually work?

Nunchi uses 1% annual demurrage. This is unconventional. I need the evidence.

- **Freicoin** — what happened? Why did it fail? Lessons?
- **Chiemgauer** (German regional currency) — the most successful demurrage experiment. What worked?
- **Gesell's stamp money** — the theoretical foundation. Is it sound?
- Have any blockchain tokens implemented demurrage? What were the results?
- What do token economists say about inflationary vs demurrage models for utility tokens?
- How do you explain "your tokens lose value by design" to investors without scaring them?
- What's the game-theoretic equilibrium for a demurrage token used for agent knowledge?
- Does 1% annual demurrage change behavior enough to matter? What rate is optimal?

#### Direction 9: Building the developer community from zero

No existing community. How do you seed one?

- How did MCP go from 0 to active community in 13 months? What channels? What content?
- What's the Discord vs GitHub Discussions vs forum tradeoff for protocol communities?
- **"Awesome lists"** and ecosystem pages — what drives early contribution?
- How do you get the first 10 community-built integrations?
- What developer content works? (Tutorials? Live coding? Conference talks? Blog posts?)
- What's the role of a developer advocate at this stage?
- How do you measure developer community health? (Stars? PRs? Discord messages? Something better?)
- What mistakes do protocol communities make in the first year?

#### Direction 10: What's happening RIGHT NOW that affects execution

- Latest Simplex consensus research and implementations
- Custom EVM chain tooling (reth/revm forks, deployment patterns)
- Latest ERC-8004 adoption data (registrations, active agents, x402 volume)
- Any new agent-blockchain projects announced
- Latest MCP/ACP/A2A protocol updates
- Agent infrastructure funding rounds in the last 30 days
- Any enterprise agent coordination case studies published recently
- EU AI Act implementation guidance updates
- Latest HDC/VSA research or commercial applications

### Evaluation criteria

For each finding:
1. **Execution impact**: Does this change what we do this week / this month / this quarter?
2. **Evidence quality**: Production data? Academic? Vendor marketing?
3. **Time sensitivity**: Window closing? Standard solidifying? Competitor moving?
4. **Cost to act**: Engineering effort, calendar time, hiring needs
5. **Risk if we don't act**: What happens if we ignore this finding?

### Output format

1. **The 30-day action list**: Top 10 things to do in the next 30 days, ranked by impact
2. **Per-direction sections**: Findings with 5-point evaluation
3. **The demo blueprint**: Specific technical plan for the 3-minute cost comparison demo
4. **Enterprise prospect list**: 10 specific companies that match the design-partner profile, with reasoning
5. **Token economics analysis**: Honest assessment of demurrage viability with historical evidence
6. **Community seeding playbook**: Week-by-week for the first 90 days of community building
7. **Full citations** with dates, venues, repos

Prioritize:
- Things we can act on this week
- Evidence from companies at our stage that succeeded (or failed)
- Specific names, companies, and contacts where possible
- Honest negative results about our unconventional choices (demurrage, HDC, active inference)
- Competitive intelligence on agent-blockchain specifically
