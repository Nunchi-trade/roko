# Unified Depth — How to Use and Extend

## What this is

This directory holds the **depth layer** for the unified spec. The spec layer lives in `docs/v2/` (28 flat files defining vocabulary, protocols, and contracts). This depth layer holds the algorithms, theoretical grounding, implementation detail, research backing, domain-specific knowledge, and novel capabilities that the spec layer references but doesn't contain.

## The two layers — what goes where

| | Spec Layer (`docs/v2/`) | Depth Layer (`docs/v2-depth/`) |
|---|---|---|
| **Purpose** | Define the vocabulary, protocols, contracts, composition rules | Algorithms, research, implementation detail, domain patterns |
| **Audience** | Developer learning the system | Developer implementing a specific subsystem |
| **Says** | *What* and *why* | *How*, *where it came from*, and *what's possible* |
| **Style** | Concise, authoritative, stable | Thorough, exploratory, evolving |
| **Example** | "The Verify protocol checks Signals against truth → Verdict" | "CaMeL achieves 77% solve rate with provable IFC via dual-LLM..." |
| **Example** | "Loop 4 evolves Blocks/Graphs via CMP scoring" | "HGM's Clade-Metaproductivity scores by descendant fitness, achieving SWE-bench 50%..." |
| **Example** | "HDC vectors enable 1μs similarity search" | "Resonator Networks factor composite vectors at 10× codebook lookup speed..." |
| **Changes** | Rarely, surgically (use `UPDATE-PROMPT.md`) | Frequently, by ingestion (use `INGEST-PROMPT.md`) |

**Rule of thumb**: If it has an arXiv citation, a specific benchmark number, a code implementation detail, or a domain-specific algorithm — it goes in depth, not spec.

## Source documents for ingestion

Content flows into depth from four source pools, in priority order:

| Priority | Source | Path | What's there |
|---|---|---|---|
| 1 | Research docs | `docs/v2-depth/RESEARCH-PROMPT*.md` | 160+ papers, algorithms, compounds, threat models |
| 2 | DeFi gap analysis | `tmp/defi/gap/` (14 files, 574KB) | DeFi-specific requirements, real-time patterns, safety constraints |
| 3 | Implementation specs | `tmp/04-21-26/` (20+ files) | HDC integration, knowledge publishing, arenas, architecture |
| 4 | Core docs (v1) | `docs/v1/` (417 files) | Per-system deep specs with algorithms and theory |

## Structure

```
docs/v2/                     ← Spec layer (28 files, flat, browsable)
  00-INDEX.md                  The vocabulary and reading order
  01-SIGNAL.md                 Signal as universal datum
  02-CELL.md                   Cell + 9 protocols
  ...

docs/v2-depth/               ← Depth layer (this directory)
  GUIDE.md                     This file
  INDEX.md                     Master index with mapping from docs/v1/
  00-index/                    Depth for 00-INDEX (vision, principles, naming)
  01-signal/                   Depth for 01-SIGNAL (engram internals, decay math)
  02-block/                    Depth for 02-BLOCK (composition, tools, verification)
  ...
```

Each numbered directory corresponds 1:1 to its spec file. Depth docs inside use unified vocabulary (Signal, Block, Graph, etc.) even when their source material used old terms.

## How to add content

### Step 1: Identify the source

Source material lives in three places:

| Source | Path | What's there |
|---|---|---|
| Core docs (v1) | `docs/v1/` (417 files) | Deep per-system specs with algorithms and theory |
| Workflow PRDs | `tmp/workflow/` (12 files) | Workflow subsystem PRDs (largely absorbed by spec layer) |
| Architecture specs | `tmp/architecture/` (21 files) | Architecture specs (largely absorbed by spec layer) |

### Step 2: Find the right depth directory

Use the mapping table in `INDEX.md` to find which depth directory a source doc belongs to. The table maps every `docs/` section to its unified parent.

### Step 3: Feed to Claude

