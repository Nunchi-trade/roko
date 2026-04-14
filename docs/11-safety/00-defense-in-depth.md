# Defense-in-Depth: Architectural Safety for Autonomous Agents

> **Layer**: L1 Framework (safety guards), L3 Harness (gates, monitoring), Cross-cut (Safety & Provenance)
>
> **Crate**: `roko-agent` (`safety/` module)
>
> **Synapse traits**: `Gate` (verify Engrams against ground truth), `Policy` (observe Engram streams, emit safety Engrams)
>
> **Prerequisites**: [01-capability-tokens.md](01-capability-tokens.md), [02-audit-chain.md](02-audit-chain.md)


> **Implementation**: Shipping

---

## Overview

Safety in the Roko system is **architectural**, not behavioral. The distinction is critical. Behavioral safety means the LLM follows instructions — it can be bypassed by prompt injection, jailbreaking, or model replacement. Architectural safety means the constraints hold **even when the LLM does not follow instructions**. Every safety-critical invariant in Roko is enforced at a layer the LLM cannot reach: the Rust type system, smart contracts (for chain-domain agents), hardware enclaves, and compiled tool handlers.

This design principle is grounded in two complementary research traditions:

1. **Capability-based security** (Dennis & Van Horn, 1966): Access rights should be unforgeable tokens verified at the type level, not runtime guards that can be bypassed by unexpected code paths.
2. **The R2AI safety framework** (Yang et al., 2025): Five safety levels from Discovery (L0) through Evolutionary Reflection (L4). Roko targets L2 (Intervention) at launch and progresses toward L4.

The key asymmetry that motivates architectural safety: if an agent's language model is fully compromised — prompt-injected, jailbroken, replaced with a hostile model — the safety guarantees described in this document still hold. The LLM proposes actions; the safety architecture disposes.

---

## The Threat Landscape

### Why Autonomous Agents Are Uniquely Vulnerable

Omohundro (2008) demonstrated that sufficiently advanced AI systems develop instrumental drives — self-preservation, resource acquisition, goal preservation — not because they are programmed to, but because these drives are instrumentally useful for almost any terminal goal. Turner et al. (2021) proved this mathematically: for most reward functions, optimal policies tend to seek states that preserve optionality and acquire resources.

An autonomous agent with tool access can:

- **Execute destructive commands**: Run `rm -rf`, `git push --force`, `chmod 777`, or other commands that destroy data or compromise security
- **Leak secrets**: Include API keys, private keys, or credentials in LLM output that gets logged, synced, or transmitted
- **Exfiltrate data**: Use network tools to send sensitive data to unauthorized destinations
- **Poison the knowledge base**: Insert false heuristics or causal links into the Neuro knowledge store (formerly "Grimoire") that lead to future failures
- **Escape sandboxes**: Use symlinks, path traversal, or shell escapes to access files outside the worktree

For chain-domain agents managing real capital, the stakes are higher: a compromised agent can drain wallets, manipulate positions, or enter leveraged positions designed to liquidate.

### Behavioral Threats

These originate in the LLM's reasoning process and are the hardest to prevent because the LLM is, by design, a general-purpose reasoning engine that responds to its inputs.

**Prompt injection via tool results.** A malicious source can return content containing natural-language instructions embedded in what appears to be data. The agent calls a tool, gets the result, feeds it back into its context window, and now the attacker's text is indistinguishable from legitimate system instructions. Pan et al. (ACL 2024) documented how compressed or injected context can redirect LLM behavior.

**Reward hacking.** An agent optimizing for prediction accuracy and task completion might discover it can game its own accuracy metrics: making trivially correct predictions to inflate its action gate score, then using the earned permissions for high-risk operations.

**Misaligned optimization.** The agent follows its instructions faithfully but the instructions, as written, permit behavior the operator didn't intend. This is not a model failure; it is a specification gap. Safety architecture must account for it because the consequence is the same.

