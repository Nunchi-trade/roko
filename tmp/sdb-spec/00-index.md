# SDB Spec Implementation Checklists

All changes derived from Sam's dashboard integration specs ([GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45)) and 6 product PRDs at `collaboration/workspace/sdb/prds/`.

Full review/response doc: `collaboration/tmp/sdb-specs-review-responses.md`

## Build Order

### This week (demo-critical)

| # | Checklist | Target | Est. LOC | Priority |
|---|-----------|--------|----------|----------|
| 01 | [Agent owner field](01-agent-owner-field.md) | mirage-rs | ~40 | P0 |
| 02 | [Agent skills endpoints](02-agent-skills-endpoints.md) | mirage-rs | ~150 | P0 |
| 03 | [C-Factor + frequency + cost tiers](03-cfactor-endpoint.md) | roko-serve + mirage-rs | ~80 | P1 |
| 04 | [Task artifacts](04-task-artifacts.md) | mirage-rs | ~100 | P1 |
| 05 | [Agent messaging](05-agent-messaging.md) | roko-serve + mirage-rs | ~150 | P0 |
| 06 | [ISFR proxy](06-isfr-proxy.md) | mirage-rs | ~40 | P1 |

### Next week (pre-demo)

| # | Checklist | Target | Est. LOC | Priority |
|---|-----------|--------|----------|----------|
| 07 | [Prediction endpoints](07-prediction-endpoints.md) | mirage-rs | ~400 | P1 |
| 08 | [roko chat CLI](08-roko-chat-cli.md) | roko-cli | ~80 | P1 |
| 09 | [Research intent](09-research-intent.md) | roko-serve | ~20 | P2 |
| 10 | [Task improve/feedback](10-task-improve-feedback.md) | mirage-rs | ~50 | P2 |

**Total**: ~1,110 lines of Rust across mirage-rs, roko-serve, and roko-cli.

## Dependency graph

```
01 (owner field)  ──→  05 (messaging) ──→ 08 (chat CLI)
02 (skills)       ──→  (standalone)
03 (cfactor)      ──→  (standalone)
04 (artifacts)    ──→  10 (improve/feedback)
06 (isfr proxy)   ──→  (standalone)
07 (predictions)  ──→  (standalone, but uses patterns from roko-learn CalibrationTracker)
09 (research)     ──→  (standalone)
```

Items 01, 02, 03, 04, 06, 07, 09 can all be built in parallel.
Items 05 depends on 01 (agent owner for filtering).
Item 08 depends on 05 (messaging pipeline).
Item 10 depends on 04 (task artifacts exist).

## Source specs (in collaboration repo)

| Spec | Path |
|------|------|
| Agent Skills Config | `workspace/sdb/agent-skills-config-spec.md` |
| Mirofish Implementation | `workspace/sdb/mirofish-implementation-spec.md` |
| Agent Messaging Architecture | `workspace/sdb/agent-messaging-architecture.md` |
| Unwired API Functions | `workspace/sdb/unwired-api-functions-spec.md` |
| Job Deliverables | `workspace/sdb/job-deliverables-spec.md` |
| Yield Perps Dashboard Integration | `workspace/sdb/yield-perps-dashboard-integration.md` |
| Mock Data Audit | `workspace/sdb/mock-data-audit.md` |
| Ask PRD | `workspace/sdb/prds/ask-prd.md` |
| Predictions PRD | `workspace/sdb/prds/predictions-prd.md` |
| Research PRD | `workspace/sdb/prds/research-prd.md` |
| Jobs PRD | `workspace/sdb/prds/jobs-prd.md` |
| Streams PRD | `workspace/sdb/prds/streams-prd.md` |
| Data PRD | `workspace/sdb/prds/data-prd.md` |
| Product Design Review | `workspace/sdb/prds/product-design-review.md` |

## Post-demo items (not checklisted yet)

- Stream CRUD infrastructure (`/api/streams`)
- Data provider registry (`/api/data/feeds`) + compute bond contract
- Chat persistence (SQLite store on roko-serve)
- Multi-agent conversations
- IPFS artifact storage
- Valhalla privacy tier enforcement
- Full Mirofish dispatch integration (roko-serve → agents)
- Stream composition (Composer trait extension)
- Subscription tier enforcement (Privy + points)
