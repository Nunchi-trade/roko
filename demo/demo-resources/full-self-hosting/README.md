# Full Self-Hosting Demo

The complete roko loop: idea → research → draft → plan → agents → gates → learn.

## The loop

```
1. Idea      →  roko prd idea "..."
2. Research  →  roko research topic "..."
3. Draft     →  roko prd draft new <slug>
4. Publish   →  roko prd draft promote <slug>
5. Plan      →  roko prd plan <slug>
6. Match     →  roko job match "..." --skills X
7. Post      →  roko job create "..." --type coding_task
8. Execute   →  roko plan run plans/
9. Learn     →  roko learn all
10. Iterate  →  (gate failures trigger replan)
```

## Dashboard walkthrough

This is the suggested demo order for showing the full self-hosting capability:

### Act 1: Capture (Atelier → Chat)
```
/idea Wire knowledge query into agent matchmaking
/idea Add cold storage archival on schedule
/idea Dashboard UI for agent creation
```

### Act 2: Research (Atelier → Research)
```
/research agent matchmaking algorithms in decentralized systems
```
Watch the research panel progress through stages.

### Act 3: Plan (Atelier → PRDs → Plans)
```
/draft wire-knowledge-matchmaking
/publish wire-knowledge-matchmaking
/plan wire-knowledge-matchmaking
```
Switch to Plans tab — see the generated task tree.

### Act 4: Match & Execute (Atelier → Chat + Coding)
```
/coding implement knowledge-weighted matchmaking
```
Accept the agent quote. Job appears in Coding tab.

### Act 5: Observe (Network tabs)
- **Agents** — Fleet roster with tiers
- **Jobs** — Active job board
- **Learning** — C-Factor, efficiency, cascade router
- **Swarm** — Network topology (if agents have heartbeats)

## Script

`demo-full-loop.sh` runs the entire flow interactively with pauses.

## What generates learning data

Each step produces data the Learning tab consumes:

| Step | Learning artifact |
|------|-------------------|
| Plan execution | Episodes in `.roko/episodes.jsonl` |
| Gate checks | Adaptive thresholds in `.roko/learn/gate-thresholds.json` |
| Model routing | Cascade router state in `.roko/learn/cascade-router.json` |
| Agent dispatch | Efficiency events in `.roko/learn/efficiency.jsonl` |
| A/B experiments | Experiment store in `.roko/learn/experiments.json` |
