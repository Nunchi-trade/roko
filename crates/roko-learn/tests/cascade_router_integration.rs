//! Integration coverage for cascade routing with a real tool dispatcher and
//! deterministic mock backends.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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
use roko_core::BehavioralState;
use roko_core::DaimonPolicy;
use roko_core::agent::AgentRole;
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolHandler, ToolPermission,
    ToolRegistry, ToolResult, VecToolRegistry,
};
use roko_learn::cascade_router::{CascadeRouter, CascadeStage};
use roko_learn::model_router::RoutingContext;
use tempfile::TempDir;
use tokio::time::sleep;

const SEED: u64 = 34_034;
const CHEAP_SLUG: &str = "claude-haiku-3-5";
const EXPENSIVE_SLUG: &str = "claude-opus-4";
const TURN_COUNT: usize = 60;

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
    reward: f64,
    success: bool,
}

struct RoutedOutcome {
    reward: f64,
    success: bool,
    final_text: String,
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
    first_turns: AtomicUsize,
}

impl MockBackend {
    const fn new(profile: MockBackendProfile) -> Self {
        Self {
            profile,
            first_turns: AtomicUsize::new(0),
        }
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
            self.first_turns.fetch_add(1, Ordering::SeqCst);
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
                                    "text": format!("probe-{}", self.profile.slug),
                                }).to_string(),
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {
                    "prompt_tokens": 8,
                    "completion_tokens": 3,
                    "total_tokens": 11
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
                "prompt_tokens": 6,
                "completion_tokens": 4,
                "total_tokens": 10
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
        let echo_tool = ToolDef::new(
            "echo",
            "return the supplied text",
            ToolCategory::Meta,
            ToolPermission::read_only(),
        )
        .with_concurrency(ToolConcurrency::Serial);
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

    async fn dispatch(&self, slug: &str, worktree: &Path, turn_nonce: u64) -> RoutedOutcome {
        let backend = self
            .backends
            .get(slug)
            .cloned()
            .unwrap_or_else(|| panic!("missing backend for slug {slug}"));
        let loop_runner = ToolLoop::new(
            Arc::clone(&self.translator),
            Arc::clone(&self.dispatcher),
            backend.clone(),
        )
        .with_max_iterations(3);

        let result = loop_runner
            .run(
                "Route one deterministic test turn.",
                &format!("dispatch-{turn_nonce}"),
                &[echo_tool()],
                &ToolContext::testing(worktree),
            )
            .await;

        assert_eq!(result.stop_reason, StopReason::Stop);
        assert_eq!(result.tool_calls.len(), 1);

        RoutedOutcome {
            reward: backend.profile.reward,
            success: backend.profile.success,
            final_text: result.final_text,
        }
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
        daimon_policy: DaimonPolicy::new(0.5, BehavioralState::Engaged),
        thinking_level: None,
        temperament: None,
        previous_model: None,
        plan_context_tokens: None,
    }
}

#[tokio::test]
async fn cascade_prefers_the_cheap_backend_after_confidence_learning() {
    let tmp = TempDir::new().unwrap_or_else(|err| panic!("tempdir: {err}"));
    let path = tmp.path().join("cascade-router.json");
    std::fs::write(&path, "{\"stale\":true}")
        .unwrap_or_else(|err| panic!("seed stale cascade file: {err}"));
    let modified_before = std::fs::metadata(&path)
        .unwrap_or_else(|err| panic!("seed file metadata: {err}"))
        .modified()
        .unwrap_or_else(|err| panic!("seed file modified time: {err}"));
    sleep(Duration::from_millis(scaled_test_timeout_ms(20))).await;

    let harness = DispatchHarness::new([
        MockBackendProfile {
            slug: EXPENSIVE_SLUG,
            reply: "expensive backend completed but missed the gate",
            reward: 0.05,
            success: false,
        },
        MockBackendProfile {
            slug: CHEAP_SLUG,
            reply: "cheap backend completed cleanly",
            reward: 0.95,
            success: true,
        },
    ]);

    let mut role_table = HashMap::new();
    role_table.insert(AgentRole::Implementer, EXPENSIVE_SLUG.to_string());
    let router = CascadeRouter::new(vec![CHEAP_SLUG.to_string(), EXPENSIVE_SLUG.to_string()])
        .with_role_table(role_table);
    let ctx = default_ctx();

    assert_eq!(router.route(&ctx).stage, CascadeStage::Static);
    assert_eq!(router.route(&ctx).primary.slug, EXPENSIVE_SLUG);

    let mut rng = ChaCha8Rng::seed_from_u64(SEED);
    for _ in 0..TURN_COUNT {
        let selected = router.route(&ctx);
        let outcome = harness
            .dispatch(&selected.primary.slug, tmp.path(), rng.next_u64())
            .await;
        assert!(
            outcome.final_text.contains("backend"),
            "unexpected final text: {}",
            outcome.final_text
        );
        router.record_observation(
            &ctx,
            &selected.primary.slug,
            outcome.reward,
            outcome.success,
        );
    }

    let learned = router.route(&ctx);
    assert_eq!(learned.stage, CascadeStage::Confidence);
    assert_eq!(learned.primary.slug, CHEAP_SLUG);
    assert_eq!(router.current_stage(), CascadeStage::Confidence);
    assert_eq!(router.total_observations(), TURN_COUNT as u64);

    let transitions = router.stage_transitions();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].from, CascadeStage::Static);
    assert_eq!(transitions[0].to, CascadeStage::Confidence);
    assert_eq!(transitions[0].observations, 50);

    router
        .save(&path)
        .unwrap_or_else(|err| panic!("save cascade router: {err}"));

    let metadata = std::fs::metadata(&path).unwrap_or_else(|err| panic!("saved metadata: {err}"));
    let modified_after = metadata
        .modified()
        .unwrap_or_else(|err| panic!("saved file modified time: {err}"));
    assert!(modified_after > modified_before);
    assert!(!path.with_extension("json.tmp").exists());

    let persisted: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|err| panic!("read cascade state: {err}")),
    )
    .unwrap_or_else(|err| panic!("parse cascade state: {err}"));
    assert_eq!(
        persisted["total_observations"].as_u64(),
        Some(TURN_COUNT as u64)
    );
    assert_eq!(
        persisted["stage_transitions"]
            .as_array()
            .map(std::vec::Vec::len),
        Some(1)
    );
    assert_eq!(
        persisted["model_slugs"].as_array().map(std::vec::Vec::len),
        Some(2)
    );

    let reloaded = CascadeRouter::load_or_new(
        &path,
        vec![CHEAP_SLUG.to_string(), EXPENSIVE_SLUG.to_string()],
    );
    assert_eq!(reloaded.current_stage(), CascadeStage::Confidence);
    assert_eq!(reloaded.total_observations(), TURN_COUNT as u64);
    assert_eq!(reloaded.route(&ctx).primary.slug, CHEAP_SLUG);
    assert_eq!(reloaded.confidence_snapshot(), router.confidence_snapshot());
    assert_eq!(reloaded.stage_transitions(), transitions);
}
