# 16 — Domain Profiles

> Sub-doc 16 of **02-agents** · Roko Documentation
>
> This document defines the canonical domain profiles for Roko deployments:
> coding, research, blockchain, data/ML, ops/SRE, and writing. It explains
> how profiles compose roles, tools, gates, heuristics, and context into an
> installable bundle, and it introduces the shared `TypedContext` and
> `Custody` primitives needed by multiple domains.
>
> See also: `../../tmp/refinements/25-domain-specific-agents.md`,
> `../00-architecture/01-naming-and-glossary.md`,
> `12-extensibility.md`, `04-agent-roles.md`, and
> `../11-safety/02-audit-chain.md`.

> **Implementation**: Proposed

---

## Profile Framing

Roko is domain-agnostic at the kernel level, but deployments are not. Most
real uses want a domain-shaped bundle that ships with the right default roles,
tools, gates, heuristics, and prompt templates from day one.

A **domain profile** is that bundle. It is the installable unit that wraps the
lower-level extension points already described in `12-extensibility.md`:

- Tier 1: role prompts and task templates.
- Tier 2: profile metadata and defaults.
- Tier 3: tool registrations and handlers.
- Tier 4: native integrations or specialized execution paths.

The profile is the thing a team installs. The roles are the things the profile
uses.

---

## Shared Composition Rules

All profiles follow the same composition model:

1. Roles are selected first, then specialized by the profile's prompts and
   tool allowlists.
2. Tools merge by union when multiple profiles are installed.
3. Gates stack unless a gate is explicitly scoped to one profile.
4. Heuristics coexist; routing chooses the best fit for the current context.
5. Profile collisions should be explicit. If two profiles claim the same role
   name or tool id, the operator needs a visible resolution policy.
6. Context should be structured, not free-form. That is the job of
   `TypedContext`.

This makes profile composition additive instead of exclusive. A deployment can
start with a single profile and grow into multi-profile operation without
changing the kernel.

---

## Canonical Profiles

| Profile | Default roles | Core tools | Core gates | Memory shape |
|---|---|---|---|---|
| Coding | Researcher, Planner, Implementer, Reviewer, Tester | fs, git, language toolchains, code MCP | compile, unit, clippy, diff | episodes, playbooks, build history |
| Research | Researcher, Analyst, Explorer, Reviewer | web, PDF, citation manager, note tools | citation, factuality, novelty | paper claims, replication ledger |
| Blockchain | Architect, Implementer, Reviewer, Operator | RPC, signer, explorer, compiler, simulator | simulation, gas, invariant, approval | chain-of-custody, audit trail |
| Data/ML | Analyst, Implementer, Tester, Reviewer | SQL, notebooks, pandas/polars, profiling | schema, sample-check, metric regression | dataset fingerprints, lineage |
| Ops/SRE | Operator, Deployer, Monitor, Reviewer | kubectl, logs, metrics, runbooks, pager | dry-run, blast-radius, change-window | incident archive, runbook library |
| Writing | DocWriter, Researcher, Reviewer | corpus search, style guide, fact-check, citation tools | style, fact, tone, plagiarism | voice fingerprint, editorial archive |

These are the canonical starting points, not the only possible bundles. The
point is to make the first working path obvious for the six common domains.

### Coding

The coding profile is the default Roko shape today. It combines implementation
and review roles with build tooling and diff-oriented gates. It should feel
boringly reliable: fast iteration, strong test feedback, and small-gate
pressure on every turn.

The coding profile benefits from `TypedContext` keys such as `language`,
`repo_root`, `file_set`, and `last_gate`. That makes code-aware gates and
heuristics cheaper than parsing free-form task text.

### Research

The research profile is tuned for evidence collection, citation quality, and
claim tracking. It should prefer retrieval, note synthesis, and claim
verification over speculative writing.

Its `TypedContext` usually contains fields like `question`, `corpus`,
`source_ids`, and `claim_set`. A claim that cannot be tied back to a source
should remain provisional until the profile's citation gate resolves it.

### Blockchain

The blockchain profile is the highest-risk profile in the set. It needs typed
intent, a simulator, and custody records for every action that can touch funds
or consensus state.

This is the clearest case for both new primitives:

- `TypedContext` carries structured intent such as chain, wallet, target,
  amount, gas ceiling, and approval state.
- `Custody` records who authorized the action, what simulation was run, and
  what on-chain witness or receipt proved the outcome.

The chain-of-custody story is not optional here. It is the audit trail that
makes the profile safe enough to operate.

### Data / ML

The data/ML profile treats datasets and notebooks as first-class artifacts.
It should know how to inspect schema drift, sample slices, and metric deltas
before it recommends a change.

