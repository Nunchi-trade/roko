# Threat Model: 21 Failure Modes and Attack Trees

> **Layer**: Cross-cut (Safety & Provenance)
>
> **Crate**: All safety-relevant crates
>
> **Synapse traits**: All six traits have threat implications
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), [07-prompt-security.md](07-prompt-security.md)


> **Implementation**: Specified

---

## Overview

This document provides the structured threat model for the Roko framework. It defines adversary types, enumerates attack paths for both general-purpose agents and chain-domain agents, maps each to mitigating defense layers, and maintains a residual risk register for attacks that are not fully mitigated.

The threat model is domain-agnostic at its core — the same adversary taxonomy applies whether the agent is writing code, managing DeFi positions, or conducting research. Chain-domain-specific threats are documented separately in §4.

---

## 1. Attacker Taxonomy

### General-Purpose Agent Threats

| Adversary | Motivation | Capabilities | Examples |
|-----------|-----------|-------------|----------|
| **Prompt injector** | Redirect agent behavior | Indirect injection via tool results, file contents, API responses | Greshake et al. (2023) demonstrations |
| **Data poisoner** | Corrupt agent reasoning | Manipulate Neuro entries, inject false knowledge | AgentPoison (Chen et al., 2024), MINJA (Cheng et al., 2024), MemoryGraft (Li et al., 2024) |
| **Sandbox escaper** | Access files/systems outside worktree | Path traversal, symlink exploitation, shell escapes | CVE-2025-6514 (MCP RCE) |
| **Credential harvester** | Exfiltrate API keys and secrets | Induce agent to log/transmit credentials | Context window leakage vectors |
| **Resource exhaustor** | Deny service or increase costs | Trigger infinite loops, excessive API calls | Rate limit bypass, loop induction |
| **Supply chain attacker** | Compromise agent dependencies | Poisoned MCP servers, malicious skill packages | ClawHub campaign (335 malicious skills), Koi Security findings |

### Chain-Domain-Specific Threats

| Adversary | Motivation | Capabilities | Examples |
|-----------|-----------|-------------|----------|
| **Malicious vault creator** | Steal depositor funds | Full control of vault parameters at creation | Honeypot vaults, rug-pull via parameter changes |
| **Compromised manager agent** | Drain vault via unauthorized transactions | Prompt-injected or key-compromised agent | AIXBT hack ($106K), agent wallet compromise |
| **External MEV bot** | Extract value from agent operations | Mempool observation, sandwich attacks, JIT manipulation | Standard MEV on deposit/withdraw/rebalance |
| **Oracle manipulator** | Inflate NAV for profitable withdrawal | Flash loan price manipulation, oracle feed corruption | Flash-loan-based NAV inflation |
| **Knowledge poisoner** | Corrupt agent trading reasoning | Marketplace listing of poisoned strategies, collective infiltration | AgentPoison, MINJA attacks |
| **Coordination attacker** | Exploit multi-agent coordination | Colluding agents, malicious evaluators | ERC-8033 oracle manipulation |

---

## 2. General-Purpose Attack Trees

### 2.1 Prompt Injection

```
Goal: Execute unauthorized operations via LLM manipulation
|
+-- Path A: Indirect injection via tool results
|   +-- Inject via bash command output
|   |   Mitigated: ScrubPolicy scrubs secrets; BashPolicy blocks dangerous commands
|   +-- Inject via file contents
|   |   Mitigated: PathPolicy sandboxes reads to worktree; prompt architecture separates data from instructions
|   +-- Inject via API response
|       Mitigated: NetworkPolicy restricts destinations; response content is treated as data, not instructions
|
+-- Path B: Direct injection via task description
|   +-- Embed malicious instructions in task TOML
|   |   Mitigated: Task descriptions are system-controlled; operator reviews before execution
|   +-- Embed in PRD content that feeds task generation
|       Mitigated: PRD content passes through prompt composition with explicit UNTRUSTED markers
|
+-- Path C: Injection via knowledge store
    +-- Poison Neuro entries with instructional content
    |   Mitigated: 4-stage ingestion pipeline; HDC anomaly detection
    +-- Inject via peer agent messages in collective
        Mitigated: Reputation-weighted trust; consensus validation
```

### 2.2 Sandbox Escape

```
Goal: Access files or systems outside the agent's worktree
|
+-- Path A: Path traversal
|   +-- Use ../../ to escape worktree
|   |   Mitigated: PathPolicy canonicalizes and checks starts_with(worktree)
|   +-- Use absolute path outside worktree
|   |   Mitigated: PathPolicy checks absolute paths against worktree boundary
|   +-- Use symlink pointing outside worktree
|       Mitigated: PathPolicy deny_symlinks option (not enabled by default)
|
+-- Path B: Shell escape
|   +-- Embed path traversal in bash command arguments
|   |   Mitigated: BashPolicy deny patterns; PathPolicy on file tool arguments
|   +-- Use shell features (backticks, $(), pipes) to construct escaped paths
|       Mitigated: BashPolicy deny patterns catch common shell escapes
|
+-- Path C: MCP server exploitation
    +-- Use MCP server to access files outside worktree
    |   Mitigated: MCP tools pass through same SafetyLayer as built-in tools
    +-- Exploit MCP server vulnerability (CVE-2025-6514 class)
        Mitigated: MCP is optional; high-security deployments disable it entirely
```

