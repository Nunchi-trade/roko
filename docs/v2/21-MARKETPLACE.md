# 21 -- Marketplace and Sharing

> Publish, discover, install, fork, rate, and attribute Cells, Graphs, Racks, and Knowledge Bundles in a community marketplace. Composition over isolation. Trust by evidence. DAW composability: Criteria as plugins, Profiles as presets, fork as fundamental. Transparent take-rates. Creator ownership. On-chain attribution via ERC-8004. Backend services expressed as Cell specializations. Publish and install flows expressed as Pipeline Graphs of Verify Cells.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal lineage, content addressing), [02-CELL](02-CELL.md) (Cell protocol, Verify), [03-GRAPH](03-GRAPH.md) (Graph, Pipeline pattern), [15-TELEMETRY](15-TELEMETRY.md) (UsageLens, TrendLens), [16-SECURITY](16-SECURITY.md) (capability intersection), [19-CONFIG](19-CONFIG.md) (5-tier SPI, TOML schemas), [22-REGISTRIES](22-REGISTRIES.md) (ERC-8004 attribution)

---

## 1. Overview

Every byproduct of using Roko is a shareable, composable primitive. The marketplace turns local Cells, Graphs, Racks, and Knowledge Bundles into community artifacts -- the DAW preset / Splice sample / Figma Community pattern applied to agent orchestration.

Design principles:
- **Composition over isolation** -- artifacts reference each other; forking preserves lineage.
- **Trust by evidence** -- Verified Run badges, gate pass rates, and community validation replace trust by authority.
- **Capability transparency** -- every artifact's permission requirements are visible before install.
- **Observation built in** -- UsageLens ([15-TELEMETRY](15-TELEMETRY.md)) tracks usage metrics for every published artifact; creators see analytics.
- **Creator-first economics** -- transparent take-rates, creators own customer relationships, all metrics published.
- **Fork as fundamental** -- forking is not a failure of the original; it is the primary mechanism for adaptation.
- **Everything is a Cell** -- backend services (storage, trending, verification) are Cell specializations implementing standard protocols. Publish and install flows are Pipeline Graphs of Verify Cells.

---

## 2. The 5-Tier Package SPI

The Service Provider Interface defines five tiers for authoring Cells, Extensions, Graphs, and agent capabilities. Each tier balances expressiveness against isolation. Progressive capability with progressive trust.

### Tier 1: Prompts (pure Markdown/TOML front-matter, no execution)

Lowest-friction tier. A prompt package is a Markdown file with TOML front-matter. No code executes -- content is injected into system prompt or context assembly.

```markdown
---
name = "code-review-system"
version = "1.0.0"
description = "System prompt for thorough code reviews"
tags = ["coding", "review"]
target = "system_prompt"
layer = "cognition"
---

# Code Review Protocol

When reviewing code changes, follow these steps:
1. **Read the diff completely** before commenting.
2. **Check for security issues** first: injection, auth bypass, data exposure.
3. **Check for correctness**: edge cases, error handling, resource leaks.
4. **Check for clarity**: naming, structure, documentation.
5. **Suggest improvements** with concrete alternatives.
```

**Sandboxing**: None needed. No execution occurs.
**Distribution**: Anyone can publish prompts.

### Tier 2: Config Profiles (TOML bundles layering onto roko.toml)

A config profile layers onto `roko.toml`, customizing agent behavior without writing code. Profiles configure existing capabilities -- they do not add new ones.

```toml
[profile]
name        = "defi-trading"
version     = "1.0.0"
description = "Config profile for DeFi trading agents"
tags        = ["trading", "defi", "finance"]
base        = "trading"

[profile.clock]
gamma_ms = 200
theta_ms = 2000
delta_ms = 60000

[profile.models]
primary   = "claude-opus-4-6"
fallback  = "claude-sonnet-4-6"
reflexive = "claude-haiku-4-5"
```

**Sandboxing**: None. Config profiles do not execute.
**Distribution**: Anyone. Profiles reviewed for sanity.

### Tier 3: Declarative Tools (TOML manifests for subprocess/HTTP/MCP, sandboxed)

Wraps a subprocess, HTTP endpoint, or MCP server as a Cell. The manifest declares I/O schemas, capability requirements, and invocation details.

```toml
[tool]
name = "github-pr-review"
version = "1.0.0"
description = "Fetch and review GitHub pull requests"

[tool.capabilities]
required = ["net"]

[tool.invoke]
kind    = "subprocess"
command = ["gh", "pr", "view", "{{input.pr_number}}", "--repo", "{{input.repo}}", "--json", "body,files,comments,diff"]
stdout  = "json"
timeout_seconds = 30
```

**Sandboxing**: OS-level process isolation. Network restricted to declared domains.
**Distribution**: Verified publishers (can invoke subprocesses).

### Tier 4: WASM (sandboxed, fuel-metered, marketplace-recommended)

WebAssembly modules using a roko-defined ABI (`wit-bindgen` interfaces). The recommended tier for marketplace artifacts: deterministic builds, sandboxing for free.

```toml
[cell.impl]
tier      = "wasm"
path      = ".roko/plugins/markdown-classify-1.2.3.wasm"
checksum  = "blake3:abc123..."
memory_mb = 64
fuel      = 100_000_000
```

