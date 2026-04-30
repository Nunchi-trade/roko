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
    #[serde(default)]
    pub agent_capabilities: AgentCapabilities,
    /// Supported authentication methods (always serialized per ACP spec).
    #[serde(default)]
    pub auth_methods: Vec<serde_json::Value>,
    /// Agent identity metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_info: Option<AgentInfo>,
}

/// Capabilities reported by the ACP agent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    /// Whether previous sessions can be loaded.
    #[serde(default)]
    pub load_session: bool,
    /// Prompt content capabilities.
    #[serde(default)]
    pub prompt_capabilities: PromptCapabilities,
    /// MCP transport capabilities.
    #[serde(default)]
    pub mcp_capabilities: McpCapabilities,
}

/// Prompt content capabilities supported by the agent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilities {
    /// Whether image input is supported.
    #[serde(default)]
    pub image: bool,
    /// Whether audio input is supported.
    #[serde(default)]
    pub audio: bool,
    /// Whether embedded context blocks are supported.
    #[serde(default)]
    pub embedded_context: bool,
}

/// MCP transport capabilities supported by the agent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCapabilities {
    /// Whether HTTP MCP servers are supported.
    #[serde(default)]
    pub http: bool,
    /// Whether SSE MCP servers are supported.
    #[serde(default)]
    pub sse: bool,
}

/// Metadata describing the agent (matches ACP `Implementation` type).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    /// Stable agent name.
    pub name: String,
    /// Agent version string.
    pub version: String,
    /// Optional human-readable agent title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    /// Available interaction modes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<ModesInfo>,
    /// Session configuration options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<ConfigOption>>,
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
    /// Reason the turn finished.
    pub stop_reason: StopReason,
}

/// Terminal reason for a prompt response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The agent completed its turn normally.
    EndTurn,
    /// The model hit a max token limit.
    MaxTokens,
    /// The maximum number of turn requests was reached.
    MaxTurnRequests,
    /// The agent refused to answer.
    Refusal,
    /// The session was cancelled.
    Cancelled,
}

/// ACP prompt content block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
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
        /// Original text before the change (for inline diff rendering).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        old_text: Option<String>,
        /// New text after the change (for inline diff rendering).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_text: Option<String>,
        /// Unified diff payload.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
    },
}

/// Reference to an ACP resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
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
    AgentThoughtChunk {
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
        /// File locations for Follow Agent (auto-navigate to edited files).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locations: Option<Vec<ToolCallLocation>>,
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
        /// File locations for Follow Agent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locations: Option<Vec<ToolCallLocation>>,
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
    ConfigOptionUpdate {
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
    /// A file read action.
    Read,
    /// A grep/find search action.
    Search,
    /// A web/API fetch action.
    Fetch,
    /// An internal reasoning step.
    Think,
    /// A file rename/move action.
    Move,
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

/// A file location referenced by a tool call for Follow Agent navigation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallLocation {
    /// File URI (e.g. `file:///absolute/path`).
    pub uri: String,
    /// Optional range within the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<LocationRange>,
}

/// A range within a file (start and end positions).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationRange {
    /// Start position.
    pub start: Position,
    /// End position.
    pub end: Position,
}

/// A position within a file (zero-indexed line and character).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    /// Zero-indexed line number.
    pub line: u32,
    /// Zero-indexed character offset.
    pub character: u32,
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

/// A single file change notification emitted after a pipeline commit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeNotification {
    /// Relative file path from the workdir root.
    pub path: String,
    /// How the file changed.
    pub change_type: FileChangeType,
}

/// How a file changed in the most recent commit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileChangeType {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// Parameters for `session/config/update`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigUpdateParams {
    /// Target session identifier.
    pub session_id: String,
    /// Configuration option identifier.
    #[serde(alias = "configId")]
    pub option_id: String,
    /// Replacement option value.
    #[serde(alias = "value")]
    pub new_value: serde_json::Value,
}

/// Result returned from `session/config/update`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigUpdateResult {
    /// Updated set of config options.
    pub config_options: Vec<ConfigOption>,
}

/// Parameters for the `session/close` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCloseParams {
    /// Target session identifier.
    pub session_id: String,
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

/// Action types that require user permission before proceeding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    /// Writing or editing a file.
    FileEdit,
    /// Creating a new file.
    FileCreate,
    /// Deleting a file.
    FileDelete,
    /// Running a terminal command.
    TerminalCommand,
    /// Making a network request.
    NetworkRequest,
    /// Running a git operation.
    GitOperation,
}

/// User decision in response to a permission request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Allow this single occurrence.
    Allow,
    /// Allow this and all future occurrences in this session/workspace.
    AlwaysAllow,
    /// Reject - do not perform the action.
    Reject,
}

