# Playbook System

> **Crate:** `roko-learn` · **Modules:** `playbook.rs`, `playbook_rules.rs`
> **Persistence:** `.roko/learn/playbooks/` (JSON per playbook), `.roko/learn/playbook-rules.toml`
> **Wiring:** `LearningRuntime::record_completed_run()` → `PlaybookStore`, `PlaybookRules`
> **Cross-references:** [00-episode-logger](00-episode-logger.md), [02-skill-library-voyager](02-skill-library-voyager.md), [05-pattern-discovery-trigram](05-pattern-discovery-trigram.md), [19-heuristics-worldviews-and-falsifiers](19-heuristics-worldviews-and-falsifiers.md), [04-decay-variants](../00-architecture/04-decay-variants.md), [25-attention-as-currency](../00-architecture/25-attention-as-currency.md), [Naming and Glossary](../00-architecture/01-naming-and-glossary.md), [REF12 demurrage proposal](../../tmp/refinements/12-knowledge-demurrage.md), [REF14 worldview validation proposal](../../tmp/refinements/14-worldview-validation.md)


> **Implementation**: Shipping

---

## Purpose

The playbook system is the concrete procedural projection of Roko's learning stack, not the whole stack by itself. REF14 adds a first-class `Heuristic` layer above episodes and patterns: heuristics capture reusable claims, predictions, falsifiers, and calibration records, while playbooks remain the highly specific ordered steps and prompt-ready rules compiled from those validated beliefs. When a rule correctly predicts outcomes across multiple subsequent executions, it earns enough reinforcement to stay warm and gets injected directly into agent prompts, preventing the agent from repeating known mistakes. Freshness is not governed by confidence alone: demurrage, successful reuse, and contradiction-driven penalties decide whether a rule remains active or cools into cold storage. See also [19-heuristics-worldviews-and-falsifiers](19-heuristics-worldviews-and-falsifiers.md) and `../../tmp/refinements/14-worldview-validation.md`.

The system has two components:

1. **PlaybookStore** — manages named sequences of steps (playbooks) with success/failure counters and freshness balance.
2. **PlaybookRules** — manages if-then rules with globset-based triggers, bounded confidence dynamics, and demurrage-driven reinforcement.

---

## Playbooks Inside The Learning Stack

```
┌──────────────────────────────────────────────────────────────────┐
│              Tier 4: Playbook Rules And Playbooks                │
│   Concrete instructions compiled from validated heuristics and   │
│   repeated strategy fragments. Confidence: 0.0 – 0.95 bounded.  │
│   Reinforcement + balance keep rules warm; demurrage cools      │
│   stale rules. Trigger: file globs, tags, categories, error     │
│   signatures, roles. Action: inject prompt-ready body text.     │
│   Lifecycle: validate / contradict / reinforce / demurrage /    │
│   prune.                                                         │
├──────────────────────────────────────────────────────────────────┤
│             Tier 3: Heuristics And Worldview Priors              │
│   Reusable rules of thumb with preconditions, predictions,       │
│   falsifier surfaces, calibration records, and episode receipts. │
│   See: 19-heuristics-worldviews-and-falsifiers.md                │
├──────────────────────────────────────────────────────────────────┤
│                    Tier 2: Patterns                               │
│   Extracted hypotheses from episode clustering.                   │
│   See: 05-pattern-discovery-trigram.md                            │
├──────────────────────────────────────────────────────────────────┤
│                    Tier 1: Episodes                               │
│   Raw observations from every agent turn.                         │
│   See: 00-episode-logger.md                                       │
└──────────────────────────────────────────────────────────────────┘
```

---

## PlaybookStore

The `PlaybookStore` manages named playbooks — ordered sequences of steps that describe a known-good approach to a task type.

### Playbook Schema

