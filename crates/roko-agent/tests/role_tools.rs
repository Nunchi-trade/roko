//! Integration tests for role-scoped tool whitelist enforcement.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use roko_agent::dispatcher::{HandlerResolver, ToolDispatcher};
use roko_agent::safety::SafetyLayer;
use roko_agent::{MANIFEST_BACKED_BUILTIN_ROLE_IDS, RolePolicyManifest};
use roko_core::config::schema::RokoConfig;
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

fn tool(name: &str, permission: ToolPermission) -> ToolDef {
    ToolDef::new(name, "role tools test tool", ToolCategory::Meta, permission)
        .with_concurrency(ToolConcurrency::Serial)
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
        "/tmp/ux27-role-tools",
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

fn dispatcher_for(
    role: &str,
    config_toml: &str,
    tool_name: &'static str,
    permission: ToolPermission,
) -> ToolDispatcher {
    let config = RokoConfig::from_toml(config_toml).expect("parse role config");
    let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
        tool_name, permission,
    )]));
    let resolver = resolver_for(vec![(
        tool_name,
        Arc::new(NoopHandler { tool_name }) as Arc<dyn ToolHandler>,
    )]);
    let layer = SafetyLayer::from_config(&config).with_role(role);
    ToolDispatcher::new(registry, resolver).with_safety(layer)
}

#[tokio::test]
async fn explicit_allow_permits_listed_tool() {
    let dispatcher = dispatcher_for(
        "sheller",
        r#"
[agent.roles.sheller]
model = "mock"
tools = ["bash"]
"#,
        "bash",
        ToolPermission::executes(),
    );

    let result = dispatcher
        .dispatch(
            ToolCall::new("allow-sheller", "bash", json!({ "command": "echo ready" })),
            &full_capability_ctx(),
        )
        .await;

    assert!(
        result.is_ok(),
        "expected explicit allow to pass, got {result:?}"
    );
}

#[tokio::test]
async fn explicit_deny_blocks_unlisted_tool() {
    let dispatcher = dispatcher_for(
        "reader",
        r#"
[agent.roles.reader]
model = "mock"
tools = ["read_file"]
"#,
        "bash",
        ToolPermission::executes(),
    );

    let result = dispatcher
        .dispatch(
            ToolCall::new("deny-reader", "bash", json!({ "command": "echo ready" })),
            &full_capability_ctx(),
        )
        .await;

    match result {
        ToolResult::Err(ToolError::PermissionDenied(message)) => {
            assert!(message.contains("reader"), "got: {message}");
            assert!(message.contains("bash"), "got: {message}");
        }
        other => panic!("expected permission denial, got {other:?}"),
    }
}

#[tokio::test]
async fn glob_allow_matches_git_star() {
    let dispatcher = dispatcher_for(
        "git-only",
        r#"
[agent.roles.git-only]
model = "mock"
tools = ["git-*"]
"#,
        "git-commit",
        ToolPermission::git_ops(),
    );

    let result = dispatcher
        .dispatch(
            ToolCall::new("allow-git-star", "git-commit", json!({})),
            &full_capability_ctx(),
        )
        .await;

    assert!(
        result.is_ok(),
        "expected git-* glob to match, got {result:?}"
    );
}

#[tokio::test]
async fn no_whitelist_keeps_role_permissive() {
    let dispatcher = dispatcher_for(
        "adhoc",
        r#"
[agent.roles.adhoc]
model = "mock"
"#,
        "bash",
        ToolPermission::executes(),
    );

    let result = dispatcher
        .dispatch(
            ToolCall::new("allow-adhoc", "bash", json!({ "command": "echo ready" })),
            &full_capability_ctx(),
        )
        .await;

    assert!(
        result.is_ok(),
        "expected missing tools whitelist to stay permissive, got {result:?}"
    );
}

#[test]
fn bundled_builtin_role_manifest_loads_without_legacy_prompt_sources() {
    let manifest = RolePolicyManifest::builtin_manifest().expect("built-in role manifest");

    for role_id in MANIFEST_BACKED_BUILTIN_ROLE_IDS {
        let (role, policy) = manifest
            .role_with_default_prompt_policy(role_id)
            .expect("manifest role policy");

        assert_eq!(role.default_prompt_policy, policy.policy_id);
        assert!(!role.version.trim().is_empty());
        assert!(
            policy
                .sections
                .iter()
                .any(|section| section.section_id == "role_identity"
                    && section.source.kind == "manifest"),
            "{role_id} should use a manifest role_identity section"
        );

        let rendered = format!("{role:?}\n{policy:?}").to_lowercase();
        assert!(!rendered.contains("mori"), "{role_id} leaked Mori source");
        assert!(!rendered.contains("bardo"), "{role_id} leaked Bardo source");
    }
}