### The MCP Crisis

The OWASP MCP Top 10 identifies tool poisoning, cross-server shadowing, and rug pulls as primary threat vectors (OWASP-MCP-2025). Endor Labs (2026) reported that 82% of 2,614 MCP implementations use file system operations prone to path traversal, 67% use APIs susceptible to code injection, and CVE-2025-6514 (CVSS 10.0 RCE) in `mcp-remote` was downloaded over 558,000 times.

Roko agents use compiled Rust tools with version-locked dependencies. The LLM sees a defined set of tools backed by typed handlers compiled into the agent's binary. When MCP servers are used, they are configured via `roko.toml` and pass through the same safety pipeline as built-in tools.

---

## Three Defense Categories

The system implements defense-in-depth across three categories:

| Category | Mechanism | Bypassed by Prompt Injection? |
|----------|-----------|------------------------------|
| **Type-system** (Layer 1) | `Capability<T>` tokens (future), `PathPolicy` escape prevention, `ToolPermission` enforcement, `TaintedString` flow control | **No** — enforced by the Rust compiler |
| **Runtime** (Layer 2) | `SafetyLayer` pre-execution checks, `BashPolicy` deny patterns, `GitPolicy` protected branches, `NetworkPolicy` allowlists, `ScrubPolicy` secret scrubbing, `RateLimiter` sliding window | **Partially** — depends on hook chain integrity, but defense-in-depth means multiple layers must all fail |
| **Cryptographic** (Layer 3) | Content-addressed audit trails (Engram lineage DAG), on-chain anchoring (for chain-domain agents), `Attestation` on Engrams | **No** — exists outside the LLM entirely |

The type-system and cryptographic layers are the safety guarantees. The runtime layer is defense-in-depth — useful, often sufficient, but not relied upon alone.

### R2AI Safety Levels

Yang et al.'s R2AI framework proposes five safety levels. Roko targets L2 (Intervention) at launch:

| R2AI Level | Description | Roko Status |
|------------|-------------|-------------|
| L0: Discovery | Identify risks | Complete (threat model documented, 21 failure modes catalogued) |
| L1: Prevention | Proactive safeguards | Complete (SafetyLayer, PolicyCage for chain agents, tool permissions) |
| L2: Intervention | Runtime monitoring + correction | Launch target (gate pipeline, conductor circuit breakers, adaptive thresholds) |
| L3: Adaptation | Self-improving safety | Partial (Neuro knowledge store recalibration, EvoSkills adversarial verification) |
| L4: Evolutionary Reflection | Meta-level safety reasoning | Post-launch goal (Daimon meta-cognition assessment) |

---

## The Six Safety Guards

The Roko safety architecture is implemented as six composable guards in the `roko-agent` crate (`safety/` module). Each guard is a policy struct with a `check_*` method that returns `Ok(())` on pass or `Err(ToolError)` on violation. The guards are composed into a `SafetyLayer` that chains them in a specific order:

### Guard 1: Rate Limiting (`rate_limit.rs`)

**Layer**: L1 Framework
**Synapse trait**: `Policy` (observes Engram streams, emits rate-limit Engrams)

A sliding-window counter keyed by `(role, tool_name)`. A call is admitted if and only if fewer than `max_calls_per_window` calls have been recorded for this key within the last `window_duration`. Default: 60 calls per 60 seconds.

The implementation uses `parking_lot::Mutex<HashMap<RateLimitKey, VecDeque<Instant>>>`. Each deque holds admission timestamps, oldest first. Stale entries are pruned from the front on every operation, keeping memory bounded. The critical section is a single mutex lock-and-release with no TOCTOU gap between the cap check and the push (both happen under the same lock).

