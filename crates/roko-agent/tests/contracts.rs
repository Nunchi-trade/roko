//! Integration tests for dispatcher-enforced safety contracts.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use roko_agent::dispatcher::{HandlerResolver, ToolDispatcher};
use roko_agent::safety::SafetyLayer;
use roko_agent::safety::contract::{AgentContract, Invariant};
use roko_core::tool::{
    NeverCancel, NoopAuditSink, NoopMetricsSink, NoopTraceSink, ToolCall, ToolCategory,
    ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler, ToolPermission, ToolRegistry,
    ToolResult, VecToolRegistry,
};
use serde_json::json;

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

fn tool(name: &str, permission: ToolPermission, concurrency: ToolConcurrency) -> ToolDef {
    ToolDef::new(name, "contract test tool", ToolCategory::Meta, permission)
        .with_concurrency(concurrency)
}

fn resolver_for(entries: Vec<(&'static str, Arc<dyn ToolHandler>)>) -> Arc<dyn HandlerResolver> {
    Arc::new(move |name: &str| {
        entries
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, handler)| Arc::clone(handler))
    })
}

fn full_capability_ctx() -> ToolContext {
    ToolContext::new(
        "/tmp/ux26-contracts",
        Duration::from_secs(5),
        ToolPermission {
            read: true,
            write: true,
            exec: true,
            git: true,
            network: true,
        },
        Arc::new(NoopAuditSink),
        Arc::new(NoopTraceSink),
        Arc::new(NoopMetricsSink),
        Arc::new(NeverCancel),
    )
}

fn token_limit(contract: &AgentContract) -> u32 {
    contract
        .invariants
        .iter()
        .find_map(|invariant| match invariant {
            Invariant::MaxTokensPerTurn(limit) => Some(*limit),
            _ => None,
        })
        .expect("contract should define MaxTokensPerTurn")
}

fn assert_permission_denied(result: ToolResult, rule: &str) {
    match result {
        ToolResult::Err(ToolError::PermissionDenied(message)) => {
            assert!(message.contains(rule), "got: {message}");
        }
        other => panic!("expected permission denial for {rule}, got {other:?}"),
    }
}

#[tokio::test]
async fn implementer_exceeds_token_budget_is_rejected() {
    let contract = AgentContract::load_for_role("implementer").expect("load implementer contract");
    assert_eq!(contract.role, "implementer");
    let limit = token_limit(&contract);
    let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
        "bash",
        ToolPermission::executes(),
        ToolConcurrency::Serial,
    )]));
    let resolver = resolver_for(vec![(
        "bash",
        Arc::new(NoopHandler { tool_name: "bash" }) as Arc<dyn ToolHandler>,
    )]);
    let dispatcher = ToolDispatcher::new(registry, resolver)
        .with_safety(SafetyLayer::with_defaults().with_contract(contract));

    let call = ToolCall::new(
        "implementer-over-budget",
        "bash",
        json!({
            "command": "echo ready",
            "estimated_tokens": limit + 1,
        }),
    );
    let result = dispatcher.dispatch(call, &full_capability_ctx()).await;

    assert_permission_denied(result, "MaxTokensPerTurn");
}

#[tokio::test]
async fn reviewer_blocked_on_network_tool() {
    let contract = AgentContract::load_for_role("reviewer").expect("load reviewer contract");
    assert_eq!(contract.role, "reviewer");
    let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
        "web_fetch",
        ToolPermission::networked(),
        ToolConcurrency::Parallel,
    )]));
    let resolver = resolver_for(vec![(
        "web_fetch",
        Arc::new(NoopHandler {
            tool_name: "web_fetch",
        }) as Arc<dyn ToolHandler>,
    )]);
    let dispatcher = ToolDispatcher::new(registry, resolver)
        .with_safety(SafetyLayer::with_defaults().with_contract(contract));

    let call = ToolCall::new(
        "reviewer-network",
        "web_fetch",
        json!({ "url": "https://example.com/research" }),
    );
    let result = dispatcher.dispatch(call, &full_capability_ctx()).await;

    assert_permission_denied(result, "NoNetworkAccess");
}

#[tokio::test]
async fn researcher_allowed_network() {
    let contract = AgentContract::load_for_role("researcher").expect("load researcher contract");
    assert_eq!(contract.role, "researcher");
    let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
        "web_fetch",
        ToolPermission::networked(),
        ToolConcurrency::Parallel,
    )]));
    let resolver = resolver_for(vec![(
        "web_fetch",
        Arc::new(NoopHandler {
            tool_name: "web_fetch",
        }) as Arc<dyn ToolHandler>,
    )]);
    let dispatcher = ToolDispatcher::new(registry, resolver)
        .with_safety(SafetyLayer::with_defaults().with_contract(contract));

    let call = ToolCall::new(
        "researcher-network",
        "web_fetch",
        json!({ "url": "https://example.com/research" }),
    );
    let result = dispatcher.dispatch(call, &full_capability_ctx()).await;
    assert!(
        result.is_ok(),
        "researcher should keep network access, got {result:?}"
    );
}

#[tokio::test]
async fn no_contract_means_permissive_default() {
    assert!(
        AgentContract::load_for_role("does-not-exist").is_err(),
        "missing assets should not load"
    );
    let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
        "bash",
        ToolPermission::executes(),
        ToolConcurrency::Serial,
    )]));
    let resolver = resolver_for(vec![(
        "bash",
        Arc::new(NoopHandler { tool_name: "bash" }) as Arc<dyn ToolHandler>,
    )]);
    let dispatcher = ToolDispatcher::new(registry, resolver)
        .with_safety(SafetyLayer::with_defaults().with_role("does-not-exist"));

    let safety = dispatcher
        .safety()
        .expect("dispatcher should retain safety");
    assert_eq!(safety.contract.role, "does-not-exist");
    assert!(safety.contract.invariants.is_empty());
    assert!(safety.contract.governance.is_empty());
    assert!(safety.contract.recovery.is_empty());

    let call = ToolCall::new(
        "permissive-default",
        "bash",
        json!({
            "command": "echo ready",
            "estimated_tokens": 50_000_u32,
        }),
    );
    let result = dispatcher.dispatch(call, &full_capability_ctx()).await;
    assert!(
        result.is_ok(),
        "permissive fallback should allow call, got {result:?}"
    );
}