**Sandboxing**: WASM sandbox. Memory isolated. Fuel-metered. Capability-gated via ABI.
**Distribution**: Marketplace (recommended default). Portable, deterministic, safe.

### Tier 5: Native Rust (compiled, full trust, in-tree only)

Compiled Rust code implementing `impl Cell for MyCell`. Highest performance, no sandboxing. Reserved for built-in components and trusted in-tree plugins.

```toml
[cell.impl]
tier  = "rust"
crate = "roko-builtin-doc"
type  = "MarkdownClassifyCell"
```

**Sandboxing**: Process-level only. Full trust.
**Distribution**: Compiled into binary. Marketplace artifacts may NOT use this tier directly.

### Tier comparison

| Property | 1. Prompts | 2. Config | 3. Declarative | 4. WASM | 5. Rust |
|---|---|---|---|---|---|
| Executes code | No | No | Subprocess/HTTP | WASM sandbox | Native |
| Sandboxing | N/A | N/A | OS-level process | WASM + fuel | Process-level |
| Capability control | N/A | N/A | Declared, enforced | ABI-gated | Full trust |
| Marketplace | Anyone | Anyone | Verified publisher | Anyone | Not directly |
| Performance | N/A | N/A | Subprocess overhead | Near-native | Native |
| Deterministic builds | N/A | N/A | No | Yes | Depends |
| Friction | Lowest | Low | Medium | Medium | Highest |

The visual editor ([20-SURFACES](20-SURFACES.md)) only writes TOML (Tiers 1-3). WASM and Rust tiers require build tools.

---

## 3. Marketplace Economics

The marketplace exists because transparent economics solve the problem that killed the GPT Store: opaque revenue sharing without reputation infrastructure destroys creator trust.

### 3.1 Take-rate structure

| Revenue band | Take-rate | Precedent |
|---|---|---|
| First $1M lifetime creator revenue | **0%** | Shopify ($0 until you succeed) |
| Above $1M lifetime | **12-15%** | Unreal Engine (5% after $1M) |

The 0% band is per-creator, lifetime. A creator who earns $999,999 pays nothing. At $1,000,001, they pay 12-15% on the $1 above the threshold -- not retroactively on the first $1M.

### 3.2 Revenue model comparison

| Platform | Take-rate | Creator owns customer? | Metrics public? | Outcome |
|---|---|---|---|---|
| **GPT Store** | Opaque | No | No | Median <$100/quarter |
| **npm** | 0% (free) | N/A (OSS) | Download counts only | Sustainability crisis |
| **Unreal Marketplace** | 12% | Partial | Yes | Healthy creator economy |
| **Unity Asset Store** | 30% | No | Partial | Creator complaints |
| **Roko Marketplace** | 0% to $1M, 12-15% above | **Yes** | **All metrics public** | Target: sustainable |

### 3.3 Published metrics (mandatory)

Every published artifact surfaces these metrics publicly. Creators cannot hide them. Consumers cannot be misled.

| Metric | Visibility | Update frequency |
|---|---|---|
| Installs (lifetime, 30d, 7d) | Public | Hourly |
| Active runs (30d) | Public | Hourly |
| Fork count (lifetime) | Public | Hourly |
| Gate pass rates (30d) | Public | Hourly |
| Mean cost per run (30d) | Public | Daily |
| Mean duration per run (30d) | Public | Daily |
| Error rate (30d) | Public | Daily |
| Revenue (creator only) | Creator-only | Real-time |

### 3.4 Creator owns the customer

Creators can:
- Export their installer list (email/wallet with opt-in consent).
- Send release notes and changelogs directly to installers.
- Offer paid tiers (free/pro/team) with feature gating via Macros ([19-CONFIG](19-CONFIG.md)).
- Run their own support channels (linked from the artifact page).

The marketplace is a distribution channel, not a walled garden. If a creator wants to take their artifacts elsewhere, they can.

### 3.5 Anti-GPT-Store lessons

| GPT Store failure | Roko marketplace response |
|---|---|
| Opaque revenue sharing | Published take-rate schedule, 0% to $1M |
| No creator analytics | Full UsageLens analytics for every artifact |
| No forking | Fork as fundamental operation |
| Platform owns the customer | Creator owns the customer |
| No quality signal beyond star ratings | Gate pass rates, active runs, Verified Run badges |
| Discoverability by editorial fiat | Algorithmic trending + editorial + search |
| No composability | Artifacts compose via Slots, Macros, sub-Graph references |
| Invite-only revenue sharing | All creators eligible from day one |

---

## 4. DAW Composability

The marketplace is modeled on DAW (Digital Audio Workstation) plugin ecosystems: Criteria are plugins, Profiles are presets, fork is the remix.

### 4.1 Criteria as plugins

A Verify Cell ([02-CELL](02-CELL.md)) is a plugin that evaluates one dimension of quality. Just as a DAW audio plugin slots into any channel strip, a Criterion Cell slots into any Verify pipeline:

- Criteria are individually publishable marketplace artifacts.
- A Verify Graph is a composition of Criterion Cells -- a "rack" of quality checks.
- Users install individual Criteria and wire them into their own verification pipelines.
- Custom Criteria are just Cells that implement the Verify protocol.

