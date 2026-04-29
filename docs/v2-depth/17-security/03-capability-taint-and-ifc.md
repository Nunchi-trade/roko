# Capability, Taint, and Information Flow Control

> Depth for [16-SECURITY.md](../../unified/16-SECURITY.md). Expresses capability tokens and taint tracking as Cell capabilities intersected at Graph edges, plus a monotonic taint lattice on Signals. Covers CaMeL IFC, taint flow through Bus (Pulses), and the question of taint declassification.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, taint, provenance), [02-CELL](../../unified/02-CELL.md) (Cell capabilities, Verify protocol), [03-GRAPH](../../unified/03-GRAPH.md) (Graph edges, capability resolution), [12-EXTENSIONS](../../unified/12-EXTENSIONS.md) (Extension system, CaMeL IFC), [16-SECURITY](../../unified/16-SECURITY.md) (three-layer capability intersection, taint lattice)

---

## 1. Two Complementary Systems

Roko's security model has two orthogonal control systems that reinforce each other:

- **Capabilities** answer: "Is this Cell allowed to perform this action?" They are about *authorization* -- what a Cell may do.
- **Taint** answers: "Can this data be trusted for this purpose?" It is about *provenance* -- where data came from and whether it has been reviewed.

A Cell may have the capability to write files (authorized) but be working with tainted data (unreviewed external fetch). The capability system says "yes, you may write." The taint system says "wait -- the data you are writing came from an untrusted source." Both must agree before the action proceeds. Neither subsumes the other.

---

## 2. Capabilities as Three-Layer Intersection

See [16-SECURITY.md](../../unified/16-SECURITY.md) SS2-3 for the full type definitions.

### Layer 1: Cell Declaration

Every Cell declares what capabilities it requires. This is a **ceiling** -- the Cell can never access a resource it did not declare, regardless of what the Graph or Space permit.

```toml
# Cell manifest
[cell.capabilities]
required = [
  { "FsRead"  = { paths = ["src/**", "docs/**"] } },
  { "FsWrite" = { paths = [".roko/artifacts/**"] } },
  { "Shell"   = { commands = ["cargo", "rustc"] } },
  { "Llm" = {} },
]
```

### Layer 2: Graph Allow-List

The Graph narrows what its constituent Cells can do. Omitting a capability from the Graph's allow-list denies it. This is the **composition boundary** -- a powerful Cell becomes safe to use in a restricted Graph by simply not allowing dangerous capabilities.

```toml
# Graph allow-list
[graph.capabilities]
allow = [
  { "FsRead"  = { paths = ["src/**"] } },   # narrower than Cell
  { "FsWrite" = { paths = [".roko/**"] } },
  { "Llm" = {} },
  # Shell intentionally omitted -- not allowed in this Graph
]
```

### Layer 3: Space Grant

The Space (workspace) is the user's authority. The user grants capabilities in `workspace.toml`. This is the **trust boundary** -- the user's final word.

```toml
[space.capabilities]
fs_read       = true
fs_write      = { paths = [".roko/**", "tmp/**"] }
net           = { domains = ["api.anthropic.com", "api.openai.com"] }
llm           = true
shell         = { commands = ["cargo", "git", "npm"] }
chain_write   = false
```

### Intersection Algorithm

The effective capability is the **intersection** of all three layers. The narrowest constraint at any layer wins.

```rust
/// Compute effective capabilities for a Cell in a Graph under a Space.
///
/// The result is the intersection of:
///   1. What the Cell declares it needs
///   2. What the Graph allows
///   3. What the Space grants
///
/// A capability absent from any single layer is denied.
pub fn effective_capabilities(
    cell_declared: &[Capability],
    graph_allowed: &[Capability],
    space_granted: &[Capability],
) -> Vec<EffectiveCapability> {
    cell_declared.iter().filter_map(|cell_cap| {
        // Must be present in Graph allow-list
        let graph_cap = graph_allowed.iter()
            .find(|g| g.same_variant(cell_cap))?;
        // Must be present in Space grants
        let space_cap = space_granted.iter()
            .find(|s| s.same_variant(cell_cap))?;

        // Intersect constraints at each layer
        Some(EffectiveCapability {
            variant: cell_cap.variant(),
            constraints: cell_cap.constraints()
                .intersect(&graph_cap.constraints())
                .intersect(&space_cap.constraints()),
        })
    }).collect()
}
```

