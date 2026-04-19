//! Integration coverage for tier-based model routing with a real tool
//! dispatcher and deterministic mock backends.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rand::RngCore;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use roko_agent::dispatcher::{HandlerResolver, ToolDispatcher};
use roko_agent::tool_loop::{LlmBackend, LlmError, StopReason, ToolLoop};
use roko_agent::translate::{
    BackendResponse, OpenAiTranslator, RenderedTools, SessionState, Translator,
};
use roko_core::DaimonPolicy;
use roko_core::agent::{AgentRole, ModelTier};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolHandler, ToolPermission,
    ToolRegistry, ToolResult, VecToolRegistry,
};
use roko_learn::model_router::{LinUCBRouter, RoutingContext};
use tempfile::TempDir;
use tokio::time::sleep;

const SEED: u64 = 60_060;
const FAST_SLUG: &str = "claude-haiku-3-5";
const PREMIUM_SLUG: &str = "claude-opus-4";
const TRAINING_ROUNDS: usize = 20;

fn scaled_test_timeout_ms(ms: u64) -> u64 {
    if std::env::var("CI").is_ok_and(|value| value == "true") {
        ms.saturating_mul(10)
    } else {
        ms
    }
}

#[derive(Clone, Copy)]
struct MockBackendProfile {
    slug: &'static str,
    reply: &'static str,
}

struct EchoHandler;

#[async_trait]
impl ToolHandler for EchoHandler {
    fn name(&self) -> &'static str {
        "echo"
    }

    async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
        let text = call
            .arguments
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map_or_else(String::new, str::to_owned);
        ToolResult::text(text)
    }
}

struct MockBackend {
    profile: MockBackendProfile,
}

impl MockBackend {
    const fn new(profile: MockBackendProfile) -> Self {
        Self { profile }
    }
}

#[async_trait]
impl LlmBackend for MockBackend {
    async fn send_turn(
        &self,
        messages: &[serde_json::Value],
        tools: &RenderedTools,
        _session: &SessionState,
    ) -> Result<BackendResponse, LlmError> {
        let RenderedTools::JsonArray(_) = tools else {
            return Err(LlmError::Backend("expected OpenAI JSON tools".to_string()));
        };

        let has_tool_result = messages
            .iter()
            .any(|message| message.get("role").and_then(serde_json::Value::as_str) == Some("tool"));

        if !has_tool_result {
            return Ok(BackendResponse::Json(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [{
                            "id": format!("call-{}", self.profile.slug),
                            "type": "function",
                            "function": {
                                "name": "echo",
                                "arguments": serde_json::json!({
                                    "text": format!("route-{}", self.profile.slug),
                                }).to_string(),
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {
                    "prompt_tokens": 7,
                    "completion_tokens": 3,
                    "total_tokens": 10
                }
            })));
        }

        Ok(BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": self.profile.reply
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 4,
                "total_tokens": 9
            }
        })))
    }
}

struct DispatchHarness {
    dispatcher: Arc<ToolDispatcher>,
    translator: Arc<dyn Translator>,
    backends: HashMap<&'static str, Arc<MockBackend>>,
}

impl DispatchHarness {
    fn new(profiles: [MockBackendProfile; 2]) -> Self {
        let echo_tool = echo_tool();
        let registry: Arc<dyn ToolRegistry> =
            Arc::new(VecToolRegistry::from_tools(vec![echo_tool]));
        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        let resolver: Arc<dyn HandlerResolver> =
            Arc::new(move |name: &str| (name == "echo").then(|| Arc::clone(&handler)));
        let backends = profiles
            .into_iter()
            .map(|profile| (profile.slug, Arc::new(MockBackend::new(profile))))
            .collect();

        Self {
            dispatcher: Arc::new(ToolDispatcher::new(registry, resolver)),
            translator: Arc::new(OpenAiTranslator),
            backends,
        }
    }

    async fn dispatch(&self, slug: &str, worktree: &Path, turn_nonce: u64) -> String {
        let backend = self
            .backends
            .get(slug)
            .cloned()
            .unwrap_or_else(|| panic!("missing backend for slug {slug}"));
        let loop_runner = ToolLoop::new(
            Arc::clone(&self.translator),
            Arc::clone(&self.dispatcher),
            backend,
        )
        .with_max_iterations(3);

        let result = loop_runner
            .run(
                "Select one deterministic model backend.",
                &format!("model-router-{turn_nonce}"),
                &[echo_tool()],
                &ToolContext::testing(worktree),
            )
            .await;

        assert_eq!(result.stop_reason, StopReason::Stop);
        assert_eq!(result.tool_calls.len(), 1);
        result.final_text
    }
}

fn echo_tool() -> ToolDef {
    ToolDef::new(
        "echo",
        "return the supplied text",
        ToolCategory::Meta,
        ToolPermission::read_only(),
    )
    .with_concurrency(ToolConcurrency::Serial)
}

fn default_ctx() -> RoutingContext {
    RoutingContext {
        task_category: TaskCategory::Implementation,
        complexity: TaskComplexityBand::Standard,
        iteration: 0,
        role: AgentRole::Implementer,
        crate_familiarity: 0.5,
        has_prior_failure: false,
        conductor_load: 0.0,
        active_agents: 0,
        ready_queue_depth: 0,
        max_queue_wait_hours: 0.0,
        daimon_policy: DaimonPolicy::default(),
        thinking_level: None,
        temperament: None,
        previous_model: None,
        plan_context_tokens: None,
        tier_thresholds: None,
    }
}

