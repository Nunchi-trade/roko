# Checklist: Proxy ISFR service endpoints through mirage-rs

## Implementation note (2026-04-15)

- Implemented in `apps/mirage-rs/src/http_api/isfr.rs` and wired in `http_api/mod.rs`
- Both proxy endpoints forward query params and return `502` on transport failure, upstream non-success, or invalid JSON
- The `ISFR_score` rename follow-up was not needed in this worktree; there are no live `ISFR_score` / `isfr_score` code references to rename

**Priority**: P1 — feeds ISFR card, sidebar sparkline, network stats
**Estimated LOC**: ~40 lines
**Source**: `workspace/sdb/yield-perps-dashboard-integration.md`, [GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45)

## Problem

Dashboard ISFR card, sidebar sparkline, and network stats all show hardcoded "6.40%". The `isfr-service` provides real ISFR data via `GET /v1/isfr/current` and `GET /v1/isfr/history`, but the dashboard talks to mirage-rs. Need a proxy so the dashboard has one API surface.

## Files to modify

### 1. New file: `apps/mirage-rs/src/http_api/isfr.rs`

- [ ] Create this file with 2 proxy handlers:

```rust
use axum::{Json, extract::State};
use serde_json::Value;
use super::{ApiError, ApiState};

const ISFR_SERVICE_URL: &str = "http://localhost:8546"; // configurable via env

/// `GET /api/isfr/current` — proxy to isfr-service for latest rate.
pub async fn isfr_current(State(_state): State<ApiState>) -> Result<Json<Value>, ApiError> {
    let url = format!("{}/v1/isfr/current",
        std::env::var("ISFR_SERVICE_URL").unwrap_or_else(|_| ISFR_SERVICE_URL.to_string()));
    let resp = reqwest::get(&url).await
        .map_err(|e| ApiError { error: format!("isfr-service unavailable: {e}"), code: 502 })?;
    let body: Value = resp.json().await
        .map_err(|e| ApiError { error: format!("isfr-service bad response: {e}"), code: 502 })?;
    Ok(Json(body))
}

/// `GET /api/isfr/history` — proxy to isfr-service for historical rates.
pub async fn isfr_history(State(_state): State<ApiState>) -> Result<Json<Value>, ApiError> {
    let url = format!("{}/v1/isfr/history",
        std::env::var("ISFR_SERVICE_URL").unwrap_or_else(|_| ISFR_SERVICE_URL.to_string()));
    let resp = reqwest::get(&url).await
        .map_err(|e| ApiError { error: format!("isfr-service unavailable: {e}"), code: 502 })?;
    let body: Value = resp.json().await
        .map_err(|e| ApiError { error: format!("isfr-service bad response: {e}"), code: 502 })?;
    Ok(Json(body))
}
```

**Note**: If `reqwest` is not already a dependency of `mirage-rs`, add it to `Cargo.toml`. Check existing dependencies first — it may already be there.

### 2. `apps/mirage-rs/src/http_api/mod.rs`

- [ ] Add `mod isfr;` to module declarations
- [ ] Add routes (after task routes, before combined stats):
```rust
.route("/isfr/current", get(isfr::isfr_current))
.route("/isfr/history", get(isfr::isfr_history))
```

## Also: rename ISFR_score → HealthScore

Sam flagged a naming collision in `workspace/sdb/yield-perps-dashboard-integration.md`: the TEE clearing code has an internal per-agent risk metric called `ISFR_score` which is NOT the ISFR index.

- [ ] Search roko codebase for `ISFR_score` or `isfr_score` and rename to `HealthScore` / `health_score` to avoid confusion

## Response shapes

### `GET /api/isfr/current`
```json
{
  "rate": 6.40,
  "sources": {
    "aave": 6.12,
    "compound": 5.89,
    "hyperliquid": 7.24,
    "ethena": 6.35
  },
  "method": "weighted_median",
  "timestamp": 1713100800
}
```

### `GET /api/isfr/history`
```json
{
  "rates": [
    { "rate": 6.40, "timestamp": 1713100800 },
    { "rate": 6.38, "timestamp": 1713097200 }
  ],
  "period": "24h"
}
```

## Testing

- [ ] `GET /api/isfr/current` with isfr-service running → returns live rate
- [ ] `GET /api/isfr/current` with isfr-service down → returns 502 with message
- [ ] `GET /api/isfr/history` → returns time series

## Dashboard impact

Replace hardcoded "6.40%" with:
```typescript
const { data: isfr } = useMirage(() => fetch(`${MIRAGE_URL}/api/isfr/current`));
```