### 4.2 Profiles as presets

A Profile is a Rack that computes a Verdict: a pre-wired composition of Criteria with tuned Macros. Just as a DAW preset is a saved configuration of plugin parameters:

- Profiles are publishable marketplace artifacts.
- A Profile bundles: which Criteria to run, in what order, with what thresholds, with what weights.
- Users install a Profile, adjust its Macros, and use it as their verification pipeline.
- "Strict code review," "fast prototype check," and "security audit" are all Profiles over the same Criteria library.

### 4.3 Fork as fundamental

Forking is not a negative signal. It is the primary adaptation mechanism:

- Every marketplace artifact is forkable with one command.
- Fork preserves lineage (visible chain on the artifact page).
- Fork chains are first-class marketplace navigation (browse a fork tree, diff between any two nodes).
- On-chain attribution via ERC-8004 means forks that succeed generate reputation for the original author.

### 4.4 Composability hierarchy

```
Criterion (plugin)
  -> Profile (preset: N Criteria + tuned Macros)
    -> Rack (parameterized Graph with Slots + Macros)
      -> Graph (full state graph of Cells)
        -> Space Template (skeleton: Graph + Triggers + capabilities + models)
```

Each level composes the level below. The marketplace supports publishing and forking at every level.

---

## 5. Publishable Artifact Types

11 artifact kinds, in launch order (smallest / lowest-risk first):

| # | Kind | Risk | Forking | Notes |
|---|---|---|---|---|
| 1 | **Snippet** | None (paste into editor) | Copy | Saved sub-graph fragments |
| 2 | **Prompt Preset** | None (system prompt + role + model) | Copy | Tier 1 SPI |
| 3 | **Criterion** | None (single Verify Cell) | Copy | Individual quality check plugin |
| 4 | **Cell** (composition) | None (pure composition) | Copy | No new execution surface |
| 5 | **Cell** (script / WASM) | Sandboxed execution | Copy | Requires capability disclosure |
| 6 | **Profile** | Composition of Criteria | Copy | Verification preset + cognitive posture |
| 7 | **Graph** | Composition of Cells | Copy | Full state graph |
| 8 | **Rack** | Graph + Macros + Slots | Copy | Parameterized Graph |
| 9 | **Trigger Binding** | Event config + bound Graph | Copy | Trigger + Graph pairing |
| 10 | **Space Template** | Workspace skeleton | Copy | Capabilities, models, deploy targets |
| 11 | **Knowledge Bundle** | Curated knowledge Signals | Reference + Fork | Are.na-style composition by reference |

v1: Snippets, Prompt Presets, Criteria, Cells, Profiles, Graphs. v1.1: Racks, Trigger Bindings. v1.2: Space Templates, Knowledge Bundles.

---

## 6. Identity and Attribution

### 6.1 Publisher identity

Marketplace identity uses GitHub OAuth for baseline reputation. Optional Google OAuth as alternate. Email-only accounts cannot publish.

Each artifact carries `publisher: "@github_handle"`. Anonymous publishing is not supported in v1.

### 6.2 Artifact identity

Marketplace artifact references use `@publisher/name@version`:

```
@my-org/markdown-classify@1.2.3
@nunchi/doc-ingest@1.0.0
@wpank/strict-pr-review@2.0.0-rc.1
```

### 6.3 Lineage

Every fork records lineage through `Signal.source` (the same content-addressed lineage that Signals carry):

```rust
pub struct ArtifactLineage {
    pub forked_from: Option<ArtifactRef>,
    pub composed_from: Vec<ArtifactRef>,
    pub forked_at: DateTime<Utc>,
    pub onchain_attestation: Option<ERC8004Ref>,
}
```

Lineage is publicly visible. On-chain lineage (via ERC-8004) provides cryptographic proof.

### 6.4 License

Default: **CC BY 4.0** for all artifact kinds. Also supported: CC BY-SA 4.0, CC0 1.0, MIT, Apache-2.0. License is required at publish. Knowledge Bundles default to CC BY-SA 4.0.

### 6.5 On-chain attribution via ERC-8004

Fork chains and provenance are anchored on-chain via ERC-8004 ([22-REGISTRIES](22-REGISTRIES.md)):

- **Fork chain provenance**: fork relationships recorded as ERC-8004 attestations. The chain of `forked_from` references forms a Merkle DAG.
- **Reputation flows upstream**: when a fork earns reputation (installs, runs, gate pass rates), a portion flows upstream to the original author.
- **ZK-attested quality**: gate pass rates can be ZK-attested (proving the rate without revealing individual run data) and anchored in ERC-8004 records.

---

## 7. Publish Flow as Pipeline Graph

The publish flow is a Pipeline Graph of Verify Cells. Each stage either passes the artifact forward or rejects with a typed reason. The Pipeline is defined in TOML and executed by the standard Graph engine -- no bespoke publish logic.

### 7.1 Pipeline Definition

