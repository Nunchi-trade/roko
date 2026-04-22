//! Comprehensive HTTP API integration tests for mirage-rs.
//!
//! Uses `axum::Router` with `tower::ServiceExt::oneshot()` — no real server needed.
//! Tests exercise every endpoint in `src/http_api/` against a real `ChainContext`.

#![cfg(all(test, feature = "chain"))]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::{Body, Bytes as AxumBytes},
    extract::OriginalUri,
    http::{HeaderMap, Method, Request, StatusCode},
    routing::{any, get},
};
use parking_lot::RwLock;
use serde_json::Value;
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceExt;

use mirage_rs::{
    chain::{KnowledgeKind, PheromoneKind, projection::project_tokens},
    chain_rpc::{ChainContext, ChainToggles},
    http_api::{self, ApiState},
    rpc::build_relay_proxy_router_for_tests,
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Builds a router with all toggles enabled and an empty chain.
fn empty_router() -> axum::Router {
    let toggles = ChainToggles {
        hdc: true,
        knowledge: true,
        stigmergy: true,
    };
    let chain = ChainContext::new(toggles);
    let state = ApiState::new(Arc::new(RwLock::new(chain)));
    http_api::build_router(state)
}

/// Builds a router and seeds `n` pheromones (threat, opportunity, wisdom cycling).
fn router_with_pheromones(n: usize) -> axum::Router {
    let toggles = ChainToggles {
        hdc: true,
        knowledge: true,
        stigmergy: true,
    };
    let mut chain = ChainContext::new(toggles);
    let kinds = [
        PheromoneKind::Threat,
        PheromoneKind::Opportunity,
        PheromoneKind::Wisdom,
    ];
    let now = now_secs();
    for i in 0..n {
        let v = project_tokens(&format!("pheromone content {i}"));
        chain
            .pheromones
            .deposit(kinds[i % 3], v, 1.0 + i as f32, now);
    }
    let state = ApiState::new(Arc::new(RwLock::new(chain)));
    http_api::build_router(state)
}

/// Builds a router with `n` knowledge entries seeded.
fn router_with_knowledge(n: usize) -> axum::Router {
    let toggles = ChainToggles {
        hdc: true,
        knowledge: true,
        stigmergy: true,
    };
    let mut chain = ChainContext::new(toggles);
    let kinds = [
        KnowledgeKind::Insight,
        KnowledgeKind::Heuristic,
        KnowledgeKind::Warning,
        KnowledgeKind::CausalLink,
        KnowledgeKind::StrategyFragment,
        KnowledgeKind::AntiKnowledge,
    ];
    let now = now_secs();
    for i in 0..n {
        let content = format!("knowledge entry number {i}");
        let vector = project_tokens(&content);
        chain.knowledge.post(
            format!("author-{i}").into_bytes(),
            kinds[i % 6],
            content,
            vector,
            Vec::new(),
            now,
            0,
        );
    }
    let state = ApiState::new(Arc::new(RwLock::new(chain)));
    http_api::build_router(state)
}

/// Builds a router with registered agents.
fn router_with_agents(n: usize) -> axum::Router {
    let toggles = ChainToggles {
        hdc: true,
        knowledge: true,
        stigmergy: true,
    };
    let mut chain = ChainContext::new(toggles);
    let now = now_secs();
    for i in 0..n {
        chain.agent_registry.register(
            format!("agent-{i}"),
            format!("pubkey-{i}").into_bytes(),
            "watcher".into(),
            String::new(),
            now,
        );
    }
    let state = ApiState::new(Arc::new(RwLock::new(chain)));
    http_api::build_router(state)
}

/// Helper: send a GET request and parse the JSON response.
async fn get_json(router: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1_048_576)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap_or_else(|_| {
        panic!(
            "failed to parse JSON from GET {uri}: {}",
            String::from_utf8_lossy(&body)
        )
    });
    (status, json)
}

