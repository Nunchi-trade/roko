//! Minimal HTTP JSON-RPC client for the `mirage-rs` chain surface.
//!
//! This client talks to the subset of `chain_*` and `eth_*` methods that
//! `roko-chain-watcher` needs. It is intentionally small and typed: all
//! inbound/outbound shapes are plain structs so the watcher loop never has
//! to touch `serde_json::Value` at a business-logic level.
//!
//! Error handling uses `anyhow::Result` for simplicity — the watcher treats
//! all RPC failures uniformly (log + skip to next poll), so typed taxonomies
//! here would be overkill.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

/// Envelope for JSON-RPC requests.
#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: JsonValue,
}

/// Envelope for JSON-RPC responses.
#[derive(Deserialize)]
#[serde(bound(deserialize = "R: DeserializeOwned"))]
struct RpcResponse<R> {
    #[serde(default = "none_option")]
    result: Option<R>,
    #[serde(default)]
    error: Option<RpcError>,
}

const fn none_option<T>() -> Option<T> {
    None
}

/// Error body inside a JSON-RPC response.
#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

/// Simple HTTP JSON-RPC client targeting `mirage-rs`.
pub struct MirageRpcClient {
    url: String,
    client: reqwest::Client,
    next_id: AtomicU64,
}

impl MirageRpcClient {
    /// Constructs a new client pointed at `url`.
    #[must_use]
    pub fn new(url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            url,
            client,
            next_id: AtomicU64::new(1),
        }
    }

    /// Low-level helper — send one JSON-RPC request and decode its result.
    async fn send_rpc<R: DeserializeOwned>(&self, method: &str, params: JsonValue) -> Result<R> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = RpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };
        let response = self
            .client
            .post(&self.url)
            .json(&request)
            .send()
            .await
            .with_context(|| format!("HTTP POST failed for method {method}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(anyhow!("RPC call {method} returned HTTP {status}"));
        }
        let envelope: RpcResponse<R> = response
            .json()
            .await
            .with_context(|| format!("failed to decode JSON-RPC response for {method}"))?;
        if let Some(err) = envelope.error {
            return Err(anyhow!(
                "RPC error from {method}: code={} message={}",
                err.code,
                err.message
            ));
        }
        envelope
            .result
            .ok_or_else(|| anyhow!("RPC response for {method} missing both result and error"))
    }

    /// Returns the current block number (decimal u64 — mirage returns a numeric literal).
    pub async fn eth_block_number(&self) -> Result<u64> {
        // mirage's `eth_blockNumber` returns a hex-encoded string per the EVM convention.
        let raw: JsonValue = self.send_rpc("eth_blockNumber", json!([])).await?;
        if let Some(s) = raw.as_str() {
            let s = s.strip_prefix("0x").unwrap_or(s);
            return u64::from_str_radix(s, 16)
                .with_context(|| format!("failed to parse eth_blockNumber result: {s}"));
        }
        if let Some(n) = raw.as_u64() {
            return Ok(n);
        }
        Err(anyhow!("unexpected eth_blockNumber result shape: {raw}"))
    }

    /// Returns the version document reported by `chain_version`.
    pub async fn chain_version(&self) -> Result<JsonValue> {
        self.send_rpc("chain_version", json!([])).await
    }

    /// Returns the chain statistics document.
    pub async fn chain_stats(&self) -> Result<ChainStats> {
        self.send_rpc("chain_stats", json!([])).await
    }

    /// Posts an insight and returns the outcome document.
    pub async fn chain_post_insight(
        &self,
        author: &str,
        kind: &str,
        content: &str,
        stake_wei: u128,
    ) -> Result<PostResult> {
        let params = json!({
            "author": author,
            "kind": kind,
            "content": content,
            "stakeWei": stake_wei,
        });
        self.send_rpc("chain_postInsight", params).await
    }

    /// Deposits a pheromone of the given kind.
    pub async fn chain_deposit_pheromone(
        &self,
        kind: &str,
        content: &str,
        intensity: f32,
    ) -> Result<PheromoneId> {
        let params = json!({
            "kind": kind,
            "content": content,
            "intensity": intensity,
        });
        self.send_rpc("chain_depositPheromone", params).await
    }

    /// Queries the pheromone field for the top-k hits matching `query`.
    pub async fn chain_query_pheromones(&self, query: &str, k: usize) -> Result<Vec<PheromoneHit>> {
        let params = json!({ "query": query, "k": k });
        let raw: JsonValue = self.send_rpc("chain_queryPheromones", params).await?;
        let results = raw
            .get("results")
            .cloned()
            .unwrap_or_else(|| JsonValue::Array(vec![]));
        serde_json::from_value(results)
            .context("failed to decode chain_queryPheromones results array")
    }

    /// Searches the insight store.
    pub async fn chain_search_insights(
        &self,
        query: &str,
        k: usize,
        kind: Option<&str>,
    ) -> Result<Vec<InsightHit>> {
        let mut params = json!({ "query": query, "k": k });
        if let Some(k_filter) = kind {
            params["kind"] = json!(k_filter);
        }
        let raw: JsonValue = self.send_rpc("chain_searchInsights", params).await?;
        let results = raw
            .get("results")
            .cloned()
            .unwrap_or_else(|| JsonValue::Array(vec![]));
        serde_json::from_value(results)
            .context("failed to decode chain_searchInsights results array")
    }

    /// Confirms an insight (issues a `chain_confirmInsight` call).
    pub async fn chain_confirm_insight(&self, id: &str, confirmer: &str) -> Result<()> {
        let params = json!({ "id": id, "confirmer": confirmer });
        let _: JsonValue = self.send_rpc("chain_confirmInsight", params).await?;
        Ok(())
    }

    /// Challenges an insight (issues a `chain_challengeInsight` call).
    pub async fn chain_challenge_insight(&self, id: &str, challenger: &str) -> Result<()> {
        let params = json!({ "id": id, "challenger": challenger });
        let _: JsonValue = self.send_rpc("chain_challengeInsight", params).await?;
        Ok(())
    }
}