```rust
pub struct Playbook {
    /// Unique playbook identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// What the playbook aims to achieve.
    pub goal: String,
    /// Ordered steps to execute.
    pub steps: Vec<PlaybookStep>,
    /// Attention balance that self-trims stale playbooks.
    pub balance: f64,
    /// Total demurrage charged against this playbook.
    pub demurrage_paid: f64,
    /// Number of times this playbook led to a gate pass.
    pub success_count: u32,
    /// Number of times this playbook led to a gate failure.
    pub failure_count: u32,
}

pub struct PlaybookStep {
    /// Zero-based step index.
    pub index: u32,
    /// Human-readable step description.
    pub description: String,
    /// What kind of action this step involves (e.g. "read", "edit", "test").
    pub action_kind: String,
    /// Engrams expected to be produced by this step.
    pub expected_signals: Vec<String>,
}
```

### Persistence

Each playbook is stored as a separate JSON file in `.roko/learn/playbooks/`, keyed by playbook ID. The store uses per-ID async mutexes for concurrent safety: multiple playbooks can be updated simultaneously, but updates to the same playbook are serialized. Writes use the atomic tempfile+rename pattern to prevent corruption on crash.

```
.roko/learn/playbooks/
├── pb-001.json    ← "Rust trait implementation" playbook
├── pb-002.json    ← "Config schema extension" playbook
├── pb-003.json    ← "Gate failure recovery" playbook
└── ...
```

### Operations

| Method | What it does |
|--------|-------------|
| `PlaybookStore::save(playbook)` | Persist a new or updated playbook |
| `PlaybookStore::load(id)` | Load a single playbook by ID |
| `PlaybookStore::load_all()` | Load all playbooks from the directory |
| `PlaybookStore::record_outcome(id, success)` | Increment success or failure counter |

---

## PlaybookRules

The `PlaybookRules` module manages if-then rules with rich trigger matching and bounded confidence dynamics. Rules are the actionable output of the learning system — they are injected into agent prompts to prevent known failure modes, and they self-trim through demurrage instead of depending on fixed-age retention windows.

### Rule Schema

```rust
pub struct Rule {
    /// Stable identifier (synthesized from clustering key).
    pub rule_id: String,
    /// Short human-readable label (≤80 chars).
    pub title: String,
    /// Text injected into the Implementer prompt.
    pub body: String,
    /// Conditions that cause this rule to fire.
    pub triggers: Triggers,
    /// Freshness balance that rises with reinforcement and falls with demurrage.
    pub balance: f64,
    /// Total demurrage charged against this rule.
    pub demurrage_paid: f64,
    /// Confidence score; bounded to [0.0, 0.95].
    pub confidence: f64,
    /// Number of validated predictions.
    pub validations: u32,
    /// Number of contradicted predictions.
    pub contradictions: u32,
    /// When last applied.
    pub last_applied: Option<DateTime<Utc>>,
    /// When first created.
    pub created_at: DateTime<Utc>,
    /// Source episode IDs that generated this rule.
    pub source_episodes: Vec<String>,
}
```

### Trigger System

Rules fire when incoming context matches their `Triggers`:

```rust
pub struct Triggers {
    /// Shell glob patterns matched against files.
    pub file_globs: Vec<String>,
    /// Tag strings (case-insensitive overlap).
    pub tags: Vec<String>,
    /// Task categories.
    pub categories: Vec<String>,
    /// Error signature strings.
    pub error_signatures: Vec<String>,
    /// Agent roles.
    pub roles: Vec<String>,
}
```

Matching uses **OR semantics** across the five trigger kinds: a rule fires if ANY of its trigger lists intersects the incoming context. An all-empty `Triggers` matches nothing — it never fires, guarding against accidental universal rules.

File glob matching uses the `globset` crate for shell-style pattern matching:

```toml
# Example rule in playbook-rules.toml
[[rule]]
rule_id = "rule-008"
title = "Auth module lifetime check"
body = "Check lifetime parameters on all auth types before using them. Use get_symbol_context to see actual signatures."
confidence = 0.92
validations = 12
contradictions = 1

[rule.triggers]
file_globs = ["src/auth/**/*.rs", "crates/roko-agent/src/auth/*"]
tags = ["lifetime", "borrow"]
categories = ["refactor", "bugfix"]
error_signatures = []
roles = ["Implementer"]
```

### Matching Context