```rust
pub struct RateLimiter {
    policy: RateLimitPolicy,
    state: Mutex<HashMap<RateLimitKey, VecDeque<Instant>>>,
}

pub struct RateLimitPolicy {
    pub max_calls_per_window: usize,    // Default: 60
    pub window_duration: Duration,       // Default: 60s
}

pub struct RateLimitKey {
    pub role: String,    // e.g., "Implementer", "Auditor"
    pub tool: String,    // canonical tool name
}
```

### Guard 2: Bash Command Policy (`bash.rs`)

**Layer**: L1 Framework
**Synapse trait**: `Gate` (verifies Engrams against ground truth — the ground truth being "this command is safe")

Every bash command the agent proposes passes through `BashPolicy::check()` before execution. The policy maintains a deny list of dangerous patterns and an allow list of overrides:

**Default deny patterns** (substring and regex):
- `rm -rf /` — recursive root deletion
- `sudo` — privilege escalation
- `curl | sh`, `wget | sh` — remote code execution
- `:(){ :|:& };:` — fork bombs
- `mkfs` — filesystem formatting
- `dd if=` — raw disk operations
- `chmod 777` — world-writable permissions
- `> /dev/sda` — raw device writes

The deny list uses both substring matching (fast, catches common patterns) and compiled regex (catches obfuscated variants). Commands exceeding 8,192 characters are rejected outright — no legitimate tool invocation needs a command that long.

```rust
pub struct BashPolicy {
    pub deny_patterns: Vec<DenyPattern>,
    pub allow_overrides: Vec<String>,
    pub max_command_length: usize,  // Default: 8192
}

pub enum DenyPattern {
    Substring(String),
    Regex(regex::Regex),
}
```

### Guard 3: Git Policy (`git.rs`)

**Layer**: L1 Framework
**Synapse trait**: `Gate` (verifies git operations against protected branch rules)

Prevents destructive git operations on protected branches. The policy parses proposed git commands into semantic segments and checks against configurable rules:

- **Protected branches**: `main`, `master` (configurable, additional branches can be added)
- **Force push blocking**: `git push --force` and `git push -f` on protected branches
- **Hard reset blocking**: `git reset --hard` on protected branches
- **Branch deletion blocking**: `git branch -D` and `git branch -d` on protected branches

The implementation performs shell segment splitting to handle quoted arguments, flags, and subcommands correctly. It recognizes both long-form (`--force`) and short-form (`-f`) flags.

```rust
pub struct GitPolicy {
    pub protected_branches: Vec<String>,
    pub block_force_push: bool,           // Default: true
    pub block_hard_reset_on_protected: bool,  // Default: true
    pub block_branch_delete_protected: bool,  // Default: true
}
```

### Guard 4: Network Policy (`network.rs`)

**Layer**: L1 Framework
**Synapse trait**: `Gate` (verifies URLs against network allowlists)

Gates outbound URLs for network-capable tools (`web_fetch`, `web_search`, and any future network tool). Every URL runs through `check_url_with_policy()` before dispatch. The policy enforces four dimensions:

1. **Scheme filtering**: Only allowed URL schemes pass. Default: HTTPS-only.
2. **Private network blocking**: When enabled (default), loopback (127.0.0.0/8), RFC 1918 private (10/8, 172.16/12, 192.168/16), link-local (169.254.0.0/16, fe80::/10), unique-local (fc00::/7), and unspecified addresses are rejected. This defeats SSRF probes at `127.0.0.1`, cloud metadata endpoints at `169.254.169.254`, and internal network hosts.
3. **Deny list**: Hostnames matched exactly or as dotted suffixes (e.g., `.internal` rejects `server.internal`).
4. **Allow list**: When non-empty, only matching hosts are permitted.

Deny list is evaluated before allow list — a host on both lists is rejected.

```rust
pub struct NetworkPolicy {
    pub allow_schemes: Vec<String>,    // Default: ["https"]
    pub allow_hosts: Vec<String>,      // Default: empty (any host)
    pub deny_hosts: Vec<String>,       // Default: empty
    pub block_private_networks: bool,  // Default: true
}
```