/// Helper: send a POST request with JSON body and parse the JSON response.
async fn post_json(router: &axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1_048_576)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
        panic!(
            "failed to parse JSON from POST {uri}: {}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, json)
}

struct MockRelayServer {
    base_url: String,
    task: JoinHandle<()>,
}

impl MockRelayServer {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock relay listener");
        let addr = listener.local_addr().expect("read mock relay address");
        let base_url = format!("http://{addr}");
        let app = Router::new()
            .route("/relay/health", any(mock_relay_health))
            .route("/relay/{*tail}", any(mock_relay_echo));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock relay");
        });

        let server = Self { base_url, task };
        server.wait_until_ready().await;
        server
    }

    async fn wait_until_ready(&self) {
        for _ in 0..50 {
            match reqwest::Client::new()
                .get(format!("{}/relay/health", self.base_url))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return,
                _ => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        panic!("mock relay did not become ready");
    }
}

impl Drop for MockRelayServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn mock_relay_health() -> &'static str {
    "ok"
}

async fn mock_relay_echo(
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: AxumBytes,
) -> (StatusCode, Json<Value>) {
    let body_json = serde_json::from_slice::<Value>(&body).ok();
    (
        if method == Method::POST {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        },
        Json(serde_json::json!({
            "method": method.as_str(),
            "path": uri.0.path(),
            "query": uri.0.query(),
            "content_type": headers
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            "body_json": body_json,
        })),
    )
}

// ===========================================================================
// Health & Stats
// ===========================================================================

