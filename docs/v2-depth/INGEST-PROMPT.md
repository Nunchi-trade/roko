# Depth Ingestion Prompt

Copy everything below the `---` line. Replace `[SOURCE]` with the folder or file path.

**Important**: This produces **depth docs** (algorithms, research, implementation detail). It does NOT modify the spec layer (`docs/v2/`).

---

## Instructions

Read the unified spec vocabulary first:

- `/Users/will/dev/nunchi/roko/roko/docs/v2/00-INDEX.md` (vocabulary, principles, concept migration table)
- `/Users/will/dev/nunchi/roko/roko/docs/v2/04-EXECUTION.md` (execution model)

Then read the guide and master index for the depth layer:

- `/Users/will/dev/nunchi/roko/roko/docs/v2-depth/GUIDE.md` (structure, conventions, vocabulary quick reference)
- `/Users/will/dev/nunchi/roko/roko/docs/v2-depth/INDEX.md` (mapping table — find which depth directory this content belongs to)

Now read all files in: `[SOURCE]`

For each depth directory the source content maps to, also read that directory's INDEX.md to see what's already there.

## Your task

**Do not copy, translate, or summarize the source material.** Instead, use it as raw intellectual input to produce something fundamentally better. You are redesigning these ideas from the ground up within the unified primitive model (Signal, Block, Graph + 9 protocols + 10 specializations).

For each depth doc you write:

### 1. Redesign, don't transcribe

Take the core insight from the source — the *why* and the *what problem it solves* — and re-derive the solution using the unified primitives. The new version should feel native to the Signal/Block/Graph model, not retrofitted. If the original was a bespoke mechanism, express it as a composition of protocols. If it was a standalone system, show how it emerges from the interaction of existing primitives.

### 2. Improve and extend

For every mechanism you redesign, ask:
- What would this look like if it were 10x better?
- What adjacent capability does this unlock that the original didn't see?
- What happens when this composes with other systems in ways the original author couldn't anticipate?
- Where does the original design break under scale, adversarial conditions, or novel domains?
- What's the version of this that makes competitors irrelevant rather than just catching up?

### 3. Innovate beyond the source

Generate ideas that don't exist in the source material at all:
- **Novel primitives**: New specialization patterns, new Lens types, new Loop configurations that the source material never considered.
- **Emergent capabilities**: Things that become possible *only* because of the unified model — cross-system compositions that couldn't exist when these were separate specs.
- **Cybernetic loops**: Every system should have a feedback path. If the source describes a static mechanism, add the loop that makes it self-improving. What observes it (Lens)? What tunes it (Loop 1 parameters)? What routes alternatives (Loop 2 strategy)? What consolidates its learnings (Loop 3 knowledge)? What proposes structural changes to it (Loop 4)?
- **Exponential scaling**: Design for network effects. What happens when 10 agents use this? 1,000? 1,000,000? Where are the superlinear returns? Where does shared knowledge compound? Where does stigmergic coordination create emergent order?
- **Unique differentiators**: What would make someone say "only Roko can do this"? HDC-native operations that float-embedding systems can't do. Decay-aware knowledge that static RAG can't match. Compositional Graphs that hardcoded pipelines can't express. Stigmergic coordination that message-passing systems can't achieve.

### 4. Find gaps and contradictions

- Where does the source material assume something that the unified model handles differently?
- Where are there missing feedback loops (something produces output but nothing observes or learns from it)?
- Where are there missing failure modes (what happens when this breaks, and who notices)?
- Where does the source over-engineer (mechanism isn't worth its complexity) or under-engineer (critical path has no redundancy)?

### 5. Combine across boundaries

The biggest wins come from combining ideas that were in separate specs:
- Daimon affect + Cascade routing = emotionally-aware model selection
- Dream consolidation + Gate feedback = self-improving verification
- Pheromone stigmergy + Marketplace reputation = emergent quality signals
- Conductor watchers + Lens telemetry = unified observability
- HDC fingerprints + Technical analysis = cross-domain pattern transfer

Look for these cross-boundary compositions. They are the novel capabilities that justify the unified model.

## Output format

Write depth docs to the appropriate directories under `/Users/will/dev/nunchi/roko/roko/docs/v2-depth/`. Each doc should:

1. Start with: `# [Title]` and `> Depth for [spec-file]. [One sentence on what this adds.]`
2. Use unified vocabulary exclusively (see GUIDE.md vocabulary table)
3. Reference spec files for type/protocol definitions: "See [02-CELL.md](../v2/02-CELL.md) §3.5"
4. Include concrete Rust-flavored pseudocode or TOML config where it makes the design tangible
5. End with a "What This Enables" section listing capabilities that didn't exist before
6. End with a "Feedback Loops" section showing how this system observes, learns, and improves itself
7. End with an "Open Questions" section for genuine unknowns worth investigating

After writing all depth docs, update each directory's INDEX.md:
- Move ingested source docs from "Pending" to "Absorbed" status
- Add new depth docs to the "Depth docs" section with one-line descriptions

## What NOT to do

- Don't preserve structure or section headings from the source — redesign from scratch
- Don't keep ideas that are weaker than what the unified model enables — replace them
- Don't write depth docs that merely restate the spec layer — add algorithmic detail, novel ideas, or cross-system compositions that the spec doesn't cover
- Don't add filler or padding — every paragraph should contain a decision, algorithm, insight, or novel idea
- Don't be conservative — the source material is a floor, not a ceiling