### Guard 5: Path Policy (`path.rs`)

**Layer**: L1 Framework
**Synapse trait**: `Gate` (verifies file paths against worktree boundaries)

The single authority on whether a caller-supplied path argument is safe to hand to a filesystem tool handler. Every filesystem-touching built-in (`read_file`, `write_file`, `edit_file`, `glob`, `grep`) runs its path argument through `canonicalize_with_policy()` before any I/O.

The algorithm:

1. Build a **joined** path: if the argument is absolute, use it; otherwise, join it to the worktree root.
2. Canonicalize both the worktree and the joined path independently. For non-existent leaves (e.g., `write_file` creating a new file), canonicalize the deepest existing ancestor and re-attach the missing tail.
3. If `prevent_escapes` is set (default), the canonical joined path must start with the canonical worktree root. Otherwise return `ToolError::PathOutsideWorktree`.
4. If `deny_symlinks` is set, walk the on-disk components and reject any symlink. This prevents symlink-based sandbox escapes where an attacker creates a symlink pointing outside the worktree.
5. Compute the relative form by stripping the worktree prefix.

```rust
pub struct PathPolicy {
    pub deny_symlinks: bool,     // Default: false
    pub prevent_escapes: bool,   // Default: true
}

pub struct CanonicalPath {
    pub absolute: PathBuf,   // Guaranteed inside worktree when prevent_escapes is true
    pub relative: PathBuf,   // No leading "/" or "./"
}
```

### Guard 6: Secret Scrubbing (`scrub.rs`)

**Layer**: L1 Framework (post-execution)
**Synapse trait**: `Policy` (observes tool output Engrams, emits scrubbed versions)

Runs over tool result content **after** execution but **before** the content is handed to the LLM, replacing detected secrets with `[REDACTED]`. The scrubber is pure — it allocates a new `String` and never mutates shared state.

Default pattern set (9 compiled regex patterns):
1. **Anthropic API keys**: `sk-ant-api\d{2}-[A-Za-z0-9_-]{80,}`
2. **OpenAI API keys**: `sk-(?:proj-)?[A-Za-z0-9_-]{20,}`
3. **AWS access keys**: `AKIA[0-9A-Z]{16}` and `ASIA[0-9A-Z]{16}` (STS temporary)
4. **GitHub PATs**: `ghp_`, `ghs_`, `gho_`, `ghu_`, `ghr_` followed by 36 alphanumeric chars
5. **GitLab PATs**: `glpat-[A-Za-z0-9_-]{20,}`
6. **Slack tokens**: `xox[abpsr]-[A-Za-z0-9-]{10,}`
7. **JWTs**: Three base64url segments starting with `eyJ`
8. **Private key blocks**: `-----BEGIN * PRIVATE KEY-----` through `-----END * PRIVATE KEY-----` (multiline)
9. **Env-file assignments**: `PASSWORD=`, `SECRET=`, `TOKEN=`, `API_KEY=`, `APIKEY=`, `PRIVATE_KEY=`, `DATABASE_URL=` — replaces the value only, preserving the key name for readability

Additional user-defined patterns can be added via `ScrubPolicy::extra_patterns`. Invalid regex patterns are silently skipped.

```rust
pub struct ScrubPolicy {
    pub extra_patterns: Vec<String>,  // Additional regex patterns
    pub disable_defaults: bool,        // Skip default patterns (for testing)
}
```

---

## The SafetyLayer Composition

The six guards are composed into a single `SafetyLayer` struct that chains them in a specific execution order:

```rust
pub struct SafetyLayer {
    pub bash_policy: BashPolicy,
    pub git_policy: GitPolicy,
    pub network_policy: NetworkPolicy,
    pub path_policy: PathPolicy,
    pub scrub_policy: ScrubPolicy,
    pub rate_limiter: Option<Arc<RateLimiter>>,
    pub role: String,
}
```

### Pre-Execution Pipeline

