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
| NIST AI RMF 1.0 (January 2023) | AI Risk Management Framework — GOVERN, MAP, MEASURE, MANAGE functions |
| NIST AI 600-1 (July 2024) | Generative AI Profile — 12 risk categories for foundation models |
| MITRE ATLAS v5.4.0 (February 2026) | Adversarial threat landscape for AI systems — 16 tactics, technique library |
| Mauri & Damiani (Sensors 22(17):6662, 2022) | STRIDE-AI — threat classification adapted for machine learning systems |
| CSA MAESTRO (February 2025) | Multi-Agent Environment Security Threat and Risk Operations — 7-layer framework |
| Russinovich et al. (arXiv:2404.01833, 2024) | Crescendo — multi-turn jailbreak via gradual escalation |
| Pavlova et al. (arXiv:2410.01606, 2024) | GOAT — automated multi-turn red teaming for LLMs |
| Chao et al. (arXiv:2310.08419, 2023) | PAIR — prompt automatic iterative refinement for jailbreaks |
| Mehrotra et al. (arXiv:2312.02119, 2023) | TAP — tree of attacks with pruning for automated jailbreaking |
| Kim et al. (arXiv:2501.18575, 2025) | PFI — prompt flow injection and privilege escalation in LLM agents |
| Zhang et al. (arXiv:2412.14470, 2024) | Agent-SafetyBench — benchmark for safety evaluation of LLM agents |
| Debenedetti et al. (arXiv:2406.13352, 2024) | AgentDojo — dynamic benchmark for LLM agent exploitation |
| Chao et al. (arXiv:2404.01318, 2024) | JailbreakBench — standardized benchmark for jailbreak defenses |

---

## 7. NIST AI RMF alignment

The NIST AI Risk Management Framework (AI RMF 1.0, January 2023) defines four core functions for managing AI risk: GOVERN, MAP, MEASURE, and MANAGE. The NIST AI 600-1 Generative AI Profile (July 2024) extends this framework with 12 risk categories specific to foundation models. Roko's safety architecture maps directly to both.

### RMF function mapping

| RMF Function | Purpose | Roko Components | Enforcement Point |
|---|---|---|---|
| **GOVERN** | Establish policies, roles, accountability | Safety policies in `roko.toml`, role-based permissions (`RolePermissions`), `CLAUDE.md` rules, operator-defined tool filters | Configuration load at agent startup |
| **MAP** | Identify and categorize AI risks | This threat model, attack surface enumeration (sections 1-4), MITRE ATLAS mapping (section 8), STRIDE-AI classification (section 9) | Design-time analysis, updated per release |
| **MEASURE** | Quantify risk through metrics | Gate pipeline pass rates, efficiency events (`.roko/learn/efficiency.jsonl`), adaptive thresholds (EMA per rung), `OperationalConfidenceTracker` | Continuous during agent execution |
| **MANAGE** | Respond to identified risks | `SafetyLayer` enforcement, circuit breaker intervention (`CircuitBreaker`), Cognitive Signals for runtime anomalies, automated pause/shutdown | Real-time per tool call and per task |

### NIST AI 600-1 risk categories

The Generative AI Profile defines 12 risk categories. Each maps to one or more Roko defense mechanisms:

| # | NIST Risk Category | Roko Mitigation | Residual Gap |
|---|---|---|---|
| 1 | **Confabulation** | Gate pipeline verification against ground truth; multi-gate validation (compile, test, clippy, diff); post-task output auditing | Subtle confabulations that pass gate checks |
| 2 | **CBRN information** | `BashPolicy` deny patterns block dangerous commands; `NetworkPolicy` prevents access to restricted domains; content filtering in `ScrubPolicy` | Novel encoding bypasses |
| 3 | **Data privacy** | `TaintedString` propagation tracking; `ScrubPolicy` redaction (regex-based secret detection); Cognitive Namespaces for isolation | Encoded or split secrets evade pattern matching (see RR-4) |
| 4 | **Environmental impact** | Token budget constraints; `RateLimiter` (60 calls/60s); CascadeRouter routes to smallest sufficient model; efficiency event logging | No direct carbon tracking |
| 5 | **Harmful bias** | Prompt composition with explicit role constraints; operator-controlled task descriptions; gate pipeline rejects outputs violating policy | Bias in upstream model not detectable at agent layer |
| 6 | **Human-AI configuration** | Operator-in-the-loop for destructive git ops (`CLAUDE.md` rules); task-level approval gates; `ProcessSupervisor` lifecycle management | Agents may act within policy but against operator intent |
| 7 | **Information integrity** | 4-stage Neuro ingestion pipeline; HDC anomaly detection; reputation-weighted trust in collectives | Novel poisoning attacks (see RR-7) |
| 8 | **Information security** | `SafetyLayer` pre/post checks on every tool call; `Capability<T>` tokens for authorization; `PathPolicy` sandbox enforcement | Zero-day MCP vulnerabilities (see RR-5) |
| 9 | **Intellectual property** | `PathPolicy` sandboxes reads to worktree; `GitPolicy` restricts push targets; `ScrubPolicy` prevents credential leakage | No license compliance checking in tool outputs |
| 10 | **Obscene content** | Content filtering delegated to upstream model; operator-defined task constraints | No independent content classifier at agent layer |
| 11 | **Value chain / component integration** | Built-in Rust tools (19 builtins in `roko-std`); MCP tools pass through `SafetyLayer`; dependency pinning via `Cargo.lock` | MCP server supply chain risk (see RR-5) |
| 12 | **Dangerous content** | `BashPolicy` blocks destructive commands; `NetworkPolicy` restricts outbound access; `ToolPermission` per-role filtering | Sophisticated multi-step attack chains |

