# PRD Workflow Demo

Demonstrate the full self-hosting loop: idea → draft → publish → plan → execute.

## The flow

```
/idea "Wire knowledge query into matchmaking"
    ↓
/draft wire-knowledge-matchmaking
    ↓
/publish wire-knowledge-matchmaking
    ↓
/plan wire-knowledge-matchmaking
    ↓
/run <plan-id>
```

## Scripts

- `demo-prd-cli.sh` — CLI-only flow (no dashboard needed)
- `demo-prd-api.sh` — HTTP API flow (drives the dashboard)

## CLI flow

```bash
# Capture an idea
roko prd idea "Wire knowledge query into matchmaking scoring"

# List ideas
roko prd list

# Draft a PRD from the idea (agent-assisted)
roko prd draft new "wire-knowledge-matchmaking"

# Promote to published
roko prd draft promote wire-knowledge-matchmaking

# Generate implementation plan
roko prd plan wire-knowledge-matchmaking

# Execute the plan
roko plan run plans/
```

## Dashboard flow

1. Open **Atelier → PRDs** tab
2. Type idea in the quick-input field, or use `/idea` in chat
3. Click the idea to draft it, or use `/draft <slug>`
4. Publish with `/publish <slug>`
5. Generate plan with `/plan <slug>`
6. Execute with `/run <plan-id>` or click Execute on the Plans tab

## What gets created

```
.roko/
├── prd/
│   ├── ideas.md                           # Quick captures
│   ├── drafts/
│   │   └── wire-knowledge-matchmaking.md  # Work in progress
│   └── published/
│       └── wire-knowledge-matchmaking.md  # Finalized
├── plans/
│   └── wire-knowledge-matchmaking/
│       └── tasks.toml                     # Generated tasks
└── episodes.jsonl                         # Agent turns recorded
```

## Dashboard tabs that light up

- **PRDs** — Shows idea → draft → published progression
- **Plans** — Shows generated plan with task tree
- **Learning** — Records efficiency events, gate results