`TypedContext` is useful for dataset name, notebook path, pipeline stage,
target metric, and schema version. The profile can then gate on typed
conditions instead of brittle text matching.

### Ops / SRE

The ops/SRE profile prioritizes low-blast-radius action, dry-run discipline,
and explainable decision traces. It should default to observation and
advisory modes unless the operator explicitly allows execution.

`Custody` is useful here even when the action is not blockchain-related:
incident commands, deploys, and remediation steps should be traceable after
the fact. The profile can attach `why`, `who`, `when`, and `result` metadata
to every meaningful operation.

### Writing

The writing profile focuses on style, factuality, and editorial voice. It is
less about tool breadth and more about consistent output quality.

Its `TypedContext` should capture target audience, publication type, tone,
source set, and voice target. The profile's fingerprint-based checks help keep
drafts close to the author's style instead of drifting into generic prose.

---

## TypedContext

`TypedContext` is the structured situation record that domain profiles share.
It replaces ad hoc free-text task summaries whenever a domain needs reliable
matching on situation shape.

```rust
pub struct TypedContext {
    pub domain: Domain,
    pub fields: BTreeMap<ContextKey, ContextValue>,
}

pub enum ContextValue {
    String(String),
    Int(i64),
    Float(f64),
    Hash(EngramHash),
    Fingerprint(HdcVector),
    List(Vec<ContextValue>),
    Nested(BTreeMap<ContextKey, ContextValue>),
}
```

The important property is not the exact shape above; it is that the profile
can declare and validate keys instead of inferring everything from prose.
That gives gates and heuristics a stable contract across domains.

Typical keys:

- Coding: `language`, `repo_root`, `file_set`, `last_gate`
- Research: `question`, `source_ids`, `claim_set`, `corpus`
- Blockchain: `chain`, `wallet`, `intent`, `simulation`
- Data/ML: `dataset`, `notebook`, `metric`, `schema_version`
- Ops/SRE: `service`, `incident_id`, `change_window`, `blast_radius`
- Writing: `audience`, `tone`, `source_set`, `voice_target`

---

## Custody

`Custody` is the chain-of-custody record that attaches accountability to
profile actions.

```rust
pub struct Custody {
    pub action: ActionHash,
    pub who: PrincipalId,
    pub when: Timestamp,
    pub why: Vec<HeuristicId>,
    pub how: Vec<ClaimId>,
    pub approved_by: Option<PrincipalId>,
    pub simulation: Option<SimulationHash>,
    pub result: Option<ResultHash>,
    pub witness: Option<ChainWitness>,
}
```

Every profile can use custody records, but the need is strongest where actions
have external consequences:

- Blockchain needs it for transaction approval and witness receipts.
- Ops/SRE needs it for deploys, rollbacks, and incident remediation.
- Data/ML needs it for lineage and reproducibility.
- Writing can use it for editorial review and source provenance.

---

## Evaluation Suites

Each profile should ship with a benchmark suite so the profile can be measured
as a bundle, not just as a set of unrelated tools.

- Coding: bug-fix tasks with frozen SHAs and test outcomes.
- Research: claim-to-source matching and follow-up paper detection.
- Blockchain: vulnerable-contract detection and false-positive tracking.
- Data/ML: dirty-dataset diagnosis and metric-regression handling.
- Ops/SRE: simulated incidents and time-to-correct-diagnosis.
- Writing: style-fidelity checks against a known author corpus.

The bundle should report results into the replication and learning layers so
profile quality improves over time rather than staying static.

---

## Profile Installation

Profiles are installable bundles, not ad hoc configuration fragments. A
deployment should be able to name the profile it wants and get a coherent
default stack in return.

```toml
[profile.coding]
roles = ["researcher", "planner", "implementer", "reviewer"]
tools = ["fs.read", "fs.write", "git.status", "cargo.build", "cargo.test"]
gates = ["unit", "type", "style", "diff"]
heuristics = "@roko/coding-heuristics-starter"
templates = "@roko/coding-templates"

[profile.research]
roles = ["researcher", "analyst", "explorer", "reviewer"]
tools = ["web.search", "pdf.extract", "citation.lookup"]
gates = ["citation", "factuality", "novelty"]
heuristics = "@roko/research-heuristics-starter"
templates = "@roko/research-templates"
```

The exact package format can evolve, but the contract should remain stable:
install a profile, get a domain-shaped agent stack.

---

## Cross-Links

- Domain bundle architecture: `12-extensibility.md`
- Role defaults and composition: `04-agent-roles.md`
- Safety and custody: `../11-safety/02-audit-chain.md`
- The source refinement: `../../tmp/refinements/25-domain-specific-agents.md`