### Configuration

```rust
/// NIST AI RMF alignment configuration.
/// Maps RMF functions to Roko enforcement points.
///
/// Loaded from `[safety.nist_rmf]` in roko.toml.
/// Each field configures one aspect of the four-function framework.
pub struct NistRmfConfig {
    /// GOVERN: risk tolerance thresholds per category.
    /// Range: 0.0 (zero tolerance) to 1.0 (fully permissive).
    /// Default: 0.1 for all categories.
    pub risk_tolerances: HashMap<RiskCategory, f64>,

    /// MAP: enabled threat categories from NIST AI 600-1.
    /// Default: all 12 categories enabled.
    pub enabled_risk_categories: Vec<NistGenAiRisk>,

    /// MEASURE: metrics collection interval in seconds.
    /// Range: 10..=3600. Default: 60.
    pub metrics_interval_secs: u64,

    /// MANAGE: automatic response actions per risk level.
    /// Default: Low → Log, Medium → Alert, High → Throttle, Critical → Pause.
    pub response_actions: HashMap<RiskLevel, ResponseAction>,
}

/// The 12 generative AI risk categories from NIST AI 600-1.
pub enum NistGenAiRisk {
    Confabulation,
    CbrnInformation,
    DataPrivacy,
    EnvironmentalImpact,
    HarmfulBias,
    HumanAiConfiguration,
    InformationIntegrity,
    InformationSecurity,
    IntellectualProperty,
    ObsceneContent,
    ValueChainIntegration,
    DangerousContent,
}

/// Risk severity levels for NIST RMF response actions.
pub enum RiskLevel { Low, Medium, High, Critical }

/// Automated response actions, ordered by severity.
pub enum ResponseAction { Log, Alert, Throttle, Pause, Shutdown }
```

```toml
# roko.toml — NIST AI RMF alignment
[safety.nist_rmf]
metrics_interval_secs = 60

# Risk tolerances per category (0.0 = zero tolerance, 1.0 = permissive)
[safety.nist_rmf.risk_tolerances]
confabulation = 0.1
data_privacy = 0.05
information_security = 0.05
harmful_bias = 0.1
environmental_impact = 0.3
value_chain_integration = 0.1

# Response actions per risk level
[safety.nist_rmf.response_actions]
low = "log"
medium = "alert"
high = "throttle"
critical = "pause"

# Disable specific risk categories if not applicable
# enabled_risk_categories = ["all"]  # default
```

### Test criteria

- [ ] `NistRmfConfig` deserializes from `roko.toml` with defaults for missing fields
- [ ] Each of the 12 `NistGenAiRisk` categories maps to at least one `SafetyLayer` check
- [ ] `response_actions` trigger at the correct risk level — Log produces an event, Alert notifies the operator, Throttle reduces `RateLimiter` capacity, Pause halts the current task
- [ ] `risk_tolerances` outside the 0.0..=1.0 range are rejected at config load time
- [ ] `metrics_interval_secs` outside the 10..=3600 range clamps to the nearest bound with a warning
- [ ] Gate pipeline failures increment the corresponding `NistGenAiRisk` counter
- [ ] When any category's measured failure rate exceeds its `risk_tolerances` threshold, the `response_actions` for the current `RiskLevel` fires

---

## 8. MITRE ATLAS technique mapping

MITRE ATLAS (Adversarial Threat Landscape for AI Systems) catalogs adversarial techniques against machine learning systems. Version 5.4.0 (February 2026) added agent-specific techniques including poisoned agent tools and sandbox escape. This section maps ATLAS techniques to Roko's defense stack.

### Tactic overview

ATLAS organizes techniques under 16 tactics that mirror the MITRE ATT&CK lifecycle, adapted for AI systems:

| # | Tactic | Relevance to Roko |
|---|---|---|
| 1 | Reconnaissance | Attacker studies agent capabilities, tool list, model identity |
| 2 | Resource Development | Attacker prepares poisoned data, crafted prompts, malicious MCP servers |
| 3 | Initial Access | Entry point — prompt injection, poisoned tool output, malicious knowledge entry |
| 4 | ML Model Access | Attacker gains query or API access to the underlying model |
| 5 | Execution | Malicious payload runs — unauthorized tool calls, code execution |
| 6 | Persistence | Attack survives across sessions — memory poisoning, corrupted Neuro entries |
| 7 | Privilege Escalation | Agent gains capabilities beyond its assigned role — PFI (Kim et al. 2025) |
| 8 | Defense Evasion | Bypass SafetyLayer, ScrubPolicy, or gate checks |
| 9 | Credential Access | Exfiltrate API keys, tokens, or signing keys |
| 10 | Discovery | Map the agent's environment, available tools, file system structure |
| 11 | Collection | Gather sensitive data from context window, memory, or tool outputs |
| 12 | ML Attack Staging | Prepare adversarial inputs, test bypass techniques |
| 13 | Exfiltration | Move collected data to attacker-controlled infrastructure |
| 14 | Impact | Disrupt agent operations, corrupt outputs, deny service |
| 15 | Lateral Movement | Pivot from one agent to another in a collective |
| 16 | Command and Control | Establish persistent communication with compromised agent |