#[tokio::test]
async fn model_router_routes_by_tier_and_persists_atomically() {
    let tmp = TempDir::new().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let path = tmp.path().join("model-router.json");
    std::fs::write(&path, "{\"stale\":true}")
        .unwrap_or_else(|err| panic!("seed stale router file: {err}"));
    let modified_before = std::fs::metadata(&path)
        .unwrap_or_else(|err| panic!("seed metadata: {err}"))
        .modified()
        .unwrap_or_else(|err| panic!("seed modified time: {err}"));
    sleep(Duration::from_millis(scaled_test_timeout_ms(20))).await;

    let harness = DispatchHarness::new([
        MockBackendProfile {
            slug: FAST_SLUG,
            reply: "fast backend handled the request",
        },
        MockBackendProfile {
            slug: PREMIUM_SLUG,
            reply: "premium backend handled the request",
        },
    ]);

    let static_table = HashMap::from([
        (ModelTier::Fast, FAST_SLUG.to_string()),
        (ModelTier::Standard, PREMIUM_SLUG.to_string()),
        (ModelTier::Premium, PREMIUM_SLUG.to_string()),
    ]);
    let router = LinUCBRouter::new(vec![FAST_SLUG.to_string(), PREMIUM_SLUG.to_string()])
        .with_persist_path(&path)
        .with_static_table(static_table);

    let mut fast_ctx = default_ctx();
    fast_ctx.complexity = TaskComplexityBand::Fast;
    let mut premium_ctx = default_ctx();
    premium_ctx.complexity = TaskComplexityBand::Complex;

    assert_eq!(router.select_model(&fast_ctx).slug, FAST_SLUG);
    assert_eq!(router.select_model(&premium_ctx).slug, PREMIUM_SLUG);

    let first_fast = harness.dispatch(FAST_SLUG, tmp.path(), SEED).await;
    let first_premium = harness.dispatch(PREMIUM_SLUG, tmp.path(), SEED + 1).await;
    assert!(first_fast.contains("fast backend"));
    assert!(first_premium.contains("premium backend"));

    let mut rng = ChaCha8Rng::seed_from_u64(SEED);
    for _ in 0..TRAINING_ROUNDS {
        let selected_fast = router.select_model(&fast_ctx);
        let fast_text = harness
            .dispatch(&selected_fast.slug, tmp.path(), rng.next_u64())
            .await;
        assert!(fast_text.contains("backend"));
        router.update(&fast_ctx, &selected_fast.slug, 0.9);

        let selected_premium = router.select_model(&premium_ctx);
        let premium_text = harness
            .dispatch(&selected_premium.slug, tmp.path(), rng.next_u64())
            .await;
        assert!(premium_text.contains("backend"));
        let reward = if selected_premium.slug == PREMIUM_SLUG {
            0.85
        } else {
            0.25
        };
        router.update(&premium_ctx, &selected_premium.slug, reward);
    }

    assert_eq!(router.total_observations(), (TRAINING_ROUNDS * 2) as u64);
    assert_eq!(router.select_model(&fast_ctx).slug, FAST_SLUG);
    assert_eq!(router.select_model(&premium_ctx).slug, PREMIUM_SLUG);

    let metadata = std::fs::metadata(&path).unwrap_or_else(|err| panic!("router metadata: {err}"));
    let modified_after = metadata
        .modified()
        .unwrap_or_else(|err| panic!("router modified time: {err}"));
    assert!(modified_after > modified_before);

    let scratch_files = std::fs::read_dir(tmp.path())
        .unwrap_or_else(|err| panic!("list tempdir: {err}"))
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter_map(|name| name.into_string().ok())
        .filter(|name| name.starts_with(".linucb_tmp_"))
        .collect::<Vec<_>>();
    assert!(
        scratch_files.is_empty(),
        "leftover tmp files: {scratch_files:?}"
    );

    let persisted: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|err| panic!("read model router state: {err}")),
    )
    .unwrap_or_else(|err| panic!("parse model router state: {err}"));
    assert_eq!(
        persisted["total_observations"].as_u64(),
        Some((TRAINING_ROUNDS * 2) as u64)
    );
    assert_eq!(
        persisted["arms"].as_array().map(std::vec::Vec::len),
        Some(2)
    );
    assert_eq!(persisted["arms"][0]["slug"], FAST_SLUG);
    assert_eq!(persisted["arms"][1]["slug"], PREMIUM_SLUG);

    let reloaded = LinUCBRouter::load(&path, vec![FAST_SLUG.to_string(), PREMIUM_SLUG.to_string()])
        .unwrap_or_else(|err| panic!("reload persisted router: {err}"));
    assert_eq!(reloaded.total_observations(), (TRAINING_ROUNDS * 2) as u64);
    assert_eq!(reloaded.select_model(&fast_ctx).slug, FAST_SLUG);
    assert_eq!(reloaded.select_model(&premium_ctx).slug, PREMIUM_SLUG);
}