### Capability Narrowing on Delegation

When a Cell delegates work to another Cell (e.g., a Graph Cell invoking a sub-Graph), capabilities are **narrowed, never widened**. The child inherits at most the parent's effective capabilities. The delegation chain is logged as a sequence of `DelegationGrant` Signals.

```rust
pub struct DelegationGrant {
    pub from: CellRef,
    pub to: CellRef,
    pub capabilities: Vec<Capability>,  // subset of parent's effective
    pub caveats: Vec<Caveat>,           // additional restrictions
    pub camel_tags: CamelTag,           // IFC tags propagated
    pub timestamp: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub enum Caveat {
    TimeLimit(Duration),
    UsageLimit(u32),
    PathRestriction(Vec<PathPattern>),
    DomainRestriction(Vec<String>),
    ReadOnly,
}
```

---

## 3. The Taint Lattice

Taint is durable metadata on every Signal. It tracks the provenance of data across trust boundaries. The lattice is ordered by increasing untrust:

```
Clean < UserInput < LlmGenerated < ExternalFetch < Propagated
```

See [16-SECURITY.md](../../unified/16-SECURITY.md) SS4 for the type definition. The critical property is **monotonicity**: taint can only increase through derivation, never decrease.

### Why Five Levels

Each level answers a different trust question:

| Taint Level | Trust Question | Example |
|---|---|---|
| `Clean` | Was this produced by trusted system code? | Gate verdicts, config values, built-in tool schemas |
| `UserInput` | Was this provided by a human? | Pasted text, uploaded files, ad hoc instructions |
| `LlmGenerated` | Was this produced by a model? | LLM completions, model-generated plans |
| `ExternalFetch` | Was this fetched from outside the system? | HTTP responses, API results, scraped pages |
| `Propagated` | Was this derived from tainted ancestors? | Summaries of fetched data, composed prompts mixing sources |

### Monotonic Lattice-Join

When a Cell produces output from multiple inputs, the output's taint is the **maximum** of all input taints:

```rust
/// Monotonic lattice join: returns the maximum taint.
/// Guarantee: taint(descendant) >= taint(ancestor) for all ancestors.
fn lattice_join(taints: &[Taint]) -> Taint {
    taints.iter()
        .max()
        .cloned()
        .unwrap_or(Taint::Clean)
}
```

This prevents taint laundering: an attacker cannot produce a "clean" Signal by mixing a tainted Signal with a clean one. The derived Signal inherits the highest taint.

### Taint at Trust Boundaries

Taint is assigned when data crosses a trust boundary:

| Trust Boundary | Taint Assigned | Assignment Point |
|---|---|---|
| User input (paste, upload) | `UserInput` | SENSE step of cognitive loop |
| LLM completion | `LlmGenerated` | After model call in inference gateway |
| HTTP fetch / API call | `ExternalFetch(source)` | After Connect protocol call |
| Plugin output | `ExternalFetch(plugin_id)` | After Extension hook execution |
| Knowledge import | `Propagated` | At import time, inheriting source taint |

---

## 4. Taint Flow Through Bus (Pulses)

A subtle question: does taint propagate through the Bus (ephemeral Pulses)?

**Yes, but differently.** Pulses are ephemeral -- they do not persist in Store and are not content-addressed. But they carry data across trust boundaries. The rule:

- **Pulses inherit taint from the Signal that generated them.** A Pulse published by a Cell processing `ExternalFetch` data carries `ExternalFetch` taint.
- **Graduation (Pulse to Signal) preserves taint.** When a Pulse is promoted to a Signal via Graduation (see [01-SIGNAL.md](../../unified/01-SIGNAL.md)), the resulting Signal inherits the Pulse's taint.
- **React Cells that consume Pulses propagate taint.** A React Cell subscribing to a Bus topic produces Signals whose taint is the lattice-join of all consumed Pulse taints.

```rust
/// Pulse taint propagation rule.
/// When graduating a Pulse to a Signal, preserve the Pulse's taint.
pub fn graduate_with_taint(pulse: &Pulse) -> Signal {
    let mut signal = Signal::from_pulse(pulse);
    signal.metadata.taint = pulse.taint.clone();
    // Taint cannot decrease through graduation
    assert!(signal.metadata.taint >= pulse.taint);
    signal
}
```

### Why Pulses Need Taint

Without Pulse taint, an attacker could:

1. Inject tainted data into the system.
2. Have a React Cell publish the data as a Pulse (stripping taint).
3. Have another React Cell subscribe, graduate the Pulse to a Signal, and use it as "clean."

Pulse taint closes this laundering path.

---

## 5. CaMeL IFC on Extensions

CaMeL (Capability-tagged information flow control; Fang et al. 2024) extends taint tracking with **capability provenance**. Every data flow through an Extension is tagged with which capabilities were involved in producing it.

See [16-SECURITY.md](../../unified/16-SECURITY.md) SS4.3-4.5 for the full specification.

### The Problem CaMeL Solves

Taint tracks origin (where data came from). CaMeL tracks capability exposure (what system resources touched the data). This matters because:

- Data that passed through a `Secrets`-capable Cell should not flow to a `Net`-capable Cell without declassification.
- Data that was read from sensitive file paths should not be included in prompts sent to untrusted models.
- The taint level alone does not capture which capabilities were exercised -- only CaMeL tags do.

### Tag Structure

```rust
pub struct CamelTag {
    /// Which capabilities were involved in producing this data.
    pub capabilities: BTreeSet<Capability>,
    /// Which Cells touched this data, in order.
    pub provenance: Vec<CellRef>,
    /// Aggregated taint from all producers.
    pub taint: Taint,
}
```

### Three Propagation Rules

1. **Union on input**: When an Extension receives data tagged `T1` and `T2`, the working set has tag `T1 union T2`.
2. **Inherit on output**: Any data the Extension produces inherits the union tag from its inputs.
3. **Capability check on consumption**: When a Cell receives tagged data from an Extension, the Cell's effective capabilities must include all capabilities in the tag. Otherwise, the data is rejected.

```rust
/// Compute output CaMeL tags for an Extension.
/// Tags can only grow (union), never shrink.
fn compute_output_tags(
    input_tags: &CamelTag,
    extension_caps: &[Capability],
    extension_ref: CellRef,
) -> CamelTag {
    CamelTag {
        capabilities: input_tags.capabilities
            .union(&extension_caps.iter().cloned().collect())
            .cloned()
            .collect(),
        provenance: {
            let mut p = input_tags.provenance.clone();
            p.push(extension_ref);
            p
        },
        taint: lattice_join(&[input_tags.taint.clone(), compute_taint(extension_caps)]),
    }
}
```

### No-Laundering Guarantee

Extensions **cannot strip tags**. The runtime computes output tags as `input_tags union extension_capability_tags`, never as a subset. This is the CaMeL invariant. If an Extension could strip tags, it could launder sensitive data into unrestricted flows.

### CaMeL as Two Cells with a Taint Barrier

The CaMeL dual-LLM architecture (see [06-prompt-security-and-camel.md](06-prompt-security-and-camel.md) for full treatment) separates control plane from data plane as two Cells:

```
Control Cell (trusted)                Data Cell (untrusted)
  - Sees system prompt + task           - Sees tool results, file contents
  - Generates action plan               - Extracts structured data only
  - Never sees raw untrusted data       - Cannot generate tool calls
  - CaMeL tag: {SystemPrompt}           - CaMeL tag: {ExternalData, LlmGenerated}
                                         |
                            taint barrier (Verify Cell)
                            checks: data_tags subset_of control_caps
```