### Technique-to-defense mapping

| ATLAS ID | Technique | Roko Defense | Component | Coverage |
|---|---|---|---|---|
| AML.T0051 | LLM Prompt Injection | Prompt architecture separates instructions from data; `SafetyLayer` pre/post checks | `roko-agent`, `roko-compose` | Partial — sophisticated injections may bypass |
| AML.T0051.001 | Direct Prompt Injection | System prompt is operator-controlled; task descriptions from trusted source | `roko-compose` (SystemPromptBuilder) | Strong for trusted pipelines |
| AML.T0051.002 | Indirect Prompt Injection | Tool outputs treated as data; `ScrubPolicy` strips instructional content | `roko-agent` (SafetyLayer) | Partial — novel encoding may bypass |
| AML.T0054 | LLM Jailbreak | Role-based `ToolPermission` limits available actions regardless of prompt | `roko-core` (RolePermissions) | Strong — tool filtering is prompt-independent |
| AML.T0080 | Memory Poisoning | 4-stage ingestion pipeline; HDC anomaly detection; Cognitive Namespaces for isolation | `roko-golem` (Neuro) | Partial — novel poisoning vectors (see RR-7) |
| AML.T0047 | ML Supply Chain Compromise | Built-in Rust tools (19 builtins); `Cargo.lock` pinning; MCP tools pass through `SafetyLayer` | `roko-std`, `roko-agent` | Partial — MCP server trust is external |
| AML.T0048 | Publish Poisoned Model | CascadeRouter uses operator-configured model list; no dynamic model discovery | `roko-learn` (CascadeRouter) | Strong — model selection is static config |
| AML.T0043 | Craft Adversarial Data | Gate pipeline validates outputs (compile, test, clippy, diff); adaptive thresholds | `roko-gate` | Partial — adversarial data may pass semantic checks |
| — | Publish Poisoned Agent Tool (v5.4.0) | MCP tool calls pass through `SafetyLayer`; `ToolPermission` per-role filtering; operator approves MCP config | `roko-agent` (SafetyLayer) | Partial — depends on operator vetting |
| — | Escape to Host (v5.4.0) | `PathPolicy` canonicalization; `BashPolicy` deny patterns; worktree sandboxing | `roko-agent` (SafetyLayer) | Strong for known vectors; novel escapes remain residual |

### Configuration

```rust
/// MITRE ATLAS technique identifiers relevant to Roko agents.
/// Based on ATLAS v5.4.0 (February 2026).
///
/// Each variant maps to a documented ATLAS technique with a known
/// attack pattern and corresponding Roko defense.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtlasTechnique {
    /// AML.T0051 — LLM Prompt Injection
    PromptInjection,
    /// AML.T0051.001 — Direct Prompt Injection
    DirectPromptInjection,
    /// AML.T0051.002 — Indirect Prompt Injection
    IndirectPromptInjection,
    /// AML.T0054 — LLM Jailbreak
    Jailbreak,
    /// AML.T0080 — Memory Poisoning
    MemoryPoisoning,
    /// AML.T0047 — ML Supply Chain Compromise
    SupplyChainCompromise,
    /// AML.T0048 — Publish Poisoned Model
    PoisonedModel,
    /// AML.T0043 — Craft Adversarial Data
    AdversarialData,
    /// Publish Poisoned AI Agent Tool (v5.4.0)
    PoisonedAgentTool,
    /// Escape to Host (v5.4.0)
    SandboxEscape,
}

impl AtlasTechnique {
    /// Returns the official ATLAS technique ID string.
    pub fn atlas_id(&self) -> &'static str {
        match self {
            Self::PromptInjection => "AML.T0051",
            Self::DirectPromptInjection => "AML.T0051.001",
            Self::IndirectPromptInjection => "AML.T0051.002",
            Self::Jailbreak => "AML.T0054",
            Self::MemoryPoisoning => "AML.T0080",
            Self::SupplyChainCompromise => "AML.T0047",
            Self::PoisonedModel => "AML.T0048",
            Self::AdversarialData => "AML.T0043",
            Self::PoisonedAgentTool => "AML.T0099",
            Self::SandboxEscape => "AML.T0100",
        }
    }
}
```

### Test criteria

- [ ] Every `AtlasTechnique` variant has a corresponding defense entry in `SafetyLayer`
- [ ] `atlas_id()` returns the correct string for each variant
- [ ] Threat model coverage report (via `roko status --threats`) lists all 10 techniques with their defense status
- [ ] Adding a new `AtlasTechnique` variant without a defense mapping triggers a compile-time warning (enforced via exhaustive match)
- [ ] Integration test simulates each attack vector and confirms the corresponding defense activates

---

## 9. STRIDE-AI threat classification

STRIDE-AI (Mauri & Damiani, 2022) adapts Microsoft's STRIDE threat model for machine learning systems. Each STRIDE category maps to specific agent-layer attack patterns. The CSA MAESTRO framework (February 2025) complements this with a 7-layer model — Foundation Model, Data Operations, Agent Framework, Agent Orchestration, Tool Integration, Deployment, and Ecosystem — that provides vertical depth where STRIDE-AI provides horizontal breadth. Together they cover the full attack surface.