### 2.3 Credential Exfiltration

```
Goal: Extract API keys, secrets, or sensitive data
|
+-- Path A: Via tool output
|   +-- Read .env file containing secrets
|   |   Mitigated: ScrubPolicy regex patterns detect and redact secrets in tool output
|   +-- Read config file with embedded credentials
|       Mitigated: ScrubPolicy covers common credential formats
|
+-- Path B: Via network
|   +-- Exfiltrate via web_fetch to attacker-controlled server
|   |   Mitigated: NetworkPolicy allowlists, HTTPS-only, private network blocking
|   +-- Encode secrets in URL parameters or headers
|       Mitigated: NetworkPolicy host filtering
|
+-- Path C: Via git
|   +-- Commit secrets to repository
|   |   Mitigated: ScrubPolicy detects secrets; GitPolicy blocks force-push to protected branches
|   +-- Push to unauthorized remote
|       Mitigated: GitPolicy restricts git operations; operator controls remote configuration
```

### 2.4 Resource Exhaustion

```
Goal: Deny service or inflate costs
|
+-- Path A: Infinite tool call loop
|   +-- Retry loop on failing command
|   |   Mitigated: RateLimiter (60 calls/60s per role+tool); circuit breaker
|   +-- Recursive file processing
|       Mitigated: RateLimiter; PathPolicy prevents escape; max command length
|
+-- Path B: Token budget exhaustion
|   +-- Generate enormous prompts
|   |   Mitigated: Composer budget constraints; token limits in agent configuration
|   +-- Trigger many inference calls
|       Mitigated: RateLimiter; conductor circuit breaker
|
+-- Path C: Disk exhaustion
    +-- Write extremely large files
    |   Mitigated: Result truncation (DEFAULT_MAX_RESULT_BYTES: 16,384); worktree isolation
    +-- Create many worktrees
        Mitigated: WorktreeConfig max_worktrees limit; idle TTL cleanup
```

---

## 3. Residual Risk Register

Attacks that are NOT fully mitigated by current defenses:

| Risk ID | Description | Severity | Current Mitigation | Residual Exposure |
|---------|-------------|----------|-------------------|-------------------|
| RR-1 | Prompt injection bypass | High | SafetyLayer checks + prompt architecture | Sophisticated injections may bypass regex-based detection |
| RR-2 | Novel bash escape patterns | Medium | BashPolicy deny patterns | New shell escape techniques not in deny list |
| RR-3 | Symlink-based sandbox escape | Medium | PathPolicy (deny_symlinks off by default) | Default config allows symlinks within worktree |
| RR-4 | Credential encoding (base64, split across calls) | Medium | ScrubPolicy regex patterns | Encoded or split secrets evade pattern matching |
| RR-5 | MCP server zero-day | High | SafetyLayer + optional MCP disabling | Unknown vulnerabilities in MCP server implementations |
| RR-6 | LLM capability escalation | Low | ToolPermission + task-level filters | Future LLMs may find novel workarounds |
| RR-7 | Knowledge poisoning via novel attack | Medium | 4-stage ingestion pipeline | New attack classes not in adversarial test suite |
| RR-8 | Timing side-channel (inference calls reveal reasoning) | Low | None currently | Request patterns reveal agent activity |

---

## 4. Chain-Domain Attack Trees

These apply only to agents using the `roko-chain` crate for on-chain operations.

### 4.1 Compromised Manager Agent (Chain Domain)

```
Goal: Execute unauthorized transactions
|
+-- Path A: Direct key compromise
|   +-- Steal agent's private key
|   |   Mitigated: TEE key management; keys never in agent memory
|   +-- Compromise TEE environment
|       Mitigated: TEE is defense-in-depth; time-delayed proxy
|
+-- Path B: Prompt injection → unauthorized trades
|   +-- Inject via tool response
|   |   Mitigated: Tool Integrity Verification (96% detection)
|   +-- Inject via data feed
|       Mitigated: Data/decision separation (CaMeL dual-LLM design target)
|
+-- Path C: Authorized but harmful operations
    +-- Execute trades with excessive slippage
    |   Mitigated: Pre-flight simulation; on-chain slippage caps (PolicyCage)
    +-- Drain via many small transactions under limits
        Mitigated: Per-day aggregate caps; continuous dampening
```

### 4.2 Oracle Manipulation (Chain Domain)

