//! ISFR domain tools — read rates, check source status, query oracle state.
//!
//! Tools follow the same `ToolDef` + `ToolHandler` pattern as other builtins.
//! When an `ISFRKeeper` is provided via `with_keeper()`, handlers return live
//! data. Without a keeper, handlers return a clear error instead of stub data.
//!
//! Registration: add `isfr::tool_def_*()` calls to `ROKO_BUILTIN_TOOLS` in
//! `builtin/mod.rs`, and add handler cases in the dispatcher's tool lookup.

use std::sync::Arc;

use async_trait::async_trait;
use roko_chain::isfr_keeper::ISFRKeeper;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};

// ─── Tool names ───────────────────────────────────────────────────────────────

/// Canonical name for the `isfr.read_rates` tool.
pub const ISFR_READ_RATES: &str = "isfr.read_rates";
/// Canonical name for the `isfr.read_rate_history` tool.
pub const ISFR_READ_RATE_HISTORY: &str = "isfr.read_rate_history";
/// Canonical name for the `isfr.oracle_status` tool.
pub const ISFR_ORACLE_STATUS: &str = "isfr.oracle_status";
/// Canonical name for the `isfr.source_status` tool.
pub const ISFR_SOURCE_STATUS: &str = "isfr.source_status";

// ─── Tool definitions ─────────────────────────────────────────────────────────