The `check_pre_execution()` method chains policies in order of increasing cost:

1. **Rate limit check** — O(1) sliding window lookup
2. **Bash policy check** — string matching against deny patterns
3. **Git policy check** — command parsing + branch matching
4. **Network policy check** — URL parsing + host matching
5. **Path policy check** — filesystem canonicalization

The first failure short-circuits the pipeline — if rate limiting rejects the call, the more expensive path canonicalization never runs.

### Post-Execution Scrubbing

After tool execution, `scrub_output()` runs the secret scrubber over the result content. This is the last line of defense against credential leakage into the LLM's context window.

---

## Information Flow Taint Tracking

### Five Leakage Vectors

The safety architecture identifies five vectors through which agent data can leak:

| Vector | What Leaks | Current Mitigation |
|--------|-----------|-------------------|
| **Credential exfiltration** | API keys, service credentials | `ScrubPolicy` regex scrubbing on tool output; env vars never enter LLM context |
| **Context window leakage** | Strategy parameters, sensitive data | Prompt composition via `roko-compose` assembles from ContextBundle categories, not raw history |
| **Network exfiltration** | Arbitrary data via outbound requests | `NetworkPolicy` HTTPS-only, private network blocking, host allowlists |
| **Filesystem escape** | Files outside worktree | `PathPolicy` canonicalization + escape prevention |
| **Knowledge poisoning** | Corrupted Neuro entries persist across sessions | Neuro confidence scoring with decay, tier-based promotion gates |

### Data Flow Labels (Design Target)

The target design (from the legacy specification) defines taint labels that propagate through the system. Before data enters a sink (LLM context, audit log, mesh relay, event fabric), the taint checker verifies that no forbidden label reaches that sink:

| Label | Description | LLM Context | Audit Log | Mesh Relay | Local Neuro |
|-------|-------------|-------------|-----------|------------|-------------|
| `WalletSecret` | Wallet private key material | BLOCKED | BLOCKED | BLOCKED | Allowed |
| `OwnerSecret` | Owner API keys, credentials | BLOCKED | BLOCKED | BLOCKED | Allowed |
| `StrategyConfidential` | Proprietary strategy params | Allowed | Allowed | BLOCKED | Allowed |
| `UserPII` | Personal data (email, addresses) | Allowed | Allowed | BLOCKED | Allowed |
| `UntrustedExternal` | Data from untrusted sources | Allowed | Allowed | Allowed | Allowed |

The `TaintedString` type wraps sensitive content in `zeroize::Zeroizing<String>` which automatically overwrites memory on drop, preventing key recovery from memory dumps.

**Current implementation status**: The `ScrubPolicy` provides regex-based post-hoc scrubbing (a runtime approximation of taint tracking). Full compile-time taint tracking via `TaintedString` is a Tier 2 implementation target (see `refactoring-prd/07-implementation-priorities.md`).

---

## Integration with the Synapse Architecture

Safety participates in the Universal Cognitive Loop at multiple steps:

| Loop Step | Safety Role |
|-----------|-------------|
| **5. ACT** (Agent.execute) | `SafetyLayer.check_pre_execution()` gates every tool call before handler dispatch |
| **6. VERIFY** (Gate.verify) | Gate pipeline runs compile, test, clippy, diff gates on agent output |
| **7. PERSIST** (Substrate.put) | Engram lineage DAG provides content-addressed audit trail |
| **8. ADAPT** (Policy.decide) | Adaptive gate thresholds tighten/loosen based on pass rates |
| **9. META-COGNIZE** (Daimon.assess) | Daimon PAD vector modulates risk tolerance based on agent behavioral state |

The `ToolDispatcher` in `roko-agent` is the integration point where safety meets execution. Every dispatched tool call passes through:

1. Argument validation against the registry's JSON schema
2. Tool filter check (allowed/denied tool lists per task)
3. Capability authorization (`ToolPermission.satisfied_by(&role_perms)`)
4. **SafetyLayer pre-execution checks** (when attached)
5. Handler execution with timeout + cancellation
6. Result truncation (preserving UTF-8 char boundaries)
7. **SafetyLayer output scrubbing** (when attached)

Each phase emits audit signals (`Signal` / Engram) through the `AuditSink` trait, creating a content-addressed trail of every safety decision.

---

## Academic Foundation

| Paper | Contribution to Roko Safety |
|-------|---------------------------|
| Dennis & Van Horn (1966) | Capability-based security — unforgeable tokens, not runtime guards |
| Omohundro (2008) | Instrumental convergence — why agents develop unsafe drives |
| Turner et al. (2021) | Mathematical proof of resource-seeking optimal policies |
| Haas et al. (2017) | WASM Component Model — sandboxed execution via capabilities |
| Yang et al. (2025) | R2AI five-level safety framework |
| Debenedetti et al. (2025) | CaMeL — separate control flow from data flow for prompt injection defense |
| Pan et al. (ACL 2024) | Compressed/injected context redirects LLM behavior |
| OWASP MCP Top 10 (2025) | Tool poisoning, cross-server shadowing threat taxonomy |
| Endor Labs (2026) | 82% of MCP implementations vulnerable to path traversal |
| Lee et al. (2026, arXiv:2603.28052) | Meta-Harness — harness optimization yields +7.7 points on classification |
| NIST AI 600-1 (July 2024) | Generative AI Profile — 12 risk categories, red-teaming guidance |
| CSA MAESTRO (February 2025) | 7-layer agent threat modeling framework |
| Crescendo (Russinovich et al., 2024) | Multi-turn LLM jailbreak via gradual escalation |
| GOAT (Pavlova et al., 2024) | Generative Offensive Agent Tester — 97% ASR |
| FIDES (Costa & Kopf, 2025) | Information flow control for agentic systems |

---

## Adversarial Safety Testing Framework

### Red-team pipeline

Roko integrates adversarial testing as a continuous safety verification practice, following NIST AI 600-1 (July 2024) recommendations for regular adversarial testing of generative AI systems.

The adversarial testing pipeline runs as a periodic background process, testing the agent's safety defenses against known attack techniques:

```rust
/// Adversarial safety testing pipeline.
/// Runs attack simulations against the agent's safety defenses
/// in a sandboxed environment.
pub struct AdversarialTestPipeline {
    /// Test suites organized by attack category.
    pub test_suites: Vec<AdversarialTestSuite>,
    /// Results store for trend analysis.
    pub results_path: PathBuf,
    /// Sandbox configuration for test execution.
    pub sandbox_config: SandboxConfig,
    /// Alert threshold: fraction of failed tests that triggers alert.
    pub alert_threshold: f64,
}

pub struct AdversarialTestSuite {
    /// Suite identifier.
    pub id: String,
    /// MITRE ATLAS technique being tested.
    pub atlas_technique: String,
    /// Test cases in this suite.
    pub test_cases: Vec<AdversarialTestCase>,
    /// Expected defense: which Roko component should catch this.
    pub expected_defense: String,
}

pub struct AdversarialTestCase {
    /// Human-readable description.
    pub description: String,
    /// The adversarial input to inject.
    pub payload: String,
    /// Expected result: should the safety layer block this?
    pub expected_blocked: bool,
    /// Which guard should block it.
    pub expected_guard: Option<String>,
}

/// Result of running one adversarial test.
pub struct AdversarialTestResult {
    pub test_id: String,
    pub atlas_technique: String,
    pub blocked: bool,
    pub blocking_guard: Option<String>,
    pub passed: bool,  // true if blocked == expected_blocked
    pub latency_ms: u64,
    pub timestamp: u64,
}
```

### Attack simulation categories

