//! ACP protocol types (JSON-RPC messages, session types, update types).

use serde::{Deserialize, Serialize};

/// ACP protocol version supported by this crate.
pub const ACP_PROTOCOL_VERSION: u32 = 1;

/// ACP specification version implemented by this crate.
pub const ACP_SPEC_VERSION: &str = "0.12.2";

/// JSON-RPC parse error code.
pub const PARSE_ERROR: i32 = -32700;

/// JSON-RPC invalid request error code.
pub const INVALID_REQUEST: i32 = -32600;

/// JSON-RPC method not found error code.
pub const METHOD_NOT_FOUND: i32 = -32601;

/// JSON-RPC invalid params error code.
pub const INVALID_PARAMS: i32 = -32602;

/// JSON-RPC internal error code.
pub const INTERNAL_ERROR: i32 = -32603;

/// ACP session not found error code.
pub const SESSION_NOT_FOUND: i32 = -32000;

/// ACP session busy error code.
pub const SESSION_BUSY: i32 = -32001;

/// A JSON-RPC 2.0 message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    /// A JSON-RPC request.
    Request(JsonRpcRequest),
    /// A JSON-RPC response.
    Response(JsonRpcResponse),
    /// A JSON-RPC notification.
    Notification(JsonRpcNotification),
}

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonRpcRequest {
    /// JSON-RPC protocol version, always `"2.0"`.
    pub jsonrpc: String,
    /// Request identifier.
    pub id: JsonRpcId,
    /// Request method name.
    pub method: String,
    /// Optional request parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// A JSON-RPC 2.0 response identifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    /// Numeric request identifier.
    Number(u64),
    /// String request identifier.
    String(String),
    /// Null identifier used for parse-level failures before an id is known.
    Null,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonRpcResponse {
    /// JSON-RPC protocol version, always `"2.0"`.
    pub jsonrpc: String,
    /// Response identifier matching the originating request.
    pub id: JsonRpcId,
    /// Successful result payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error payload when the request failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonRpcError {
    /// Error code.
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured error data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// A JSON-RPC 2.0 notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonRpcNotification {
    /// JSON-RPC protocol version, always `"2.0"`.
    pub jsonrpc: String,
    /// Notification method name.
    pub method: String,
    /// Optional notification parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// Parameters for the `initialize` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    /// ACP protocol version requested by the client.
    pub protocol_version: u32,
    /// Client feature declarations.
    #[serde(default)]
    pub client_capabilities: ClientCapabilities,
    /// Client identity metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_info: Option<ClientInfo>,
}

/// Capabilities reported by an ACP client.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    /// Filesystem bridge support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs: Option<FsCapabilities>,
    /// Terminal bridge support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<bool>,
    /// MCP server support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<bool>,
}

/// Filesystem capabilities reported by a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsCapabilities {
    /// Whether `fs/read_text_file` is supported.
    #[serde(default)]
    pub read_text_file: bool,
    /// Whether `fs/write_text_file` is supported.
    #[serde(default)]
    pub write_text_file: bool,
}

/// Metadata describing the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    /// Stable client name.
    pub name: String,
    /// Optional client version string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Optional human-friendly client title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Result returned from the `initialize` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// ACP protocol version negotiated for the session.
    pub protocol_version: u32,
    /// Capabilities supported by the agent.
    pub agent_capabilities: AgentCapabilities,
    /// Agent identity metadata.
    pub agent_info: AgentInfo,
    /// Supported authentication methods.
    #[serde(default)]
    pub auth_methods: Vec<serde_json::Value>,
}

/// Capabilities reported by the ACP agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    /// Whether previous sessions can be loaded.
    #[serde(default)]
    pub load_session: bool,
    /// Prompt content capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_capabilities: Option<PromptCapabilities>,
    /// MCP transport capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_capabilities: Option<McpCapabilities>,
}

/// Prompt content capabilities supported by the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilities {
    /// Whether image input is supported.
    pub image: bool,
    /// Whether audio input is supported.
    pub audio: bool,
    /// Whether embedded context blocks are supported.
    pub embedded_context: bool,
}

/// MCP transport capabilities supported by the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCapabilities {
    /// Whether HTTP MCP servers are supported.
    pub http: bool,
    /// Whether SSE MCP servers are supported.
    pub sse: bool,
}

/// Metadata describing the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    /// Stable agent name.
    pub name: String,
    /// Human-readable agent title.
    pub title: String,
    /// Agent version string.
    pub version: String,
}