/// `isfr.read_rates` — get the current ISFR composite rate.
pub fn tool_def_read_rates() -> ToolDef {
    ToolDef::new(
        ISFR_READ_RATES,
        "Read the current ISFR composite rate and per-class breakdown \
         (lending, structured, funding, staking). Returns the most recently \
         computed epoch result.",
        ToolCategory::Network,
        ToolPermission::networked(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "epoch": {
                "type": "integer",
                "description": "Epoch number to query. Omit for the latest."
            }
        },
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(5_000)
}

/// `isfr.read_rate_history` — get historical ISFR rates.
pub fn tool_def_read_rate_history() -> ToolDef {
    ToolDef::new(
        ISFR_READ_RATE_HISTORY,
        "Read historical ISFR composite rates. Returns a list of past epoch \
         results from the 256-epoch ring buffer.",
        ToolCategory::Network,
        ToolPermission::networked(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "from_epoch": {
                "type": "integer",
                "description": "Start epoch (inclusive)."
            },
            "to_epoch": {
                "type": "integer",
                "description": "End epoch (inclusive). Defaults to current."
            },
            "limit": {
                "type": "integer",
                "description": "Maximum entries to return. Default 10, max 256."
            }
        },
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(10_000)
}

/// `isfr.oracle_status` — get the oracle's current epoch and voter state.
pub fn tool_def_oracle_status() -> ToolDef {
    ToolDef::new(
        ISFR_ORACLE_STATUS,
        "Get ISFROracle status: current epoch, active voter count, \
         pending range submissions, and bounty pool balance.",
        ToolCategory::Network,
        ToolPermission::networked(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(5_000)
}

/// `isfr.source_status` — check individual rate source liveness.
pub fn tool_def_source_status() -> ToolDef {
    ToolDef::new(
        ISFR_SOURCE_STATUS,
        "Check ISFR rate source liveness and the last reported reading. \
         Returns status for all sources, or a single source if `source` is given.",
        ToolCategory::Network,
        ToolPermission::networked(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "source": {
                "type": "string",
                "description": "Source name to filter on. Omit to list all sources."
            }
        },
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(5_000)
}

/// All ISFR tool definitions in registration order.
pub fn all_tool_defs() -> Vec<ToolDef> {
    vec![
        tool_def_read_rates(),
        tool_def_read_rate_history(),
        tool_def_oracle_status(),
        tool_def_source_status(),
    ]
}

/// Canonical names of ISFR tools, in [`all_tool_defs`] order.
pub const ISFR_TOOL_NAMES: [&str; 4] = [
    ISFR_READ_RATES,
    ISFR_READ_RATE_HISTORY,
    ISFR_ORACLE_STATUS,
    ISFR_SOURCE_STATUS,
];

// ─── Handler ──────────────────────────────────────────────────────────────────

/// Handler for all ISFR tools.
///
/// Without a keeper (`keeper: None`) every call returns a clear error so
/// callers know the keeper has not been initialised, rather than silently
/// returning stale stub data.
///
/// Wire live data by constructing with [`ISFRHandler::with_keeper`] and
/// passing the same `Arc<ISFRKeeper>` that is running in the server.
#[derive(Clone)]
pub struct ISFRHandler {
    keeper: Option<Arc<ISFRKeeper>>,
}

impl std::fmt::Debug for ISFRHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ISFRHandler")
            .field("keeper", &self.keeper.as_ref().map(|k| &k.keeper_id))
            .finish()
    }
}

impl Default for ISFRHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ISFRHandler {
    /// Create a handler with no keeper. All tool calls will return an error
    /// indicating that the keeper has not been initialised.
    pub fn new() -> Self {
        Self { keeper: None }
    }

    /// Create a handler backed by a live `ISFRKeeper`.
    pub fn with_keeper(keeper: Arc<ISFRKeeper>) -> Self {
        Self {
            keeper: Some(keeper),
        }
    }
}

#[async_trait]
impl ToolHandler for ISFRHandler {
    fn name(&self) -> &str {
        // This handler responds to multiple tool names.
        // The dispatcher routes by name; returning a sentinel here is fine.
        "isfr.*"
    }

    async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
        match call.name.as_str() {
            ISFR_READ_RATES => {
                let Some(keeper) = &self.keeper else {
                    return ToolResult::Err(ToolError::Other(
                        "ISFRKeeper not initialized".to_string(),
                    ));
                };

                match keeper.current_rate() {
                    Some(rate) => ToolResult::structured(
                        serde_json::json!({
                            "composite_bps": rate.composite_bps,
                            "lending_bps": rate.lending_bps,
                            "structured_bps": rate.structured_bps,
                            "funding_bps": rate.funding_bps,
                            "staking_bps": rate.staking_bps,
                            "confidence_bps": rate.confidence_bps,
                            "source_count": rate.readings.len(),
                            "timestamp_ms": rate.timestamp_ms,
                        })
                        .to_string(),
                    ),
                    None => ToolResult::Err(ToolError::Other(
                        "ISFRKeeper has not yet completed a poll cycle; no rate available"
                            .to_string(),
                    )),
                }
            }

            ISFR_READ_RATE_HISTORY => {
                // Rate history is accumulated in AppState (serve layer), not in the
                // keeper itself, which only stores the most recent composite. Return
                // a clear error so callers know to use the HTTP API instead.
                ToolResult::Err(ToolError::Other(
                    "Rate history requires serve integration; query /api/isfr/history via the HTTP control plane".to_string(),
                ))
            }

            ISFR_ORACLE_STATUS => {
                let Some(keeper) = &self.keeper else {
                    return ToolResult::Err(ToolError::Other(
                        "ISFRKeeper not initialized".to_string(),
                    ));
                };

                let metas = keeper.source_metas();
                let source_count = metas.len();
                let running = keeper.current_rate().is_some();

                ToolResult::structured(
                    serde_json::json!({
                        "keeper_id": keeper.keeper_id,
                        "running": running,
                        "source_count": source_count,
                    })
                    .to_string(),
                )
            }

            ISFR_SOURCE_STATUS => {
                let Some(keeper) = &self.keeper else {
                    return ToolResult::Err(ToolError::Other(
                        "ISFRKeeper not initialized".to_string(),
                    ));
                };

                let filter = call.arguments.get("source").and_then(|v| v.as_str());
                let metas = keeper.source_metas();

                let sources: Vec<serde_json::Value> = metas
                    .iter()
                    .filter(|m| filter.map_or(true, |name| m.name == name))
                    .map(|m| {
                        serde_json::json!({
                            "name": m.name,
                            "class": m.class.as_str(),
                            "weight": m.weight,
                            "status": format!("{:?}", m.status).to_lowercase(),
                            "consecutive_failures": m.consecutive_failures,
                            "last_rate_bps": m.last_reading.as_ref().map(|r| r.rate_bps),
                            "last_timestamp_ms": m.last_reading.as_ref().map(|r| r.timestamp_ms),
                        })
                    })
                    .collect();

                ToolResult::structured(
                    serde_json::json!({
                        "sources": sources,
                    })
                    .to_string(),
                )
            }

            _ => ToolResult::Err(ToolError::Other(format!(
                "ISFR tool '{}' not found",
                call.name
            ))),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCall, ToolContext};

    fn make_ctx() -> ToolContext {
        ToolContext::testing("/tmp/test-workdir")
    }

    fn make_call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall::new("test-id", name, args)
    }

    #[test]
    fn tool_defs_have_correct_count() {
        assert_eq!(all_tool_defs().len(), ISFR_TOOL_NAMES.len());
    }

    #[test]
    fn tool_def_names_match_constants() {
        for (def, name) in all_tool_defs().iter().zip(ISFR_TOOL_NAMES.iter()) {
            assert_eq!(def.name, *name, "name mismatch at tool definition");
        }
    }

    #[test]
    fn read_tools_are_idempotent_and_parallel() {
        for def in all_tool_defs() {
            assert!(def.idempotent, "tool {} should be idempotent", def.name);
            assert_eq!(
                def.concurrency,
                ToolConcurrency::Parallel,
                "tool {} should be Parallel",
                def.name
            );
        }
    }

    // ── No-keeper path: every keeper-dependent tool returns an error ───────────

    #[tokio::test]
    async fn read_rates_without_keeper_returns_err() {
        let handler = ISFRHandler::new();
        let ctx = make_ctx();
        let call = make_call(ISFR_READ_RATES, serde_json::json!({}));
        let result = handler.execute(call, &ctx).await;
        assert!(matches!(result, ToolResult::Err(_)));
        if let ToolResult::Err(ToolError::Other(msg)) = result {
            assert!(msg.contains("not initialized"), "unexpected message: {msg}");
        }
    }

    #[tokio::test]
    async fn oracle_status_without_keeper_returns_err() {
        let handler = ISFRHandler::new();
        let ctx = make_ctx();
        let call = make_call(ISFR_ORACLE_STATUS, serde_json::json!({}));
        let result = handler.execute(call, &ctx).await;
        assert!(matches!(result, ToolResult::Err(_)));
    }

    #[tokio::test]
    async fn source_status_without_keeper_returns_err() {
        let handler = ISFRHandler::new();
        let ctx = make_ctx();
        let call = make_call(ISFR_SOURCE_STATUS, serde_json::json!({}));
        let result = handler.execute(call, &ctx).await;
        assert!(matches!(result, ToolResult::Err(_)));
    }

    /// Rate history always returns an error regardless of keeper state because
    /// the history ring is tracked in AppState (roko-serve), not in the keeper.
    #[tokio::test]
    async fn read_rate_history_returns_err() {
        let handler = ISFRHandler::new();
        let ctx = make_ctx();
        let call = make_call(
            ISFR_READ_RATE_HISTORY,
            serde_json::json!({ "from_epoch": 1, "limit": 10 }),
        );
        let result = handler.execute(call, &ctx).await;
        assert!(matches!(result, ToolResult::Err(_)));
        if let ToolResult::Err(ToolError::Other(msg)) = result {
            assert!(
                msg.contains("serve integration"),
                "unexpected message: {msg}"
            );
        }
    }

    // ── Live keeper path ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn read_rates_with_keeper_before_tick_returns_err() {
        use roko_chain::isfr_keeper::{ISFRKeeper, ISFRKeeperConfig};
        let keeper = Arc::new(ISFRKeeper::mock_keeper(
            "test-keeper",
            ISFRKeeperConfig::default(),
        ));
        let handler = ISFRHandler::with_keeper(keeper);
        let ctx = make_ctx();
        let call = make_call(ISFR_READ_RATES, serde_json::json!({}));
        let result = handler.execute(call, &ctx).await;
        // No tick yet — current_rate() returns None.
        assert!(matches!(result, ToolResult::Err(_)));
    }

    #[tokio::test]
    async fn read_rates_with_keeper_after_tick_returns_structured() {
        use roko_chain::isfr_keeper::{ISFRKeeper, ISFRKeeperConfig};
        let keeper = Arc::new(ISFRKeeper::mock_keeper(
            "test-keeper",
            ISFRKeeperConfig::default(),
        ));
        keeper.tick().await;
        let handler = ISFRHandler::with_keeper(keeper);
        let ctx = make_ctx();
        let call = make_call(ISFR_READ_RATES, serde_json::json!({}));
        let result = handler.execute(call, &ctx).await;
        assert!(matches!(result, ToolResult::Ok { .. }));
        if let ToolResult::Ok { content, .. } = result {
            let v: serde_json::Value = serde_json::from_str(&content).unwrap();
            assert!(v["composite_bps"].as_u64().unwrap() > 0);
            assert_eq!(v["confidence_bps"], 10_000u64);
            assert_eq!(v["source_count"], 4u64);
        }
    }

    #[tokio::test]
    async fn oracle_status_with_keeper_after_tick_returns_structured() {
        use roko_chain::isfr_keeper::{ISFRKeeper, ISFRKeeperConfig};
        let keeper = Arc::new(ISFRKeeper::mock_keeper(
            "test-keeper",
            ISFRKeeperConfig::default(),
        ));
        keeper.tick().await;
        let handler = ISFRHandler::with_keeper(keeper);
        let ctx = make_ctx();
        let call = make_call(ISFR_ORACLE_STATUS, serde_json::json!({}));
        let result = handler.execute(call, &ctx).await;
        assert!(matches!(result, ToolResult::Ok { .. }));
        if let ToolResult::Ok { content, .. } = result {
            let v: serde_json::Value = serde_json::from_str(&content).unwrap();
            assert_eq!(v["keeper_id"], "test-keeper");
            assert_eq!(v["running"], true);
            assert_eq!(v["source_count"], 4u64);
        }
    }

    #[tokio::test]
    async fn source_status_with_keeper_returns_all_sources() {
        use roko_chain::isfr_keeper::{ISFRKeeper, ISFRKeeperConfig};
        let keeper = Arc::new(ISFRKeeper::mock_keeper(
            "test-keeper",
            ISFRKeeperConfig::default(),
        ));
        keeper.tick().await;
        let handler = ISFRHandler::with_keeper(keeper);
        let ctx = make_ctx();
        let call = make_call(ISFR_SOURCE_STATUS, serde_json::json!({}));
        let result = handler.execute(call, &ctx).await;
        assert!(matches!(result, ToolResult::Ok { .. }));
        if let ToolResult::Ok { content, .. } = result {
            let v: serde_json::Value = serde_json::from_str(&content).unwrap();
            let sources = v["sources"].as_array().unwrap();
            assert_eq!(sources.len(), 4);
            // All mock sources should be live after a successful tick.
            for src in sources {
                assert_eq!(src["status"], "live");
                assert!(src["last_rate_bps"].as_u64().unwrap() > 0);
            }
        }
    }

    #[tokio::test]
    async fn source_status_filters_by_name() {
        use roko_chain::isfr_keeper::{ISFRKeeper, ISFRKeeperConfig};
        let keeper = Arc::new(ISFRKeeper::mock_keeper(
            "test-keeper",
            ISFRKeeperConfig::default(),
        ));
        keeper.tick().await;
        // Get actual source names from the keeper so the test isn't brittle.
        let metas = keeper.source_metas();
        let target_name = metas[0].name.clone();

        let handler = ISFRHandler::with_keeper(keeper);
        let ctx = make_ctx();
        let call = make_call(
            ISFR_SOURCE_STATUS,
            serde_json::json!({ "source": target_name }),
        );
        let result = handler.execute(call, &ctx).await;
        assert!(matches!(result, ToolResult::Ok { .. }));
        if let ToolResult::Ok { content, .. } = result {
            let v: serde_json::Value = serde_json::from_str(&content).unwrap();
            let sources = v["sources"].as_array().unwrap();
            assert_eq!(sources.len(), 1);
            assert_eq!(sources[0]["name"], target_name);
        }
    }

    #[tokio::test]
    async fn unknown_tool_name_returns_err() {
        let handler = ISFRHandler::new();
        let ctx = make_ctx();
        let call = make_call("isfr.nonexistent", serde_json::json!({}));
        let result = handler.execute(call, &ctx).await;
        assert!(matches!(result, ToolResult::Err(_)));
    }
}
