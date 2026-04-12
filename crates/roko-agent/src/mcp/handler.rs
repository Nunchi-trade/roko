//! MCP-backed tool handlers for the dispatcher.
//!
//! MCP-discovered tools are registered under `{server}__{tool}` names via
//! [`super::mcp_to_tool_def`]. [`McpHandlerResolver`] uses that prefix to route
//! dynamic tool calls to the correct live MCP client while still deferring to a
//! static resolver for built-in tools first.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use roko_core::tool::{ToolCall, ToolContext, ToolError, ToolHandler, ToolResult};

use super::client::{McpContent, McpToolResult};
use super::{McpClient, Transport};
use crate::dispatcher::HandlerResolver;

const MCP_TOOL_SEPARATOR: &str = "__";

/// Dispatcher resolver that falls back from built-in handlers to live MCP
/// clients for namespaced MCP tools.
pub struct McpHandlerResolver<T: Transport> {
    static_resolver: Arc<dyn HandlerResolver>,
    mcp_clients: HashMap<String, Arc<McpClient<T>>>,
}

impl<T: Transport> McpHandlerResolver<T> {
    /// Create a resolver that checks `static_resolver` first, then resolves MCP
    /// tools using the `{server}__{tool}` naming convention.
    #[must_use]
    pub fn new(
        static_resolver: Arc<dyn HandlerResolver>,
        mcp_clients: HashMap<String, Arc<McpClient<T>>>,
    ) -> Self {
        Self {
            static_resolver,
            mcp_clients,
        }
    }
}

impl<T: Transport + 'static> HandlerResolver for McpHandlerResolver<T> {
    fn resolve(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        if let Some(handler) = self.static_resolver.resolve(name) {
            return Some(handler);
        }

        let (server_name, remote_name) = split_prefixed_tool_name(name)?;
        let client = self.mcp_clients.get(server_name)?;

        Some(Arc::new(McpToolHandler::new(
            Arc::clone(client),
            name.to_string(),
            remote_name.to_string(),
        )))
    }
}

/// Concrete [`ToolHandler`] that executes a routed MCP tool via
/// `tools/call`.
pub struct McpToolHandler<T: Transport> {
    client: Arc<McpClient<T>>,
    exposed_name: String,
    remote_name: String,
}

impl<T: Transport> McpToolHandler<T> {
    /// Construct a handler for one discovered MCP tool.
    #[must_use]
    pub fn new(
        client: Arc<McpClient<T>>,
        exposed_name: impl Into<String>,
        remote_name: impl Into<String>,
    ) -> Self {
        Self {
            client,
            exposed_name: exposed_name.into(),
            remote_name: remote_name.into(),
        }
    }
}

#[async_trait]
impl<T: Transport + 'static> ToolHandler for McpToolHandler<T> {
    fn name(&self) -> &str {
        &self.exposed_name
    }

    async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
        match self
            .client
            .call_tool(&self.remote_name, call.arguments)
            .await
        {
            Ok(result) => render_mcp_result(&self.exposed_name, result),
            Err(err) => ToolResult::err(ToolError::Other(format!(
                "mcp tool `{}` failed: {err}",
                self.exposed_name
            ))),
        }
    }
}

fn split_prefixed_tool_name(name: &str) -> Option<(&str, &str)> {
    let (server_name, remote_name) = name.split_once(MCP_TOOL_SEPARATOR)?;
    if server_name.is_empty() || remote_name.is_empty() {
        return None;
    }
    Some((server_name, remote_name))
}

fn render_mcp_result(tool_name: &str, result: McpToolResult) -> ToolResult {
    let content = mcp_result_text(&result.content);
    if result.is_error {
        let message = if content.is_empty() {
            format!("mcp tool `{tool_name}` returned an error")
        } else {
            format!("mcp tool `{tool_name}` returned an error: {content}")
        };
        ToolResult::err(ToolError::Other(message))
    } else {
        ToolResult::text(content)
    }
}

