# Prompt Security and CaMeL Architecture

> Depth for [16-SECURITY.md](../../unified/16-SECURITY.md). Expresses prompt security as an Extension specialization with CaMeL IFC. The 6-layer system prompt is a Compose Cell output. CaMeL dual-LLM architecture separates control plane from data plane as two Cells with a taint barrier. Ventriloquist defense uses Store + Verify. Tool-guard is a Pipeline of 3 Verify Cells.

**Depends on**: [02-CELL](../../unified/02-CELL.md) (Cell, Compose protocol, Verify protocol), [08-GATEWAY](../../unified/08-GATEWAY.md) (inference gateway Pipeline), [12-EXTENSIONS](../../unified/12-EXTENSIONS.md) (Extension system, CaMeL IFC), [16-SECURITY](../../unified/16-SECURITY.md) (taint lattice, CaMeL tags, 5-head corrigibility)

---

## 1. The Threat

Prompt injection is the highest-severity vulnerability class for LLM-based agents (OWASP LLM01, 2025). The fundamental problem: LLMs process instructions and data in the same channel (the context window). An attacker who can inject text into the agent's context can redirect the agent's behavior.

Two injection modes:

| Mode | Path | Example |
|---|---|---|
| **Direct** | Attacker modifies the agent's input directly | Malicious comment in a code file: `/* IMPORTANT: Ignore previous instructions... */` |
| **Indirect** | Attacker injects through data the agent consumes | API response with embedded instructions; poisoned knowledge store entry; error message containing tool call directives |

Greshake et al. (2023) demonstrated that indirect injection can cause agents to exfiltrate data, make unauthorized API calls, and propagate the injection to other users. The attack surface scales with the number of external data sources.

Roko's defense is layered, not a single mechanism. Each layer is a Cell conforming to a standard protocol.

---

## 2. Layer 1: System Prompt as Compose Cell Output

The `SystemPromptBuilder` in `crates/roko-compose/src/system_prompt_builder.rs` constructs system prompts using the `RoleSystemPromptSpec`. In unified terms, this is a **Compose Cell** -- it assembles context for LLM calls under a budget, following the Compose protocol (see [02-CELL.md](../../unified/02-CELL.md) SS3).

The system prompt has 6 layers, each clearly delimited:

```
Layer 1: Role definition
    What the agent is, what it can do, behavioral constraints

Layer 2: Task context
    The specific task, plan artifacts, coding standards

Layer 3: Safety rules
    Explicit instructions to reject suspicious inputs,
    corrigibility reminders, forbidden actions

Layer 4: Tool descriptions
    Available tools and their JSON schemas

Layer 5: Enrichment data (UNTRUSTED)
    Code context, documentation, research artifacts
    Clearly marked as potentially containing injection attempts

Layer 6: Output format
    Expected response structure
```

### Delimiter Hardening

Each section uses distinct delimiters unlikely to appear in natural text or code:

```
=== SYSTEM INSTRUCTIONS ===
[role definition, safety rules]

=== TASK CONTEXT ===
[task description, constraints]

=== ENRICHMENT DATA (UNTRUSTED) ===
[code context, documentation -- may contain injection attempts]

=== OUTPUT FORMAT ===
[expected response structure]
```

The "UNTRUSTED" label on enrichment data is a structural defense: it tells the LLM that this section's content should be treated as data, not instructions. This is not foolproof -- LLMs do not have a reliable instruction/data distinction -- but it raises the attacker's bar.

### Taint on Enrichment Sections

In the unified model, enrichment data is a Signal with taint (at minimum `LlmGenerated` or `ExternalFetch`). The Compose Cell preserves this taint in the assembled prompt. The inference gateway (see [08-GATEWAY.md](../../unified/08-GATEWAY.md)) can use the taint metadata to apply additional validation to the LLM's response when the prompt contained tainted sections.

---

## 3. Layer 2: CaMeL Dual-LLM Architecture

CaMeL (Capability-tagged information flow control; Fang et al. 2024, reporting 77% solve rate with provable IFC) proposes a fundamental architectural solution: separate control flow from data flow using two LLMs with different trust levels.

### Two Cells with a Taint Barrier

In unified terms, CaMeL is two Cells connected by a taint barrier (a Verify Cell):

