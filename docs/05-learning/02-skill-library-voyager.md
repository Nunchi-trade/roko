# Skill Library (Voyager-Style)

> **Crate:** `roko-learn` · **Module:** `skill_library.rs`
> **Persistence:** `.roko/learn/skills.json`
> **Wiring:** `LearningRuntime::record_completed_run()` → `SkillLibrary::record_use()`
> **Academic basis:** Wang et al. 2023 ("Voyager: An Open-Ended Embodied Agent with Large Language Models")
> **Cross-references:** [00-episode-logger](00-episode-logger.md), [01-playbook-system](01-playbook-system.md), [13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)

---

## Purpose

The skill library implements a Voyager-style capability accumulation system (Wang et al. 2023). Where playbook rules capture defensive knowledge ("watch out for X"), skills capture offensive knowledge ("here is how to do X"). Each skill is a named, reusable capability with a prompt template, tool dependencies, example I/O pairs, and usage telemetry that tracks how often the skill succeeds when injected into agent prompts.

The Voyager insight is that agent systems should monotonically accumulate skills from successful executions, building a growing library that makes future tasks cheaper and more reliable. A crate that has been successfully modified 50 times has accumulated patterns for trait implementation, test scaffolding, config extension, and error handling — patterns that a new agent can inherit rather than rediscovering through trial and error.

---

## Skill Schema

```rust
pub struct Skill {
    // ── Core identity ──────────────────────────────────────────
    /// Unique, human-readable identifier (snake_case).
    pub name: String,
    /// One-line description of what the skill does.
    pub summary: String,
    /// Prompt template injected when the skill is selected.
    pub prompt_template: String,
    /// Tools this skill expects the caller to expose.
    pub required_tools: Vec<String>,
    /// Illustrative inputs.
    pub example_inputs: Vec<String>,
    /// Illustrative outputs.
    pub example_outputs: Vec<String>,
    /// Free-form tags for search.
    pub tags: Vec<String>,

    // ── Usage telemetry ────────────────────────────────────────
    /// Smoothed success rate in [0.0, 1.0].
    pub success_rate: f64,
    /// Number of times record_use has been called.
    pub usage_count: u64,

    // ── Voyager-style extraction fields (§16.3.2–16.3.4) ──────
    /// Longer description (1-2 sentences).
    pub description: String,
    /// Plan identifier where this skill was first extracted.
    pub plan_id: String,
    /// Files touched in the originating task.
    pub files: Vec<String>,
    /// Numbered-step recipe from a successful episode (≤750 chars).
    pub pattern: String,
    /// Eval score from the originating episode, in [0.0, 1.0].
    pub score: f64,
    /// When the skill was first extracted.
    pub first_seen: Option<DateTime<Utc>>,
    /// When the skill was last injected into a prompt.
    pub last_matched: Option<DateTime<Utc>>,
    /// How many prompts have had this skill injected.
    pub match_count: u32,
    /// Of those injections, how many led to a gate pass.
    pub validated_count: u32,
    /// Task category for dedup.
    pub task_category: String,
}
```

### Deduplication

Skills sharing ≥70% of their tags AND the same `task_category` are considered duplicates. When a duplicate is detected during registration, the library keeps the skill with the higher `score` and merges the usage telemetry from the lower-scoring duplicate.

---

## Voyager Architecture

The Voyager paper (Wang et al. 2023) describes a three-component system for open-ended skill acquisition:

1. **Automatic Curriculum** — proposes tasks of increasing complexity.
2. **Skill Library** — stores and retrieves reusable code/procedures.
3. **Iterative Prompting** — refines skills through feedback loops.

Roko implements an adapted version where:

| Voyager Component | Roko Equivalent |
|-------------------|-----------------|
| Automatic curriculum | Plan generator (`roko prd plan`) creates tasks from PRDs |
| Skill library | `SkillLibrary` in `roko-learn` with JSON persistence |
| Iterative prompting | Gate pipeline validates output, failed attempts retry with context |
| Environment feedback | Gate verdicts (compile, test, lint, diff) |
| Code verification | 11-gate pipeline in `roko-gate` (see [04-verification](../04-verification/INDEX.md)) |

The key difference from Voyager (which operates in Minecraft) is that Roko's environment is a real codebase with deterministic verification: the gate pipeline provides ground-truth feedback that the skill either works or doesn't. This makes confidence tracking more reliable than in open-ended environments where success criteria are ambiguous.

---

## Skill Extraction Pipeline

Skills are extracted from successful episodes:

```
Successful Episode (gate pass)
    │
    ▼
Analyze execution trace:
    ├── What files were touched, in what order?
    ├── What tools were used?
    ├── What prompt sections were most relevant?
    └── What was the numbered-step recipe?
    │
    ▼
Construct Skill:
    ├── name: derived from task category + file pattern
    ├── prompt_template: generalized version of the successful prompt
    ├── required_tools: tools actually used during the episode
    ├── pattern: numbered-step recipe (≤750 chars)
    ├── files: files touched
    ├── score: episode eval score
    └── tags: derived from file paths, task category, error types
    │
    ▼
SkillLibrary::register(skill)
    ├── Check for duplicates (≥70% tag overlap + same category)
    ├── If duplicate: keep higher-score skill, merge telemetry
    └── If new: add to library, persist to skills.json
```

### Template Pattern Generation

The `TemplatePatternGenerator` trait provides a standardized interface for generating skill templates from episode data. Implementations can use heuristic extraction (analyzing tool calls and file modifications) or LLM-based extraction (asking a cheap model to summarize the episode into a reusable recipe).

---

## Skill Retrieval and Injection

When composing a prompt for a new task, the skill library is queried for relevant skills:

```
Task spec (files, category, tags)
    │
    ▼
SkillLibrary::search_by_tag(tags)
SkillLibrary::search_by_files(files)
    │
    ▼
Filter: success_rate ≥ 0.5, usage_count ≥ 2
    │
    ▼
Rank by: score × success_rate × recency_bonus
    │
    ▼
Top-3 skills injected into prompt as "Recommended approach":
    "Skill: rust_trait_implementation (confidence: 0.87)
     1. Read the existing trait definition with get_symbol_context
     2. Create the impl block in the target file
     3. Add #[cfg(test)] mod tests with at least one smoke test
     4. Run cargo test --lib to verify
     5. Run cargo clippy to check for lint warnings"
```

### Validation Tracking

After a skill is injected, the system tracks whether the task succeeded:

| Outcome | Update |
|---------|--------|
| Gate pass | `skill.validated_count += 1`, `skill.match_count += 1` |
| Gate fail | `skill.match_count += 1` (validated_count unchanged) |

The validation rate (`validated_count / match_count`) provides a direct measure of skill utility. Skills that are frequently matched but rarely validated are candidates for revision or removal.

---

## Persistence and Thread Safety

The `SkillLibrary` is an in-memory `BTreeMap<String, Skill>` guarded by a `parking_lot::RwLock`. Read operations (search, retrieve) acquire a shared read lock. Write operations (register, record_use) acquire an exclusive write lock.

Persistence uses `tokio::fs` with the atomic tempfile+rename pattern:

```
1. Serialize library to JSON
2. Write to temporary file (skills.json.tmp)
3. fsync the temporary file
4. Rename skills.json.tmp → skills.json (atomic on POSIX)
```

This ensures that `skills.json` is always a complete, valid JSON document. A crash during step 2 leaves the temporary file (which is ignored on next load), while the original `skills.json` remains intact.

### Startup Behavior

On startup, `SkillLibrary::new(path)` loads the existing `skills.json` if present. If the file does not exist, the library starts empty. If the file exists but is corrupt (invalid JSON), the library fails to initialize with `SkillLibraryError::Serde`.

---

## Monotonic Growth Property

The skill library is designed to grow monotonically: skills are added but never removed in normal operation. This mirrors the Voyager insight that accumulated knowledge should only increase over time. The only mechanisms that reduce the library are:

1. **Deduplication** — when a new skill duplicates an existing one, the lower-scoring duplicate is discarded.
2. **Manual pruning** — an operator can edit `skills.json` to remove obsolete skills.

There is no automatic pruning based on low usage or low success rate. The rationale: a skill that hasn't been used recently may still be valuable when a matching task appears. The cost of storing unused skills is negligible (a few KB each), while the cost of re-extracting a pruned skill is significant (requires a successful episode to trigger extraction again).

---

## Cross-Crate and Cross-Project Transfer

Skills that use structural patterns (trait implementation, test scaffolding, config extension) rather than project-specific identifiers are transferable across codebases. A skill for "Rust trait implementation" works in any Rust project, not just the one where it was extracted.

The transfer mechanism:

```
Project A skills.json → export → Project B skills.json
    │
    ├── Reset usage_count to 0
    ├── Reset success_rate to 0.5 (neutral prior)
    ├── Keep pattern, prompt_template, required_tools
    └── Skills must re-earn confidence in project B
```

This is analogous to the cross-project HDC fingerprint matching described in the episode logger — structural similarity, not nominal identity, determines transferability.

---

## Error Handling

```rust
pub enum SkillLibraryError {
    /// A skill with the requested name already exists.
    Duplicate(String),
    /// No skill with the requested name exists.
    NotFound(String),
    /// I/O error while reading or writing the persistence file.
    Io(io::Error),
    /// JSON (de)serialization error.
    Serde(serde_json::Error),
}
```

The `Duplicate` error is raised when `register()` is called with a skill name that already exists. Callers can use `register_or_update()` to upsert instead.

---

## Relationship to Voyager and Other Frameworks

| Framework | Skill Representation | Retrieval | Validation |
|-----------|---------------------|-----------|------------|
| Voyager (Wang et al. 2023) | JavaScript functions | Embedding similarity | Environment feedback |
| ExpeL (Zhao et al. 2023) | Natural language insights | Task-type matching | Success/failure tracking |
| Roko SkillLibrary | Prompt templates + tool lists | Tag + file matching + HDC | Gate pipeline verdicts |

Key differences from Voyager:
- **Language-agnostic skills**: Roko skills are prompt templates, not code in a specific language. The agent interprets the template and generates appropriate code.
- **Deterministic validation**: Gate pipeline provides ground-truth success/failure, unlike Minecraft's ambiguous environment feedback.
- **Bounded confidence**: Skills can never reach 1.0 confidence (bounded by validation rate tracking), preventing epistemic closure.

---

## Relationship to Other Documents

- **[00-episode-logger](00-episode-logger.md)** — Successful episodes are the source material for skill extraction.
- **[01-playbook-system](01-playbook-system.md)** — Playbook rules are defensive ("watch out for X"), skills are offensive ("here is how to do X"). They complement each other in prompt composition.
- **[05-pattern-discovery-trigram](05-pattern-discovery-trigram.md)** — Patterns identify recurring sequences; skills capture the full procedure associated with successful sequences.
- **[13-8-missing-feedback-loops](13-8-missing-feedback-loops.md)** — Loop 5 (Skills→Prompts) describes how skills feed back into prompt composition.
- **[17-adas-and-autocatalytic](17-adas-and-autocatalytic.md)** — The autocatalytic thesis posits that monotonically growing skill libraries are a key mechanism for compound improvement.

See also: EvoSkills (Chen et al. 2023) for evolutionary skill optimization, described in [17-adas-and-autocatalytic](17-adas-and-autocatalytic.md).
