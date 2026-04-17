# 14 — roko-plugin SDK

> Five-tier SPI for prompts, profiles, declarative tools and MCPs, native trait implementations,
> and WASM sandboxed extensions. See also
> [tmp/refinements/17-plugin-extension-architecture.md](../../tmp/refinements/17-plugin-extension-architecture.md)
> and [tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md)
> and [docs/00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

> **Implementation**: Specified

---

## Overview

`roko-plugin` is the user-facing SDK for Roko's extension story. The kernel-facing stable
contract is the SPI: a layered set of extension points that let third parties contribute a
Substrate, a Gate, a Scorer axis, a Composer template, a tool, an MCP server, or an entire Role
without forking core code.

The design goal is explicit:

- each tier has one power envelope,
- each tier has one discovery path,
- each tier has one default sandbox,
- and the loader selects the lowest tier that can satisfy the requested capability.

The five tiers are:

| Tier | Extension shape | Typical payload | Default sandbox |
|---|---|---|---|
| 1 | Prompt/template bundle | Role prompts, tool descriptions, system-message overlays | None, pure data |
| 2 | Configuration profile | Team presets, model profiles, domain profile bundles | None, pure data |
| 3 | Declarative tool or MCP manifest | JSON-schema tools, subprocess wrappers, MCP servers | Existing tool safety layer |
| 4 | Native trait implementation | Substrate, Bus, Scorer, Gate, Router, Composer, Policy | Process + ABI isolation |
| 5 | WASM sandboxed extension | Arbitrary logic with bounded host imports | Capability sandbox |

The canonical naming rules live in the glossary. This chapter uses current terms only.

---

## SPI Surface

The stable contract is intentionally small. An extension advertises what it is, what it can do,
and what it needs.

```rust
pub trait Extension: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> semver::Version;
    fn tier(&self) -> Tier;
    fn capabilities(&self) -> &[Capability];
    fn permissions(&self) -> &Permissions;
    fn health(&self) -> Health;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier {
    Prompt,
    Profile,
    Declarative,
    Native,
    Wasm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    PromptTemplate,
    ProfileBundle,
    Tool,
    McpServer,
    Substrate,
    Gate,
    Scorer,
    Router,
    Composer,
    Policy,
    Role,
}
```

The real extension contract is manifest-first. The trait above is the runtime view of a loaded
plugin; the manifest is the discovery-time source of truth.

---

## Manifest Shape

Every plugin declares its tier, entrypoint, capabilities, and permissions in a manifest file.
The manifest is what discovery reads, audits, and installs.

```toml
id = "org.example.cargo-udeps"
version = "0.1.0"
tier = 3
kind = "tool"
description = "Find unused dependencies in a Rust workspace"

[entrypoint]
type = "subprocess"
command = "cargo"
args = ["+nightly", "udeps", "--workspace"]
cwd = "{{workspace_root}}"

[capabilities]
provides = ["Tool"]

[permissions]
roles = ["researcher", "implementer"]
network = false
files_read = ["{{workspace_root}}/**"]
files_write = []
bus_subscribe = ["gate.verdict.emitted"]
bus_publish = ["tool.audit.*"]
timeout_ms = 300000
```

Tier-specific shapes are all variations on the same contract:

- Tier 1 points at Markdown or front-matter prompt bundles.
- Tier 2 points at profile bundles that layer on top of `roko.toml`.
- Tier 3 points at a tool wrapper or an MCP server definition.
- Tier 4 points at a native crate or `cdylib` entrypoint plus an ABI version.
- Tier 5 points at a `.wasm` module plus host capability grants.

Tier 2 is the preferred bundle boundary for domain-specific agents: a profile can package
tools, roles, gates, heuristics, templates, and the typed context schema that downstream tools
expect. The bundle remains data, not executable policy.

Discovery should never depend on a user editing a central plugin registry by hand.

---

## Tier 1 And 2

Tier 1 and Tier 2 are pure data. They are safe to load without executing code.

### Tier 1 - Prompt and template bundles

Use Tier 1 when the plugin is just text:

- role prompts,
- system overlays,
- tool descriptions,
- evaluator notes,
- or reusable template fragments.

Typical layout:

```text
plugins/prompts/compliance-reviewer/
  manifest.toml
  prompt.md
  overlays/
```

The manifest names the bundle and its intended roles; the loader concatenates the data into the
appropriate prompt surface.

### Tier 2 - Configuration profiles

Use Tier 2 for opinionated presets:

- model defaults,
- domain-specific safety settings,
- routing preferences,
- or team-level tool allowlists.

In the domain-agent model, Tier 2 is the installable profile bundle. The user gets a coherent
starting point for a domain rather than a bag of unrelated extensions, and the loader can
validate the bundle before activation.

Typical layout:

```text
plugins/profiles/rust-oss/
  manifest.toml
  profile.toml
```

Profiles can inherit from built-in defaults but remain explicit data, not executable policy.

Typical profile bundle contents:

- `tools` or tool categories to expose by default,
- `roles` that should exist at boot,
- `gates` that should wrap tool execution,
- `heuristics` and starter prompts,
- and any domain-specific context or custody metadata that loaders need to enforce before
  activation.

---

## Tier 3 Declarative Tools And MCPs

Tier 3 is the biggest ecosystem win. A third party should be able to ship a useful tool or MCP
server without writing Rust.

Two patterns are canonical:

- a declarative tool wrapper around a subprocess or HTTP call,
- an MCP server manifest that the loader can spawn and interrogate.

```toml
id = "org.example.github-search"
version = "1.4.0"
tier = 3
kind = "mcp"
description = "Search GitHub repositories and issues"

[entrypoint]
type = "mcp"
command = "roko-mcp-github"
args = ["--token-env", "GITHUB_TOKEN"]

[permissions]
roles = ["researcher", "operator"]
network = true
bus_subscribe = ["agent.msg.chunk"]
bus_publish = ["tool.result.*"]
timeout_ms = 15000
```

Tier 3 extensions are still governed by the existing tool safety layer:

- role allowlists,
- file and network bounds,
- timeouts,
- and tool-call auditing.

---

## Tier 4 Native Implementations

Tier 4 is for extensions that must participate directly in the kernel trait set.

Examples:

- a custom vector store implementing `Substrate`,
- a routing strategy implementing `Router`,
- a custom gate pipeline implementing `Gate`,
- a scorer axis bundle,
- or a domain-specific composer.

Native extensions need an ABI bridge because Rust does not promise a stable plugin ABI.
The recommended shape is:

- a narrow `roko-extension-abi` crate,
- a manifest declaring the ABI version,
- and either a `cdylib` or in-tree workspace build path.

```toml
id = "org.example.medical-gate"
version = "0.2.1"
tier = 4
kind = "native"

[entrypoint]
type = "cdylib"
path = "./plugins/native/medical_gate.so"
abi = "roko-extension-abi/1"

[capabilities]
provides = ["Gate"]

[permissions]
roles = ["reviewer"]
files_read = ["**/*.md", "**/*.json"]
network = false
timeout_ms = 2000
```

The loader validates the ABI version before instantiation and keeps the native plugin inside the
same policy and observability envelope as built-in traits.

---

## Tier 5 WASM Extensions

Tier 5 is the safest path for third-party code that still needs logic.
It uses a capability sandbox with explicit host imports and bounded resources.

Host imports are intentionally small:

```rust
pub mod host {
    pub fn bus_publish(pulse_ptr: u32, pulse_len: u32) -> i64;
    pub fn bus_subscribe(filter_ptr: u32, filter_len: u32) -> i32;
    pub fn bus_recv(handle: i32, buf_ptr: u32, buf_cap: u32) -> i64;
    pub fn substrate_query_similar(fp_ptr: u32, radius_bits: u32, limit: u32,
                                   out_ptr: u32, out_cap: u32) -> i64;
    pub fn log(level: u32, msg_ptr: u32, msg_len: u32);
    pub fn now_ms() -> i64;
}
```

The sandbox enforces:

- no ambient filesystem access,
- no ambient network access,
- bounded memory,
- bounded CPU or wall-clock time,
- and rate limits on Bus and Substrate calls.

Tier 5 is where we expect serious third-party development to land when safety matters more than
in-process performance.

---

## Permissions And Health

Every loaded plugin declares permissions up front.
The runtime uses them to decide whether the plugin can be installed, enabled, or executed.

```toml
[permissions]
network = false
files_read = ["**/*.rs"]
files_write = []
bus_subscribe = ["gate.verdict.emitted"]
bus_publish = ["plugin.violation"]
substrate_read = ["GateVerdict"]
substrate_write = []
memory_mb = 64
cpu_ms = 100
```

The runtime also tracks health:

- manifest validation,
- capability drift,
- ABI compatibility,
- heartbeat freshness,
- error rate,
- and resource use.

Health is part of the SPI contract, not an afterthought.

---

## Discovery And CLI

Discovery is by location and manifest, not by mutating `roko.toml` to enumerate every plugin.
`roko.toml` may still carry global defaults, but it is not the plugin catalog.

Canonical CLI surface:

```bash
roko plugin list
roko plugin search <query>
roko plugin install <id>
roko plugin enable <id>
roko plugin disable <id>
roko plugin uninstall <id>
roko plugin info <id>
roko plugin audit
```

The CLI works with installed manifests, registry metadata, and local plugin roots. It should be
possible to install a plugin and use it without hand-editing a runtime configuration file.

---

## Example Flows

### Add A Prompt Bundle

A team ships a Tier 1 compliance reviewer prompt. The loader discovers a manifest and Markdown
bundle under `plugins/prompts/`, the CLI enables it, and the runtime wires the template into the
Role surface.

### Add A Declarative Tool

A team wants a `cargo udeps` integration. They ship a Tier 3 manifest that wraps a subprocess,
declare the role allowlist and file scope, and install it through `roko plugin install`.

### Add A Native Gate

A team wants a Gate implementation that inspects a domain-specific substrate. They ship a Tier 4
crate plus ABI manifest, and the loader validates the bridge before registering the trait.

### Add A Sandboxed Extension

A third party wants to publish logic with bounded authority. They ship a Tier 5 WASM bundle,
declare its host imports and rate limits, and the loader runs it inside the capability sandbox.

---

## Cross References

- `docs/18-tools/16-plugin-loading.md` covers discovery, load order, and the `roko plugin` CLI in
  more detail.
- `docs/00-architecture/01-naming-and-glossary.md` is the canonical vocabulary reference for
  `Bus`, `Topic`, `Pulse`, `Engram`, and related terms.
- `tmp/refinements/17-plugin-extension-architecture.md` is the source refinement for this chapter.
- `docs/12-interfaces/INDEX.md` contains the user-facing surfaces that consume this SPI.