```toml
[graph]
name    = "marketplace-publish-pipeline"
pattern = "pipeline"

[[graph.nodes]]
id    = "checksum"
cell  = "roko:checksum-verify-cell@^1"
[graph.nodes.params]
algorithm = "blake3"

[[graph.nodes]]
id    = "signature"
cell  = "roko:signature-verify-cell@^1"
[graph.nodes.params]
require_publisher_key = true

[[graph.nodes]]
id    = "capability-check"
cell  = "roko:capability-disclosure-cell@^1"
[graph.nodes.params]
warn_unrestricted = true

[[graph.nodes]]
id    = "semver-check"
cell  = "roko:semver-verify-cell@^1"
[graph.nodes.params]
allow_prerelease = true

[[graph.nodes]]
id    = "schema-validate"
cell  = "roko:schema-verify-cell@^1"
[graph.nodes.params]
strict = true

[[graph.nodes]]
id    = "store"
cell  = "roko:artifact-store-cell@^1"
[graph.nodes.params]
storage_backend = "s3"
index_backend   = "postgres"

[[graph.edges]]
from = "checksum"
to   = "signature"

[[graph.edges]]
from = "signature"
to   = "capability-check"

[[graph.edges]]
from = "capability-check"
to   = "semver-check"

[[graph.edges]]
from = "semver-check"
to   = "schema-validate"

[[graph.edges]]
from = "schema-validate"
to   = "store"
```

### 7.2 Publish Pipeline Cells

Each Cell in the pipeline implements the Verify protocol. Typed I/O:

| Cell | Input | Output | Rejects When |
|---|---|---|---|
| `ChecksumVerifyCell` | `ArtifactBundle` | `ArtifactBundle + ContentHash` | Checksum mismatch |
| `SignatureVerifyCell` | `ArtifactBundle + ContentHash` | `ArtifactBundle + Signature` | Missing or invalid publisher key |
| `CapabilityDisclosureCell` | `ArtifactBundle + Signature` | `ArtifactBundle + CapabilityReport` | Undisclosed dangerous capabilities |
| `SemverVerifyCell` | `ArtifactBundle + CapabilityReport` | `ArtifactBundle + VersionRecord` | Duplicate version, invalid semver |
| `SchemaVerifyCell` | `ArtifactBundle + VersionRecord` | `ArtifactBundle + SchemaReport` | TOML schema validation failure |
| `ArtifactStoreCell` | `ArtifactBundle + SchemaReport` | `PublishedArtifact` | Storage failure |

### 7.3 CLI Surface

```
$ roko market publish doc-ingest
Validating doc-ingest@1.0.0...
  [checksum]         blake3:abc... computed
  [signature]        publisher @wpank verified
  [capability-check] FsRead, FsWrite, Llm, Net declared
  [semver-check]     1.0.0 valid, no conflicts
  [schema-validate]  TOML schema valid

Capabilities your artifact requires (consumers will see this):
  FsRead              "any path"   (recommend: restrict)
  FsWrite             "any path"   (recommend: restrict)
  Llm                 "any provider"
  Net                 "*"          (recommend: list specific domains)
[continue anyway / restrict capabilities / abort]

Visibility:  ( ) public   ( ) org-only   (*) private
Tags:        doc, ingest, authoring
README:      .roko/graphs/doc-ingest.README.md  (auto-detected)
Sample input: .roko/graphs/doc-ingest.fixtures/  (auto-detected, 2 fixtures)

Publish? [Y/n]
Publishing...
  Storing artifact via ArtifactStoreCell
  Anchoring lineage to ERC-8004 (if creator opted in)
Published as @wpank/doc-ingest@1.0.0
```

Publish is also available via Dashboard ("Publish" button in the visual editor), TUI (`p` from F2 Graphs), and as a meta-Graph.

---

## 8. Browse and Discovery

### 8.1 Faceted browse

Three primary tabs:
- **Featured** -- editorial picks; refreshed weekly
- **Trending** -- install velocity over last 7 days (driven by MarketplaceTrendLens, see SS8.3)
- **All** -- full search

Filters: Kind, Category, Tags, Capabilities required, Verified Run badge, Gate pass rate (minimum), License, Publisher, Recency, Revenue band.

### 8.2 Preview ("Tinker Mode")

In-place preview. Marketplace artifacts have a "Preview" button that runs against bundled sample input in a sandboxed worker, returns within ~30s, displays output in-page. No install required.

For Graphs that take >30s, preview shows a recorded "last successful run" trace as fallback. For Snippets and Prompt Presets, preview is instant.

### 8.3 MarketplaceTrendLens -- Formal Definition

The trending algorithm is a **Lens Cell** -- a Cell implementing the Observe protocol as defined in [15-TELEMETRY](15-TELEMETRY.md) SS4.8. It chains from UsageLens output and computes install velocity trends for the Trending tab. This is the same TrendLens pattern used throughout the telemetry system, specialized for marketplace install metrics.