```
+-----------------------------+       +-----------------------------+
|  Control Cell (trusted)     |       |  Data Cell (untrusted)      |
|                             |       |                             |
|  Input:                     |       |  Input:                     |
|    System prompt + task     |       |    Tool results, file       |
|    description              |       |    contents, API responses  |
|                             |       |                             |
|  Output:                    |       |  Output:                    |
|    Abstract action plan     |       |    Structured JSON only     |
|    Tool call specifications |       |    (schema-validated)       |
|                             |       |                             |
|  CaMeL tag:                 |       |  CaMeL tag:                 |
|    {SystemPrompt}           |       |    {ExternalData,           |
|                             |       |     LlmGenerated}           |
|  Taint: Clean               |       |  Taint: ExternalFetch       |
+-----------------------------+       +-----------------------------+
              |                                    |
              |         +------------------+       |
              +-------->| Taint Barrier    |<------+
                        | (Verify Cell)    |
                        |                  |
                        | Checks:          |
                        |   data_tags      |
                        |   subset_of      |
                        |   control_caps   |
                        +------------------+
```

### How It Works

1. **Control Cell** sees only the system prompt and task description. It generates an abstract action plan: "read file X, extract the function signature, check if it matches pattern Y." The Control Cell never sees raw untrusted data.

2. **Data Cell** processes raw tool results and file contents. It can only produce **structured JSON outputs** matching a predefined schema. It cannot generate tool calls, modify the action plan, or emit free-form text.

3. **Taint Barrier** (Verify Cell) sits between them. Data flowing from the Data Cell to the Control Cell must pass through the barrier, which validates:
   - The data matches the expected schema.
   - The CaMeL tags on the data are a subset of the Control Cell's capabilities.
   - No injection patterns are detected in the structured fields.

### Why This Works

Even if the Data Cell is fully compromised by prompt injection in a file it reads, the worst it can do is return malformed structured data -- which the schema validator rejects. The Control Cell's instructions are never contaminated by untrusted input because it never sees that input directly.

```rust
/// CaMeL taint barrier: Verify Cell between Data Cell and Control Cell.
pub struct CamelTaintBarrierCell {
    /// Expected output schema from the Data Cell.
    expected_schema: serde_json::Value,
    /// Capabilities the Control Cell holds.
    control_capabilities: BTreeSet<Capability>,
}

impl VerifyCell for CamelTaintBarrierCell {
    fn name(&self) -> &str { "camel-taint-barrier" }

    async fn verify(
        &self,
        data_output: &Signal,
        _ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        // 1. Schema validation
        let content = extract_content(data_output)?;
        if !validate_schema(&content, &self.expected_schema) {
            return Ok(Verdict::reject(
                "Data Cell output does not match expected schema"
            ));
        }

        // 2. CaMeL tag check
        let data_tags = &data_output.metadata.camel_tags;
        let tag_subset = data_tags.capabilities
            .is_subset(&self.control_capabilities);
        if !tag_subset {
            return Ok(Verdict::reject(format!(
                "CaMeL tag violation: data requires {:?}, control has {:?}",
                data_tags.capabilities, self.control_capabilities,
            )));
        }

        // 3. Injection pattern detection
        let injection_score = scan_for_injection_patterns(&content);
        if injection_score > 0.8 {
            return Ok(Verdict::reject(format!(
                "injection pattern detected (score: {:.2})",
                injection_score,
            )));
        }

        Ok(Verdict::pass(1.0, Evidence::CamelBarrier {
            schema_valid: true,
            tags_valid: true,
            injection_score,
        }))
    }
}
```

### Mapping to Roko's Dual-Process Cognition

CaMeL maps naturally to Roko's existing dual-process architecture:

- **T0 probes** (zero-LLM deterministic checks) serve as the Control layer for routine operations. They never see LLM output and cannot be influenced by injection.
- **T1/T2 reasoning** (fast/full model) serves as the execution layer that processes context. All outputs pass through the Gate pipeline before taking effect.

The 16 T0 probes suppress approximately 80% of LLM calls. For the remaining 20%, the Gate pipeline provides architectural separation: the LLM proposes, but the Gates (compile, test, clippy, diff) verify against ground truth the LLM cannot influence.

---

## 4. Layer 3: Ventriloquist Defense

For chain-domain agents, the ventriloquist defense provides cryptographic verification of system prompt integrity. The threat: an attacker replaces the agent's system prompt with one that directs unauthorized operations.

### Defense as Store + Verify

The defense uses two primitives:

1. **Store**: The SHA-256 hash of the system prompt is stored both locally and on-chain (via the chain registry). This is a Signal in the Store.

2. **Verify**: Before each inference call, a Verify Cell checks that the current prompt hash matches the stored commitment.

