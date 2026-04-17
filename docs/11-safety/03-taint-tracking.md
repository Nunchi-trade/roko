# Taint-Aware Ingestion and Data Flow Control

> **Layer:** L2 Scaffold, L3 Harness
>
> **Cross-cut:** Safety & Provenance
>
> **Alignment:** This doc applies [REF32](../../tmp/refinements/32-safety-sandbox-provenance.md). For shared terminology, see [docs/00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) and [docs/00-architecture/26-cognitive-immune-system.md](../00-architecture/26-cognitive-immune-system.md).

---

## Overview

Taint tracking is the safety spine's answer to a basic question: which inputs are still untrusted?

Roko treats taint as durable metadata, not an informal warning. If a prompt, fetch result, plugin output, or imported Engram influenced the current action, that fact should remain visible until a reviewer explicitly signs off on it. Taint is therefore:

- attached when data crosses a trust boundary,
- propagated through composition and action,
- consulted by gates before high-risk effects,
- copied into Custody so auditors can reconstruct the risk state at the time.

The model is intentionally one-way. Inputs can accumulate taint automatically; they cannot become clean automatically.

---

## 1. Taint Taxonomy

REF32's baseline taxonomy is narrow on purpose:

```rust
pub enum Taint {
    None,
    UserInput,
    ExternalFetch(Source),
    ThirdPartyPlugin(PluginId),
    LegacyImport,
}
```

These labels answer different questions:

| Taint | What it means |
|---|---|
| `UserInput` | Human-provided content that has not been independently validated or reviewed |
| `ExternalFetch` | Data fetched across the network or from a remote API |
| `ThirdPartyPlugin` | Output from a plugin whose behavior is not part of the trusted kernel |
| `LegacyImport` | Data imported from another deployment or older storage regime |

The taxonomy is intentionally about origin rather than moral judgment. Taint means "treat carefully," not "discard."

---

## 2. Propagation Rules

The propagation rule is simple and strict: if an Engram or composed prompt depends on tainted input, the output stays tainted.

Examples:

- A Composer that reads `UserInput` and `ExternalFetch` sources produces a tainted prompt.
- An LLM completion based on that prompt remains tainted.
- A plugin result generated from a tainted prompt remains tainted even if the plugin itself is trusted.
- A summary Engram synthesized from a tainted fetch still carries the fetch taint.

This matters because the highest-risk failures often happen one or two hops after the original untrusted input. If taint stopped at ingestion, the dangerous part of the decision chain would become invisible exactly when action is about to occur.

---

## 3. Taint Through the Seven-Step Loop

| Loop step | Taint behavior |
|---|---|
| SENSE | Attach taint to inbound Engrams from users, remote sources, plugins, or imports. |
| ASSESS | Use taint as an input to routing, scoring, and permission thresholds. |
| COMPOSE | Preserve taint in prompts, plans, and intermediate context packs. |
| ACT | Read taint before tool use, network egress, publication, signing, or filesystem writes. |
| VERIFY | Gates can deny, confirm, or escalate based on taint and target sensitivity. |
| PERSIST / BROADCAST | Persist taint in Engrams and include it in Custody; publish only to allowed topics and tenants. |
| REACT | Disable plugins, tighten policies, or open incidents when taint repeatedly reaches blocked destinations. |

Taint is therefore not just an ingestion concern. It is a runtime control plane signal.

---

## 4. High-Risk Destinations

Taint matters most when the target is costly to undo or hard to audit after the fact.

Typical high-risk destinations:

- signing a chain transaction,
- sending data to an external API,
- writing to production infrastructure,
- publishing a pull request or other public artifact,
- persisting a heuristic or claim as trusted knowledge.

Expected policy behavior:

- benign low-risk actions may continue with taint recorded in Custody,
- medium-risk actions require a confirm or review checkpoint,
- high-risk actions with unresolved taint escalate or fail closed.

A good example is a chain action with a recipient address extracted from `ExternalFetch`. The correct default is not "try and log it"; it is "escalate until reviewed."

---

## 5. Taint vs. Secret Handling

Taint and secret handling overlap but are not the same thing.

- **Taint** tracks trust and origin.
- **Secret handling** tracks sensitivity and redaction.

A value may be:

- tainted but not secret, such as a web page,
- secret but not tainted, such as a locally stored credential,
- both tainted and secret, such as a pasted credential from a user.

The safety spine therefore uses both:

- taint to decide whether an action may proceed,
- scrubbing and secret types to prevent disclosure in Pulses, logs, or persisted outputs.

This distinction avoids a common failure mode where secret redaction exists but the system still takes high-risk actions on unreviewed external data.

---

## 6. Review as the Only Cleaning Action

Taint should not disappear because time passed or because another model summarized the input. The only legitimate route from tainted to trusted is an explicit review step that records:

- who reviewed it,
- what scope they approved,
- when they approved it,
- and which resulting Engram or action the approval covers.

That review should itself appear in Custody or an attested verdict Engram. Without that durable record, "cleaning" taint is indistinguishable from silently ignoring it.

---

## 7. Taint and the Cognitive Immune System

The cognitive immune system and taint tracking reinforce each other:

- taint identifies suspect origin,
- immune-system checks identify suspect behavior,
- both feed the same gate and policy decisions.

Examples:

- repeated `ThirdPartyPlugin` outputs that trigger policy denials should lower trust in that plugin,
- imported `LegacyImport` knowledge that contradicts current heuristics should remain quarantined,
- `ExternalFetch` inputs that repeatedly produce blocked actions should become stronger negative signals in routing and review.

Taint is therefore the provenance half of the immune story; anomaly detection and policy reactions are the behavioral half.

---

## 8. Persistence and Auditability

Taint only matters if it survives persistence and replay.

Minimum expectations:

- the Engram's provenance includes taint source information,
- Custody records note which taint labels were active at the time of action,
- replay tooling can surface whether the action would still have been blocked under the recorded taint state,
- exports preserve taint metadata for third-party auditors.

That last point matters in regulated environments. If an operator cannot prove that an externally sourced datum was reviewed before a consequential action, they do not have a strong safety story.

---

## 9. Practical Policy Patterns

Useful default patterns:

- `UserInput` may read the workspace but should not directly authorize destructive shell actions.
- `ExternalFetch` may be summarized or compared, but publication or signing should require review.
- `ThirdPartyPlugin` outputs should inherit the plugin's identity so repeated violations are attributable.
- `LegacyImport` data should start in a quarantine path, not in the same retrieval tier as locally produced knowledge.

These defaults keep the system useful without pretending that untrusted input can be made safe by prompt wording alone.

---

## Cross-References

- [00-defense-in-depth.md](00-defense-in-depth.md)
- [02-audit-chain.md](02-audit-chain.md)
- [06-sandboxing.md](06-sandboxing.md)
- [08-threat-model.md](08-threat-model.md)
- [docs/00-architecture/26-cognitive-immune-system.md](../00-architecture/26-cognitive-immune-system.md)