```rust
/// MarketplaceTrendLens: a Lens Cell (Cell + Observe protocol).
///
/// Chains from UsageLens to compute install velocity trends per artifact.
/// Used to drive the "Trending" tab in marketplace browse.
///
/// Cell:     MarketplaceTrendLens
/// Protocol: Observe (Lens specialization)
/// Input:    Vec<Signal { kind: Observation }> from UsageLens
/// Output:   Vec<Signal { kind: Trend }> with install velocity slope
pub struct MarketplaceTrendLens {
    id: CellId,
    /// Window for trend computation (default: 7 days).
    window: Duration,
    /// Minimum data points before emitting a trend (default: 10).
    min_data_points: usize,
}

impl Cell for MarketplaceTrendLens {
    fn id(&self) -> CellId { self.id }
    fn name(&self) -> &str { "marketplace-trend-lens" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
    fn capabilities(&self) -> &Capabilities { Capabilities::read_only() }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let install_series = extract_install_timeseries(&input)?;
        let mut trends = Vec::new();
        for (artifact_ref, series) in &install_series {
            if series.len() < self.min_data_points { continue; }
            let slope = linear_regression_slope(series);
            let ema = exponential_moving_average(series, 0.3);
            let direction = if slope > 0.1 {
                TrendDirection::Rising
            } else if slope < -0.1 {
                TrendDirection::Falling
            } else {
                TrendDirection::Stable
            };
            trends.push(Signal::new(
                Kind::Trend,
                TrendPayload {
                    source_lens: "usage-lens".into(),
                    metric: format!("installs:{}", artifact_ref),
                    window: self.window,
                    slope,
                    ema,
                    ema_previous: ema,
                    direction,
                    r_squared: r_squared(series, slope),
                    data_points: series.len(),
                },
            ));
        }
        Ok(trends)
    }
}

impl Observe for MarketplaceTrendLens {
    async fn observe(&self, event: &ObservableEvent) -> Result<Vec<Signal>> {
        self.execute(vec![event.as_signal()], &CellContext::default()).await
    }

    fn observes(&self) -> &[ObservableEventKind] {
        // Observes UsageLens output (which emits CellLifecycle observations)
        &[ObservableEventKind::CellLifecycle]
    }

    fn scope(&self) -> LensScope {
        // Chains from UsageLens -- observes its output, not raw events
        LensScope::Lens(LensRef::named("usage-lens"))
    }
}
```

**TOML configuration:**

```toml
[[lenses]]
name  = "marketplace-trending"
cell  = "roko:marketplace-trend-lens@^1"
scope = "lens:usage-lens"
[lenses.params]
window          = "7d"
min_data_points = 10
```

**Trending sort**: artifacts are ranked by `slope` from MarketplaceTrendLens output. Ties broken by `ema` (smoothed install rate). Only artifacts with `direction = Rising` and `r_squared > 0.5` appear in the Trending tab.

---

## 9. Install Flow as Pipeline Graph

The install flow is a Pipeline Graph of Verify Cells, symmetric to the publish flow. Each stage validates one aspect of the downloaded artifact before it is installed locally.

### 9.1 Pipeline Definition

```toml
[graph]
name    = "marketplace-install-pipeline"
pattern = "pipeline"

[[graph.nodes]]
id    = "resolve"
cell  = "roko:semver-resolve-cell@^1"
[graph.nodes.params]
registry = "marketplace.roko.dev"

[[graph.nodes]]
id    = "download"
cell  = "roko:artifact-download-cell@^1"
[graph.nodes.params]
cdn = "cdn.roko.dev"

[[graph.nodes]]
id    = "checksum"
cell  = "roko:checksum-verify-cell@^1"
[graph.nodes.params]
algorithm = "blake3"

[[graph.nodes]]
id    = "signature"
cell  = "roko:signature-verify-cell@^1"

[[graph.nodes]]
id    = "capability-intersect"
cell  = "roko:capability-intersect-cell@^1"
[graph.nodes.params]
prompt_on_gap = true

[[graph.nodes]]
id    = "install"
cell  = "roko:artifact-install-cell@^1"
[graph.nodes.params]
lockfile = ".roko/marketplace.lock"

[[graph.edges]]
from = "resolve"
to   = "download"

[[graph.edges]]
from = "download"
to   = "checksum"

[[graph.edges]]
from = "checksum"
to   = "signature"

[[graph.edges]]
from = "signature"
to   = "capability-intersect"

[[graph.edges]]
from = "capability-intersect"
to   = "install"
```

### 9.2 Install Pipeline Cells

| Cell | Input | Output | Rejects When |
|---|---|---|---|
| `SemverResolveCell` | `ArtifactRef + VersionConstraint` | `ResolvedVersion + RegistryMetadata` | No matching version, yanked |
| `ArtifactDownloadCell` | `ResolvedVersion` | `ArtifactBundle (raw bytes)` | Download failure, CDN error |
| `ChecksumVerifyCell` | `ArtifactBundle` | `ArtifactBundle + ContentHash` | Checksum mismatch (tampered) |
| `SignatureVerifyCell` | `ArtifactBundle + ContentHash` | `ArtifactBundle + Signature` | Invalid publisher signature |
| `CapabilityIntersectCell` | `ArtifactBundle + Signature` | `ArtifactBundle + EffectiveCapabilities` | Capability gap not granted by user |
| `ArtifactInstallCell` | `ArtifactBundle + EffectiveCapabilities` | `InstalledArtifact` | Write failure, lockfile conflict |

### 9.3 CLI Surface