fn mcp_result_text(content: &[McpContent]) -> String {
    let text_blocks: Vec<&str> = content
        .iter()
        .filter_map(|block| (block.content_type == "text").then_some(block.text.as_deref()))
        .flatten()
        .collect();

    if !text_blocks.is_empty() {
        return text_blocks.join("\n");
    }

    serde_json::to_string(content).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatcher::ToolDispatcher;
    use crate::mcp::{McpRequest, McpResponse, McpToolDef, mcp_to_tool_def};
    use roko_core::tool::{ToolRegistry, VecToolRegistry};
    use serde_json::json;
    use std::sync::Mutex;

    struct MockTransport {
        responses: Mutex<Vec<McpResponse>>,
        requests: Mutex<Vec<McpRequest>>,
    }

    impl MockTransport {
        fn new(responses: Vec<McpResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn take_requests(&self) -> Vec<McpRequest> {
            self.requests.lock().unwrap().drain(..).collect()
        }
    }

    #[async_trait]
    impl Transport for Arc<MockTransport> {
        async fn roundtrip(
            &self,
            request: &McpRequest,
        ) -> Result<McpResponse, super::super::client::McpError> {
            self.requests.lock().unwrap().push(request.clone());
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(super::super::client::McpError::Transport(
                    "no more canned responses".into(),
                ));
            }
            Ok(responses.remove(0))
        }
    }

    struct StaticHandler;

    #[async_trait]
    impl ToolHandler for StaticHandler {
        fn name(&self) -> &str {
            "local__echo"
        }

        async fn execute(&self, _call: ToolCall, _ctx: &ToolContext) -> ToolResult {
            ToolResult::text("static")
        }
    }

    fn ok_response(id: u64, result: serde_json::Value) -> McpResponse {
        McpResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    #[tokio::test]
    async fn mcp_handler_resolver_routes_calls_through_dispatcher() {
        let transport = Arc::new(MockTransport::new(vec![ok_response(
            1,
            json!({
                "content": [{"type": "text", "text": "hello from mcp"}],
                "isError": false
            }),
        )]));
        let client = Arc::new(McpClient::new(Arc::clone(&transport)));
        let registry = Arc::new(VecToolRegistry::from_tools(vec![mcp_to_tool_def(
            &McpToolDef {
                name: "echo".to_string(),
                description: Some("Echo".to_string()),
                input_schema: Some(json!({
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"]
                })),
            },
            "local",
        )])) as Arc<dyn ToolRegistry>;

        let resolver: Arc<dyn HandlerResolver> = Arc::new(McpHandlerResolver::new(
            Arc::new(|_: &str| None),
            HashMap::from([("local".to_string(), client)]),
        ));
        let dispatcher = ToolDispatcher::new(Arc::clone(&registry), resolver);

        let result = dispatcher
            .dispatch(
                ToolCall::new("call-1", "local__echo", json!({"text": "hello"})),
                &ToolContext::testing("/tmp/mcp-handler-resolver"),
            )
            .await;

        assert_eq!(result, ToolResult::text("hello from mcp"));

        let requests = transport.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "tools/call");
        assert_eq!(requests[0].params["name"], "echo");
        assert_eq!(requests[0].params["arguments"], json!({"text": "hello"}));
    }

    #[test]
    fn mcp_handler_resolver_prefers_static_handlers() {
        let transport = Arc::new(MockTransport::new(Vec::new()));
        let client = Arc::new(McpClient::new(transport));
        let resolver = McpHandlerResolver::new(
            Arc::new(|name: &str| {
                (name == "local__echo").then(|| Arc::new(StaticHandler) as Arc<dyn ToolHandler>)
            }),
            HashMap::from([("local".to_string(), client)]),
        );

        let handler = resolver.resolve("local__echo").expect("handler");
        assert_eq!(handler.name(), "local__echo");
    }

    #[test]
    fn mcp_handler_resolver_returns_none_for_unknown_server() {
        let resolver: McpHandlerResolver<Arc<MockTransport>> =
            McpHandlerResolver::new(Arc::new(|_: &str| None), HashMap::new());
        assert!(resolver.resolve("missing__echo").is_none());
        assert!(resolver.resolve("not-prefixed").is_none());
    }
}