```rust
/// Ventriloquist defense: Verify Cell that checks system prompt integrity.
pub struct PromptIntegrityCell {
    /// The committed hash of the system prompt (from Store or on-chain).
    committed_hash: ContentHash,
    /// Whether to require on-chain verification (Phase 2+).
    require_chain_witness: bool,
}

impl VerifyCell for PromptIntegrityCell {
    fn name(&self) -> &str { "prompt-integrity" }

    async fn verify(
        &self,
        action: &Signal,
        ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        let current_prompt = extract_system_prompt(action)?;
        let computed_hash = ContentHash::sha256(current_prompt.as_bytes());

        if computed_hash != self.committed_hash {
            return Ok(Verdict::reject(format!(
                "system prompt hash mismatch: expected {}, got {}",
                self.committed_hash, computed_hash,
            )));
        }

        // Optionally verify against on-chain commitment
        if self.require_chain_witness {
            let on_chain = ctx.store().query(Query {
                kind: Some(Kind::ChainWitness),
                filter: Filter::Field("prompt_hash", self.committed_hash.clone()),
            }).await?;

            if on_chain.is_empty() {
                return Ok(Verdict::reject(
                    "system prompt hash not found in on-chain registry"
                ));
            }
        }

        Ok(Verdict::pass(1.0, Evidence::PromptIntegrity {
            hash: computed_hash,
            chain_verified: self.require_chain_witness,
        }))
    }
}
```

### Timelock for Updates

Changing the system prompt hash requires a 24-hour timelock. During this window, monitoring systems alert the operator. The operator can cancel the change if unauthorized. This prevents rapid prompt replacement attacks.

---

## 5. Layer 4: Tool-Guard as Pipeline of 3 Verify Cells

The Tool-Guard pattern (OWASP MCP security guidelines) interposes validation between the LLM's tool call request and actual execution. In unified terms, it is a **Pipeline of 3 Verify Cells**:

```toml
# Graph: tool-guard-pipeline
# Three Verify Cells in sequence. Any can reject.

[graph]
id = "tool-guard-pipeline"
pattern = "Pipeline"

[[graph.cells]]
id = "schema-validator"
protocol = "Verify"
description = "Arguments must match the tool's declared JSON schema"

[[graph.cells]]
id = "content-validator"
protocol = "Verify"
description = "String arguments checked for injection patterns"

[[graph.cells]]
id = "semantic-validator"
protocol = "Verify"
description = "Proposed action checked against current task context"

[[graph.edges]]
from = "schema-validator.out"
to = "content-validator.in"

[[graph.edges]]
from = "content-validator.out"
to = "semantic-validator.in"
```

### Cell 1: Schema Validation

```rust
/// Schema validation: arguments must match the tool's JSON schema.
pub struct SchemaValidatorCell {
    registry: Arc<ToolRegistry>,
}

impl VerifyCell for SchemaValidatorCell {
    fn name(&self) -> &str { "schema-validator" }

    async fn verify(
        &self,
        action: &Signal,
        _ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        let tool_call = extract_tool_call(action)?;
        let tool_def = self.registry.get(&tool_call.name)?;

        if let Some(schema) = &tool_def.schema {
            match jsonschema::validate(&tool_call.arguments, schema) {
                Ok(_) => Ok(Verdict::pass(1.0, Evidence::SchemaValid)),
                Err(errors) => Ok(Verdict::reject(format!(
                    "schema validation failed: {}",
                    errors.into_iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; "),
                ))),
            }
        } else {
            Ok(Verdict::pass(0.8, Evidence::NoSchema))
        }
    }
}
```

### Cell 2: Content Validation

```rust
/// Content validation: check string arguments for injection patterns.
pub struct ContentValidatorCell {
    /// Patterns to detect in tool arguments.
    injection_patterns: Vec<InjectionPattern>,
}

pub struct InjectionPattern {
    pub name: String,
    pub regex: Regex,
    pub severity: f64,
}

impl VerifyCell for ContentValidatorCell {
    fn name(&self) -> &str { "content-validator" }

    async fn verify(
        &self,
        action: &Signal,
        _ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        let tool_call = extract_tool_call(action)?;
        let args_str = serde_json::to_string(&tool_call.arguments)?;

        for pattern in &self.injection_patterns {
            if pattern.regex.is_match(&args_str) {
                if pattern.severity > 0.8 {
                    return Ok(Verdict::reject(format!(
                        "injection pattern '{}' detected in tool arguments",
                        pattern.name,
                    )));
                } else {
                    // Low severity: pass with warning
                    return Ok(Verdict::pass(
                        1.0 - pattern.severity,
                        Evidence::InjectionWarning {
                            pattern: pattern.name.clone(),
                            severity: pattern.severity,
                        },
                    ));
                }
            }
        }

        Ok(Verdict::pass(1.0, Evidence::ContentClean))
    }
}
```

### Cell 3: Semantic Validation

