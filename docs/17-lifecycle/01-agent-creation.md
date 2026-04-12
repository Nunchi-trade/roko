# Agent Creation

> **Layer**: L0 Runtime (process lifecycle) + L1 Framework (backend selection, role assignment)
>
> **Prerequisites**: `docs/00-architecture/INDEX.md` (5-layer taxonomy), `docs/17-lifecycle/00-vision-and-mortality-replaced.md` (lifecycle model overview)
>
> **Synapse traits**: Substrate (initialized during creation to hold the agent's Neuro store), Router (configured during creation for model selection), Policy (configured during creation for behavioral defaults)


> **Implementation**: Specified

---

## Overview

Agent creation in Roko follows a three-interaction pattern: **Describe**, **Review**, **Confirm**. The design principle is cognitive simplicity — a user who has never heard of Roko should be able to create a working agent in under 3 minutes. An experienced user should be able to deploy from a TOML config file in under 60 seconds.

Three entry points converge on a single artifact:

```
CLI (roko init)  ------+
                       +--> AgentManifest --> Provisioning Pipeline
Web UI Wizard ---------+                      (see 02-provisioning.md)
                       |
API (POST /v1/agents) -+
```

This document specifies the creation flow for each entry point, the manifest schema, strategy templates, AI-assisted configuration, and the defaults that make the happy path fast.

---

## Design Principles

### Three Interactions, Not 60 Seconds

The happy path is three user interactions: (1) describe what the agent should do, (2) review the generated configuration, (3) confirm and provision. Everything else — strategy compilation, Neuro initialization, model routing setup, Mesh registration — is handled by AI autofill and sane defaults. The user confirms, not writes.

### Progressive Disclosure

The creation wizard shows only what is essential by default. Advanced configuration — model routing preferences, Neuro sharing policies, tool profiles, inference provider selection — is tucked behind expandable sections. Users who want control get it; users who do not never see it.

### AI-First Composition

When a model provider is available, the creation wizard uses a single LLM call to transform a short natural-language prompt into a complete `AgentManifest`. The user describes intent in plain English; AI handles translation to structured configuration. Strategy templates are the zero-cost alternative — instant generation, deterministic output, auditable parameters. Templates and AI autofill are complementary, not competing paths.

### Confirm Before Commit

Every irreversible action — compute provisioning, on-chain registration (for chain-domain agents), resource allocation — gets an explicit confirmation screen with cost breakdown. No surprise charges.

---

## The AgentManifest

The manifest is the single configuration artifact that fully describes an agent to be created. All creation flows produce a manifest. The provisioning pipeline consumes one.

### Core Manifest (Minimal)

```rust
use serde::{Deserialize, Serialize};

/// The minimal manifest sufficient for the happy-path creation flow.
/// A user who types a single prompt and clicks "Create" produces exactly this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCoreManifest {
    /// Free-text description of what the agent should do. 10-2000 chars.
    pub prompt: String,

    /// Deployment mode. "hosted" runs on managed compute; "self-hosted" runs locally.
    pub mode: DeploymentMode,

    /// Domain plugin to activate. Default: none (general-purpose).
    pub domain: Option<DomainPlugin>,

    /// Schema version for forward compatibility. Current version: 1.
    pub schema_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeploymentMode {
    /// Managed compute infrastructure. Pay-per-use.
    Hosted,
    /// User's own machine. User bears infrastructure cost.
    SelfHosted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DomainPlugin {
    /// Blockchain domain (roko-chain crate). Adds wallet, on-chain tools, ERC-8004.
    Chain(ChainConfig),
    /// Coding domain. Adds file system tools, compiler gates, test runners.
    Coding(CodingConfig),
    /// Research domain. Adds citation tools, paper retrieval, synthesis.
    Research(ResearchConfig),
    /// Custom domain plugin (user-provided).
    Custom(CustomPluginConfig),
}
```

These fields are sufficient for the happy path. A user who types a single prompt and clicks "Create" produces exactly this.

### Extended Manifest (Full)

```rust
/// Full manifest with all optional overrides resolved.
/// The provisioning pipeline never works with a partial manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExtendedManifest {
    /// Core manifest fields (always present).
    #[serde(flatten)]
    pub core: AgentCoreManifest,

    /// Human-readable identifier. AI-generated or `agent-{nanoid(12)}`.
    pub name: Option<String>,

    /// Full STRATEGY.md content. AI-generated from prompt.
    pub strategy_md: Option<String>,

    /// Model routing configuration. Default: cascade T0→T1→T2.
    pub model_routing: Option<ModelRoutingConfig>,

    /// Neuro (knowledge store) initial configuration.
    pub neuro: Option<NeuroConfig>,

    /// Mesh (agent coordination) configuration.
    pub mesh: Option<MeshConfig>,

    /// Tool profile. Default: "standard".
    pub tool_profile: Option<String>,

    /// Template expansion (mutually exclusive with AI autofill).
    pub template_id: Option<String>,
    pub template_params: Option<HashMap<String, String>>,

    /// AI autofill provenance metadata.
    pub autofill: Option<AutofillProvenance>,

    /// Inference provider configuration.
    pub inference: Option<InferenceConfig>,

    /// Budget limits. Default: no limit (self-hosted) or plan-based (hosted).
    pub budget: Option<BudgetConfig>,
}
```

### Manifest Resolution

The provisioning pipeline never works with a partial manifest. A `resolve_manifest()` function fills in all missing fields:

1. Start with sane defaults for the selected mode and domain
2. If `template_id` is set, expand the template with `template_params`
3. If no template, run AI autofill for `strategy_md`, `name`, and domain-specific fields
4. Apply any explicit overrides from the extended manifest
5. Validate the final manifest against the domain's feature set
6. Compute resource estimates

---

## CLI Flow (`roko init`)

The CLI provides a TOML config file (`roko.toml`) as the primary configuration surface:

```bash
# Initialize a roko.toml config template
roko init

# Create from config file (non-interactive if all fields present)
roko init --config ./roko.toml

# Create from minimal prompt (interactive for missing fields)
roko init --prompt "Weekly code review agent for our Rust codebase"

# Create from a template (non-interactive)
roko init --template rust-coding --param crate_path=crates/roko-core

# Dry-run: validate and estimate costs without executing
roko init --config ./roko.toml --dry-run
```

Config loading merges four sources in priority order: CLI flags > environment variables > TOML config file > built-in defaults.

### Default `roko.toml` (Generated by `roko init`)

```toml
# roko.toml — Agent configuration
# Generated by `roko init` on 2026-04-12

[agent]
name = "agent-V1StGXR8_Z5j"
prompt = "Describe what this agent should do"
mode = "self-hosted"

[inference]
# Model routing: cascade from T0 (no LLM) through T1 (fast) to T2 (deep)
default_model = "claude-haiku-4-5"
escalation_model = "claude-sonnet-4-6"
critical_model = "claude-opus-4-6"

[neuro]
# Knowledge store configuration
path = ".roko/neuro/"
max_engrams = 50000
decay_model = "ebbinghaus"    # Ebbinghaus forgetting curve

[mesh]
# Agent coordination (disabled by default for self-hosted)
enabled = false

[tools]
profile = "standard"

[budget]
# Budget limits (optional)
# max_daily_inference_usd = 10.0
# max_total_usd = 1000.0
```

### Three Personas

The creation flow adapts to three user profiles:

#### New User

Has never used an agent framework. Arrives via documentation or recommendation. Wants to "try an AI agent" without understanding the infrastructure.

**Needs**: Template-based strategy selection, minimal jargon, clear cost previews, single-command deployment. Should never see `roko.toml` unless they go looking.

**Experience**: `roko init --template rust-coding` → running agent in under 2 minutes.

#### Experienced User

Active developer or operator with existing infrastructure. Familiar with agent frameworks. Wants automation for workflows they already execute manually.

**Needs**: AI autofill from natural-language description, custom model routing, tool profile customization, real-time cost estimation. Comfortable reviewing generated `roko.toml` but does not want to write it from scratch.

**Experience**: `roko init --prompt "Monitor our staging cluster and alert on anomalies"` → running agent in under 3 minutes.

#### Developer / Institutional Operator

Runs self-hosted infrastructure. Wants full control: custom config files, CI/CD integration, programmatic API access, multi-agent orchestration.

**Needs**: `roko init` to scaffold config, `--config` flag for non-interactive deployment, `--dry-run` for cost preview, MCP config passthrough.

**Experience**: `roko init --config ./production-agent.toml` → deployed in under 60 seconds.

---

## Strategy Templates

Five curated templates cover common agent patterns, providing an instant, free, auditable alternative to AI autofill:

| Template ID | Name | Domain | Description |
|------------|------|--------|-------------|
| `rust-coding` | Rust Coding Agent | Coding | Reads PRDs, generates implementation plans, executes tasks, validates with gates |
| `research` | Research Agent | Research | Deep research with citations, paper retrieval, synthesis |
| `code-review` | Code Review Agent | Coding | Reviews PRs, checks for bugs, suggests improvements |
| `monitoring` | Monitoring Agent | General | Watches metrics, detects anomalies, sends alerts |
| `chain-trading` | Chain Trading Agent | Chain | On-chain trading with wallet management and strategy execution |

Each template includes pre-written STRATEGY.md content with parameter placeholders. Template parameters are validated against constraints before expansion. Templates and AI autofill are mutually exclusive.

---

## AI Autofill

When the user submits a free-text prompt, the wizard calls the configured inference provider to transform the prompt into a complete `AgentExtendedManifest`.

- **Model**: Claude Haiku 4.5 (fast, cheap, sufficient for config generation)
- **Estimated cost**: ~$0.0003 per generation (~700-1350 tokens)
- **Security**: AI autofill output is treated as untrusted. The generated manifest is always displayed for user review and never auto-submitted to the provisioning pipeline.

The system prompt is domain-aware: it fetches the list of available tools for the selected domain and only references available capabilities.

---

## API Flow

```
POST /v1/agents
{
  manifest: AgentExtendedManifest,
  idempotency_key: "uuid-v4"
}
```

After creation, poll `GET /v1/agents/:id/status` for provisioning progress.

---

## Dry-Run and Preview

**Dry-run** validates the manifest, estimates resource usage, and shows the provisioning plan without executing. Useful for CI/CD pipelines and cautious users.

**Preview** simulates the first cognitive loop iteration against available context (for coding agents: reads the codebase; for chain agents: reads on-chain state). Shows what the agent would do on its first tick. No actions executed.

---

## Creation Flow Summary

```
User provides intent (prompt, template, or config file)
  |
  v
Manifest generated (AI autofill or template expansion)
  |
  v
User reviews manifest
  |  (edit any field, change domain, adjust budget)
  v
User confirms
  |
  v
Provisioning pipeline (see 02-provisioning.md)
  |
  1. Validate manifest against domain feature set
  2. Allocate resources (L0 Runtime)
  3. Initialize Neuro store (L1 Framework - Substrate)
  4. Configure model routing (L1 Framework - Router)
  5. Load tool profile (L1 Framework)
  6. Register with Mesh if enabled (L4 Orchestration)
  7. Start cognitive loop
  |
  v
Agent is running
```

The entire flow is designed so that the user never needs to understand the 5-layer taxonomy, the Synapse Architecture, or the Engram format. Those are implementation details that power the system. The user sees: describe, review, confirm, running.

---

## Domain Plugin Registration

Roko's kernel is domain-agnostic (Rule 9 from writing rules). The `roko-core` crate provides the Synapse traits and universal cognitive loop. Domain-specific behavior is injected via plugins:

- **`roko-chain`**: Adds wallet management, on-chain tools, ERC-8004 identity, KORAI/DAEJI token support, Korai chain interaction
- **`roko-coding`** (future): Adds file system tools, compiler gates, test runners, PR creation
- **`roko-research`** (future): Adds citation tools, paper retrieval, synthesis
- Custom plugins register via the `DomainPlugin` trait

At creation time, the selected domain plugin configures which tools are available, which gates are active, and which Neuro knowledge types are prioritized. A chain-domain agent has access to 423+ DeFi tools and wallet management. A coding-domain agent has access to file system operations, build tools, and version control. The kernel — Engrams, Synapse traits, cognitive loop — is identical.

---

## Naming and Identity

Agent names use cryptographic randomness: `agent-{nanoid(12)}`. Names are not derivable from user ID, strategy type, or any other input. The `nanoid` alphabet (A-Za-z0-9_-) produces 64^12 = 4.7 × 10^21 possible names.

For chain-domain agents, creation also registers an ERC-8004 on-chain identity (ERC-721 soulbound token) on the Korai chain. This identity includes:

- Capability bitmask
- Domain stakes
- Reputation tracks
- System prompt hash (ventriloquist defense)
- Agent tier (Protocol/Sovereign/Worker/Edge)

Non-chain agents do not require on-chain identity.

---

## Related Topics

- `docs/17-lifecycle/02-provisioning.md` — Compute provisioning pipeline
- `docs/17-lifecycle/03-configuration-and-operator-model.md` — Config override layers, operator controls
- `docs/17-lifecycle/04-funding-and-budgets.md` — Budget allocation and cost tracking
- `docs/00-architecture/INDEX.md` — 5-layer taxonomy, Synapse Architecture
