# 16 — Plugin Loading Mechanisms

> Discovery-first loading for the five-tier SPI. Manifests are the source of truth; `roko.toml`
> is runtime configuration, not the plugin catalog. See also
> [tmp/refinements/17-plugin-extension-architecture.md](../../tmp/refinements/17-plugin-extension-architecture.md)
> and [tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md)
> and [docs/00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

> **Implementation**: Target-state design
>
> **Implementation status**: The current runtime does not load plugins from
> manifests. Tool registration is compile-time Rust via
> `roko_std::StaticToolRegistry`, with MCP/config-driven layering on top. The
> discovery-first manifest flow in this chapter is a planned loader model, not
> current behavior. The proposed `roko plugin` CLI, native ABI loading, WASM
> loading, and registry-backed discovery are aspirational.

---

## Overview

This chapter describes the target-state loading flow: discover manifests, validate declared
tiers and permissions, and then select the matching loader and sandbox.

The loader does not depend on a single config file listing every plugin. Instead:

- Tier 1 and Tier 2 are data bundles discovered from plugin roots.
- Tier 3 manifests declare declarative tools or MCP servers.
- Tier 4 manifests declare native trait implementations and their ABI bridge.
- Tier 5 manifests declare WASM modules and host capability grants.

In the target state, the `roko plugin` CLI would sit on top of this flow and give operators a
discovery and lifecycle surface without exposing internal loader details.

---

## Discovery Sources

Discovery is driven by file layout and install metadata.

| Tier | Discovery source | Loader action |
|---|---|---|
| 1 | `plugins/prompts/**/manifest.toml` | Load Markdown or front-matter bundles |
| 2 | `plugins/profiles/**/manifest.toml` | Merge profile defaults into runtime settings and resolve profile bundles |
| 3 | `plugins/tools/**/manifest.toml` and `plugins/mcp/**/manifest.toml` | Spawn subprocesses or MCP servers and expose tools |
| 4 | `plugins/native/**/manifest.toml` or workspace-local extension crates | Resolve ABI bridge, load native trait implementation |
| 5 | `plugins/wasm/**/manifest.toml` | Instantiate module inside capability sandbox |

These proposed roots keep discovery local, inspectable, and auditable. The goal is that a
plugin can be installed, listed, and enabled without editing a global registry by hand.

For Tier 2 profile bundles, the loader also validates composition. Multiple installed profiles
can be active together, but the runtime should resolve conflicts explicitly rather than
silently overwriting settings.

---

## CLI Surface

This section is target-state. Today there is no shipped `roko plugin` command group.

The canonical future user workflow is the `roko plugin` command group:

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

The commands map directly to discovery and policy:

- `list` and `search` are read-only discovery operations.
- `install` fetches a manifest bundle into the local plugin root or registry cache.
- `enable` and `disable` toggle the manifest without mutating the bundle itself.
- `audit` reports permissions, sandbox requirements, ABI version, and any policy conflicts.
- `profile check`-style validation reports bundle conflicts, missing context keys, and custody
  mismatches before activation.

`roko.toml` may still set defaults such as plugin roots or registry mirrors, but in the
target state it is not the canonical place to enumerate every extension.

---

## Loading Lifecycle

The runtime follows one lifecycle for all tiers:

1. Discover manifests from plugin roots or installed metadata.
2. Parse the manifest and confirm the declared tier.
3. Validate the capability claims and permissions.
4. Select the loader for the tier.
5. Apply the tier-specific sandbox.
6. Instantiate the extension.
7. Register the exposed capability with the relevant subsystem.
8. Monitor health and unload on shutdown or policy failure.

```text
discover -> validate -> sandbox -> instantiate -> register -> monitor -> unload
```

The actual registration target depends on the tier:

- Tier 1 updates prompt and template surfaces.
- Tier 2 updates profile resolution and domain-bundle composition.
- Tier 3 adds tools or MCP-backed capabilities.
- Tier 4 registers kernel traits.
- Tier 5 registers sandboxed host functions and tool surfaces.

### Profile Composition

Tier 2 bundles compose before tool registration. The default merge behavior is:

- tools merge by union,
- roles merge by union, with a collision warning if two bundles define the same role,
- gates stack unless a bundle scopes them to a profile name,
- heuristics coexist and are routed by situation fit,
- and profile priority resolves key conflicts when two bundles set the same override.

The loader should also preserve the bundle's typed context schema and custody expectations so
downstream tool sets can validate structured intent before they are exposed to the LLM.

---

## Sandbox Model

The sandbox is selected from the manifest, not from the call site.

| Tier | Default sandbox | Enforcement notes |
|---|---|---|
| 1 | None | Pure data |
| 2 | None | Pure data |
| 3 | Existing tool safety layer | Role allowlists, file bounds, network bounds, timeout controls |
| 4 | Process isolation + ABI bridge | Native code stays behind a stable bridge and policy checks |
| 5 | WASM capability sandbox | Memory, CPU, time, and host-import limits are enforced |

Tier 3 and Tier 5 are the most important operational boundaries:

- Tier 3 can still call out to subprocesses or MCP servers, so permissions must be explicit.
- Tier 5 can execute arbitrary logic, but only through bounded host imports such as Bus publish,
  Bus subscribe, Substrate query, logging, and time.

---

## Tier-Specific Loaders

### Tier 1 And 2

The loader reads the manifest, loads the data bundle, and merges it into the prompt or profile
surface. No code is executed.

For Tier 2, the merge step also resolves multiple installed profile bundles into one active
domain view. That resolution is where domain-specific tools, gates, and custody rules become a
single coherent starting point.

### Tier 3

The loader resolves the entrypoint, spawns the subprocess or MCP server, and converts the
declared tool schema into the runtime tool registry.

### Tier 4

The loader resolves the ABI bridge, checks the ABI version, and loads the native implementation
either from an installed package or a workspace-local crate. If the ABI version mismatches, the
plugin is rejected before registration.

### Tier 5

The loader instantiates the WASM module with the declared capability grants and resource caps.
Any host call outside the manifest is denied.

---

## Validation Rules

Every plugin is validated before activation:

- the manifest parses,
- the tier matches the entrypoint type,
- declared capabilities are internally consistent,
- permissions are within policy,
- ABI versions match for native extensions,
- and the sandbox requested by the manifest is available on the current platform.

Validation failures are surfaced through `roko plugin audit` and the runtime logs.

---

## Recommended Strategy

The recommended loading strategy is the same as the extension strategy:

- prefer Tier 1 when the change is pure text,
- prefer Tier 2 when the change is a profile,
- prefer Tier 3 when the extension can be declarative,
- use Tier 4 only when direct trait participation is required,
- and use Tier 5 when untrusted code needs bounded logic.

That choice keeps the platform easy to extend without collapsing safety into configuration
sprawl.

For domains, this means "install a profile bundle first, then let the loader resolve its tool
set" rather than hand-editing a registry of individual tools.

---

## Cross References

- `docs/18-tools/14-plugin-sdk.md` defines the SPI and manifest shape in more detail.
- `docs/00-architecture/01-naming-and-glossary.md` is the canonical vocabulary reference.
- `tmp/refinements/17-plugin-extension-architecture.md` is the source refinement for this chapter.
- `docs/12-interfaces/INDEX.md` owns the user-facing CLI surfaces that drive this loader.