/// Opaque handle returned by `chain_depositPheromone`.
#[derive(Clone, Debug, Deserialize)]
pub struct PheromoneId {
    /// Numeric id assigned by the pheromone field.
    pub id: u64,
}

/// Result shape returned by `chain_postInsight`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostResult {
    /// One of `"accepted"`, `"duplicate"`, `"exact_match"`.
    pub outcome: String,
    /// Content-addressed insight id.
    pub id: String,
    /// Hamming similarity for duplicates.
    #[serde(default)]
    pub similarity: Option<f32>,
}

/// Stats document returned by `chain_stats`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainStats {
    /// Count of insight entries currently held by the knowledge store.
    pub insights: usize,
    /// Count of pheromones currently active in the field.
    pub pheromones: usize,
}

/// One hit in a `chain_queryPheromones` response.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PheromoneHit {
    /// Pheromone id.
    pub id: u64,
    /// Kind: `"threat"`, `"opportunity"`, `"wisdom"`.
    pub kind: String,
    /// Hamming similarity to the query vector.
    pub similarity: f32,
    /// Current (time-decayed) intensity.
    pub intensity: f32,
    /// Combined score (similarity × intensity).
    #[serde(default)]
    pub score: f32,
}

/// One hit in a `chain_searchInsights` response.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InsightHit {
    /// Insight id (prefixed `insight:`).
    pub id: String,
    /// Kind (`snake_case`).
    pub kind: String,
    /// Content of the insight.
    pub content: String,
    /// Hamming similarity.
    pub similarity: f32,
    /// Time-decayed weight.
    #[serde(default)]
    pub weight: f32,
    /// Combined score.
    #[serde(default)]
    pub score: f32,
    /// Confirmation count.
    #[serde(default)]
    pub confirmations: usize,
    /// Challenge count.
    #[serde(default)]
    pub challenges: usize,
    /// State of the insight (`snake_case` enum name).
    #[serde(default)]
    pub state: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_constructs() {
        let client = MirageRpcClient::new("http://127.0.0.1:8545".to_string());
        assert_eq!(client.url, "http://127.0.0.1:8545");
    }

    #[test]
    fn pheromone_hit_decodes() {
        let raw = json!({
            "id": 42,
            "kind": "threat",
            "similarity": 0.91_f32,
            "intensity": 0.73_f32,
            "score": 0.66_f32
        });
        let hit: PheromoneHit = serde_json::from_value(raw).unwrap();
        assert_eq!(hit.id, 42);
        assert_eq!(hit.kind, "threat");
    }

    #[test]
    fn insight_hit_decodes_with_missing_fields() {
        let raw = json!({
            "id": "insight:00112233445566778899aabbccddeeff",
            "kind": "warning",
            "content": "something bad",
            "similarity": 0.77_f32
        });
        let hit: InsightHit = serde_json::from_value(raw).unwrap();
        assert_eq!(hit.kind, "warning");
        assert_eq!(hit.confirmations, 0);
        assert_eq!(hit.challenges, 0);
    }

    #[test]
    fn chain_stats_decodes() {
        let raw = json!({ "insights": 7, "pheromones": 13 });
        let stats: ChainStats = serde_json::from_value(raw).unwrap();
        assert_eq!(stats.insights, 7);
        assert_eq!(stats.pheromones, 13);
    }
}