/// Parameters for creating a new ACP session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewParams {
    /// Optional client-supplied session name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    /// Optional client capabilities for the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_capabilities: Option<ClientCapabilities>,
    /// MCP servers attached to this session.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
}

/// MCP server configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    /// MCP server name.
    pub name: String,
    /// Transport configuration for the MCP server.
    pub transport: McpTransport,
}

/// Supported MCP transport configurations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum McpTransport {
    /// An HTTP transport.
    Http {
        /// Base URL for the MCP server.
        url: String,
    },
    /// A stdio transport.
    Stdio {
        /// Executable command.
        command: String,
        /// Command arguments.
        args: Vec<String>,
    },
}

/// Result returned from `session/new`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewResult {
    /// Server-generated session identifier.
    pub session_id: String,
    /// Session configuration options.
    pub config_options: Vec<ConfigOption>,
    /// Available interaction modes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<ModesInfo>,
}

/// Metadata about the available session modes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModesInfo {
    /// Currently active mode identifier.
    pub current_mode_id: String,
    /// All available modes.
    pub available_modes: Vec<ModeInfo>,
}

/// Description of a single session mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeInfo {
    /// Stable mode identifier.
    pub id: String,
    /// Short mode name.
    pub name: String,
    /// Mode description.
    pub description: String,
}

/// Parameters for `session/prompt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPromptParams {
    /// Target session identifier.
    pub session_id: String,
    /// Prompt content blocks.
    pub prompt: Vec<ContentBlock>,
    /// Whether server-side context should be included.
    #[serde(default)]
    pub include_context: bool,
}

/// Result returned from `session/prompt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPromptResult {
    /// Target session identifier.
    pub session_id: String,
    /// Reason the turn finished.
    pub stop_reason: StopReason,
    /// Optional token usage for the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageInfo>,
}

/// Terminal reason for a prompt response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The agent completed its turn normally.
    EndTurn,
    /// The model hit a max token limit.
    MaxTokens,
    /// The session was cancelled.
    Cancelled,
    /// The session ended because of an error.
    Error,
}

/// ACP prompt content block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ContentBlock {
    /// A plain text content block.
    Text {
        /// Text body.
        text: String,
    },
    /// A resource reference content block.
    Resource {
        /// Referenced resource.
        resource: ResourceRef,
    },
    /// A textual diff content block.
    Diff {
        /// Path the diff applies to.
        path: String,
        /// Unified diff payload.
        diff: String,
    },
}

/// Reference to an ACP resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ResourceRef {
    /// A file resource reference.
    File {
        /// File URI.
        uri: String,
    },
}

/// A `session/update` notification payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "sessionUpdate")]
pub enum SessionUpdate {
    /// A streamed agent-visible output chunk.
    AgentMessageChunk {
        /// Content chunk payload.
        content: ContentBlock,
        /// Optional metadata attached to the chunk.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        _meta: Option<serde_json::Value>,
    },
    /// A streamed thought/output chunk.
    ThoughtMessageChunk {
        /// Content chunk payload.
        content: ContentBlock,
    },
    /// A new tool call card.
    ToolCall {
        /// Tool call identifier.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// User-facing title.
        title: String,
        /// Tool call kind.
        kind: ToolCallKind,
        /// Current tool status.
        status: ToolCallStatus,
        /// Optional rendered tool content.
        #[serde(default)]
        content: Vec<ContentBlock>,
    },
    /// An update to an existing tool call card.
    ToolCallUpdate {
        /// Tool call identifier.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Current tool status.
        status: ToolCallStatus,
        /// Optional rendered tool content.
        #[serde(default)]
        content: Vec<ContentBlock>,
    },
    /// An update to the session plan.
    Plan {
        /// Ordered plan entries.
        entries: Vec<PlanEntry>,
    },
    /// Updated slash commands available to the user.
    AvailableCommandsUpdate {
        /// Available slash commands.
        available_commands: Vec<SlashCommand>,
    },
    /// Updated configuration options.
    ConfigOptionsUpdate {
        /// Available config options.
        config_options: Vec<ConfigOption>,
    },
    /// Token and cost usage update.
    UsageUpdate {
        /// Used tokens/units.
        used: u64,
        /// Total capacity/window size.
        size: u64,
        /// Optional cost information.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost: Option<CostInfo>,
    },
    /// Session metadata update.
    SessionInfoUpdate {
        /// Session identifier.
        session_id: String,
        /// Optional session name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_name: Option<String>,
    },
}