/// Parameters for the `session/request_permission` server-to-client request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionParams {
    /// The session this request is for.
    pub session_id: String,
    /// The tool call that needs permission.
    pub tool_call: PermissionToolCall,
    /// Available permission options for the user.
    pub options: Vec<PermissionOption>,
}

/// Tool call metadata included in a permission request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionToolCall {
    /// Tool call identifier.
    pub tool_call_id: String,
    /// Human-readable title for the permission dialog.
    pub title: String,
}

/// One selectable option in a permission request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    /// Stable option identifier.
    pub option_id: String,
    /// User-facing option name.
    pub name: String,
    /// Option behaviour kind.
    pub kind: PermissionOptionKind,
}

/// Kind of permission option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOptionKind {
    /// Allow the action once.
    AllowOnce,
    /// Allow the action for this session/workspace going forward.
    AllowAlways,
    /// Reject the action once.
    RejectOnce,
    /// Reject the action permanently for this session.
    RejectAlways,
}

impl PermissionOptionKind {
    /// Wire identifier matching serde snake_case.
    pub fn option_id(&self) -> &'static str {
        match self {
            Self::AllowOnce => "allow_once",
            Self::AllowAlways => "allow_always",
            Self::RejectOnce => "reject_once",
            Self::RejectAlways => "reject_always",
        }
    }

    /// Maps a wire `option_id` back to a [`PermissionDecision`].
    pub fn decision_from_option_id(id: &str) -> Option<PermissionDecision> {
        match id {
            "allow_once" => Some(PermissionDecision::Allow),
            "allow_always" => Some(PermissionDecision::AlwaysAllow),
            "reject_once" | "reject_always" => Some(PermissionDecision::Reject),
            _ => None,
        }
    }

    /// The four standard permission options for the editor dialog.
    pub fn standard_options() -> Vec<PermissionOption> {
        use PermissionOptionKind::*;
        [
            (AllowOnce, "Allow"),
            (AllowAlways, "Always allow"),
            (RejectOnce, "Reject"),
            (RejectAlways, "Always reject"),
        ]
        .into_iter()
        .map(|(kind, name)| PermissionOption {
            option_id: kind.option_id().to_string(),
            name: name.to_string(),
            kind,
        })
        .collect()
    }
}

/// Response from the editor to a `session/request_permission` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionResponse {
    /// The user's decision outcome.
    pub outcome: PermissionOutcome,
}

/// Outcome of a permission request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum PermissionOutcome {
    /// The user cancelled the permission dialog.
    Cancelled,
    /// The user selected an option.
    Selected {
        /// The selected option identifier.
        #[serde(alias = "optionId")]
        option_id: String,
    },
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

    #[test]
    fn permission_request_round_trip() {
        let params = RequestPermissionParams {
            session_id: "sess-1".to_string(),
            tool_call: PermissionToolCall {
                tool_call_id: "tc-1".to_string(),
                title: "Allow editing src/lib.rs?".to_string(),
            },
            options: vec![
                PermissionOption {
                    option_id: "allow_once".to_string(),
                    name: "Allow".to_string(),
                    kind: PermissionOptionKind::AllowOnce,
                },
                PermissionOption {
                    option_id: "allow_always".to_string(),
                    name: "Always allow".to_string(),
                    kind: PermissionOptionKind::AllowAlways,
                },
            ],
        };

        let serialized = serde_json::to_value(&params).expect("serialize permission params");
        let deserialized: RequestPermissionParams =
            serde_json::from_value(serialized).expect("deserialize permission params");
        assert_eq!(deserialized.session_id, params.session_id);
        assert_eq!(deserialized.tool_call.tool_call_id, "tc-1");
        assert_eq!(deserialized.options.len(), 2);

        let response = PermissionResponse {
            outcome: PermissionOutcome::Selected {
                option_id: "allow_always".to_string(),
            },
        };
        let serialized = serde_json::to_value(&response).expect("serialize permission response");
        let deserialized: PermissionResponse =
            serde_json::from_value(serialized).expect("deserialize permission response");
        assert!(matches!(
            deserialized.outcome,
            PermissionOutcome::Selected { .. }
        ));

        // Verify wire-format deserialization (what Zed actually sends).
        let wire_json = json!({ "outcome": { "type": "selected", "optionId": "allow_always" } });
        let from_wire: PermissionResponse =
            serde_json::from_value(wire_json).expect("deserialize wire format");
        match from_wire.outcome {
            PermissionOutcome::Selected { option_id } => {
                assert_eq!(option_id, "allow_always");
            }
            _ => panic!("expected Selected variant"),
        }
    }
}