```
$ roko market install @wpank/doc-ingest@^1
[resolve]       @wpank/doc-ingest@^1 -> 1.0.0
[download]      Downloading bundle...  (124 KB)
[checksum]      blake3:abc... verified
[signature]     publisher @wpank verified
[capability]    Inspecting capabilities:
  FsRead    any path     -> granted by Space
  FsWrite   any path     -> granted by Space
  Llm                    -> granted by Space
  Net       api.perplexity.ai, arxiv.org -> Space grants Net.* (covered)
  All capabilities covered. Continue? [Y/n]
[install]       Installing Cell dependencies (8 Cells)...
Installed as @wpank/doc-ingest@1.0.0
  Run with: roko run @wpank/doc-ingest --input source_dir=...
```

If a capability is not covered, the install pauses for explicit grant:

```
This artifact requires:
  Shell  ["cargo", "git", "rustc"]
Space currently grants: Shell = false
Grant `Shell ["cargo", "git", "rustc"]` to this Space? (y/N/configure)
```

---

## 10. Versioning, Lockfile, Yanking, Deprecation

### Semver

All artifacts use semver. Pre-release supported. Marketplace surfaces highest stable version by default.

### Lockfile

`<workspace>/.roko/marketplace.lock` pins exact versions and checksums. Ensures reproducible installs.

### Yanking

Publisher can yank a version (broken / vulnerable). Yanked versions remain installable for pinned users but hidden from browse.

### Deprecation

```toml
[deprecation]
since       = "2026-06-01"
reason      = "Replaced by improved engine"
replacement = "@wpank/doc-ingest-v2@^1"
```

### Vulnerability disclosure

Published artifacts depending on vulnerable Cells receive automatic banners. Maintainers notified. Consumers see warnings.

---

## 11. Forking with Lineage Chain

### Local fork

```
$ roko market fork @wpank/doc-ingest@1.0.0 my-doc-ingest
Forking @wpank/doc-ingest@1.0.0...
  Local name: my-doc-ingest
  Lineage:    @wpank/doc-ingest@1.0.0
  Files written to: .roko/graphs/my-doc-ingest.toml
Forked. Edit with: roko graph edit my-doc-ingest
```

### Fork chain visualization

```
@alice/code-review@1.0.0
   -> @bob/strict-code-review@2.0.0      (changed: model=opus, strictness=high)
        -> @carol/security-review@1.5.0  (changed: focus_areas=security)
             -> @wpank/strict-security@2.1.0  (you are here)
```

Click any node to view that version. Diff between any two versions. On-chain lineage (ERC-8004) provides cryptographic proof.

### Composition by reference

For Knowledge Bundles and small artifacts, composition by reference is preferred over fork:

```toml
[[graph.knowledge_bundle]]
ref = "@nunchi/safety-knowledge@^1"
```

The bundle is referenced, not copied. Updates reach the consumer with compatibility checks.

---

## 12. Rating and Trust

### Bayesian quality signals

- **Install count** (lifetime, 30d, 7d)
- **Active runs** (last 30d) -- strongest quality signal (Splice retention pattern)
- **Fork count** -- a fork is a positive signal
- **Comment quality** (length, threaded depth)
- **Rating** -- thumbs-up / thumbs-down by verified installers only (must have run at least once)

Aggregate "would recommend" shown with N>=10 ratings; below that, raw counts only.

### Gate pass rates

For Graphs including Verify Cells, the marketplace tracks aggregate gate pass rates from community runs. A Graph at 95% surfaces higher than one at 60%.

### Community validation

Strong pass rates + active runs earn "Community Validated" status. Distinct from Verified Run (CI-level).

### On-chain reputation via ERC-8004

Publisher reputation reflected on-chain ([22-REGISTRIES](22-REGISTRIES.md)):
- TraceRank reputation from verified work.
- Fork-chain attestations showing provenance graph.
- ZK-attested gate pass rates for cryptographic quality verification.

### Editorial "Featured"

Weekly editorial pass selects up to 5 artifacts. Selection criteria (transparently documented): useful in real Graphs, well-documented, Verified Run badge, sensible capability disclosures.

### Creator analytics via UsageLens

| Metric | Description | Period |
|---|---|---|
| Installs | New installs | 7d, 30d, lifetime |
| Active runs | Distinct runs | 7d, 30d |
| Forks | New forks created | 7d, 30d, lifetime |
| Error rate | % of runs that failed | 7d, 30d |
| Avg cost | Mean USD cost per run | 30d |
| Avg duration | Mean wall-clock per run | 30d |
| Gate pass rate | Verify pass % | 30d |
| Retention | % of installers who ran more than once | 30d |
| Revenue | Creator revenue (take-rate applied) | 7d, 30d, lifetime |
| Upstream attribution | Revenue from downstream forks | 30d, lifetime |

---

## 13. Anti-Spam and Safety

- New-account publishing throttle: 1 publish/day for first 30 days; 5/day thereafter for unverified; 50/day for verified
- Static analysis on WASM Cells (banned imports, fuel limits, memory limits)
- LLM-based duplicate detection on prompts and configs; near-duplicates surface "similar to @x/y"
- One-click "Report" on any artifact; reports go to moderation
- Featured / unfeatured / suspended states transparently logged
- Marketplace Cells must be Composition or WASM tier (not Script unless verified publisher). Native Rust not directly publishable.