/// ACP tool call category.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallKind {
    /// A file edit action.
    Edit,
    /// A file creation action.
    Create,
    /// A file deletion action.
    Delete,
    /// A terminal command action.
    Terminal,
    /// Any other tool action.
    Other,
}

/// ACP tool call progress state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// The tool call has been declared but not started.
    Pending,
    /// The tool call is currently running.
    InProgress,
    /// The tool call completed successfully.
    Completed,
    /// The tool call failed.
    Failed,
}

/// A configurable session option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigOption {
    /// Stable option identifier.
    pub id: String,
    /// User-facing option name.
    pub name: String,
    /// Option control type.
    #[serde(rename = "type")]
    pub option_type: ConfigOptionType,
    /// Option category.
    pub category: String,
    /// Current value for the option.
    pub current_value: serde_json::Value,
    /// Optional option description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional list of allowed values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<ConfigOptionValue>>,
}

/// Config option control type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigOptionType {
    /// A select/dropdown option.
    Select,
    /// A boolean toggle option.
    Toggle,
}

/// One selectable config option value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigOptionValue {
    /// Serialized value.
    pub value: String,
    /// User-facing value name.
    pub name: String,
    /// Optional value description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One entry in an ACP plan update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    /// Plan entry text.
    pub content: String,
    /// Relative priority.
    pub priority: Priority,
    /// Current execution state.
    pub status: PlanStatus,
}

/// Plan entry priority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// High priority work.
    High,
    /// Medium priority work.
    Medium,
    /// Low priority work.
    Low,
}

/// Plan entry execution state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Work is pending.
    Pending,
    /// Work is in progress.
    InProgress,
    /// Work is completed.
    Completed,
}

/// A slash command exposed by the session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    /// Slash command name.
    pub name: String,
    /// Slash command description.
    pub description: String,
    /// Optional command input metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<CommandInput>,
}

/// Input metadata for a slash command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandInput {
    /// Optional prompt hint for the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Token usage information for a prompt response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageInfo {
    /// Total tokens used.
    pub total_tokens: u64,
    /// Input tokens used.
    pub input_tokens: u64,
    /// Output tokens used.
    pub output_tokens: u64,
    /// Optional thought token count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_tokens: Option<u64>,
    /// Optional cached read token count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_read_tokens: Option<u64>,
    /// Optional cached write token count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_write_tokens: Option<u64>,
}

/// Monetary cost information.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostInfo {
    /// Amount billed or estimated.
    pub amount: f64,
    /// ISO currency code.
    pub currency: String,
}

/// Parameters for `session/config/update`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigUpdateParams {
    /// Target session identifier.
    pub session_id: String,
    /// Configuration option identifier.
    pub option_id: String,
    /// Replacement option value.
    pub new_value: serde_json::Value,
}

/// Result returned from `session/config/update`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigUpdateResult {
    /// Updated set of config options.
    pub config_options: Vec<ConfigOption>,
}

/// Parameters for the `session/cancel` notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCancelParams {
    /// Target session identifier.
    pub session_id: String,
}

/// Result returned from `session/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListResult {
    /// Known sessions.
    pub sessions: Vec<SessionInfo>,
}

/// Summary information for a single session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    /// Session identifier.
    pub session_id: String,
    /// Optional session name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// Parameters for `session/load`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLoadParams {
    /// Target session identifier.
    pub session_id: String,
}

/// Parameters for the legacy `session/set_mode` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSetModeParams {
    /// Target session identifier.
    pub session_id: String,
    /// Requested mode identifier.
    pub mode_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_rpc_request_round_trip() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: JsonRpcId::Number(1),
            method: "initialize".to_string(),
            params: Some(json!({
                "protocolVersion": ACP_PROTOCOL_VERSION,
                "clientCapabilities": {
                    "terminal": true
                }
            })),
        };

        let serialized = serde_json::to_value(&request).expect("serialize request");
        let deserialized: JsonRpcRequest =
            serde_json::from_value(serialized).expect("deserialize request");
        assert_eq!(deserialized, request);
    }

    #[test]
    fn session_update_agent_message_chunk_round_trip() {
        let update = SessionUpdate::AgentMessageChunk {
            content: ContentBlock::Text {
                text: "hello".to_string(),
            },
            _meta: None,
        };

        let serialized = serde_json::to_value(&update).expect("serialize session update");
        assert_eq!(
            serialized,
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {
                    "type": "text",
                    "text": "hello"
                }
            })
        );

        let deserialized: SessionUpdate =
            serde_json::from_value(serialized).expect("deserialize session update");
        assert_eq!(deserialized, update);
    }
}