When composing a prompt for an agent, the system constructs a `MatchContext` from the current task:

```rust
pub struct MatchContext {
    /// Files the task will modify.
    pub files: Vec<String>,
    /// Tags from the task spec.
    pub tags: Vec<String>,
    /// Task category.
    pub category: Option<String>,
    /// Error signature from the previous failed attempt (if retrying).
    pub error_signature: Option<String>,
    /// Agent role.
    pub role: Option<String>,
}
```

The `PlaybookRules::select(context)` method returns all rules whose triggers match the context, sorted by confidence (highest first). The prompt composer injects the top-N rules (typically 3-5) into the agent's system prompt as "lessons from previous builds."

### Confidence Dynamics

Confidence is update-driven, not time-based, and freshness is governed by balance rather than a hard retention window:

| Trigger | Confidence change | Balance change | Effect |
|-------|------------------|----------------|--------|
| Validation (rule predicted correctly) | `+0.05` | Reinforcement bonus | Keeps a useful rule warm |
| Contradiction (rule predicted incorrectly) | `−0.10` | Reinforcement loss plus cooling pressure | Stale or wrong rules cool faster |
| Successful reuse / citation | N/A | Reinforcement bonus | Returns attention to the rule |
| Demurrage tick | N/A | Holding cost | Unused rules drift toward cold storage |
| Prune threshold | N/A | Balance or confidence below floor | Rule removed if it can no longer justify retention |

The asymmetric update rate (contradictions penalize 2× more than validations reward) ensures that rules which stop being accurate are quickly demoted. Demurrage makes that demotion continuous instead of relying on periodic cleanup: a rule that is no longer cited or successfully reused loses balance over time even if its confidence remains superficially high. The confidence ceiling of 0.95 prevents any rule from becoming "certain" — there is always a small probability that the rule is wrong, which keeps the system open to revision.

```
Confidence lifecycle:
    new rule → 0.50 (default)
        │
        ├── validated → 0.55 → 0.60 → ... → 0.95 (ceiling)
        │
        └── contradicted → 0.40 → 0.30 → ... → 0.0 (pruned)
```

### Why 0.95 Ceiling?

The confidence ceiling prevents epistemic closure. A rule at 1.0 confidence would never be questioned, even if the codebase changes in ways that invalidate the rule's assumptions. The 0.95 ceiling means that every rule, no matter how well-validated, retains a 5% "doubt margin" that allows contradictions to eventually demote it.

---

## Rule Lifecycle

```
Episode Stream
    │
    ▼
Pattern Discovery (trigram mining, HDC clustering)
    │
    ▼
Pattern extracted: "Auth module types have lifetime parameters"
    │
    ├── support_count < 5 → stays as Pattern (Tier 2)
    │
    └── support_count ≥ 5 → promoted to Rule (Tier 3)
            │
            ├── Validated in subsequent builds → confidence climbs
            │
            ├── Contradicted → confidence drops
            │
            └── confidence < min_confidence → pruned (removed)
```

### Promotion Criteria

A pattern is promoted to a rule when:
1. It has appeared in 5+ distinct episodes.
2. Its confidence (proportion of episodes where the predicted outcome matched) exceeds the minimum threshold.
3. The trigger conditions can be expressed as globs, tags, categories, or error signatures.

### Demotion and Pruning

Rules that stop being accurate are automatically demoted:
1. Each contradiction reduces confidence by 0.10 and cuts into balance.
2. Each demurrage tick reduces balance even when the rule is not contradicted.
3. Successful reuse or citation replenishes balance, so rules stay warm only when they keep earning attention.
4. When confidence or balance drops below `min_confidence` / `min_balance` (configurable), the rule is pruned or moved to cold storage.
5. Pruned rules are removed from the TOML file on the next save.

This creates a self-cleaning knowledge base: rules that were valid for an older version of the codebase but no longer apply are automatically removed as contradictions accumulate and their balance drains. It also fixes stale-playbook petrification, where a once-good rule would otherwise sit in prompts forever just because it had a high historical confidence score.

---

## Integration with Prompt Composition

