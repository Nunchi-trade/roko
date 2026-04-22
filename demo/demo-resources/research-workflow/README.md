# Research Workflow Demo

Demonstrate research dispatch, PRD enhancement, and the research panel.

## CLI commands

```bash
# Direct topic research (writes to .roko/research/)
roko research topic "MEV landscape on Ethereum L2s"

# Deep research (async, 1-10 min, more thorough)
roko research topic "MEV landscape on Ethereum L2s" --deep

# Direct search (fast, structured results)
roko research search "libp2p relay implementation patterns"

# Enhance a PRD with research citations
roko research enhance-prd wire-knowledge-matchmaking

# Optimize a plan with research
roko research enhance-plan plans/wire-knowledge-matchmaking

# Analyze execution episodes for insights
roko research analyze
```

## Dashboard flow

1. Open **Atelier → Chat**
2. Type `/research MEV landscape on Ethereum L2s`
3. Watch the **Research bounty** tab — stages progress: dispatching → gathering → analyzing → synthesizing → complete
4. Results show sources with relevance scores, findings with confidence levels, gaps, and follow-ups

## HTTP API

```bash
# Dispatch research
curl -X POST http://localhost:6677/api/research/topic \
  -H 'Content-Type: application/json' \
  -d '{"topic":"MEV landscape on Ethereum L2s","depth":"deep"}'

# List research artifacts
curl http://localhost:6677/api/research

# Enhance a PRD
curl -X POST http://localhost:6677/api/research/enhance-prd/wire-knowledge-matchmaking

# Analyze episodes
curl -X POST http://localhost:6677/api/research/analyze
```

## What gets created

```
.roko/research/
├── mev-landscape-on-ethereum-l2s.md    # Research output
└── ...
```

## Dashboard tabs that light up

- **Atelier → Research bounty** — Live research session with stages
- **Atelier → PRDs** — Enhanced PRD shows citations
- **Network → Learning** — Research efficiency recorded