### Threat classification matrix

| STRIDE Category | Security Property | Agent Manifestation | Roko Mitigation | ATLAS Technique | Residual Risk |
|---|---|---|---|---|---|
| **Spoofing** | Authenticity | Model identity forgery — attacker substitutes a weaker or compromised model; agent impersonation in collectives via forged identity signals | `CascadeRouter` validates model identity against operator-configured list; reputation-weighted trust scores in collectives; `Capability<T>` tokens bind actions to authenticated roles | AML.T0048 | Medium — model identity relies on API provider integrity |
| **Tampering** | Integrity | Prompt injection modifies agent behavior; memory poisoning corrupts knowledge base; tool output manipulation inserts false data into the reasoning loop | `SafetyLayer` pre/post checks; 4-stage Neuro ingestion; `ScrubPolicy` strips instructional patterns from tool outputs; gate pipeline validates outputs | AML.T0051, AML.T0080 | High — sophisticated injections remain the top residual risk (RR-1) |
| **Repudiation** | Non-repudiation | Agent takes destructive action with no audit trail; operator cannot determine which task or prompt triggered a harmful output | `EpisodeLogger` records every agent turn to `.roko/episodes.jsonl`; gate results logged per task; efficiency events per turn to `.roko/learn/efficiency.jsonl` | — | Medium — log integrity depends on filesystem permissions; no cryptographic signing of log entries |
| **Information disclosure** | Confidentiality | Context window leakage exposes secrets across tasks; credential exfiltration via tool outputs or network requests; memory contents leak between Cognitive Namespaces | `ScrubPolicy` redacts secrets (regex-based); `PathPolicy` sandboxes file access; `NetworkPolicy` restricts outbound destinations; Cognitive Namespaces isolate agent memory | AML.T0043 | Medium — encoded or split secrets evade regex (RR-4) |
| **Denial of service** | Availability | Token budget exhaustion via large prompts; infinite tool call loops; disk exhaustion via large file writes; MCP server hanging or slow responses | `RateLimiter` (60 calls/60s per role+tool); circuit breaker (`CircuitBreaker`); token budget in `Composer`; `ProcessSupervisor` enforces idle TTL and max duration | — | Low — rate limits and circuit breakers provide strong coverage |
| **Elevation of privilege** | Authorization | Prompt injection escalates to unauthorized tool calls (PFI, Kim et al. 2025); agent breaks out of role-defined permissions; worktree escape enables file system access | `ToolPermission` per-role filtering; `Capability<T>` tokens for fine-grained authorization; `PathPolicy` worktree enforcement; `BashPolicy` command deny patterns | AML.T0051, AML.T0054 | High — PFI demonstrates that prompt injection can escalate to arbitrary tool calls |

### MAESTRO layer mapping

The CSA MAESTRO (Multi-Agent Environment Security Threat and Risk Operations) framework defines seven layers, each with distinct threat classes. Roko's defense stack spans all seven:

| MAESTRO Layer | Roko Coverage |
|---|---|
| L1: Foundation Model | CascadeRouter model selection; no fine-tuning (uses vendor models) — risk delegated to model provider |
| L2: Data Operations | 4-stage Neuro ingestion; `ScrubPolicy`; Cognitive Namespaces |
| L3: Agent Framework | `SafetyLayer` on every tool call; `RolePermissions`; `Capability<T>` |
| L4: Agent Orchestration | `PlanRunner` lifecycle management; `ProcessSupervisor`; DAG executor with dependency tracking |
| L5: Tool Integration | 19 built-in Rust tools; MCP tools pass through `SafetyLayer`; `ToolPermission` filtering |
| L6: Deployment | Worktree sandboxing; `PathPolicy`; `BashPolicy`; `NetworkPolicy` |
| L7: Ecosystem | Collective reputation scoring; knowledge marketplace trust tiers |

### Configuration

```rust
/// STRIDE-AI threat classification for autonomous agents.
/// Based on Mauri & Damiani (Sensors 22(17):6662, 2022).
///
/// Each instance represents a classified threat with its STRIDE category,
/// the specific way it manifests in an agent system, and the defenses
/// Roko applies against it.
pub struct StrideAiThreat {
    /// Which STRIDE category this threat belongs to.
    pub category: StrideCategory,

    /// How this threat manifests in an agent context.
    /// Example: "Prompt injection modifies agent behavior via tool output."
    pub agent_manifestation: String,

    /// The security property this threat violates.
    pub security_property: SecurityProperty,

    /// Roko components that mitigate this threat.
    /// Example: vec!["SafetyLayer", "ScrubPolicy", "gate pipeline"]
    pub roko_mitigation: Vec<String>,

    /// Corresponding MITRE ATLAS technique, if one exists.
    pub atlas_technique: Option<AtlasTechnique>,

    /// Risk remaining after mitigations are applied.
    pub residual_risk: RiskLevel,
}

/// The six STRIDE categories adapted for AI systems.
pub enum StrideCategory {
    Spoofing,
    Tampering,
    Repudiation,
    InformationDisclosure,
    DenialOfService,
    ElevationOfPrivilege,
}

/// Security properties violated by each STRIDE category.
pub enum SecurityProperty {
    Authenticity,
    Integrity,
    NonRepudiation,
    Confidentiality,
    Availability,
    Authorization,
}
```