---

## 14. Capability Declarations (Three-Layer Intersection)

Capabilities are declared on Cells, granted at the Space, and intersected at runtime. See [16-SECURITY](16-SECURITY.md) SS2 for the full capability model.

### Layer 1: Cell declarations

```toml
[cell.capabilities]
required = [
  { "FsRead"  = { paths = ["docs/**", "src/**"] } },
  { "FsWrite" = { paths = [".roko/artifacts/**"] } },
  { "Shell"   = { commands = ["cargo", "rustc", "git"] } },
  { "Net"     = { domains = ["api.openai.com", "api.anthropic.com"] } },
]
```

### Layer 2: Graph allow-list

```toml
[graph.capabilities]
allowed = ["FsRead", "FsWrite", "Llm"]
```

### Layer 3: Space grants

```toml
[space.capabilities]
fs_read  = true
fs_write = true
net      = { domains = ["*"] }
llm      = true
shell    = false
```

### Three-layer intersection

```
Cell declaration (intersection) Graph allow-list (intersection) Space grant = effective capabilities
```

Missing at any layer = denied. The system fails closed.

---

## 15. Backend as Cell Specializations

The marketplace backend is not a bespoke service -- it is a composition of Cell specializations implementing standard protocols. Each backend component maps to a protocol from [02-CELL](02-CELL.md).

### 15.1 ArtifactStoreCell (Store protocol)

Persists and retrieves artifact bundles. Implements the Store protocol (put/get/query/query_similar/prune) with S3/Tigris as the backing medium and Postgres for metadata indexing.

```rust
/// Cell:     ArtifactStoreCell
/// Protocol: Store
/// Input:    Signal { kind: ArtifactBundle } -- the packaged artifact
/// Output:   Signal { kind: PublishedArtifact } -- stored artifact with CDN URL
pub struct ArtifactStoreCell {
    id: CellId,
    /// S3-compatible storage backend.
    storage: S3Client,
    /// Postgres metadata store.
    metadata: PgPool,
    /// CDN base URL for artifact serving.
    cdn_base: Url,
}

impl Cell for ArtifactStoreCell {
    fn id(&self) -> CellId { self.id }
    fn name(&self) -> &str { "artifact-store" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Store] }
    fn capabilities(&self) -> &Capabilities {
        Capabilities::from(&[Capability::Net, Capability::Store])
    }
}

impl Store for ArtifactStoreCell {
    type Key = ArtifactRef;
    type Value = ArtifactBundle;

    async fn put(&self, key: &ArtifactRef, value: &ArtifactBundle) -> Result<ContentHash> {
        let hash = blake3::hash(&value.bytes);
        self.storage.put_object(&key.s3_path(), &value.bytes).await?;
        self.metadata.insert_artifact(key, &hash, &value.manifest).await?;
        Ok(hash)
    }

    async fn get(&self, key: &ArtifactRef) -> Result<Option<ArtifactBundle>> {
        let bytes = self.storage.get_object(&key.s3_path()).await?;
        Ok(bytes.map(|b| ArtifactBundle::from_bytes(b)))
    }

    async fn query(&self, filter: &Query) -> Result<Vec<ArtifactRef>> {
        self.metadata.query_artifacts(filter).await
    }
}
```

### 15.2 MarketplaceTrendLens (Observe protocol)

Defined in SS8.3 above. A Lens Cell that chains from UsageLens and computes install velocity trends for the Trending tab. Consistent with the TrendLens pattern in [15-TELEMETRY](15-TELEMETRY.md) SS4.8.

### 15.3 VerifiedRunCell (Verify protocol)

Runs published fixtures via CI and emits Verified Run badges. Implements the Verify protocol.

```rust
/// Cell:     VerifiedRunCell
/// Protocol: Verify
/// Input:    Signal { kind: ArtifactBundle } -- artifact with fixtures
/// Output:   Signal { kind: Verdict } -- pass/fail with evidence
pub struct VerifiedRunCell {
    id: CellId,
    /// CI runner configuration.
    runner: CIRunnerConfig,
    /// Timeout for fixture runs.
    timeout: Duration,
}

impl Cell for VerifiedRunCell {
    fn id(&self) -> CellId { self.id }
    fn name(&self) -> &str { "verified-run" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Verify] }
    fn capabilities(&self) -> &Capabilities {
        Capabilities::from(&[Capability::Shell, Capability::Net])
    }
}

impl Verify for VerifiedRunCell {
    async fn verify_post(
        &self,
        artifact: &Signal,
        _output: &Signal,
    ) -> Result<Verdict> {
        let bundle = ArtifactBundle::from_signal(artifact)?;
        let fixtures = bundle.fixtures()?;
        let mut evidence = Vec::new();
        let mut all_passed = true;

        for fixture in &fixtures {
            let result = self.runner
                .run_fixture(&bundle, fixture, self.timeout)
                .await?;
            evidence.push(Evidence {
                kind: EvidenceKind::FixtureRun,
                data: serde_json::to_value(&result)?,
            });
            if !result.passed { all_passed = false; }
        }

        Ok(Verdict {
            passed: all_passed,
            reward: if all_passed { 1.0 } else { 0.0 },
            evidence,
            badge: if all_passed {
                Some(Badge::VerifiedRun)
            } else {
                None
            },
        })
    }
}
```