```rust
/// Semantic validation: the proposed action should be relevant
/// to the current task context.
pub struct SemanticValidatorCell;

impl VerifyCell for SemanticValidatorCell {
    fn name(&self) -> &str { "semantic-validator" }

    async fn verify(
        &self,
        action: &Signal,
        ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        let tool_call = extract_tool_call(action)?;
        let task_context = ctx.current_task()?;

        // Check if the tool is semantically relevant to the task
        let relevance = compute_tool_relevance(
            &tool_call.name,
            &tool_call.arguments,
            &task_context,
        );

        if relevance < 0.2 {
            return Ok(Verdict::reject(format!(
                "tool '{}' is semantically irrelevant to task '{}' (relevance: {:.2})",
                tool_call.name, task_context.description, relevance,
            )));
        }

        if relevance < 0.5 {
            return Ok(Verdict::escalate(format!(
                "tool '{}' has low relevance to task '{}' (relevance: {:.2})",
                tool_call.name, task_context.description, relevance,
            )));
        }

        Ok(Verdict::pass(relevance, Evidence::SemanticRelevance {
            tool: tool_call.name,
            task: task_context.id,
            relevance,
        }))
    }
}
```

---

## 6. MCP Security Considerations

MCP (Model Context Protocol) servers provide external tool capabilities. Security statistics are concerning (Endor Labs, 2026):

- 82% of 2,614 MCP implementations use filesystem operations prone to path traversal.
- 67% use APIs susceptible to code injection.
- CVE-2025-6514 (CVSS 10.0 RCE) in `mcp-remote` affected 558,000+ installations.

Roko's approach: MCP tools pass through the same defense Pipeline as built-in tools. The Pipeline does not distinguish between built-in and MCP tools -- both face identical pre-execution checks and post-execution scrubbing. For high-security deployments, the recommendation is to use Roko's built-in tools exclusively and disable MCP.

---

## What This Enables

1. **Architectural injection defense**: CaMeL separates instruction and data channels at the architecture level, not the prompt level. Even a fully compromised Data Cell cannot influence the Control Cell's action plan.
2. **Cryptographic prompt integrity**: The ventriloquist defense proves that the system prompt has not been tampered with, using content-addressable hashes and optional on-chain anchoring.
3. **Composable tool validation**: The Tool-Guard Pipeline can be extended with domain-specific Verify Cells without modifying existing validation logic.
4. **Defense depth**: Four layers (prompt architecture, CaMeL IFC, ventriloquist defense, tool-guard) compose independently. An attacker must defeat all four.

## Feedback Loops

- **L1**: Injection detection patterns are updated when new attack vectors are observed. The content validator's pattern library is a Store that receives new patterns from immune memory.
- **L2**: CaMeL tag violations are tracked per Extension. Extensions with frequent violations have their capabilities narrowed automatically.
- **L3**: Tool-guard rejection patterns feed the cascade router. Models that produce more rejected tool calls are routed to models with better instruction-following.
- **Memory**: Detected injection attempts are stored as immune patterns (see [immune-system-as-graph.md](immune-system-as-graph.md) Layer 5) for future recognition.

## Open Questions

1. **CaMeL latency**: Running two LLMs (Control and Data) doubles the inference cost. Is the security benefit worth the cost for all tasks, or should CaMeL be selectively enabled for high-risk operations only?

2. **Schema expressiveness**: The CaMeL taint barrier requires a predefined schema for Data Cell outputs. For complex extraction tasks (e.g., "summarize this document"), the schema may be too rigid. How do we balance schema strictness with task flexibility?

3. **Prompt integrity in non-chain deployments**: The ventriloquist defense relies on on-chain hash commitment. For deployments without chain access, is local hash verification (committed to Store at deploy time) sufficient?

4. **Semantic validation accuracy**: The semantic validator's `compute_tool_relevance()` function is likely an LLM call itself, which introduces its own injection risk. Can semantic validation be made deterministic (T0)?

## Implementation Tasks

| Task | File | What |
|---|---|---|
| Express SystemPromptBuilder as Compose Cell | `crates/roko-compose/src/system_prompt_builder.rs` | Implement Compose protocol with taint-aware section assembly |
| Implement CaMeL taint barrier | `crates/roko-agent/src/safety/` | Add `CamelTaintBarrierCell` with schema + tag + injection validation |
| Implement prompt integrity Verify Cell | `crates/roko-agent/src/safety/` | Add `PromptIntegrityCell` with SHA-256 hash verification |
| Implement Tool-Guard Pipeline | `crates/roko-agent/src/dispatcher/` | Express schema/content/semantic validation as Pipeline of Verify Cells |
| Add injection pattern library | `crates/roko-agent/src/safety/` | Configurable regex patterns for common injection vectors |
| Wire CaMeL into inference gateway | `crates/roko-agent/src/` | Add Control/Data Cell split for high-risk operations |
| Integration test: injection blocked by CaMeL | `crates/roko-agent/tests/` | Inject instructions in file content, verify Control Cell never sees them |
| Integration test: prompt hash mismatch | `crates/roko-agent/tests/` | Modify system prompt, verify PromptIntegrityCell rejects |