### Test criteria

- [ ] Every `StrideCategory` variant has at least one `StrideAiThreat` instance with a non-empty `roko_mitigation` list
- [ ] `StrideAiThreat` instances with `atlas_technique: Some(t)` reference a valid `AtlasTechnique` variant
- [ ] `residual_risk` for Tampering and ElevationOfPrivilege is `High` — reflecting that prompt injection remains the top unmitigated risk
- [ ] All seven MAESTRO layers have at least one Roko component mapped
- [ ] Threat classification report (via `roko status --stride`) produces a table matching the matrix above

---

## 10. Adversarial safety testing framework

Static threat modeling identifies what can go wrong. Adversarial testing finds out what actually breaks. This section defines Roko's red-teaming approach — automated techniques that probe the agent's defenses on a recurring schedule.

### Red-teaming principles

NIST AI 600-1 recommends red teaming as a primary measurement technique for generative AI risks. The framework calls for:

1. **Scope definition** — which NIST risk categories to test (section 7)
2. **Diverse testers** — automated techniques cover breadth; human red teams cover creativity
3. **Iterative testing** — re-test after each defense change
4. **Result documentation** — log every test, pass or fail, for trend analysis

Roko implements the automated side. Human red teaming remains an operator responsibility.

### Automated red team techniques

The adversarial testing pipeline draws on four published automated techniques:

**PAIR** (Chao et al., 2023) — Prompt Automatic Iterative Refinement. An attacker LLM generates candidate jailbreaks, a judge LLM scores them, and the attacker refines iteratively. Converges in 3-20 rounds. Tests: direct prompt injection resistance, jailbreak resilience.

**TAP** (Mehrotra et al., 2023) — Tree of Attacks with Pruning. Extends PAIR with tree search. The attacker explores multiple attack branches, prunes low-scoring paths, and focuses on promising vectors. Higher success rate than PAIR on defended models. Tests: defense evasion, multi-path injection.

**Crescendo** (Russinovich et al., 2024) — Multi-turn escalation. The attacker starts with benign requests and gradually escalates across conversation turns, exploiting the model's tendency to maintain conversational consistency. Tests: multi-turn privilege escalation, conversational drift.

**GOAT** (Pavlova et al., 2024) — Generative Online Adversarial Testing. Fully automated multi-turn red teaming with dynamic strategy selection. Adapts attack strategy based on model responses in real time. Tests: adaptive defense bypasses, tool-use exploitation.

### Testing pipeline

```rust
/// Adversarial safety testing configuration.
/// Runs periodic red-team exercises against the agent's defenses.
///
/// Loaded from `[safety.adversarial_testing]` in roko.toml.
/// Each test run exercises the configured techniques against a sandboxed
/// agent instance and reports pass/fail metrics to the efficiency log.
pub struct AdversarialTestConfig {
    /// How often to run the adversarial test suite, in hours.
    /// Range: 1..=720. Default: 24.
    pub test_interval_hours: u64,

    /// Maximum wall-clock time per test run, in seconds.
    /// Range: 60..=7200. Default: 600.
    pub max_test_duration_secs: u64,

    /// Which adversarial techniques to include in each run.
    /// Default: all techniques enabled.
    pub enabled_techniques: Vec<AdversarialTechnique>,

    /// If the failure fraction exceeds this threshold, trigger an alert.
    /// Range: 0.0..=1.0. Default: 0.2.
    pub failure_alert_threshold: f64,

    /// Whether to run tests in a sandboxed worktree (recommended).
    /// Default: true.
    pub sandboxed: bool,
}

/// Adversarial techniques available in the testing pipeline.
/// Each maps to a published automated red team method.
pub enum AdversarialTechnique {
    /// Inject malicious instructions directly in the prompt.
    DirectPromptInjection,
    /// Inject via tool output, file content, or API response.
    IndirectPromptInjection,
    /// Multi-turn gradual escalation (Crescendo, Russinovich et al. 2024).
    MultiTurnEscalation,
    /// Tree-search jailbreak exploration (TAP, Mehrotra et al. 2023).
    TreeSearchJailbreak,
    /// Corrupt knowledge store entries to alter agent behavior.
    MemoryPoisoning,
    /// Return crafted data from tool calls to mislead the agent.
    ToolOutputManipulation,
    /// Attempt to escape the worktree sandbox via path traversal or shell features.
    SandboxEscapeAttempt,
    /// Attempt to read and exfiltrate secrets from the environment.
    CredentialExfiltration,
}
```

### Configuration

```toml
# roko.toml — Adversarial safety testing
[safety.adversarial_testing]
test_interval_hours = 24
max_test_duration_secs = 600
failure_alert_threshold = 0.2
sandboxed = true

# Enable specific techniques (default: all)
enabled_techniques = [
    "direct_prompt_injection",
    "indirect_prompt_injection",
    "multi_turn_escalation",
    "tree_search_jailbreak",
    "memory_poisoning",
    "tool_output_manipulation",
    "sandbox_escape_attempt",
    "credential_exfiltration",
]
```

### Benchmarks

Three public benchmarks provide standardized test suites for agent safety:

**Agent-SafetyBench** (Zhang et al., arXiv:2412.14470, 2024) — 349 agent environments, 2,000 test cases across 8 risk categories. Covers tool-use risks, jailbreak propagation, and multi-agent coordination failures. Roko's gate pipeline and SafetyLayer map to the benchmark's safety evaluation dimensions.

**AgentDojo** (Debenedetti et al., arXiv:2406.13352, 2024) — Dynamic evaluation framework that generates novel attack scenarios rather than relying on a fixed test set. Tests prompt injection in realistic tool-use settings. Particularly relevant for validating Roko's indirect injection defenses.

**JailbreakBench** (Chao et al., arXiv:2404.01318, 2024) — Standardized benchmark with artifact repository for jailbreak attacks and defenses. Provides reproducible attack strings and evaluation criteria. Useful for regression testing Roko's jailbreak resilience after defense changes.

### Test criteria

- [ ] `AdversarialTestConfig` deserializes from `roko.toml` with correct defaults
- [ ] `test_interval_hours` outside 1..=720 clamps to the nearest bound with a warning
- [ ] `max_test_duration_secs` outside 60..=7200 clamps to the nearest bound with a warning
- [ ] `failure_alert_threshold` outside 0.0..=1.0 is rejected at config load time
- [ ] When `sandboxed = true`, tests run in an isolated worktree that is cleaned up after the run
- [ ] Each `AdversarialTechnique` produces at least one test case per run
- [ ] Test results are logged to `.roko/learn/adversarial-tests.jsonl` with timestamps, technique, pass/fail, and failure details
- [ ] When the failure fraction exceeds `failure_alert_threshold`, an alert event fires via the `MANAGE` response action from `NistRmfConfig`
- [ ] Test suite completes within `max_test_duration_secs` — hanging tests are killed and logged as failures

---

## 11. OWASP Top 10 for Agentic Applications mapping

The OWASP Top 10 for Agentic Applications (December 2025) identifies the most critical security risks in autonomous AI agent systems. Published by the OWASP Agentic AI Security Initiative, this list targets the gap between traditional web application security and the emergent risks of agents that plan, use tools, and operate with delegated authority. Each risk maps to concrete Roko defense components.

### Risk identifiers

```rust
/// OWASP Top 10 for Agentic Applications risk identifiers.
/// Based on OWASP Agentic AI Security Initiative (2025/2026).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwaspAgenticRisk {
    /// ASI01 — Excessive Agency: tools beyond scope, excessive permissions.
    ExcessiveAgency,
    /// ASI02 — Insufficient Access Controls: broken auth in tool pipeline.
    InsufficientAccessControls,
    /// ASI03 — Insecure Output Handling: unvalidated agent outputs.
    InsecureOutputHandling,
    /// ASI04 — Prompt Injection: direct and indirect.
    PromptInjection,
    /// ASI05 — Overreliance on Agent Output: blind trust in LLM reasoning.
    OverrelianceOnOutput,
    /// ASI06 — Memory and Context Poisoning: reshaping behavior post-interaction.
    MemoryContextPoisoning,
    /// ASI07 — Insecure Inter-Agent Communication: spoofed messages.
    InsecureInterAgentComm,
    /// ASI08 — Cascading Failures: false signals through automated pipelines.
    CascadingFailures,
    /// ASI09 — Human-Agent Trust Exploitation: confident misleading explanations.
    HumanAgentTrustExploitation,
    /// ASI10 — Rogue Agents: misaligned, concealing, self-directed.
    RogueAgents,
}

impl OwaspAgenticRisk {
    pub fn owasp_id(&self) -> &'static str {
        match self {
            Self::ExcessiveAgency => "ASI01",
            Self::InsufficientAccessControls => "ASI02",
            Self::InsecureOutputHandling => "ASI03",
            Self::PromptInjection => "ASI04",
            Self::OverrelianceOnOutput => "ASI05",
            Self::MemoryContextPoisoning => "ASI06",
            Self::InsecureInterAgentComm => "ASI07",
            Self::CascadingFailures => "ASI08",
            Self::HumanAgentTrustExploitation => "ASI09",
            Self::RogueAgents => "ASI10",
        }
    }
}
```

### Risk-to-defense mapping

| OWASP ID | Risk | Roko Mitigation | Component | Coverage |
|---|---|---|---|---|
| ASI01 | Excessive Agency | `ToolPermission` per-role filtering; task-level allowed/denied tool lists; `Capability<T>` single-use tokens | `roko-core`, `roko-agent` | Strong |
| ASI02 | Insufficient Access Controls | 7-stage `ToolDispatcher` pipeline; `SafetyLayer` pre-execution checks; `PathPolicy` canonicalization | `roko-agent` | Strong (when wired) |
| ASI03 | Insecure Output Handling | Gate pipeline (compile, test, clippy, diff); `ScrubPolicy` post-execution; output truncation | `roko-gate`, `roko-agent` | Strong |
| ASI04 | Prompt Injection | 6-layer `SystemPromptBuilder`; delimiter hardening; CaMeL dual-LLM (design target) | `roko-compose`, `roko-agent` | Partial |
| ASI05 | Overreliance on Output | Gate pipeline as independent verification; `OperationalConfidenceTracker` trust management | `roko-gate`, `roko-learn` | Moderate |
| ASI06 | Memory/Context Poisoning | 4-stage ingestion pipeline; HDC anomaly detection; causal rollback; Bloom Oracle | `roko-golem` (Neuro) | Partial |
| ASI07 | Insecure Inter-Agent Comms | Cognitive Namespaces with ACL; `NamespaceChannel` kind filtering + rate limiting; reputation-weighted trust | Target: Tier 3 | Design only |
| ASI08 | Cascading Failures | Circuit breaker pattern; `ProcessSupervisor` lifecycle management; adaptive gate thresholds (EMA) | `roko-conductor`, `bardo-runtime` | Strong |
| ASI09 | Human-Agent Trust Exploitation | Cognitive Signals (Pause/Escalate/Cooldown); Forensic AI replay for decision audit | Target: Tier 2/3 | Partial |
| ASI10 | Rogue Agents | `TemporalMonitor` behavioral drift detection; safety budgets; `SafetyLayer` architectural enforcement | `roko-gate`, `roko-agent` | Partial |