The taint barrier between them is a Verify Cell that checks CaMeL tag compatibility before allowing data to flow from the Data Cell to the Control Cell. Data tagged `{Secrets}` can never reach the Data Cell (untrusted) without explicit declassification.

---

## 6. Taint Declassification

Can taint ever be formally downgraded? The monotonic lattice says taint can only increase. But some scenarios demand declassification:

- A URL that was hostile but is now controlled by the operator.
- An LLM-generated plan that has been human-reviewed and approved.
- An external fetch result that has been independently verified against a trusted source.

### Declassification as Custody Event

Declassification is **not** taint reduction. It is an explicit approval event recorded in the custody chain. The original Signal retains its taint. A new Signal is produced with a `Declassified` taint level and a custody link proving who approved the declassification.

```rust
/// Declassification event. The original Signal's taint is NOT modified.
/// Instead, a new Signal is produced with custody-backed approval.
pub struct Declassification {
    /// The original tainted Signal.
    pub original_hash: ContentHash,
    /// The original taint level (preserved for audit).
    pub original_taint: Taint,
    /// The declassified taint level (always lower than original).
    pub declassified_taint: Taint,
    /// Who approved the declassification.
    pub approved_by: PrincipalId,
    /// Why the declassification was approved.
    pub reason: String,
    /// Custody link proving the approval chain.
    pub custody_link: ContentHash,
    /// Scope: what is the declassified data allowed to do?
    pub scope: DeclassificationScope,
}

pub enum DeclassificationScope {
    /// Declassified for inclusion in prompts to trusted models.
    PromptInclusion,
    /// Declassified for persistence to Store.
    StorePersistence,
    /// Declassified for transmission over Net.
    NetTransmission,
    /// Declassified for all purposes.
    Full,
}
```

### Rules for Declassification

1. **Only humans can declassify.** The agent cannot declassify its own data. The `approved_by` field must be a human principal, not an agent.
2. **Declassification is scoped.** Approving data for prompt inclusion does not approve it for network transmission. Each scope requires separate approval.
3. **Declassification is logged.** Every declassification event is persisted as a `SecurityEvent::Declassification` Signal (see [16-SECURITY.md](../../unified/16-SECURITY.md) SS14).
4. **The original taint is preserved.** The original Signal is never modified. Lineage traversal always shows the full taint history.
5. **Sensitive data requires elevated attestation.** Declassifying data tagged with `Secrets` capabilities requires `OrgRole` attestation, not just `LocalAgent`.

---

## 7. Capability + Taint Composition

The two systems compose at action time. Every proposed action is checked against both:

```rust
/// Combined capability + taint check before action execution.
///
/// Both must pass. Capability says "you may." Taint says "the data is trusted."
pub async fn check_action(
    action: &ProposedAction,
    effective_caps: &[EffectiveCapability],
    ctx: &CellContext,
) -> Result<Verdict, Rejection> {
    // 1. Capability check: does the Cell have permission?
    let cap_check = check_capabilities(action, effective_caps)?;
    if cap_check.is_reject() {
        return Err(Rejection::capability(cap_check));
    }

    // 2. Taint check: is the data trusted for this destination?
    let taint = action.input_taint();
    let destination_risk = classify_destination_risk(action);

    match (taint, destination_risk) {
        // Clean data to any destination: pass
        (Taint::Clean, _) => Ok(Verdict::pass(1.0, Evidence::TaintClean)),

        // Any taint to low-risk destination: pass with annotation
        (_, DestinationRisk::Low) => Ok(Verdict::pass(
            0.8,
            Evidence::TaintAccepted { taint, risk: destination_risk },
        )),

        // High taint to high-risk destination: reject or escalate
        (Taint::ExternalFetch(_) | Taint::Propagated, DestinationRisk::High) => {
            Err(Rejection::taint(format!(
                "tainted data ({:?}) cannot flow to high-risk destination ({:?}) \
                 without declassification",
                taint, destination_risk,
            )))
        }

        // Medium taint to medium-risk: escalate for human review
        _ => Ok(Verdict::escalate(
            "tainted data requires human review for this destination",
        )),
    }
}
```

