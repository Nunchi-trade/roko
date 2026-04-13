# Market Microstructure and DeFi Theory

> Academic foundations for automated market making, liquidity provision, vault mechanisms, and DeFi protocol design relevant to Roko's chain domain plugin.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md)
**Key sources**: `bardo-backup/prd/shared/citations.md` §§1-2, §§14-20

> **Implementation**: Reference

---

## Abstract

Roko is domain-agnostic, but its first domain plugin is DeFi. This section collects the market microstructure research that grounds the chain agent's tools and strategies. These citations are chain-domain-specific — they inform the tools in the chain plugin, not the core cognitive architecture. The AMM/LP theory, vault mechanisms, and MEV protection research are preserved here for completeness.

---

## AMM and LP Theory

- Milionis, J., Moallemi, C., Roughgarden, T., & Zhang, A. (2023). Automated Market Making and Loss-Versus-Rebalancing. _Journal of Financial Economics_.
  *Grounds: LVR — formalizes loss-versus-rebalancing as the dominant cost of passive LP in AMMs.*

- Adams, H. et al. (2024). UniswapX: Aggregating Automated Market Makers. Uniswap Labs.
  *Grounds: MEV protection — Dutch auction order protocol for MEV-protected swap execution.*

- Adams, H. et al. (2025). am-AMM: Auction-Managed Automated Market Maker. Uniswap Research.
  *Grounds: Auction-managed AMM — winning bidder controls pool fees and captures arbitrage.*

- Hasbrouck, J., Rivera, T.J., & Saleh, F. (2025). Economic Model of DEX with Concentrated Liquidity. _Management Science_.
  *Grounds: Concentrated liquidity economics — formal economic model of DEX with concentrated liquidity provision.*

---

## Vault Mechanisms

- Ethereum Foundation (2022). ERC-4626: Tokenized Vault Standard. EIPs.
  *Grounds: Vault interface — industry-standard deposit/withdraw/share accounting interface.*

- Ethereum Foundation (2023). ERC-7265: Circuit Breaker Standard. EIPs.
  *Grounds: Vault circuit breakers — rate-limiting mechanism for DeFi protocols.*

- Ethereum Foundation (2023). ERC-7540: Asynchronous Redemption Vaults. EIPs.
  *Grounds: Async redemptions — extends ERC-4626 with request/claim lifecycle for delayed withdrawals.*

---

## Risk and Decision Theory (Financial)

- Kelly, J.L. Jr. (1956). A New Interpretation of Information Rate. _Bell System Technical Journal_, 35(4), 917-926.
  *Grounds: Kelly criterion — optimal bet sizing as information rate. Foundational for position sizing in financial agents.*

- Roy, A.D. (1952). Safety First and the Holding of Assets. _Econometrica_, 20(3), 431-449.
  *Grounds: Safety-first principle — portfolio optimization subject to a safety constraint (max probability of loss below threshold).*

- Peters, O. (2019). The Ergodicity Problem in Economics. _Nature Physics_, 15(12), 1216-1221.
  *Grounds: Ergodicity economics — the distinction between ensemble average and time average returns. Log-wealth maximization (Kelly) is optimal for individual agents in non-ergodic settings.*

- Taleb, N.N. (2012). _Antifragile: Things That Gain from Disorder_. Random House.
  *Grounds: Convex response — antifragility as gaining from disorder. Systems with convex response to stressors improve under volatility. Grounds the antifragility design principle (reframed from death to challenge).*

- Taleb, N.N. & Douady, R. (2013). Mathematical Definition, Mapping, and Detection of (Anti)Fragility. _Quantitative Finance_, 13(11), 1677-1689.
  *Grounds: Formal antifragility — mathematical definition of fragility as sensitivity to perturbation of the probability distribution.*

---

## Cross-References

- See [21-mechanism-design.md](./21-mechanism-design.md) for Vickrey auctions and VCG mechanism
- See [22-protocol-standards.md](./22-protocol-standards.md) for ERC standards
- See [11-streaming-algorithms.md](./11-streaming-algorithms.md) for online statistics