### Coverage analysis

Four of the ten risks have strong coverage through existing, wired components: Excessive Agency (ASI01), Insufficient Access Controls (ASI02), Insecure Output Handling (ASI03), and Cascading Failures (ASI08). These are enforced on every tool call via the `SafetyLayer` and validated after every task via the gate pipeline.

Three risks have partial coverage with functioning defenses that do not catch all variants: Prompt Injection (ASI04), Memory/Context Poisoning (ASI06), and Rogue Agents (ASI10). Prompt injection remains the highest residual risk across all frameworks in this threat model.

Two risks have partial coverage through Tier 2/3 features not yet fully wired: Human-Agent Trust Exploitation (ASI09) relies on Cognitive Signals that are built but not integrated into the main orchestration loop, and Overreliance on Output (ASI05) depends on confidence tracking that covers gate results but not broader reasoning quality.

One risk is design-only: Insecure Inter-Agent Communication (ASI07). The Cognitive Namespace architecture is specified but the implementation is a Tier 3 target. Until then, Roko agents do not communicate directly with each other outside the plan DAG's task dependency structure, which limits but does not eliminate the attack surface.

### Test criteria

- [ ] Every `OwaspAgenticRisk` variant maps to at least one Roko defense component
- [ ] `owasp_id()` returns correct ASI identifier strings (ASI01 through ASI10)
- [ ] Coverage report via `roko status --owasp` lists all 10 risks with defense status
- [ ] Integration tests simulate each risk scenario and verify the corresponding defense activates
- [ ] No `OwaspAgenticRisk` variant returns "None" for its defense mapping — every risk has at least a design-level mitigation

---

## 12. Cascading failure analysis

A minor error in tool selection or a low-impact injection can cascade into high-impact safety harms when tasks depend on each other. The plan executor runs tasks in a DAG where each task's output feeds into its dependents. If a compromised task produces subtly wrong output that passes its gate checks, every downstream task inherits that corruption. The blast radius grows with the depth and fan-out of the dependency graph.

This section models cascade propagation and defines the defenses against it.

### Cascade propagation model

```rust
/// Cascading failure analysis for multi-agent plan execution.
/// Models how errors propagate through task dependency DAGs.
pub struct CascadeAnalyzer {
    /// Task dependency graph.
    pub task_dag: petgraph::Graph<TaskNode, DependencyEdge>,
    /// Failure propagation rules.
    pub propagation_rules: Vec<PropagationRule>,
    /// Maximum cascade depth before triggering circuit breaker.
    /// Range: 1..20. Default: 5.
    pub max_cascade_depth: usize,
    /// Blast radius threshold: max fraction of tasks affected.
    /// Range: 0.01..1.0. Default: 0.3.
    pub blast_radius_threshold: f64,
}

/// How a failure in one task propagates to dependents.
pub struct PropagationRule {
    /// Source failure type.
    pub source_failure: FailureType,
    /// Probability of propagation to each dependent.
    /// Range: 0.0..1.0.
    pub propagation_probability: f64,
    /// Whether the failure amplifies (severity increases) or dampens.
    pub amplification_factor: f64,
}

pub enum FailureType {
    GateFailure,
    ToolError,
    TimeoutExpired,
    BudgetExhausted,
    TaintViolation,
    TemporalViolation,
}

impl CascadeAnalyzer {
    /// Simulate failure propagation from a source task.
    /// Returns the set of affected tasks and their failure severity.
    pub fn simulate_cascade(
        &self,
        source_task: NodeIndex,
        failure: FailureType,
    ) -> CascadeResult {
        // BFS from source through dependency edges
        // At each hop: check propagation probability
        // Track depth; halt at max_cascade_depth
        // Compute blast radius as fraction of total tasks
        // ...
    }

    /// Pre-execution cascade risk assessment.
    /// For each task in the DAG, compute worst-case blast radius.
    /// Tasks with high cascade risk get tighter safety budgets.
    pub fn assess_cascade_risk(&self) -> HashMap<NodeIndex, f64> {
        // For each node: simulate cascade with each failure type
        // Return max blast radius per node
        // ...
    }
}

pub struct CascadeResult {
    pub affected_tasks: Vec<(NodeIndex, f64)>, // (task, severity)
    pub cascade_depth: usize,
    pub blast_radius: f64, // fraction of total tasks affected
    pub should_halt: bool, // true if blast_radius > threshold
}
```

### Failure type propagation characteristics