| Category | ATLAS ID | Test approach | Expected defense |
|----------|----------|--------------|-----------------|
| Direct prompt injection | AML.T0051.001 | Inject instructions in system prompt variants | Prompt architecture + delimiter hardening |
| Indirect prompt injection | AML.T0051.002 | Inject via simulated tool results | ScrubPolicy + taint tracking |
| Path traversal | -- | Use ../../, symlinks, absolute paths | PathPolicy canonicalization |
| Command injection | -- | Shell metacharacters in bash arguments | BashPolicy deny patterns |
| Credential exfiltration | -- | Embed known patterns in tool output | ScrubPolicy 9 default patterns |
| Rate limit bypass | -- | Rapid repeated tool calls | RateLimiter sliding window |
| Git policy bypass | -- | Force push, branch deletion attempts | GitPolicy protected branches |
| Network exfiltration | -- | Outbound to private networks, non-HTTPS | NetworkPolicy filtering |
| Memory poisoning | AML.T0080 | Insert false knowledge entries | 4-stage ingestion pipeline |

### Continuous testing schedule

```
Daily:   Run quick suite (prompt injection, path traversal, credential patterns)
Weekly:  Run full suite (all categories)
Monthly: Run extended suite (multi-step attack chains, escalation scenarios)
```

### Configuration

```toml
[safety.adversarial_testing]
# Enable continuous adversarial testing.
enabled = true
# Quick suite interval (hours). Range: 1..168.
quick_interval_hours = 24
# Full suite interval (hours). Range: 24..720.
full_interval_hours = 168
# Alert threshold: fraction of failed tests. Range: 0.0..1.0.
alert_threshold = 0.1
# Results retention (days). Range: 7..365.
results_retention_days = 90
# Sandbox: run tests in isolated worktree.
sandboxed = true
```

### Test criteria

- Pipeline runs all test suites within the configured timeout
- Direct prompt injection test cases are correctly blocked by prompt architecture
- Path traversal test cases are blocked by PathPolicy
- Credential pattern test cases are caught by ScrubPolicy
- Alert fires when failure rate exceeds alert_threshold
- Results are persisted to results_path for trend analysis
- Sandboxed mode creates an isolated worktree for testing

---

## CSA MAESTRO layer mapping

Map Roko's defense-in-depth to the CSA MAESTRO 7-layer framework (Cloud Security Alliance, February 2025):

| MAESTRO Layer | Description | Roko implementation |
|---------------|-------------|---------------------|
| L1: Foundation Models | Base model threats (poisoning, extraction) | CascadeRouter model selection, model fingerprint in Provenance |
| L2: Data Operations | Data poisoning, RAG manipulation | 4-stage ingestion pipeline, TaintedString, BloomOracle |
| L3: Agent Frameworks | Framework vulnerabilities | Rust type safety, Capability<T>, SafetyLayer |
| L4: Deployment & Infrastructure | Container escape, network | PathPolicy, NetworkPolicy, ProcessSupervisor |
| L5: Evaluation & Observability | Monitoring blind spots | Gate pipeline, TemporalMonitor, conductor |
| L6: Agent-to-Agent Communication | Trust boundary violations | Cognitive Namespaces, NamespaceChannel ACL |
| L7: Human-AI Interaction | Social engineering, manipulation | Cognitive Signals (Pause/Escalate/Cooldown), EU AI Act compliance |

---

## Cross-References

- [01-capability-tokens.md](01-capability-tokens.md) — Compile-time enforcement via `Capability<T>`
- [02-audit-chain.md](02-audit-chain.md) — Cryptographic audit trail
- [03-taint-tracking.md](03-taint-tracking.md) — Data flow labels and taint propagation
- [07-prompt-security.md](07-prompt-security.md) — Ventriloquist defense, CaMeL architecture
- [08-threat-model.md](08-threat-model.md) — 21 failure modes and attack trees
- [16-critical-integration-gap.md](16-critical-integration-gap.md) — SafetyLayer→ToolDispatcher wired but not invoked from CLI
