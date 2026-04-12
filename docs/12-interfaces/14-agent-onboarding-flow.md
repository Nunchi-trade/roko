# Agent Onboarding Flow

> The journey from "new agent" to "fully operational Spectre-bearing cognitive agent" — domain selection, template instantiation, model routing, knowledge bootstrapping, Spectre generation, and first-task execution.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-cli-overview.md](./00-cli-overview.md), [04-configuration-layered-resolution.md](./04-configuration-layered-resolution.md), [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §3, `refactoring-prd/10-developer-guide.md`, `bardo-backup/prd/18-interfaces/00-portal.md`

---

## Abstract

Agent onboarding is the process of bringing a new cognitive agent from zero to operational. This includes: choosing the agent's domain and role, selecting a template, configuring model routing, bootstrapping initial knowledge, generating the Spectre creature identity, and executing the first task to validate the setup.

The onboarding flow is designed for progressive disclosure — a minimal onboarding path requires only a project name and a prompt, while the full path exposes every configuration knob. Both the CLI and Web Portal provide onboarding flows, with the CLI optimized for speed and the Portal optimized for visual guidance.

---

## Onboarding Paths

### Minimal Path (CLI, ~30 seconds)

```bash
# Initialize project
roko init

# Run first task — agent is created automatically
roko run "Add error handling to the auth module"
```

What happens automatically:
1. `roko init` creates `.roko/` directory and default `roko.toml`
2. `roko run` auto-detects language (Rust), creates a default coding agent
3. Agent receives default template (`code-implementer`), default model (`sonnet-4.6`)
4. Spectre is generated from agent ID hash
5. Task executes through the universal cognitive loop

### Standard Path (CLI, ~2 minutes)

```bash
# Initialize with domain
roko init --domain rust

# Configure models
roko config set agent.models.t1 "claude-sonnet-4-6"
roko config set agent.models.t2 "claude-opus-4-6"

# Create a plan
roko prd idea "Implement authentication middleware"
roko prd draft new "auth-middleware"
roko prd plan auth-middleware

# Execute with multiple agents
roko plan run plans/auth-middleware/
```

### Full Path (CLI or Portal, ~5 minutes)

Every configuration option is explicitly set:

```bash
# 1. Initialize
roko init --domain rust --name "my-project"

# 2. Configure agent templates
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
roko plan run plans/
```

---

## Stage 1: Domain Selection

The first onboarding decision is choosing the agent's primary domain.

### Auto-Detection

`roko init` auto-detects the domain from the project directory:

| Detection Signal | Domain | Confidence |
|---|---|---|
| `Cargo.toml` present | Rust | 0.95 |
| `package.json` + TypeScript files | TypeScript | 0.90 |
| `go.mod` present | Go | 0.90 |
| `pyproject.toml` / `setup.py` | Python | 0.85 |
| `Makefile` only | Generic | 0.50 |
| Empty directory | Generic | 0.10 |

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

### Manual Override

```bash
roko init --domain rust    # Force Rust domain
roko init --domain generic # No domain-specific configuration
```

---

## Stage 2: Template Instantiation

Agent templates define the role, tools, and behavioral profile of an agent.

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

The Spectre identity (shape seed) persists across sessions in `.roko/agents/{agent-id}.json`. The same agent always produces the same Spectre body, allowing operators to recognize agents visually across sessions.

---

## Stage 6: First Task Execution

The onboarding is validated by executing the first task through the full cognitive loop.

### Validation Checklist

The first task exercises every critical path:

| Step | Validation | What it proves |
|---|---|---|
| PERCEIVE | Substrate query returns project context | Knowledge store is connected |
| EVALUATE | Scorer rates context relevance | Scorer is configured |
| ATTEND | Router selects relevant context | Router is working |
| INTEGRATE | Composer builds prompt under budget | Context engineering is functional |
| ACT | LLM produces output | Model routing is connected |
| VERIFY | Gate pipeline runs | Gates are configured |
| PERSIST | Output stored as Engram | Substrate write is working |
| ADAPT | Policy emits learning events | Learning loop is connected |
| META-COGNIZE | Daimon updates PAD vector | Behavioral system is functional |

### First-Task Output

After the first task, the agent has:
- A Spectre with behavioral state history
- At least one episode in `.roko/episodes.jsonl`
- At least one efficiency event in `.roko/learn/efficiency.jsonl`
- Initial CascadeRouter state
- Gate pass/fail history
- First knowledge entries (if any insights were generated)

---

## Portal Onboarding Wizard

The Web Portal provides a visual wizard for the full onboarding path:

### Step 1: Welcome

```
┌─────────────────────────────────────────┐
│                                          │
│          Welcome to Roko                 │
│                                          │
│  Let's set up your cognitive agents.    │
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
│  Domain:  [○ Rust ○ TypeScript ○ Go]    │
│           [○ Python ○ Generic]          │
│                                          │
│  Detected: Rust (Cargo.toml found)      │
│                                          │
│  [← Back]  [Next →]                     │
│                                          │
└──────────────────────────────────────────┘
```

### Step 3: Agent Configuration

```
┌─────────────────────────────────────────┐
│                                          │
│  Agent Templates:                       │
│                                          │
│  ┌─ Implementer ──────────────────────┐ │
│  │ Model: [claude-sonnet-4-6 ▾]      │ │
│  │ Tools: [read ✓] [write ✓] [bash ✓]│ │
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

### Step 4: Gate Pipeline

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

### Step 5: Budget & Limits

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

### Step 6: Ready

```
┌─────────────────────────────────────────┐
│                                          │
│  ✓ Project configured                  │
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
2. Each developer runs `roko init` (detects existing config)
3. API keys are configured locally (`roko config set auth.api_key "..."`)
4. Knowledge store is shared via `.roko/` directory (or synced via mesh)

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
- Portal onboarding wizard
- Visual template editor
- Knowledge bootstrapping (auto-sharing across agents)
- Spectre generation from identity hash
- First-task validation checklist
- Migration import tools
- Team onboarding flow

---

## Cross-references

- See [00-cli-overview.md](./00-cli-overview.md) for the CLI command structure
- See [04-configuration-layered-resolution.md](./04-configuration-layered-resolution.md) for the configuration system
- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for Spectre generation
- See [13-web-portal.md](./13-web-portal.md) for the Portal wizard
- See topic [04-harness](../04-verification/INDEX.md) for the gate pipeline
- See topic [06-learning](../05-learning/INDEX.md) for the CascadeRouter
