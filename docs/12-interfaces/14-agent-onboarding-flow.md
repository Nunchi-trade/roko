# Agent Onboarding Flow

> The journey from "new agent" to "fully operational Spectre-bearing cognitive agent" - domain-profile selection, template instantiation, model routing, knowledge bootstrapping, Spectre generation, and first-task execution.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-cli-overview.md](./00-cli-overview.md), [04-configuration-layered-resolution.md](./04-configuration-layered-resolution.md), [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §3, `refactoring-prd/10-developer-guide.md`, `bardo-backup/prd/18-interfaces/00-portal.md`

---

## Abstract

Agent onboarding is the process of bringing a new cognitive agent from zero to operational. This includes choosing the agent's domain and role, selecting or installing a domain-profile bundle, composing domain profiles when needed, selecting a template, configuring model routing, bootstrapping initial knowledge, generating the Spectre creature identity, and executing the first task to validate the setup.

This chapter follows the refinements in [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md) and [tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md): onboarding should feel familiar-first, interactive, fast, provider/plugin/MCP-aware, domain-profile-aware, and resumable. The same underlying verbs should be reachable from all four surfaces, so a user can start in CLI and continue in TUI, Chat, or Web without relearning the workflow. The target is first useful output in under 30 seconds, with domain-profile install and composition treated as part of first-run rather than a separate admin task.

---

## Onboarding Paths

The canonical verb set for the onboarding surfaces is:

- `ask` for a single-turn request
- `plan` for a proposal without execution
- `do` for execution
- `watch` for progress
- `inspect` for episodes, Engrams, and heuristics
- `replay` for rerunning a prior session or episode
- `learn` for heuristic and playbook curation
- `tune` for configuration changes
- `connect` for plugins, MCP servers, credentials, and domain-profile bundles

That verb set is rendered differently in CLI, TUI, Chat, and Web, but it should behave like one system rather than four separate ones. See also the glossary at [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) for the canonical terms used here.

### Minimal Path (CLI, under 30 seconds)

```bash
# Initialize project
roko plugin install @roko/coding-profile
roko init

# Ask the first useful question
roko ask "Add error handling to the auth module"
```

What happens automatically:
1. `roko plugin install` adds a domain profile bundle and records its declared tools, gates, heuristics, and templates
2. `roko init` opens an interactive setup flow and creates `.roko/` plus a resumable `roko.toml`
3. The setup flow auto-detects the project domain and preferred model providers, then offers the matching domain profile or a blank starter
4. If the user accepts the defaults, Roko chooses a safe starter template, a working provider configuration, and a domain-profile-specific `TypedContext`
5. The init flow checks plugins and MCP servers opportunistically, but never blocks first success on a failed remote check
6. The first `roko ask` or `roko do` can start immediately, with live output visible on every surface and custody records attached to any sensitive activation

### Standard Path (CLI, ~2 minutes)

```bash
# Initialize with guided choices
roko init --profile coding

# Review the proposed setup
roko inspect onboarding

# Propose work
roko plan "Implement authentication middleware"

# Execute the first task
roko do "Implement authentication middleware"
```

### Full Path (CLI or Portal, ~5 minutes)

Every configuration option is explicitly set:

```bash
# 1. Initialize
roko init --profile coding --name "my-project"

# 2. Configure agent templates inside the selected profile
roko config set agent.templates.implementer.model "claude-sonnet-4-6"
roko config set agent.templates.implementer.tools ["read_file", "write_file", "edit_file", "bash"]
roko config set agent.templates.reviewer.model "claude-sonnet-4-6"
roko config set agent.templates.reviewer.tools ["read_file", "bash"]

# 3. Configure gates
roko config set gates.pipeline ["fmt", "compile", "clippy", "test", "diff"]
roko config set gates.test.timeout_secs 120

# 4. Configure model routing
roko config set routing.t0_probes 16
roko config set routing.t1_model "claude-sonnet-4-6"
roko config set routing.t2_model "claude-opus-4-6"
roko config set routing.cascade_threshold 0.7

# 5. Configure knowledge
roko config set neuro.decay.insight_half_life_hours 168
roko config set neuro.promotion_threshold 0.5

# 6. Configure budget
roko config set budget.daily_limit_usd 50.00
roko config set budget.alert_threshold 0.8

# 7. Run
roko do "Run the first validated task"
```

---

## Interactive `roko init`

`roko init` should behave as an onboarding assistant, not a static scaffold command. The flow should:

- Discover or install the best matching domain profile before the first task runs.
- Let the user compose multiple profiles when the project spans more than one domain.
- Detect the project domain from the working tree, then let the user override it.
- Probe provider availability, including local and remote model options, and keep the best working choice.
- Offer to import or defer plugin and MCP setup, with a visible skip path for every failed probe.
- Write onboarding state incrementally so an interrupted setup can resume from the last successful step.
- Surface the same choices in CLI, TUI, Chat, and Web so the user can move between surfaces without losing context.

The first screen should be concise and interactive, not a wall of flags:

```text
Welcome to Roko. Let's set up your first agent.

What would you like to do?
  [x] Start with a fast default setup
  [ ] Choose providers, profiles, templates, and tools manually
  [ ] Import an existing project or session

Which providers should we check?
  [x] Anthropic
  [x] OpenAI
  [x] Local Ollama
  [ ] Other

Should we look for plugins, domain profiles, and MCP servers?
  [x] Yes, auto-discover
  [ ] No, configure later
```

Every failure state should be recoverable in place:

- Missing API key: offer paste now, open docs, skip, or configure later.
- Local model unavailable: offer retry, skip, or continue with remote providers.
- Plugin or MCP probe failure: offer retry, skip errored entries, or open diagnostics.
- Interruptions: preserve progress so `roko init` can resume from the last completed step.

The practical rule is simple: no single setup check should block the user from reaching their first useful output.

---

## Stage 1: Domain-Profile Selection and Domain Mapping

The first onboarding decision is choosing the agent's primary domain profile. That choice should be made once, then carried with the session as the user moves across CLI, TUI, Chat, and Web. A domain profile may be a single domain bundle or a composition of multiple bundles when the project spans more than one domain.

### Auto-Detection

`roko init` auto-detects the domain from the project directory:

| Detection Signal | Profile hint | Confidence |
|---|---|---|
| `Cargo.toml` present | coding profile | 0.95 |
| `package.json` + TypeScript files | coding profile | 0.90 |
| `go.mod` present | coding profile | 0.90 |
| `pyproject.toml` / `setup.py` | research or data profile | 0.85 |
| `Makefile` only | blank starter | 0.50 |
| Empty directory | blank starter | 0.10 |

### Domain Effects

The selected domain configures:

| Configuration | Effect |
|---|---|
| **Language support** | Loads `roko-lang-{domain}` crate for parsing, symbol extraction |
| **Default gates** | Rust: `fmt + compile + clippy + test`; TypeScript: `lint + typecheck + test` |
| **Default tools** | Domain-specific tool set (e.g., `cargo_check`, `npm_test`) |
| **System prompt** | Domain-specific context in the system prompt builder |
| **Knowledge types** | Domain-relevant heuristics pre-loaded |
| **Index settings** | Code parser configuration for `roko-index` |
| **TypedContext schema** | Declares the keys gates and heuristics can match on without parsing free text |
| **Custody expectations** | Marks install/approve/execute actions that need a chain-of-custody record |
| **Session defaults** | The same onboarding session can be resumed from any surface |

### Manual Override

```bash
roko init --profile coding    # Force the coding profile
roko init --profile blank     # No domain-specific configuration
```

---

## Stage 2: Template Instantiation

Agent templates define the role, tools, and behavioral bias of an agent. In the domain-profile-first
flow, templates are the lower-level defaults that a domain-profile bundle exports for a particular task
shape. The domain profile can supply a template directly, override specific fields, or offer a small set
of template choices inside the same wizard.

### Built-in Templates

| Template | Role | Default Tools | Behavioral Bias |
|---|---|---|---|
| `code-implementer` | Write code, implement features | read, write, edit, bash, search | Engaged/Focused |
| `code-reviewer` | Review changes, suggest improvements | read, bash, search | Focused |
| `researcher` | Research topics, gather information | read, search, web (if available) | Exploring |
| `architect` | Design systems, plan structure | read, search | Exploring/Focused |
| `tester` | Write and run tests | read, write, bash | Focused |
| `orchestrator` | Coordinate other agents | all tools | Engaged |

### Template Configuration

Templates are defined in `roko.toml`:

```toml
[agent.templates.implementer]
role = "code-implementer"
model = "claude-sonnet-4-6"
tools = ["read_file", "write_file", "edit_file", "bash", "glob", "grep"]
max_turns = 50
context_budget = 100000  # tokens

[agent.templates.reviewer]
role = "code-reviewer"
model = "claude-sonnet-4-6"
tools = ["read_file", "bash", "glob", "grep"]
max_turns = 20
context_budget = 80000
```

### System Prompt Assembly

When an agent is instantiated from a template, the `SystemPromptBuilder` (from `roko-compose`) assembles a 6-layer system prompt:

```
Layer 1: Base identity (from template role)
Layer 2: Domain context (from language detection)
Layer 3: Project context (from roko.toml, README, recent activity)
Layer 4: Task context (from plan task description, dependencies)
Layer 5: Knowledge context (from Neuro store, relevant entries)
Layer 6: Behavioral directives (from Daimon state, risk tolerance)
```

**Source**: `roko-compose/src/system_prompt_builder.rs`, `RoleSystemPromptSpec` in `roko-cli/src/orchestrate.rs`

The onboarding flow should keep template selection lightweight. A first-time user should be able to accept a default template, get the session moving, and refine templates later through `tune` or `learn`. The UI should make it obvious which profile owns the default and which fields are inherited from a composed profile set.

---

## Stage 3: Model Routing Configuration

Model routing determines which LLM handles which types of cognitive work.

### Dual-Process Tier Assignment

```
T0: Zero-LLM probes (16 built-in probes)
    - File existence checks
    - Syntax validation
    - Cache lookups
    - Pattern matching
    → ~80% of cognitive ticks (no LLM cost)

T1: Fast model (e.g., claude-sonnet-4-6)
    - Standard code generation
    - Routine analysis
    - Known patterns
    → ~15% of cognitive ticks

T2: Full model (e.g., claude-opus-4-6)
    - Complex reasoning
    - Novel problem solving
    - Architecture decisions
    → ~5% of cognitive ticks
```

### Configuration

```toml
[routing]
# T0 probes (always enabled, no model needed)
t0_probes = 16

# T1: fast model for routine work
t1_model = "claude-sonnet-4-6"
t1_max_tokens = 4096

# T2: full model for complex reasoning
t2_model = "claude-opus-4-6"
t2_max_tokens = 8192

# Cascade threshold: uncertainty above this triggers T2
cascade_threshold = 0.7

# CascadeRouter persistence
router_state = ".roko/learn/cascade-router.json"
```

`roko init` should probe the common providers up front and only surface working model choices. If a provider is missing credentials or unreachable, the flow should explain the issue, suggest a next command, and let the user continue with the remaining providers.

### CascadeRouter Learning

The CascadeRouter uses LinUCB (contextual bandit) to learn which model to route to, based on:
- Task complexity estimate
- Agent behavioral state (Daimon PAD)
- Historical performance on similar tasks
- Cost constraints

The router state persists in `.roko/learn/cascade-router.json` and improves across sessions.

**Source**: `roko-learn` CascadeRouter, `roko-agent` model routing

---

## Stage 4: Knowledge Bootstrapping

New agents start with an empty knowledge store but can be bootstrapped from existing knowledge.

### Automatic Bootstrapping

When a new agent joins a plan with existing agents, knowledge is bootstrapped automatically:

1. **Neuro inheritance**: Persistent-tier knowledge entries are shared with the new agent
2. **Pheromone absorption**: Active pheromones in the mesh are replayed to the new agent
3. **Episode summary**: Recent episode summaries from the plan are injected as context

### Manual Knowledge Seeding

```bash
# Seed knowledge from a file
roko neuro inject --file knowledge-seed.jsonl

# Seed from another project's knowledge store
roko neuro import --from /other/project/.roko/neuro/

# Seed specific knowledge types
roko neuro inject --type Heuristic --content "Always check error returns in Go"
```

### Provider, Profile, Plugin, and MCP Discovery

The onboarding flow should treat provider checks, profile setup, plugin setup, and MCP discovery as related setup tasks, not separate product surfaces. Users should be able to:

- Import a known provider configuration from a prior session or another project.
- Install or compose a domain profile before the first task.
- Auto-discover MCP servers and accept the working ones while skipping failures.
- Add plugins and credentials later without restarting the onboarding flow.
- Resume discovery after an interruption without repeating completed steps.

This is especially important for the Web surface, which should present the same setup state as CLI and TUI rather than a separate wizard model. The shared contract should surface the profile's `TypedContext` keys and any `Custody` requirements for activating sensitive tools or gates so the user understands what the profile will expect before they commit.

### Knowledge Types Available at Bootstrap

| Type | Description | Decay |
|---|---|---|
| Insight | General understanding | Half-life: 168h (Persistent tier) |
| Heuristic | Practical rule of thumb | Half-life: 72h (Working tier) |
| Warning | Known hazard or pitfall | Half-life: 48h (Working tier) |
| CausalLink | If-then relationship | Half-life: 120h |
| StrategyFragment | Partial strategy or approach | Half-life: 96h |
| AntiKnowledge | Known incorrect approach | Half-life: 240h (long-lived) |

---

## Stage 5: Spectre Generation

When the agent is instantiated, its Spectre creature is generated deterministically from the agent's identity.

### Generation Process

```
Agent ID + Template Name
    │
    ▼
BLAKE3 hash → 32-byte shape seed
    │
    ▼
Morphological parameter extraction:
  body_archetype, symmetry, limb_count, limb_style,
  eye_count, eye_style, domain_texture, color_offset,
  proportion_ratios, detail_seed
    │
    ▼
Dot-cloud geometry generation
    │
    ▼
Spring connection assignment
    │
    ▼
Initial state: Resting (waiting for first task)
    │
    ▼
On first task assignment: transition to Engaged
```

### First Appearance

The Spectre first appears in the Resting state — minimal form, slow breathing, dim glow. When the agent receives its first task, the Spectre transitions to Engaged over ~500ms using the luxury easing curve, visually "waking up":

```
Resting:           Transition:        Engaged:
   ╭╮              ╭─╮                 ╭─╮
  ╭──╮            ╭╯ ╰╮           ╭───╯ ╰───╮
  │──│    →→→     │○ ○│    →→→    │  ◉    ◉  │
  ╰──╯            ╰───╯           ╰─────────╯
 (dim)           (brightening)      (full glow)
```

### Spectre Persistence

The Spectre identity (shape seed) persists across sessions in `.roko/agents/{agent-id}.json`. The same agent always produces the same Spectre body, allowing operators to recognize agents visually across sessions and across surfaces.

---

## Stage 6: First Task Execution

The onboarding is validated by executing the first task through the full cognitive loop. The result should be visible as live progress in CLI, TUI, Chat, and Web without the user needing to re-enter the task.

### Validation Checklist

The first task exercises every critical path:

| Step | Validation | What it proves |
|---|---|---|
| SENSE | Substrate query returns project context | Knowledge store is connected |
| ASSESS | Scorer rates context relevance | Scorer is configured |
| COMPOSE | Composer builds prompt under budget | Context engineering is functional |
| ACT | LLM produces output | Model routing is connected |
| VERIFY | Gate pipeline runs | Gates are configured |
| PERSIST | Output stored as Engram | Substrate write is working |
| BROADCAST | Bus publishes progress pulses | Live progress is visible on every surface |
| REACT | Policy emits learning events | Learning loop is connected |

### First-Task Output

After the first task, the agent has:
- A Spectre with behavioral state history
- At least one episode in `.roko/episodes.jsonl`
- At least one efficiency event in `.roko/learn/efficiency.jsonl`
- Initial CascadeRouter state
- Gate pass/fail history
- First knowledge entries (if any insights were generated)
- A shared session record that can be resumed from CLI, TUI, Chat, or Web

---

## Portal Onboarding Wizard

The Web surface provides a visual wizard for the full onboarding path, but it should mirror the same state machine as `roko init` rather than inventing a parallel setup flow. The wizard should expose profile install, profile composition, and the same TypedContext/Custody preview that the CLI shows.

### Step 1: Welcome

```
┌─────────────────────────────────────────┐
│                                          │
│          Welcome to Roko                 │
│                                          │
│  Let's set up your first agent.         │
│                                          │
│  [Start Setup →]                         │
│                                          │
└──────────────────────────────────────────┘
```

### Step 2: Project Configuration

```
┌─────────────────────────────────────────┐
│                                          │
│  Project: [my-project_____________]     │
│  Profile: [○ Coding ○ Research ○ Ops]   │
│           [○ Blockchain ○ Writing ○ +]  │
│                                          │
│  Detected: Coding profile (Cargo.toml)  │
│  Session:  [resume from CLI ▾]          │
│                                          │
│  [← Back]  [Next →]                     │
│                                          │
└──────────────────────────────────────────┘
```

### Step 3: Profile Composition

```
┌─────────────────────────────────────────┐
│                                          │
│  Active profiles:                       │
│                                          │
│  ┌─ Coding ───────────────────────────┐ │
│  │ Tools: fs, git, cargo, mcp-code    │ │
│  │ Gates: unit, type, style, diff     │ │
│  │ TypedContext: language, repo_root  │ │
│  └────────────────────────────────────┘ │
│                                          │
│  ┌─ Research ─────────────────────────┐ │
│  │ Tools: web, pdf, citations         │ │
│  │ Gates: citation, factuality        │ │
│  │ Custody: required for publish      │ │
│  └────────────────────────────────────┘ │
│                                          │
│  [Install another profile]             │
│  [Resolve collisions]                   │
│                                          │
│  [← Back]  [Next →]                     │
│                                          │
└──────────────────────────────────────────┘
```

### Step 4: Agent Configuration

```
┌─────────────────────────────────────────┐
│                                          │
│  Agent Templates:                       │
│                                          │
│  ┌─ Implementer ──────────────────────┐ │
│  │ Model: [claude-sonnet-4-6 ▾]      │ │
│  │ Tools: [read ✓] [write ✓] [bash ✓] │ │
│  │ Max turns: [50____]               │ │
│  └────────────────────────────────────┘ │
│                                          │
│  ┌─ Reviewer ─────────────────────────┐ │
│  │ Model: [claude-sonnet-4-6 ▾]      │ │
│  │ Tools: [read ✓] [bash ✓]          │ │
│  │ Max turns: [20____]               │ │
│  └────────────────────────────────────┘ │
│                                          │
│  [+ Add Template]                       │
│                                          │
│  [← Back]  [Next →]                     │
│                                          │
└──────────────────────────────────────────┘
```

### Step 5: Gate Pipeline

```
┌─────────────────────────────────────────┐
│                                          │
│  Gate Pipeline (drag to reorder):       │
│                                          │
│  ✓ [1] Format Check                    │
│  ✓ [2] Compile                         │
│  ✓ [3] Clippy Lint                     │
│  ✓ [4] Test Suite                      │
│  ✓ [5] Diff Review                     │
│  ○ [6] AI Review (optional)            │
│                                          │
│  [← Back]  [Next →]                     │
│                                          │
└──────────────────────────────────────────┘
```

### Step 6: Budget & Limits

```
┌─────────────────────────────────────────┐
│                                          │
│  Budget:                                │
│  Daily limit: [$50.00_________]         │
│  Alert at:    [80%____________]         │
│                                          │
│  Parallelism:                           │
│  Max agents:  [4______________]         │
│  Max worktrees: [4____________]         │
│                                          │
│  [← Back]  [Finish Setup →]            │
│                                          │
└──────────────────────────────────────────┘
```

### Step 7: Ready

```
┌─────────────────────────────────────────┐
│                                          │
│  ✓ Project configured                  │
│  ✓ Profiles installed                  │
│  ✓ Templates created                   │
│  ✓ Gates enabled                       │
│  ✓ Budget set                          │
│                                          │
│  Your agents are ready.                 │
│                                          │
│  [Create First Plan]  [Go to Dashboard] │
│                                          │
└──────────────────────────────────────────┘
```

---

## Onboarding for Existing Projects

### Migration from Other Systems

Projects migrating from other agent frameworks can import configuration:

```bash
# Import from existing roko.toml (different version)
roko init --import /old/project/roko.toml

# Import knowledge from existing project
roko neuro import --from /old/project/.roko/neuro/
```

### Team Onboarding

When multiple developers work on the same roko project:

1. `roko.toml` is committed to version control
2. Each developer runs `roko init`, which detects existing state and offers to resume instead of starting over
3. API keys are configured locally, with provider checks handled in the same onboarding session
4. Knowledge store is shared via `.roko/` directory, and the same session can be picked up in CLI, TUI, Chat, or Web

For shared work, the important property is continuity: a user should be able to start onboarding on one surface, finish provider setup on another, and pick up the same session later without losing the setup state.

---

## Current Status and Gaps

**Built:**
- `roko init` command with domain auto-detection
- `roko.toml` configuration system
- Template-based agent instantiation
- SystemPromptBuilder with 6-layer assembly
- CascadeRouter with LinUCB persistence
- Gate pipeline configuration
- Episode logging
- Basic agent lifecycle (spawn → execute → gate → persist)

**Not yet built:**
- Fully interactive, resumable onboarding across all four surfaces
- Provider-aware setup that degrades gracefully when keys or local runtimes are missing
- Profile install and composition workflow that is visible on every surface
- Plugin and MCP discovery with skip/retry/diagnostic flows
- First-party Web onboarding wizard that mirrors CLI state
- Session continuity across onboarding and the first task
- Visual template editor
- TypedContext and Custody surface contracts in the onboarding UI

---

## Cross-References

- See [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md) for the full proposal
- See [tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md) for profile install, TypedContext, and Custody details
- See [00-cli-overview.md](./00-cli-overview.md) for the CLI command structure
- See [04-configuration-layered-resolution.md](./04-configuration-layered-resolution.md) for the configuration system
- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for Spectre generation
- See [13-web-portal.md](./13-web-portal.md) for the Portal wizard
- See topic [04-harness](../04-verification/INDEX.md) for the gate pipeline
- See topic [06-learning](../05-learning/INDEX.md) for the CascadeRouter
