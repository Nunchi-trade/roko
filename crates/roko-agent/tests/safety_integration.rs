//! Integration tests: the safety layer wired into the dispatcher pipeline.
//!
//! Each test constructs a `ToolDispatcher` with a `SafetyLayer`, then
//! dispatches a tool call that should be blocked. The handler is a no-op
//! echo — the point is that the dispatcher never reaches it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use roko_agent::dispatcher::{HandlerResolver, ToolDispatcher};
use roko_agent::safety::SafetyLayer;
use roko_agent::safety::rate_limit::{RateLimitPolicy, RateLimiter};
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, VecToolRegistry,
};

// ─── Mock handler (never reached on blocked calls) ───────────────────────

struct NoopHandler {
    tool_name: &'static str,
}

#[async_trait]
impl ToolHandler for NoopHandler {
    fn name(&self) -> &str {
        self.tool_name
    }
    async fn execute(&self, _call: ToolCall, _ctx: &ToolContext) -> ToolResult {
        ToolResult::text("handler reached")
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

fn tool(name: &str, perm: ToolPermission, conc: ToolConcurrency) -> ToolDef {
    ToolDef::new(name, "test tool", ToolCategory::Meta, perm).with_concurrency(conc)
}

fn resolver_from(entries: Vec<(&'static str, Arc<dyn ToolHandler>)>) -> Arc<dyn HandlerResolver> {
    Arc::new(move |name: &str| {
        entries
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, h)| Arc::clone(h))
    })
}

fn ctx_with_exec() -> ToolContext {
    ToolContext::new(
        "/tmp",
        Duration::from_secs(5),
        ToolPermission {
            read: true,
            write: true,
            exec: true,
            git: true,
            network: true,
        },
        Arc::new(roko_core::tool::NoopAuditSink),
        Arc::new(roko_core::tool::NoopTraceSink),
        Arc::new(roko_core::tool::NoopMetricsSink),
        Arc::new(roko_core::tool::NeverCancel),
    )
}

// ─── Test 1: bash `rm -rf /` blocked by dispatcher ──────────────────────

#[tokio::test]
async fn bash_rm_rf_blocked_by_dispatcher() {
    let registry: Arc<dyn roko_core::tool::ToolRegistry> =
        Arc::new(VecToolRegistry::from_tools(vec![tool(
            "bash",
            ToolPermission::executes(),
            ToolConcurrency::Serial,
        )]));
    let resolver = resolver_from(vec![(
        "bash",
        Arc::new(NoopHandler { tool_name: "bash" }) as Arc<dyn ToolHandler>,
    )]);

    let layer = SafetyLayer::with_defaults();
    let dispatcher = ToolDispatcher::new(registry, resolver).with_safety(layer);

    let call = ToolCall::new("c1", "bash", serde_json::json!({ "command": "rm -rf /" }));
    let result = dispatcher.dispatch(call, &ctx_with_exec()).await;

    match result {
        ToolResult::Err(ToolError::CommandNotAllowed(msg)) => {
            assert!(
                msg.contains("rm -rf /"),
                "error should mention the denied pattern, got: {msg}"
            );
        }
        other => panic!("expected CommandNotAllowed, got {other:?}"),
    }
}

#[tokio::test]
async fn run_tests_rm_rf_blocked_by_dispatcher() {
    let registry: Arc<dyn roko_core::tool::ToolRegistry> =
        Arc::new(VecToolRegistry::from_tools(vec![tool(
            "run_tests",
            ToolPermission::executes(),
            ToolConcurrency::Serial,
        )]));
    let resolver = resolver_from(vec![(
        "run_tests",
        Arc::new(NoopHandler {
            tool_name: "run_tests",
        }) as Arc<dyn ToolHandler>,
    )]);

    let layer = SafetyLayer::with_defaults();
    let dispatcher = ToolDispatcher::new(registry, resolver).with_safety(layer);

    let call = ToolCall::new(
        "c1-run-tests",
        "run_tests",
        serde_json::json!({ "command": "rm -rf /" }),
    );
    let result = dispatcher.dispatch(call, &ctx_with_exec()).await;

    match result {
        ToolResult::Err(ToolError::CommandNotAllowed(msg)) => {
            assert!(
                msg.contains("rm -rf /"),
                "error should mention the denied pattern, got: {msg}"
            );
        }
        other => panic!("expected CommandNotAllowed, got {other:?}"),
    }
}

// ─── Test 2: network RFC1918 blocked ─────────────────────────────────────

#[tokio::test]
async fn network_rfc1918_blocked() {
    let registry: Arc<dyn roko_core::tool::ToolRegistry> =
        Arc::new(VecToolRegistry::from_tools(vec![tool(
            "web_fetch",
            ToolPermission::networked(),
            ToolConcurrency::Parallel,
        )]));
    let resolver = resolver_from(vec![(
        "web_fetch",
        Arc::new(NoopHandler {
            tool_name: "web_fetch",
        }) as Arc<dyn ToolHandler>,
    )]);

    let layer = SafetyLayer::with_defaults();
    let dispatcher = ToolDispatcher::new(registry, resolver).with_safety(layer);

    // HTTP scheme to a private IP: blocked both by scheme and by private-network policy.
    let call = ToolCall::new(
        "c2",
        "web_fetch",
        serde_json::json!({ "url": "http://192.168.1.1" }),
    );
    let result = dispatcher.dispatch(call, &ctx_with_exec()).await;

    match result {
        ToolResult::Err(ToolError::NetworkBlocked(msg)) => {
            assert!(
                msg.contains("scheme") || msg.contains("private"),
                "error should mention scheme or private network, got: {msg}"
            );
        }
        other => panic!("expected NetworkBlocked, got {other:?}"),
    }

    // HTTPS to a private IP: blocked by private-network policy.
    let registry2: Arc<dyn roko_core::tool::ToolRegistry> =
        Arc::new(VecToolRegistry::from_tools(vec![tool(
            "web_fetch",
            ToolPermission::networked(),
            ToolConcurrency::Parallel,
        )]));
    let resolver2 = resolver_from(vec![(
        "web_fetch",
        Arc::new(NoopHandler {
            tool_name: "web_fetch",
        }) as Arc<dyn ToolHandler>,
    )]);
    let layer2 = SafetyLayer::with_defaults();
    let dispatcher2 = ToolDispatcher::new(registry2, resolver2).with_safety(layer2);

    let call2 = ToolCall::new(
        "c3",
        "web_fetch",
        serde_json::json!({ "url": "https://192.168.1.1/admin" }),
    );
    let result2 = dispatcher2.dispatch(call2, &ctx_with_exec()).await;

    match result2 {
        ToolResult::Err(ToolError::NetworkBlocked(msg)) => {
            assert!(
                msg.contains("private"),
                "error should mention private network, got: {msg}"
            );
        }
        other => panic!("expected NetworkBlocked for HTTPS to private IP, got {other:?}"),
    }
}

// ─── Test 3: git force-push blocked ──────────────────────────────────────

#[tokio::test]
async fn git_force_push_blocked() {
    let registry: Arc<dyn roko_core::tool::ToolRegistry> =
        Arc::new(VecToolRegistry::from_tools(vec![tool(
            "bash",
            ToolPermission::executes(),
            ToolConcurrency::Serial,
        )]));
    let resolver = resolver_from(vec![(
        "bash",
        Arc::new(NoopHandler { tool_name: "bash" }) as Arc<dyn ToolHandler>,
    )]);

    let layer = SafetyLayer::with_defaults();
    let dispatcher = ToolDispatcher::new(registry, resolver).with_safety(layer);

    let call = ToolCall::new(
        "c4",
        "bash",
        serde_json::json!({ "command": "git push --force origin main" }),
    );
    let result = dispatcher.dispatch(call, &ctx_with_exec()).await;

    match result {
        ToolResult::Err(ToolError::CommandNotAllowed(msg)) => {
            assert!(
                msg.contains("block_force_push"),
                "error should reference the git policy rule, got: {msg}"
            );
        }
        other => panic!("expected CommandNotAllowed for git force push, got {other:?}"),
    }
}

// ─── Test 4: rate limit exceeded ─────────────────────────────────────────

#[tokio::test]
async fn rate_limit_exceeded() {
    let registry: Arc<dyn roko_core::tool::ToolRegistry> =
        Arc::new(VecToolRegistry::from_tools(vec![tool(
            "bash",
            ToolPermission::executes(),
            ToolConcurrency::Serial,
        )]));
    let resolver = resolver_from(vec![(
        "bash",
        Arc::new(NoopHandler { tool_name: "bash" }) as Arc<dyn ToolHandler>,
    )]);

    let tight_limiter = RateLimiter::new(RateLimitPolicy {
        max_calls_per_window: 3,
        window_duration: Duration::from_secs(60),
    });

    let mut layer = SafetyLayer::with_defaults();
    layer.rate_limiter = Some(Arc::new(tight_limiter));
    let dispatcher = ToolDispatcher::new(registry, resolver).with_safety(layer);
    let ctx = ctx_with_exec();

    // First 3 calls succeed (cap = 3).
    for i in 0..3 {
        let call = ToolCall::new(
            format!("ok-{i}"),
            "bash",
            serde_json::json!({ "command": "echo hello" }),
        );
        let res = dispatcher.dispatch(call, &ctx).await;
        assert!(res.is_ok(), "call {i} should succeed, got {res:?}");
    }

    // 4th call should be blocked by rate limiter.
    let call = ToolCall::new(
        "blocked",
        "bash",
        serde_json::json!({ "command": "echo hello" }),
    );
    let result = dispatcher.dispatch(call, &ctx).await;

    match result {
        ToolResult::Err(ToolError::Other(msg)) => {
            assert!(
                msg.contains("rate limit"),
                "error should mention rate limit, got: {msg}"
            );
        }
        other => panic!("expected Other(rate limit), got {other:?}"),
    }
}

// ─── Test 5: safe calls pass through when all policies are active ────────

#[tokio::test]
async fn safe_calls_pass_through_with_all_policies() {
    let registry: Arc<dyn roko_core::tool::ToolRegistry> =
        Arc::new(VecToolRegistry::from_tools(vec![tool(
            "bash",
            ToolPermission::executes(),
            ToolConcurrency::Serial,
        )]));
    let resolver = resolver_from(vec![(
        "bash",
        Arc::new(NoopHandler { tool_name: "bash" }) as Arc<dyn ToolHandler>,
    )]);

    let layer = SafetyLayer::with_defaults();
    let dispatcher = ToolDispatcher::new(registry, resolver).with_safety(layer);

    let call = ToolCall::new("safe", "bash", serde_json::json!({ "command": "ls -la" }));
    let result = dispatcher.dispatch(call, &ctx_with_exec()).await;
    assert!(
        result.is_ok(),
        "safe command should pass all safety checks, got {result:?}"
    );
}

// ─── Test 6: no safety policy = backward-compatible pass-through ─────────

#[tokio::test]
async fn no_policy_means_pass_through() {
    let registry: Arc<dyn roko_core::tool::ToolRegistry> =
        Arc::new(VecToolRegistry::from_tools(vec![tool(
            "bash",
            ToolPermission::executes(),
            ToolConcurrency::Serial,
        )]));
    let resolver = resolver_from(vec![(
        "bash",
        Arc::new(NoopHandler { tool_name: "bash" }) as Arc<dyn ToolHandler>,
    )]);

    // No safety layer attached: calls pass through for backward compatibility.
    let dispatcher = ToolDispatcher::new(registry, resolver);

    // This dangerous command would be blocked with policies, but passes
    // through without them (backward compatibility).
    let call = ToolCall::new(
        "danger",
        "bash",
        serde_json::json!({ "command": "rm -rf /" }),
    );
    let result = dispatcher.dispatch(call, &ctx_with_exec()).await;
    assert!(
        result.is_ok(),
        "without safety policy, even dangerous calls reach the handler, got {result:?}"
    );
}