When the prompt composer assembles a system prompt for an agent, it queries the playbook rules:

```
Task spec (files, tags, category, role)
    │
    ▼
PlaybookRules::select(MatchContext)
    │
    ▼
Top-N matching rules (sorted by confidence)
    │
    ▼
Inject into system prompt as "Lessons from previous builds":
    "Note: past builds show that auth module types have
     lifetime parameters. Check actual signatures before
     using them. (confidence: 0.92, validated 12 times)"
```

The injected rules typically consume 50-100 tokens per rule. For a typical task with 2-3 matching rules, this adds ~200 tokens to the prompt — a trivial cost that prevents multi-thousand-token debugging loops where the agent discovers the issue through trial and error.

See [03-composition](../03-composition/INDEX.md) for the full prompt assembly pipeline and how playbook rules fit into the 6-layer `SystemPromptBuilder`.

---

## Persistence Format

Playbook rules are stored in TOML (not JSON) for human readability:

```toml
min_confidence = 0.10
max_body_tokens = 200

[[rule]]
rule_id = "rule-001"
title = "Serde derive for config types"
body = "All types in roko-core::config that cross serialization boundaries need #[derive(Serialize, Deserialize)]. Check the type definition before using it in a TOML/JSON context."
confidence = 0.85
validations = 8
contradictions = 1
created_at = "2026-03-15T10:30:00Z"
source_episodes = ["ep-042", "ep-043", "ep-051", "ep-067", "ep-089"]

[rule.triggers]
file_globs = ["crates/roko-core/src/config/**/*.rs"]
tags = ["serde", "config"]
categories = ["bugfix"]
error_signatures = ["E0277.*Serialize"]
roles = ["Implementer"]
```

The TOML format was chosen over JSON because:
1. Rules are often edited by humans (adding triggers, adjusting confidence).
2. TOML's comment syntax allows annotating rules with context.
3. TOML's array-of-tables syntax (`[[rule]]`) maps naturally to the rule list.

---

## Cross-Project Transfer

Playbook rules are project-agnostic when their triggers use structural patterns rather than project-specific identifiers. A rule triggered by `error_signatures = ["E0277.*Serialize"]` applies to any Rust project, not just the one where it was extracted.

The cross-project transfer workflow:
1. Export rules from project A: `cp .roko/learn/playbook-rules.toml /shared/rules/project-a.toml`
2. Import into project B: merge into `.roko/learn/playbook-rules.toml`
3. Reset confidence to 0.50 and balance to a starter value (rules must re-earn both in the new context)
4. Rules that validate in project B climb in confidence and balance; rules that contradict or go unused lose balance and are pruned.

This enables a form of cross-project knowledge transfer that operates at ~50ns per pattern lookup (via HDC fingerprint matching) rather than requiring expensive embedding-based retrieval.

---

## Relationship to Other Documents

- **[00-episode-logger](00-episode-logger.md)** — Episodes are the raw data from which playbook rules are eventually extracted.
- **[02-skill-library-voyager](02-skill-library-voyager.md)** — Skills capture reusable procedures; playbook rules capture validated predictions. They are complementary: a skill says "how to do X," while a rule says "watch out for Y when doing X."
- **[05-pattern-discovery-trigram](05-pattern-discovery-trigram.md)** — Patterns are the intermediate tier between episodes and rules. Patterns with sufficient support are promoted to rules.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — The Skills→Prompts feedback loop (loop 5) describes how playbook rules feed back into prompt composition.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Confidence dynamics prevent oscillation: the asymmetric update rate and 0.95 ceiling are stability mechanisms.
- **[04-decay-variants](../00-architecture/04-decay-variants.md)** — Demurrage supersedes simple decay for retention.
- **[25-attention-as-currency](../00-architecture/25-attention-as-currency.md)** — Playbook freshness is an attention-economy problem.
- **[Naming and Glossary](../00-architecture/01-naming-and-glossary.md)** — Canonical vocabulary for the learning and memory layers.
- **See also:** [REF12 demurrage proposal](../../tmp/refinements/12-knowledge-demurrage.md)