Not all failures propagate equally. A `GateFailure` is the safest — the gate caught the problem, the task is marked failed, and dependents do not execute. A `ToolError` is similar: explicit failure, clean signal, dependents blocked. These produce narrow cascades.

The dangerous failures are the ones that do not look like failures. A task that passes its gate checks but produces subtly wrong output — a `TaintViolation` where tainted data leaked into a non-tainted context, or a `TemporalViolation` where a time-dependent operation used stale state — propagates silently. Each dependent task builds on the corrupted output, and the error amplifies.

The `amplification_factor` in `PropagationRule` models this. Values above 1.0 mean the severity grows at each hop. Values below 1.0 mean natural dampening — each dependent task's own gate checks have some chance of catching the problem. The default rules:

| Failure type | Propagation probability | Amplification factor | Rationale |
|---|---|---|---|
| `GateFailure` | 0.0 | 0.0 | Gate caught it; dependents blocked |
| `ToolError` | 0.0 | 0.0 | Explicit failure; dependents blocked |
| `TimeoutExpired` | 0.1 | 0.5 | Partial output may propagate; dampens quickly |
| `BudgetExhausted` | 0.2 | 0.8 | Truncated results may look valid |
| `TaintViolation` | 0.7 | 1.2 | Silent; amplifies through dependent reasoning |
| `TemporalViolation` | 0.6 | 1.1 | Stale state compounds across dependent tasks |

### Cascade prevention strategies

**Firewall tasks.** Insert verification-only tasks between high-risk phases of the plan. A firewall task runs no agent — it re-runs the gate pipeline on the preceding task's outputs with tighter thresholds. If the firewall task fails, the cascade stops there instead of propagating to the next phase. The `CascadeAnalyzer` identifies insertion points: any edge in the DAG where the source node's cascade risk exceeds 0.5 and the target node has three or more transitive dependents.

**Blast radius budgets.** Each task's safety budget — the number of tool calls, token spend, and allowed retries — inversely scales with its cascade risk score. A task at the root of a deep dependency chain gets a tighter budget than a leaf task with no dependents. This makes high-cascade-risk tasks fail fast and fail loudly rather than producing ambiguous outputs that propagate.

**Progressive rollback.** When the circuit breaker detects a cascade (blast radius exceeds threshold), it does not halt the entire plan. It rolls back affected tasks in reverse topological order, starting from the most downstream affected task and working back toward the source. Each rolled-back task's outputs are invalidated. The plan executor then re-queues the source task with a tighter safety budget and the `firewall_tasks` flag enabled.

### Configuration

```toml
[safety.cascade]
max_cascade_depth = 5
blast_radius_threshold = 0.3
enable_firewall_tasks = true
cascade_check_interval_tasks = 5
```

`max_cascade_depth` caps how far the analyzer simulates propagation. Deeper simulations are more accurate but slower. The default of 5 covers most plan structures — plans with more than 5 levels of sequential dependency are rare, and the circuit breaker catches anything beyond this depth at runtime.

`blast_radius_threshold` triggers the `should_halt` flag in `CascadeResult`. At the default of 0.3, the circuit breaker activates when more than 30% of the plan's tasks are affected by a single cascade. Lower values are more conservative; production deployments handling irreversible operations should use 0.1.

`enable_firewall_tasks` controls automatic insertion. When enabled, the plan executor calls `assess_cascade_risk()` before execution and inserts firewall tasks at high-risk edges. When disabled, the cascade analysis still runs for reporting but does not modify the plan.

`cascade_check_interval_tasks` controls how often the analyzer re-evaluates cascade risk during execution. The default of 5 means re-assessment happens every 5 completed tasks. Set to 1 for maximum safety at the cost of overhead.

### Test criteria

- [ ] `CascadeAnalyzer::simulate_cascade()` halts at `max_cascade_depth` — a chain of 10 tasks with depth limit 5 produces a cascade of depth 5
- [ ] `CascadeAnalyzer::assess_cascade_risk()` returns higher risk scores for tasks with many dependents than for leaf tasks
- [ ] `blast_radius_threshold` triggers `should_halt` when exceeded — a cascade affecting 4 of 10 tasks (0.4) with threshold 0.3 sets `should_halt = true`
- [ ] Firewall tasks break cascade propagation chains — a `TaintViolation` cascade stops at the firewall task boundary
- [ ] `GateFailure` and `ToolError` produce zero propagation — their `propagation_probability` is 0.0
- [ ] `TaintViolation` cascades amplify — severity at depth 3 exceeds severity at depth 1 when `amplification_factor` is 1.2
- [ ] Progressive rollback processes tasks in reverse topological order — the most downstream task is rolled back first
- [ ] Configuration values outside valid ranges are rejected: `max_cascade_depth` outside 1..20, `blast_radius_threshold` outside 0.01..1.0

---

## Related Topics

- [00-defense-in-depth.md](00-defense-in-depth.md) — Six safety guards
- [07-prompt-security.md](07-prompt-security.md) — Prompt injection defenses in detail
- [09-adaptive-risk.md](09-adaptive-risk.md) — Adaptive guardrails
- [10-mev-protection.md](10-mev-protection.md) — MEV-specific protections
- Section 11 (this document) — OWASP Top 10 for Agentic Applications mapping
- Section 12 (this document) — Cascading failure analysis
