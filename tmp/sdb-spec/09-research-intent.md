# Checklist: Add intent field to research submission

## Implementation note (2026-04-15)

- `POST /api/research/topic` now accepts and validates `intent`, defaults it to `explore`, and echoes it in the accepted response payload
- The selected intent is embedded in both the spawned operation kind and the runtime prompt so downstream report structure can follow the chosen mode

**Priority**: P2 — improves research output actionability
**Estimated LOC**: ~20 lines
**Source**: `workspace/sdb/prds/research-prd.md`, `workspace/sdb/prds/product-design-review.md`, [GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45)

## Problem

Research reports are structured like internal docs, not like something a trader or risk manager uses. A trader wants a position recommendation; a risk manager wants an action checklist. Decision [firm] from Research PRD: add intent selector to research submission with 5 output formats.

## What already exists

`crates/roko-serve/src/routes/research.rs`:
- `POST /api/research/topic` — accepts `{ topic: string }`, spawns research
- `GET /api/research` — list research artifacts
- `POST /api/research/analyze` — analyze content

## Files to modify

### 1. `crates/roko-serve/src/routes/research.rs`

- [ ] Extend the `ResearchTopicRequest` struct (find the existing deserialization struct) to add:
```rust
/// Research intent — determines output format.
/// One of: "position", "evaluate", "monitor", "explore", "audit"
#[serde(default = "default_intent")]
pub intent: String,

fn default_intent() -> String { "explore".to_string() }
```

- [ ] Pass `intent` through to the runtime's research handler so the agent synthesizer uses it to adjust the final report structure

- [ ] Validate intent is one of the 5 allowed values, return 400 if not:
```rust
const VALID_INTENTS: &[&str] = &["position", "evaluate", "monitor", "explore", "audit"];
if !VALID_INTENTS.contains(&req.intent.as_str()) {
    return Err(ApiError::bad_request(format!("invalid intent: '{}'. Must be one of: {:?}", req.intent, VALID_INTENTS)));
}
```

## Intent-based output tailoring

| Intent | Report ends with |
|--------|-----------------|
| `position` | Directional recommendation + confidence + key risk |
| `evaluate` | Risk scores + red flags + comparison to alternatives |
| `monitor` | Timeline of changes + impact assessment + alerts to set |
| `explore` | Landscape map + key players + knowledge gaps |
| `audit` | Checklist of verified claims + unverified gaps + severity |

The agent synthesizer (in roko-core or the runtime) should receive the intent and use it to select a report template for the final section.

## Request shape

### `POST /api/research/topic` (extended)
```json
{
  "topic": "AAVE USDC supply rate risk",
  "intent": "position"
}
```

## Testing

- [ ] `POST /api/research/topic` with valid intent → accepted
- [ ] `POST /api/research/topic` with no intent → defaults to "explore"
- [ ] `POST /api/research/topic` with invalid intent → returns 400