Give Claude:
1. The **spec file** from `docs/v2/` (so it knows the vocabulary)
2. The **source file(s)** from `docs/v1/` (the content to assimilate)
3. The **depth directory INDEX.md** (so it knows what's already there)

Prompt template:

```
Read the spec file at docs/v2/05-AGENT.md for vocabulary context.
Read the depth index at docs/v2-depth/07-agent-runtime/INDEX.md for existing coverage.
Now read [source file(s)].

Write a depth doc at docs/v2-depth/07-agent-runtime/[topic].md that:
- Uses unified vocabulary throughout (Signal not Engram, Block not Module, etc.)
- Preserves all algorithms, formulas, thresholds, and implementation detail
- References the spec file for protocol/type definitions instead of re-defining them
- Starts with a one-line summary of what this doc adds beyond the spec
```

### Feeding a whole folder

For an entire `docs/v1/` section (e.g., `docs/v1/09-daimon/`), feed all files at once:

```
Read docs/v2/05-AGENT.md for vocabulary.
Read docs/v2-depth/07-agent-runtime/INDEX.md for existing coverage.
Now read all files in docs/v1/09-daimon/.

Produce depth docs in docs/v2-depth/07-agent-runtime/ covering the daimon/affect
system. Split into logical docs (e.g., pad-vectors.md, somatic-markers.md,
behavioral-states.md). Use unified vocabulary throughout.
Update the INDEX.md with the new docs.
```

### Feeding file by file

For individual files:

```
Read docs/v2/06-MEMORY.md for vocabulary.
Read docs/v1/06-neuro/04-hdc-vsa-foundations.md.

Write docs/v2-depth/11-memory/hdc-vsa-foundations.md using unified vocabulary.
Update the INDEX.md.
```

## Vocabulary quick reference

When writing depth docs, use these terms:

| Use this | Not this | Context |
|---|---|---|
| Signal | Engram, Artifact, Knowledge Entry, Pheromone | The universal datum |
| Block | Module, Recipe stage | The universal computation |
| Graph | Workflow, StateGraph, Recipe pipeline | The universal composition |
| Flow | Workflow execution, Run | Graph at runtime |
| Rack | Parameterized Workflow | Graph + Macros + Slots |
| Lens | Monitor, Watcher, Probe | Observe-protocol Block |
| Loop | Feedback cycle, DreamCycle | Graph with feedback edge |
| Memory | Knowledge store, Grimoire | Store-protocol Block + decay |
| Space | Workspace, Environment | Isolation boundary |
| Store protocol | Substrate trait | put/get/query/prune |
| Score protocol | Scorer trait | Rate along dimensions |
| Verify protocol | Gate trait | Check → Verdict |
| Route protocol | Router trait | Select among candidates |
| Compose protocol | Composer trait | Combine under budget |
| React protocol | Policy trait | Watch stream, emit |
| Observe protocol | (new) | Read-only observation |
| Connect protocol | Connector trait | External I/O |
| Trigger protocol | (new) | Event → fire Graph |

## What NOT to put here

- Don't duplicate spec-layer content. If the spec file already defines a type or protocol, reference it: "See [02-CELL.md](../v2/02-CELL.md) §3.5 for the Compose protocol definition."
- Don't add depth that contradicts the spec. If you find a conflict, update the spec first.
- Don't add docs that are purely historical or superseded. Only living knowledge.

## Codebase Audit

Depth docs aren't just theoretical — they connect to the actual codebase. When writing depth docs, the agent also audits the relevant crates and identifies:

| Gap type | What it means | Example |
|---|---|---|
| **Dead code** | Built but not connected to runtime | `CascadeRouter::load()` exists but never called from dispatch |
| **Missing impl** | Spec says X, code doesn't have it | Spec defines Pulse struct, no Rust type exists |
| **Rename needed** | Old name still used | `Engram` used instead of `Signal` |
| **Test gap** | Changed code path has no test | New wiring added but no integration test |
| **Wire needed** | Code exists in wrong place | Function in library crate, not called from CLI |

Each gap becomes a migration batch in the runner. The depth doc's "Open Questions" section should note any gaps that aren't straightforward to fix.

### Depth → Migration Pipeline

```
docs/v1/[folder]
  ↓ (INGEST-PROMPT.md)
docs/v2-depth/[dir]/[topic].md     ← depth doc
```

## Conventions

- File names: `kebab-case.md`
- Each depth doc starts with: `# [Title]` + `> Depth for [spec-file]. Covers [topic].`
- Each depth directory has an `INDEX.md` listing its docs and source mapping
- Cross-references use relative paths: `../v2/05-AGENT.md`
Be sure to add all releevant context to context files, as well as prompts and files, write things from the persepctive of someone looking at them for the first time wihtout any prior context. 
