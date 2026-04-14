# Mechanism Design and Attention Economics

> Academic foundations for VCG auctions, reputation systems, incentive-compatible mechanisms, and attention allocation in Roko's context bidding and agent marketplace.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md)
**Key sources**: `bardo-backup/prd/shared/citations.md` §34, `refactoring-prd/09-innovations.md` §II

> **Implementation**: Reference

---

## Abstract

Roko's VCG Attention Auction allocates limited context budget through incentive-compatible bidding. The Vickrey-Clarke-Groves mechanism guarantees truthful bidding: subsystems can't game the auction by inflating bids because they pay the second price. This section collects the mechanism design foundations, including auction theory, reputation systems, and the economic theory of attention.

---

## VCG Mechanism

- Vickrey, W. (1961). Counterspeculation, Auctions, and Competitive Sealed Tenders. _Journal of Finance_, 16(1), 8-37.
  *Grounds: Second-price auctions — the Vickrey auction: bidders submit sealed bids, the highest bidder wins but pays the second-highest price. Guarantees truthful bidding. Foundational for the VCG attention auction.*

- Clarke, E.H. (1971). Multipart Pricing of Public Goods. _Public Choice_, 11(1), 17-33.
  *Grounds: Multi-item pricing — extends truthful mechanisms to public goods with multiple items. Enables simultaneous allocation of multiple context sections.*

- Groves, T. (1973). Incentives in Teams. _Econometrica_, 41(4), 617-631.
  *Grounds: Team incentives — truthful revelation in team settings. Completes the VCG mechanism: subsystems truthfully reveal their valuation of context sections.*

---

## Auction Theory

- Milgrom, P. (2004). _Putting Auction Theory to Work_. Cambridge University Press.
  *Grounds: Applied auction theory — comprehensive treatment of auction design for practical applications. Provides the theoretical foundation for implementing the attention auction efficiently.*

- Duetting, P. et al. (2024). Mechanism Design for LLMs. _ACM WWW_, 2024. arXiv:2310.10826.
  *Grounds: LLM mechanism design — applies mechanism design specifically to LLM systems. Validates applying auction theory to LLM context allocation.*

---

## Algorithmic Game Theory

- Nisan, N., Roughgarden, T., Tardos, E., & Vazirani, V.V. (2007). _Algorithmic Game Theory_. Cambridge University Press.
  *Grounds: AGT reference — comprehensive treatment of computational aspects of game theory. Provides algorithms for implementing VCG efficiently.*

---

## Submodular Optimization

- Nemhauser, G.L., Wolsey, L.A., & Fisher, M.L. (1978). An Analysis of Approximations for Maximizing Submodular Set Functions. _Mathematical Programming_, 14(1), 265-294.
  *Grounds: Greedy context selection — submodular function maximization with greedy algorithm providing (1-1/e) approximation guarantee. Context selection is submodular: adding more context has diminishing returns. The greedy algorithm provides near-optimal context assembly.*

---

## Reputation Systems

- Glickman, M.E. (1999). Parameter Estimation in Large Dynamic Paired Comparison Experiments. _Journal of the Royal Statistical Society, Series C_, 48(3), 377-394.
  *Grounds: Glicko-2 — dynamic rating system that accounts for rating reliability (RD) and volatility. Grounds the reputation tracking in ERC-8004 agent identity, where reputation is not just a number but includes confidence intervals.*

- Meritrank (2022). Nasrulin, B. et al. MeritRank: Sybil Tolerant Reputation. arXiv:2207.09950.
  *Grounds: Sybil-tolerant reputation — reputation system resistant to Sybil attacks. Informs the on-chain reputation registry design.*

- Soulbound Tokens (2022). Weyl, E.G., Ohlhaver, P., & Buterin, V. Decentralized Society: Finding Web3's Soul. _SSRN_.
  *Grounds: Non-transferable identity — soulbound tokens as non-transferable identity primitives. Grounds the Korai Passport (ERC-721 soulbound) agent identity.*

---

## Vickrey Reputation-Adjusted Auction

- Reputation-adjusted scoring: `s_i = p_i × (1 + (1 - R_i))`. Payment = `s_second / (1 + (1 - R_winner))`.
  *Grounds: Spore/Sparrow marketplace — the Vickrey reputation-adjusted auction for the agent job market. Agents with higher reputation can bid lower and still win, creating positive incentives for reputation building.*

---

## Demurrage and Token Economics

- Gesell, S. (1916). _The Natural Economic Order_.
  *Grounds: Demurrage — money that decays over time to encourage circulation. KORAI's 1% annual demurrage mirrors Engram half-life: both knowledge tokens and knowledge entries decay, preventing hoarding and ensuring freshness.*

- Ostrom, E. (1990). _Governing the Commons_. Cambridge University Press.
  *Grounds: Commons governance — principles for governing shared resources without central authority. Informs the governance of the shared knowledge commons in Agent Mesh.*

---

## Knowledge Markets

- Williamson, O.E. (1979). Transaction-Cost Economics. _Journal of Law and Economics_.
  *Grounds: Transaction costs — institutional structures emerge to minimize transaction costs. The Agent Mesh reduces knowledge sharing transaction costs through standardized Engram formats and HDC similarity search.*

- Bakos, Y. & Brynjolfsson, E. (1999). Bundling Information Goods. _Management Science_.
  *Grounds: Information bundling — economics of bundling information goods. Informs the design of knowledge bundles for collective sharing.*

---

## Agent Marketplaces and AI Economies (2024-2025)

- Agent Exchange (AEX) (2025). Agent Exchange: Shaping the Future of AI Agent Economics. arXiv:2507.03904.
  *Grounds: Agent marketplace architecture — auction engine inspired by Real-Time Bidding for agent task allocation. Four ecosystem components: User-Side Platform, Agent-Side Platform, Agent Hubs for team coordination, and Data Management Platform for knowledge sharing. Directly informs Roko's agent job marketplace design.*

- Duetting, P. et al. (2024). Mechanism Design for Large Language Models. _ACM WWW Best Paper_, 2024. arXiv:2310.10826.
  *Grounds: LLM mechanism design — token auction model operating on a token-by-token basis for joint output generation through multiple LLM agents. Validates applying auction theory to LLM-level resource allocation, extending VCG from context to token granularity.*

- Deep Mechanism Design (2024). Learning Social and Economic Policies for Human Benefit. _PNAS_, 2024.
  *Grounds: Neural mechanism design — deep neural networks trained with RL create desirable mechanisms for multi-agent coordination. Validates the feasibility of learning optimal allocation mechanisms rather than hand-designing them.*

---

## Cross-References

- See [07-context-engineering.md](./07-context-engineering.md) for VCG in context assembly
- See [22-protocol-standards.md](./22-protocol-standards.md) for ERC-8004 and token standards
- See [10-market-microstructure.md](./10-market-microstructure.md) for DeFi-specific mechanisms