### 15.4 Backend Service Mapping

| Backend Component | Cell Specialization | Protocol | Backing Infrastructure |
|---|---|---|---|
| Artifact storage | `ArtifactStoreCell` | Store | S3 / Tigris + CDN |
| Metadata index | `ArtifactStoreCell` (query path) | Store (query) | Postgres |
| Trending algorithm | `MarketplaceTrendLens` | Observe (Lens) | In-memory (chained from UsageLens) |
| Verified Run CI | `VerifiedRunCell` | Verify | GitHub Actions runner |
| Install counts | `UsageLens` ([15-TELEMETRY](15-TELEMETRY.md)) | Observe (Lens) | Redis counters |
| On-chain attestation | `ERC8004AttestationCell` | Store + Verify | Chain RPC ([22-REGISTRIES](22-REGISTRIES.md)) |

Auth (GitHub OAuth + JWT) uses the auth Pipeline from [17-AUTH](17-AUTH.md). CDN (CloudFront / Bunny) is a deployment concern, not a Cell -- artifact bundles are served directly and checksums verified client-side. Mirroring uses the same ArtifactStoreCell with a different S3 endpoint for orgs (`@my-org/...` namespace on private endpoint).

---

## 16. CLI Surface

```
roko market browse [--query <q>] [--tag <t>] [--kind <k>] [--featured]
roko market show <ref>
roko market install <ref>
roko market uninstall <ref>
roko market upgrade [<ref>]                  # all installed if no ref
roko market list-installed
roko market fork <ref> [<new-name>]
roko market publish <local-name>
roko market unpublish <ref>                  # yank
roko market deprecate <ref> [--replacement <ref>]
roko market mirror <url>                     # alternate marketplace endpoint
roko market verify <ref>                     # checksum + signature check
roko market analytics <ref>                  # creator analytics
roko market revenue [--period 30d]           # creator revenue summary
```

---

## 17. Acceptance Criteria

| Criterion | Verification |
|---|---|
| `roko market publish` runs the publish Pipeline Graph (SS7): checksum, signature, capability, semver, schema, store | End-to-end test against staging |
| `roko market install` runs the install Pipeline Graph (SS9): resolve, download, checksum, signature, capability-intersect, install | Install + capability-gap test |
| Publish Pipeline rejects on checksum mismatch (ChecksumVerifyCell) | Negative test |
| Install Pipeline rejects on invalid signature (SignatureVerifyCell) | Negative test |
| ArtifactStoreCell implements Store protocol (put/get/query) | Store protocol conformance test |
| MarketplaceTrendLens computes correct install velocity slopes | TrendLens unit test |
| VerifiedRunCell runs fixtures and emits Verdict with badge | Verify protocol conformance test |
| WASM Cells execute under fuel + memory limits; over-limit Cells fail closed | Resource-limit test |
| Browse facets work: filter by kind, category, tag, capability, license, gate pass rate | Faceted query test |
| Verified Run CI re-runs fixtures and emits badge state | CI integration test |
| Fork chain renders correctly with all ancestors clickable | Visual + DB test |
| Lineage walk works across the marketplace API | API test |
| Yanked versions warn on install but install for already-pinned | Yank test |
| Anti-spam throttling: new account cannot publish > 1/day for first 30 days | Account-limit test |
| UsageLens metrics appear on creator analytics page within 1h of event | Metrics pipeline test |
| MarketplaceTrendLens drives "Trending" sort correctly | Trending algorithm test |
| 0% take-rate applied for creator revenue under $1M lifetime | Revenue calculation test |
| 12-15% take-rate applied above $1M threshold, non-retroactive | Revenue calculation test |
| All mandatory metrics published | Metrics completeness test |
| On-chain lineage attestation anchored in ERC-8004 for opted-in creators | Chain attestation test |
| Fork generates upstream reputation credit for original author | Attribution flow test |
| Criterion artifacts installable as Verify pipeline plugins | Round-trip: install, wire into Profile, run Verify |
| Profile artifacts apply as verification presets with adjustable Macros | Round-trip: install, adjust, run |
| Creator can export installer list with opt-in consent | Export + consent test |
| 11 artifact types each publish and install correctly | Per-type round-trip test |
| Preview ("Tinker Mode") runs against fixtures within 30s | Latency test |
| Three-layer capability intersection: Cell, Graph, Space | Intersection enforcement test |
| Tier 1 prompt loads and injects into system prompt context | Prompt injection test |
| Tier 2 config profile deep-merges over base profile | Profile merge test |
| Tier 3 declarative tool invokes subprocess with sandboxed capabilities | Tool sandbox test |
| Tier 4 WASM plugin loads, runs, sandboxed (no fs without capability) | WASM sandbox test |
| Tier 5 Rust Cell compiles and runs with full access | Built-in Cell test |
| All TOML examples use `[cell.impl]` not `[block.impl]` | Naming consistency check |