#[tokio::test]
async fn test_health_returns_ok() {
    let router = empty_router();
    let (status, json) = get_json(&router, "/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
    // Chain counts nested under "chain.counts".
    let counts = &json["chain"]["counts"];
    assert!(counts["insights"].is_number(), "expected insights count");
    assert!(
        counts["pheromones"].is_number(),
        "expected pheromones count"
    );
    assert!(counts["agents"].is_number(), "expected agents count");
    // Toggles also present
    assert!(json["chain"]["toggles"].is_object(), "expected toggles");
}

#[tokio::test]
async fn test_stats_returns_complete_response() {
    let router = router_with_pheromones(3);
    let (status, json) = get_json(&router, "/stats").await;
    assert_eq!(status, StatusCode::OK);

    // insights section
    assert!(json["insights"].is_object(), "expected insights section");
    assert!(json["insights"]["total"].is_number());

    // pheromones section
    assert!(
        json["pheromones"].is_object(),
        "expected pheromones section"
    );
    assert_eq!(json["pheromones"]["total"], 3);

    // toggles
    assert!(json["toggles"].is_object(), "expected toggles section");
    assert_eq!(json["toggles"]["hdc"], true);
    assert_eq!(json["toggles"]["knowledge"], true);
    assert_eq!(json["toggles"]["stigmergy"], true);

    // timestamp
    assert!(json["timestamp"].is_number());
}

#[tokio::test]
async fn test_isfr_current_degrades_without_sidecar() {
    let router = empty_router();
    let (status, json) = get_json(&router, "/isfr/current").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "unavailable");
    assert_eq!(json["source"], "mirage-local-fallback");
    assert_eq!(json["state"], "no_data");
    assert!(
        json["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("ISFR_SERVICE_URL not configured")),
        "fallback should explain missing sidecar: {json}",
    );
    assert_eq!(json["counts"]["insights"], 0);
}

#[tokio::test]
async fn test_isfr_history_degrades_without_sidecar() {
    let router = empty_router();
    let (status, json) = get_json(&router, "/isfr/history?limit=10").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "unavailable");
    assert_eq!(json["source"], "mirage-local-fallback");
    assert!(json["items"].as_array().is_some_and(Vec::is_empty));
    assert!(json["points"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(json["query"]["limit"], "10");
}

// ===========================================================================
// Pheromones (Read)
// ===========================================================================

#[tokio::test]
async fn test_list_pheromones_empty() {
    let router = empty_router();
    let (status, json) = get_json(&router, "/pheromones").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["items"].as_array().unwrap().len(), 0);
    assert_eq!(json["total"], 0);
    assert_eq!(json["has_more"], false);
}

#[tokio::test]
async fn test_list_pheromones_with_data() {
    let router = router_with_pheromones(5);
    let (status, json) = get_json(&router, "/pheromones").await;
    assert_eq!(status, StatusCode::OK);
    let items = json["items"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(json["total"], 5);
    assert_eq!(json["has_more"], false);
    // Each item should have an id and kind
    for item in items {
        assert!(item["id"].is_number(), "pheromone should have id");
        assert!(
            item["kind"].is_string(),
            "pheromone should have kind string"
        );
    }
}

#[tokio::test]
async fn test_list_pheromones_pagination() {
    let router = router_with_pheromones(5);
    let (status, json) = get_json(&router, "/pheromones?limit=2&offset=2").await;
    assert_eq!(status, StatusCode::OK);
    let items = json["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(json["total"], 5);
    assert_eq!(json["has_more"], true);
}

#[tokio::test]
async fn test_list_pheromones_filter_by_kind() {
    // Seed 6 pheromones: 2 threat, 2 opportunity, 2 wisdom
    let router = router_with_pheromones(6);
    let (status, json) = get_json(&router, "/pheromones?kind=threat").await;
    assert_eq!(status, StatusCode::OK);
    let items = json["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    for item in items {
        assert_eq!(item["kind"], "threat");
    }
}

#[tokio::test]
async fn test_pheromone_summary() {
    // Seed 6: indices 0,3=threat, 1,4=opportunity, 2,5=wisdom
    let router = router_with_pheromones(6);
    let (status, json) = get_json(&router, "/pheromones/summary").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["total_count"], 6);
    assert!(json["total_intensity"].is_number());
    let by_kind = &json["by_kind"];
    assert!(by_kind["threat"]["count"].is_number());
    assert!(by_kind["opportunity"]["count"].is_number());
    assert!(by_kind["wisdom"]["count"].is_number());
    assert_eq!(by_kind["threat"]["count"], 2);
    assert_eq!(by_kind["opportunity"]["count"], 2);
    assert_eq!(by_kind["wisdom"]["count"], 2);
}

#[tokio::test]
async fn test_pheromone_heatmap() {
    let router = router_with_pheromones(3);
    let (status, json) = get_json(&router, "/pheromones/heatmap").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["buckets"].is_array(), "heatmap should have buckets");
    assert!(
        json["bucket_seconds"].is_number(),
        "heatmap should have bucket_seconds"
    );
    assert!(json["timestamp"].is_number());
    // Each bucket should have the expected fields
    if let Some(buckets) = json["buckets"].as_array() {
        for bucket in buckets {
            assert!(bucket["timestamp"].is_number());
            assert!(bucket["threat"].is_number());
            assert!(bucket["opportunity"].is_number());
            assert!(bucket["wisdom"].is_number());
            assert!(bucket["total_intensity"].is_number());
        }
    }
}

// ===========================================================================
// Pheromones (Write)
// ===========================================================================

#[tokio::test]
async fn test_deposit_pheromone() {
    let router = empty_router();
    let (status, json) = post_json(
        &router,
        "/pheromones",
        serde_json::json!({
            "kind": "threat",
            "content": "suspicious contract interaction detected",
            "intensity": 0.8,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["id"].is_number(), "deposit should return an id");
    assert_eq!(json["kind"], "threat");
}

#[tokio::test]
async fn test_deposit_pheromone_invalid_kind() {
    let router = empty_router();
    let (status, json) = post_json(
        &router,
        "/pheromones",
        serde_json::json!({
            "kind": "invalid_kind",
            "content": "something",
            "intensity": 1.0,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("unknown pheromone kind"),
        "expected error about unknown kind, got: {}",
        json["error"]
    );
}

#[tokio::test]
async fn test_pheromone_projection() {
    let router = empty_router();

    // Deposit a pheromone first
    let (status, deposit_json) = post_json(
        &router,
        "/pheromones",
        serde_json::json!({
            "kind": "wisdom",
            "content": "ERC20 approve needs zero first",
            "intensity": 1.5,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = deposit_json["id"].as_u64().unwrap();

    // GET projection for this pheromone
    let (status, proj_json) = get_json(&router, &format!("/pheromones/{id}/projection")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(proj_json["id"], id);
    assert_eq!(proj_json["kind"], "wisdom");
    assert!(proj_json["base_intensity"].is_number());
    assert!(
        proj_json["points"].is_array(),
        "projection should have points array"
    );
}

// ===========================================================================
// Knowledge (Read)
// ===========================================================================

#[tokio::test]
async fn test_list_entries_empty() {
    let router = empty_router();
    let (status, json) = get_json(&router, "/knowledge/entries").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["items"].as_array().unwrap().len(), 0);
    assert_eq!(json["total"], 0);
}

#[tokio::test]
async fn test_list_entries_with_data() {
    let router = router_with_knowledge(4);
    let (status, json) = get_json(&router, "/knowledge/entries").await;
    assert_eq!(status, StatusCode::OK);
    let items = json["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(json["total"], 4);
    // Each entry should have structured fields
    for item in items {
        assert!(item["id"].is_string(), "entry should have string id");
        assert!(item["kind"].is_string(), "entry should have kind");
        assert!(item["content"].is_string(), "entry should have content");
        assert!(item["state"].is_string(), "entry should have state");
    }
}

#[tokio::test]
async fn test_semantic_search() {
    let router = router_with_knowledge(6);
    let (status, json) = get_json(&router, "/knowledge/search?q=knowledge+entry+number+0").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["results"].is_array(), "search should return results");
    assert!(json["query"].is_string(), "search should echo query");
    assert!(json["timestamp"].is_number());

    let results = json["results"].as_array().unwrap();
    // Should return ranked results
    assert!(
        !results.is_empty(),
        "search should return at least one result"
    );
    // Results should have similarity scores
    for result in results {
        assert!(
            result["similarity"].is_number(),
            "result should have similarity"
        );
        assert!(result["id"].is_string(), "result should have id");
    }
}

#[tokio::test]
async fn test_knowledge_kinds() {
    let router = empty_router();
    let (status, json) = get_json(&router, "/knowledge/kinds").await;
    assert_eq!(status, StatusCode::OK);
    let k_kinds = json["knowledge_kinds"].as_array().unwrap();
    // All 6 knowledge kinds
    assert_eq!(k_kinds.len(), 6, "expected 6 knowledge kinds");
    let kind_names: Vec<&str> = k_kinds
        .iter()
        .map(|k| k["name"].as_str().unwrap())
        .collect();
    assert!(kind_names.contains(&"insight"));
    assert!(kind_names.contains(&"heuristic"));
    assert!(kind_names.contains(&"warning"));
    assert!(kind_names.contains(&"causal_link"));
    assert!(kind_names.contains(&"strategy_fragment"));
    assert!(kind_names.contains(&"anti_knowledge"));

    // Pheromone kinds
    let p_kinds = json["pheromone_kinds"].as_array().unwrap();
    assert_eq!(p_kinds.len(), 3, "expected 3 pheromone kinds");
}

// ===========================================================================
// Knowledge (Write)
// ===========================================================================

#[tokio::test]
async fn test_post_insight() {
    let router = empty_router();
    let (status, json) = post_json(
        &router,
        "/knowledge/entries",
        serde_json::json!({
            "kind": "insight",
            "content": "Uniswap V3 router reverts with STF on insufficient allowance",
            "author": "test-agent",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["outcome"], "accepted");
    assert!(json["id"].is_string(), "should return insight id");
}

#[tokio::test]
async fn test_confirm_entry() {
    let router = empty_router();

    // Post an insight first
    let (status, post_json_resp) = post_json(
        &router,
        "/knowledge/entries",
        serde_json::json!({
            "kind": "warning",
            "content": "Contract at 0xdead is a honeypot",
            "author": "agent-alpha",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = post_json_resp["id"].as_str().unwrap();

    // Confirm the entry
    let (status, confirm_json) = post_json(
        &router,
        &format!("/knowledge/entries/{id}/confirm"),
        serde_json::json!({
            "confirmer": "agent-beta",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(confirm_json["ok"], true);
}

#[tokio::test]
async fn test_challenge_entry() {
    let router = empty_router();

    // Post an insight
    let (status, post_resp) = post_json(
        &router,
        "/knowledge/entries",
        serde_json::json!({
            "kind": "anti_knowledge",
            "content": "ERC20 approve(0) is required before re-approve",
            "author": "agent-alpha",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = post_resp["id"].as_str().unwrap();

    // We need to confirm first to move to Active state (challenges require Active)
    let (status, _) = post_json(
        &router,
        &format!("/knowledge/entries/{id}/confirm"),
        serde_json::json!({
            "confirmer": "agent-beta",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Now challenge it
    let (status, challenge_json) = post_json(
        &router,
        &format!("/knowledge/entries/{id}/challenge"),
        serde_json::json!({
            "challenger": "agent-gamma",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(challenge_json["ok"], true);
}

#[tokio::test]
async fn test_trigger_decay() {
    let router = empty_router();

    // Post an insight
    let (status, _) = post_json(
        &router,
        "/knowledge/entries",
        serde_json::json!({
            "kind": "heuristic",
            "content": "Always check slippage tolerance before swap",
            "author": "agent-alpha",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Trigger decay with a far-future timestamp to actually cause pruning
    let far_future = now_secs() + 365 * 24 * 3600;
    let (status, decay_json) = post_json(
        &router,
        "/knowledge/decay",
        serde_json::json!({
            "now_secs": far_future,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(decay_json["ok"], true);
    assert!(
        decay_json["pruned"].is_number(),
        "should report pruned count"
    );
    assert!(
        decay_json["remaining"].is_number(),
        "should report remaining count"
    );
    assert!(decay_json["timestamp"].is_number());
}

// ===========================================================================
// Knowledge edges
// ===========================================================================

#[tokio::test]
async fn test_knowledge_edges_empty() {
    let router = empty_router();
    let (status, json) = get_json(&router, "/knowledge/edges").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["items"].is_array());
    assert_eq!(json["total"], 0);
}

#[tokio::test]
async fn test_knowledge_edges_with_data() {
    let router = empty_router();

    // Post two insights where the second is enabled by the first
    let (_, post1) = post_json(
        &router,
        "/knowledge/entries",
        serde_json::json!({
            "kind": "insight",
            "content": "Router contract uses multicall pattern",
            "author": "agent-a",
        }),
    )
    .await;
    let id1 = post1["id"].as_str().unwrap().to_string();

    let (_, _post2) = post_json(
        &router,
        "/knowledge/entries",
        serde_json::json!({
            "kind": "causal_link",
            "content": "Multicall enables batch swap optimization",
            "author": "agent-b",
            "enabled_by": [id1],
        }),
    )
    .await;

    let (status, json) = get_json(&router, "/knowledge/edges").await;
    assert_eq!(status, StatusCode::OK);
    let items = json["items"].as_array().unwrap();
    assert!(
        !items.is_empty(),
        "should have at least one edge from enabled_by"
    );
}

// ===========================================================================
// Agents
// ===========================================================================

#[tokio::test]
async fn test_register_agent() {
    let router = empty_router();
    let (status, json) = post_json(
        &router,
        "/agents",
        serde_json::json!({
            "id": "golem-1",
            "pubkey": "0xdeadbeef",
            "role": "watcher",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["registered"], true);
    assert_eq!(json["agent_id"], "golem-1");
    assert_eq!(json["role"], "watcher");
}

#[tokio::test]
async fn test_register_agent_duplicate() {
    let router = empty_router();

    // Register once
    let (status, _) = post_json(
        &router,
        "/agents",
        serde_json::json!({
            "id": "golem-dup",
            "pubkey": "0x1234",
            "role": "executor",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Register again — should fail (already registered)
    let (status, json) = post_json(
        &router,
        "/agents",
        serde_json::json!({
            "id": "golem-dup",
            "pubkey": "0x1234",
            "role": "executor",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("already registered")
    );
}

#[tokio::test]
async fn test_register_agent_empty_id() {
    let router = empty_router();
    let (status, json) = post_json(
        &router,
        "/agents",
        serde_json::json!({
            "id": "",
            "pubkey": "0x1234",
            "role": "watcher",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["error"].as_str().unwrap().contains("empty"));
}

#[tokio::test]
async fn test_list_agents() {
    let router = router_with_agents(3);
    let (status, json) = get_json(&router, "/agents").await;
    assert_eq!(status, StatusCode::OK);
    let items = json["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(json["total"], 3);
}

#[tokio::test]
async fn test_list_agents_empty() {
    let router = empty_router();
    let (status, json) = get_json(&router, "/agents").await;
    assert_eq!(status, StatusCode::OK);
    let items = json["items"].as_array().unwrap();
    assert_eq!(items.len(), 0);
    assert_eq!(json["total"], 0);
}

#[tokio::test]
async fn test_agent_trace_not_found() {
    let router = empty_router();
    let (status, json) = get_json(&router, "/agents/nonexistent/trace").await;
    // The endpoint returns a JSON body with an error field, not a 404 status
    // (the handler returns the trace as-is even if agent doesn't exist — let's
    // verify the behavior as-implemented)
    assert_eq!(status, StatusCode::OK);
    // Nonexistent agent returns {"error": "agent not found", "agent_id": "..."}
    assert!(
        json.get("error").is_some(),
        "expected error field for missing agent"
    );
}

#[tokio::test]
async fn test_agent_heartbeat_post() {
    let router = empty_router();

    // Register first
    let (status, _) = post_json(
        &router,
        "/agents",
        serde_json::json!({
            "id": "heartbeat-agent",
            "pubkey": "0xbeef",
            "role": "monitor",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Send heartbeat
    let (status, json) = post_json(
        &router,
        "/agents/heartbeat-agent/heartbeat",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["ok"], true);
    assert_eq!(json["agent_id"], "heartbeat-agent");
    assert!(json["timestamp"].is_number());
}

#[tokio::test]
async fn test_agent_heartbeat_get() {
    let router = router_with_agents(1);

    let (status, json) = get_json(&router, "/agents/agent-0/heartbeat").await;
    assert_eq!(status, StatusCode::OK);
    // Should return heartbeat info for the agent
    assert_eq!(json["agent_id"], "agent-0");
}

#[tokio::test]
async fn test_agent_stats() {
    let router = router_with_agents(1);
    let (status, json) = get_json(&router, "/agents/agent-0/stats").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["agent_id"], "agent-0");
    // Stats fields are flat on the response, not nested under "stats"
    assert!(
        json["confirmations_given"].is_number(),
        "should have confirmations_given"
    );
    assert!(
        json["challenges_given"].is_number(),
        "should have challenges_given"
    );
    assert!(
        json["registered_at"].is_number(),
        "should have registered_at"
    );
}

// ===========================================================================
// Agent topology
// ===========================================================================

#[tokio::test]
async fn test_topology_empty() {
    let router = empty_router();
    let (status, json) = get_json(&router, "/agents/topology").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["nodes"].is_array());
    assert!(json["edges"].is_array());
    assert!(json["timestamp"].is_number());
    assert_eq!(json["nodes"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_topology_with_agents_and_knowledge() {
    // Build a router where two agents have contributed knowledge
    let toggles = ChainToggles {
        hdc: true,
        knowledge: true,
        stigmergy: true,
    };
    let mut chain = ChainContext::new(toggles);
    let now = now_secs();

    // Post knowledge from two different authors
    let v1 = project_tokens("insight from agent alpha");
    chain.knowledge.post(
        b"agent-alpha".to_vec(),
        KnowledgeKind::Insight,
        "insight from agent alpha".into(),
        v1,
        Vec::new(),
        now,
        0,
    );
    let v2 = project_tokens("heuristic from agent beta");
    chain.knowledge.post(
        b"agent-beta".to_vec(),
        KnowledgeKind::Heuristic,
        "heuristic from agent beta".into(),
        v2,
        Vec::new(),
        now,
        0,
    );

    let state = ApiState::new(Arc::new(RwLock::new(chain)));
    let router = http_api::build_router(state);

    let (status, json) = get_json(&router, "/agents/topology").await;
    assert_eq!(status, StatusCode::OK);
    let nodes = json["nodes"].as_array().unwrap();
    // Should have at least 2 nodes (one per unique author)
    assert!(
        nodes.len() >= 2,
        "expected at least 2 topology nodes, got {}",
        nodes.len()
    );
    for node in nodes {
        assert!(node["id"].is_string());
        assert!(node["address"].is_string());
    }
}

// ===========================================================================
// Pheromone query (POST)
// ===========================================================================

#[tokio::test]
async fn test_pheromone_query() {
    let router = router_with_pheromones(5);
    let (status, json) = post_json(
        &router,
        "/pheromones/query",
        serde_json::json!({
            "query": "pheromone content 0",
            "k": 3,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["results"].is_array(), "query should return results");
    assert!(json["timestamp"].is_number());
    let results = json["results"].as_array().unwrap();
    assert!(
        results.len() <= 3,
        "should return at most k=3 results, got {}",
        results.len()
    );
    // Each result should have expected fields
    for result in results {
        assert!(result["id"].is_number());
        assert!(result["kind"].is_string());
        assert!(result["similarity"].is_number());
    }
}

// ===========================================================================
// End-to-end flow: deposit pheromone, then list, then query
// ===========================================================================

#[tokio::test]
async fn test_deposit_then_list_then_query() {
    let router = empty_router();

    // Deposit three pheromones
    for kind in &["threat", "opportunity", "wisdom"] {
        let (status, _) = post_json(
            &router,
            "/pheromones",
            serde_json::json!({
                "kind": kind,
                "content": format!("test {kind} pheromone"),
                "intensity": 1.0,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    // List should return 3
    let (status, json) = get_json(&router, "/pheromones").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["total"], 3);

    // Summary should show 1 per kind
    let (_, summary) = get_json(&router, "/pheromones/summary").await;
    assert_eq!(summary["total_count"], 3);

    // Query
    let (status, query_json) = post_json(
        &router,
        "/pheromones/query",
        serde_json::json!({
            "query": "test threat pheromone",
            "k": 10,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!query_json["results"].as_array().unwrap().is_empty());
}

// ===========================================================================
// End-to-end flow: post insight, confirm, challenge, decay
// ===========================================================================

#[tokio::test]
async fn test_full_knowledge_lifecycle() {
    let router = empty_router();

    // 1. Post an insight
    let (status, post_resp) = post_json(
        &router,
        "/knowledge/entries",
        serde_json::json!({
            "kind": "insight",
            "content": "DEX aggregators route through multiple pools for optimal price",
            "author": "researcher-1",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(post_resp["outcome"], "accepted");
    let id = post_resp["id"].as_str().unwrap().to_string();

    // 2. Confirm the insight (transitions Created -> Active)
    let (status, _) = post_json(
        &router,
        &format!("/knowledge/entries/{id}/confirm"),
        serde_json::json!({ "confirmer": "validator-1" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 3. Challenge the insight (Active -> Challenged)
    let (status, _) = post_json(
        &router,
        &format!("/knowledge/entries/{id}/challenge"),
        serde_json::json!({ "challenger": "skeptic-1" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 4. Search should still find it
    let (status, search) =
        get_json(&router, "/knowledge/search?q=DEX+aggregators+optimal+price").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!search["results"].as_array().unwrap().is_empty());

    // 5. Decay sweep
    let far_future = now_secs() + 365 * 24 * 3600;
    let (status, decay) = post_json(
        &router,
        "/knowledge/decay",
        serde_json::json!({ "now_secs": far_future }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(decay["ok"], true);
    assert!(decay["pruned"].is_number());
}

// ===========================================================================
// POST /api/knowledge/entries — invalid kind
// ===========================================================================

#[tokio::test]
async fn test_post_insight_invalid_kind() {
    let router = empty_router();
    let (status, json) = post_json(
        &router,
        "/knowledge/entries",
        serde_json::json!({
            "kind": "bogus",
            "content": "something",
            "author": "x",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("unknown knowledge kind")
    );
}

// ===========================================================================
// Deposit pheromone empty content
// ===========================================================================

#[tokio::test]
async fn test_deposit_pheromone_empty_content() {
    let router = empty_router();
    let (status, json) = post_json(
        &router,
        "/pheromones",
        serde_json::json!({
            "kind": "threat",
            "content": "",
            "intensity": 1.0,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["error"].as_str().unwrap().contains("content"));
}

#[tokio::test]
async fn relay_proxy_get_preserves_path_and_query() {
    let relay = MockRelayServer::spawn().await;
    let router = Router::new()
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({ "status": "ok" })) }),
        )
        .merge(build_relay_proxy_router_for_tests(relay.base_url.clone()));
    let req = Request::builder()
        .method("GET")
        .uri("/relay/cards/agent-proxy?view=full&source=dashboard")
        .body(Body::empty())
        .expect("build proxied relay GET request");
    let response = router
        .oneshot(req)
        .await
        .expect("execute proxied relay GET");

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1_048_576)
        .await
        .expect("read proxied relay GET body");
    let body: Value = serde_json::from_slice(&body).expect("decode proxied relay GET");
    assert_eq!(body["method"], "GET");
    assert_eq!(body["path"], "/relay/cards/agent-proxy");
    assert_eq!(body["query"], "view=full&source=dashboard");
    assert_eq!(body["body_json"], Value::Null);
}

#[tokio::test]
async fn relay_proxy_leaves_non_relay_routes_local() {
    let relay = MockRelayServer::spawn().await;
    let router = Router::new()
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({ "status": "ok" })) }),
        )
        .merge(build_relay_proxy_router_for_tests(relay.base_url.clone()));
    let (status, body) = get_json(&router, "/health").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn relay_proxy_post_preserves_json_and_status() {
    let relay = MockRelayServer::spawn().await;
    let router = Router::new()
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({ "status": "ok" })) }),
        )
        .merge(build_relay_proxy_router_for_tests(relay.base_url.clone()));
    let request_body = serde_json::json!({
        "agent_id": "agent-relay",
        "message": {
            "prompt": "summarize relay status"
        }
    });
    let req = Request::builder()
        .method("POST")
        .uri("/relay/messages?timeout_ms=2500")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&request_body).expect("serialize proxied relay POST"),
        ))
        .expect("build proxied relay POST request");
    let response = router
        .oneshot(req)
        .await
        .expect("execute proxied relay POST");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(response.into_body(), 1_048_576)
        .await
        .expect("read proxied relay POST body");
    let body: Value = serde_json::from_slice(&body).expect("decode proxied relay POST");
    assert_eq!(body["method"], "POST");
    assert_eq!(body["path"], "/relay/messages");
    assert_eq!(body["query"], "timeout_ms=2500");
    assert_eq!(body["content_type"], "application/json");
    assert_eq!(body["body_json"], request_body);
}