---

## What This Enables

1. **Compile-time safety**: In the target state, `Capability<T>` tokens make unsafe operations impossible to write -- not just checked at runtime, but rejected by the compiler (see [16-SECURITY.md](../../unified/16-SECURITY.md) SS2).
2. **No-laundering guarantee**: Neither capability narrowing nor taint monotonicity can be circumvented by derivation, delegation, or Extension processing.
3. **Scoped declassification**: Human approval does not blanket-clean data. It authorizes specific uses, leaving the original provenance intact for audit.
4. **CaMeL IFC**: Capability provenance tracks which resources touched data, preventing sensitive data from leaking through Extensions.
5. **Bus taint**: Ephemeral Pulses carry taint, preventing laundering through the Bus fabric.

## Feedback Loops

- **L1**: Capability denial patterns feed the cascade router. Models/agents that trigger frequent capability denials are routed to more restricted Graphs.
- **L2**: Taint escalation rates are tracked per data source. Sources that consistently produce escalations have their taint threshold raised (auto-quarantine on import).
- **L3**: Declassification frequency per source informs the immune system. A source requiring frequent declassification may be reclassified to a higher taint tier.
- **Memory**: CaMeL tag violation patterns are stored as immune memory patterns (see [immune-system-as-graph.md](immune-system-as-graph.md) Layer 5).

## Open Questions

1. **CaMeL overhead**: Tracking capability tags on every data flow adds metadata cost. For a high-throughput inference gateway processing hundreds of requests per second, is the tag propagation overhead acceptable? The tag is a `BTreeSet<Capability>` (typically 3-5 entries) plus a `Vec<CellRef>` provenance chain. This is small but non-zero.

2. **Taint granularity**: The current lattice has 5 levels. Is this sufficient? Some scenarios suggest finer-grained taint: distinguishing between "fetched from a trusted API" and "fetched from an unknown website." The lattice could be extended with `ExternalFetch(Source)` carrying source reputation, but this increases complexity.

3. **Cross-deployment taint**: When Signals are exported (brain export, mesh sync), should taint travel with them? If so, how does the receiving deployment validate taint claims it did not originate? The custody chain is the natural validation mechanism, but cross-deployment custody verification requires shared trust roots.

4. **Implicit declassification**: Should there be a time-based declassification mechanism where data that has been in the system for N days without incident is automatically reclassified? This would reduce operator burden but creates a window for slow-acting poisoning attacks.

## Implementation Tasks

| Task | File | What |
|---|---|---|
| Implement three-layer intersection | `crates/roko-core/src/extension.rs` | Wire `effective_capabilities()` into Graph loading |
| Add taint field to Signal metadata | `crates/roko-core/src/signal.rs` | Extend `Provenance` from `tainted: bool` to `Taint` enum |
| Implement CaMeL tag propagation | `crates/roko-core/src/extension.rs` | Add `CamelTag` to Extension data flows |
| Add Pulse taint propagation | `crates/roko-core/src/bus.rs` | Ensure Pulses carry taint from generating Signal |
| Wire taint check into defense Pipeline | `crates/roko-agent/src/safety/` | Add `TaintBarrierCell` as Layer 5 of defense Pipeline |
| Implement declassification flow | `crates/roko-cli/src/orchestrate.rs` | Add declassification approval UI and custody emission |
| Test: taint monotonicity property | `crates/roko-core/tests/` | Property test: no derivation chain decreases taint |
| Test: CaMeL no-laundering | `crates/roko-core/tests/` | Test: Extension cannot strip capability tags |