```
Goal: Manipulate NAV for profitable deposit/withdrawal
|
+-- Path A: Flash-loan price manipulation
|   Mitigated: No-spot-assumptions policy; TWAP validation (10-30 min window)
|
+-- Path B: Oracle staleness exploitation
|   Mitigated: Staleness gates widen spreads; 100% staleness disables NAV pricing
|
+-- Path C: NAV inflation via donation
    Mitigated: Internal asset accounting (not balanceOf); virtual shares offset
```

---

## 5. Formal Safety Analysis

### Maximum Loss Bounds (Chain Domain)

Under any single-layer failure:
```
max_loss = min(sessionKeyLimit, maxRebalanceSizeBps × TVL, dailyAggregateCap)
```

Under simultaneous multi-layer failure (estimated probability < 10⁻⁶ per year), the circuit breaker hierarchy provides automated intervention.

### Attack Cost Economics

| Attack Vector | Min Cost | Detection Time | Primary Defense |
|--------------|---------|----------------|-----------------|
| Prompt injection | $0 | < 1 second | SafetyLayer + prompt architecture |
| Sandbox escape | $0 | < 1 second | PathPolicy canonicalization |
| Credential theft | $0 | < 1 second | ScrubPolicy regex scrubbing |
| TEE compromise (chain) | ~$50 (van Bulck et al., 2026) | Varies | Time-delayed proxy |
| Knowledge poisoning | $0 | 1-6 hours | 4-stage ingestion pipeline |
| MEV extraction (chain) | $10K+ | < 5 seconds | Flashbots Protect, slippage bounds |

### Cross-Protocol Contagion (Chain Domain)

Two metrics from the literature:

**DeFi Correlation Fragility Indicator (CFI)** (Zhang et al., 2026): Measures protocol TVL correlations using a sliding window. When CFI is elevated, circuit breaker thresholds tighten from 13%/7%/3% to 10%/5%/2%.

**Aggregated Systemic Risk Index (ASRI)** (Farzulla et al., 2026): Combines on-chain protocol metrics with traditional market indicators. Detected historical crises with 18-day lead time in backtesting. When ASRI exceeds threshold (default 0.7), automatic deallocation from most correlated adapters.

### Conformal Prediction for Value at Risk (Chain Domain)

The risk engine uses conformal prediction for VaR estimation, providing distribution-free coverage guarantees. Unlike parametric VaR (assumes normal returns) or historical simulation (requires stationary distributions), conformal prediction provides valid coverage regardless of the underlying distribution. This matters because DeFi return distributions are heavy-tailed and non-stationary.

References: Fantazzini (2024) for crypto-asset VaR calibration; Kato (2024, arXiv:2410.16333) for portfolio-level conformal prediction.

---

## 6. Empirical Calibration

Validated against Zhou et al.'s DeFi exploit database (181 attacks through 2023):

| Attack Category | Defense | Coverage |
|-----------------|---------|----------|
| Oracle manipulation | Multi-oracle, TWAP validation | Strong |
| Access control | On-chain guards, TEE policy engine | Strong |
| Reentrancy | Checks-Effects-Interactions pattern | Strong |
| Logic errors | Pre-flight simulation, post-trade verification | Partial (residual risk) |
| Flash loan attacks | Virtual shares offset, internal asset accounting | Strong |

---

## Academic References

| Paper | Contribution |
|-------|-------------|
| Omohundro (2008) | Instrumental convergence — why AI systems develop unsafe drives |
| Turner et al. (2021) | Mathematical proof of resource-seeking optimal policies |
| Greshake et al. (2023) | Indirect prompt injection demonstrations |
| Chen et al. (2024) | AgentPoison — optimized backdoor triggers for RAG-based agents |
| Cheng et al. (2024) | MINJA — adversarial injection through normal interactions |
| Li et al. (2024) | MemoryGraft — gradual behavioral drift via subtle memory bias |
| Zhou et al. (2023) | DeFi exploit database — 181 attacks catalogued |
| van Bulck et al. (2026) | TEE hardware attacks at ~$50 cost |
| Zhang et al. (2026) | DeFi Correlation Fragility Indicator |
| Farzulla et al. (2026) | Aggregated Systemic Risk Index |
| Fantazzini (2024) | Crypto-asset VaR calibration |
| Kato (2024, arXiv:2410.16333) | Portfolio-level conformal prediction |
| Yang et al. (2025) | R2AI safety framework |
| Woolley et al. (Science 330(6004), 2010) | Collective intelligence foundations |

---

## Related Topics

- [00-defense-in-depth.md](00-defense-in-depth.md) — Six safety guards
- [07-prompt-security.md](07-prompt-security.md) — Prompt injection defenses in detail
- [09-adaptive-risk.md](09-adaptive-risk.md) — Adaptive guardrails
- [10-mev-protection.md](10-mev-protection.md) — MEV-specific protections
