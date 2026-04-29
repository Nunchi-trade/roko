//! Unified agent session for interactive and one-shot CLI modes.
//!
//! This module owns the session state that will later be passed to the Claude
//! CLI adapter or to API-backed provider adapters.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use roko_agent::agent::Agent;
use roko_agent::claude_cli_agent::ClaudeCliAgent;
use roko_agent::safety::contract::AgentContract;
use roko_compose::system_prompt_builder::SystemPromptBuilder;
use roko_compose::{ProjectConventions, TokenCounter, detect_conventions};
use roko_core::foundation::ChatMessage;
use roko_core::{Body, Context, Engram, Kind};

use crate::config::Config;
use crate::model_selection::EffectiveModelSelection;

const CHAT_SYSTEM_PROMPT_TOKEN_BUDGET: usize = 4_000;
const MAX_WORKSPACE_SAMPLE_BYTES: usize = 16_384;
const MAX_WORKSPACE_SAMPLE_FILES: usize = 8;
const MAX_WORKSPACE_SCAN_DEPTH: usize = 5;
const SKIP_DIR_NAMES: [&str; 12] = [
    ".git",
    ".next",
    ".roko",
    ".turbo",
    ".venv",
    "__pycache__",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
    "venv",
];

/// Result of processing a potential slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashResult {
    /// Command recognized and session state updated. String is user-facing message.
    Updated(String),
    /// Command recognized but had an error. String is the error message.
    Error(String),
    /// Input starts with `/` but command is not recognized.
    Unknown(String),
    /// Input is not a slash command.
    NotACommand,
}

/// Summary of a tool call captured during a turn.
#[derive(Debug, Clone)]
pub struct ToolCallSummary {
    /// Tool name (for example `Read`, `Bash`, or `Edit`).
    pub name: String,
    /// Abbreviated input, capped at the first 200 characters.
    pub input_abbrev: String,
    /// Whether the tool call succeeded.
    pub success: bool,
}

/// Result of a single agent turn.
#[derive(Debug, Clone)]
pub struct TurnResult {
    /// The model's text response.
    pub text: String,
    /// Which model responded.
    pub model: String,
    /// Input tokens consumed during the turn.
    pub input_tokens: u64,
    /// Output tokens produced during the turn.
    pub output_tokens: u64,
    /// Tool calls executed during the turn.
    pub tool_calls: Vec<ToolCallSummary>,
    /// Session identifier for `--resume`.
    ///
    /// This batch uses `ClaudeCliAgent::run`, which does not surface the
    /// stream `result` event session id into `AgentResult`, so this stays
    /// `None` until the streaming turn path lands.
    pub session_id: Option<String>,
    /// Wall-clock duration of the turn.
    pub duration: Duration,
    /// Whether the turn was cancelled by the user.
    pub cancelled: bool,
}

/// Unified agent session for interactive and one-shot CLI modes.
///
/// Delegates to `ClaudeCliAgent` for Claude CLI turns and to provider
/// adapters for API turns, instead of duplicating command construction.
pub struct ChatAgentSession {
    /// Working directory for the agent.
    pub workdir: PathBuf,
    /// Mutable model string used by slash commands and future turns.
    pub model: String,
    /// Resolved model identity (provider + slug + source).
    pub model_selection: EffectiveModelSelection,
    /// Reasoning effort level: `"low"`, `"medium"`, `"high"`, `"max"`.
    pub effort: String,
    /// System prompt built by `SystemPromptBuilder`.
    pub system_prompt: String,
    /// Tool allowlist as comma-separated names for `--tools`.
    pub allowed_tools_csv: String,
    /// Path to MCP config file, if discovered.
    pub mcp_config: Option<PathBuf>,
    /// Session ID from previous turn, reused via `--resume`.
    pub session_id: Option<String>,
    /// API message history for non-CLI providers.
    pub api_history: Vec<ChatMessage>,
    /// Shared HTTP client for API providers.
    pub http_client: reqwest::Client,
    /// Path to Claude CLI settings JSON file.
    pub settings_json: Option<PathBuf>,
    /// Per-turn timeout.
    pub timeout: Option<Duration>,
}

impl ChatAgentSession {
    /// Create a new session from CLI config and working directory.
    ///
    /// Resolves system prompt via `SystemPromptBuilder`, tool policy from
    /// safety contracts, and MCP config from discovery paths. Creates one
    /// shared `reqwest::Client`.
    #[must_use]
    pub fn new(
        config: &Config,
        workdir: PathBuf,
        model_selection: EffectiveModelSelection,
    ) -> Result<Self> {
        let system_prompt = build_chat_system_prompt(&workdir, config);
        let allowed_tools_csv = resolve_tool_policy(&workdir);
        let mcp_config = resolve_mcp_config(&workdir, config);
        let effort = config.agent.effort.clone();
        let timeout =
            (config.agent.timeout_ms > 0).then(|| Duration::from_millis(config.agent.timeout_ms));
        let model = model_selection.effective_model_key.clone();

        Ok(Self {
            workdir,
            model,
            model_selection,
            effort,
            system_prompt,
            allowed_tools_csv,
            mcp_config,
            session_id: None,
            api_history: Vec::new(),
            http_client: shared_http_client(),
            settings_json: None,
            timeout,
        })
    }

    /// Process input that may be a slash command.
    ///
    /// If the input starts with `/`, parses the command and mutates session
    /// state as needed. Regular chat text returns [`SlashResult::NotACommand`].
    pub fn handle_slash_command(&mut self, input: &str) -> SlashResult {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return SlashResult::NotACommand;
        }

        let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
        let cmd = parts[0];
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

        match cmd {
            "/system" => {
                if arg.is_empty() {
                    let preview = preview_text(&self.system_prompt, 200);
                    SlashResult::Updated(format!(
                        "Current system prompt ({} chars): {}",
                        self.system_prompt.chars().count(),
                        preview,
                    ))
                } else {
                    self.system_prompt = arg.to_string();
                    SlashResult::Updated(format!(
                        "System prompt set ({} chars)",
                        arg.chars().count()
                    ))
                }
            }
            "/model" => {
                if arg.is_empty() {
                    SlashResult::Updated(format!("Current model: {}", self.model))
                } else {
                    self.model = arg.to_string();
                    SlashResult::Updated(format!("Model set to: {arg}"))
                }
            }
            "/effort" => match arg {
                "low" | "medium" | "high" | "max" => {
                    self.effort = arg.to_string();
                    SlashResult::Updated(format!("Effort set to: {arg}"))
                }
                "" => SlashResult::Updated(format!("Current effort: {}", self.effort)),
                other => SlashResult::Error(format!(
                    "Invalid effort level: {other} (use low/medium/high/max)"
                )),
            },
            "/reset" => {
                self.session_id = None;
                self.api_history.clear();
                SlashResult::Updated("Session reset: cleared session_id and history".to_string())
            }
            "/tools" => {
                if arg.is_empty() {
                    SlashResult::Updated(format!("Current tools: {}", self.allowed_tools_csv))
                } else {
                    self.allowed_tools_csv = arg.to_string();
                    SlashResult::Updated(format!("Tools set to: {arg}"))
                }
            }
            "/mcp" => {
                if arg.is_empty() {
                    let status = match &self.mcp_config {
                        Some(path) => format!("MCP config: {}", path.display()),
                        None => "No MCP config".to_string(),
                    };
                    SlashResult::Updated(status)
                } else {
                    let path = PathBuf::from(arg);
                    if path.exists() {
                        self.mcp_config = Some(path.clone());
                        SlashResult::Updated(format!("MCP config set to: {}", path.display()))
                    } else {
                        SlashResult::Error(format!("MCP config not found: {}", path.display()))
                    }
                }
            }
            _ => SlashResult::Unknown(cmd.to_string()),
        }
    }

    /// Build a `ClaudeCliAgent` with the current session state.
    ///
    /// This is kept as a helper so tests can inspect the configured agent
    /// without needing to spawn a turn.
    pub fn build_agent(&self) -> Result<ClaudeCliAgent> {
        let mut agent = ClaudeCliAgent::new("claude", self.workdir.clone(), self.model.clone())
            .with_effort(&self.effort)
            .with_bare_mode(false);

        if !self.system_prompt.is_empty() {
            agent = agent.with_system_prompt(&self.system_prompt);
        }

        if !self.allowed_tools_csv.is_empty() {
            agent = agent.with_tools(&self.allowed_tools_csv);
        }

        if let Some(ref mcp_path) = self.mcp_config {
            agent = agent.with_mcp_config(mcp_path.clone());
        }

        if let Some(ref sid) = self.session_id {
            agent = agent.with_resume(sid.clone());
        }

        if let Some(timeout) = self.timeout {
            agent = agent.with_timeout_ms(timeout.as_millis() as u64);
        }

        Ok(agent)
    }

    /// Build the input engram for a user prompt.
    pub fn build_engram(&self, prompt: &str) -> Engram {
        Engram::builder(Kind::Prompt)
            .body(Body::text(prompt))
            .build()
    }

    /// Send a single turn through the configured Claude CLI agent.
    pub async fn send_turn(&mut self, prompt: &str) -> Result<TurnResult> {
        let started = Instant::now();
        let agent = self.build_agent()?;
        let input = self.build_engram(prompt);
        let ctx = Context::default();
        let result = agent.run(&input, &ctx).await;

        let text = result
            .output
            .body
            .as_text()
            .ok()
            .map(str::to_string)
            .unwrap_or_default();

        Ok(TurnResult {
            text,
            model: self.model.clone(),
            input_tokens: u64::from(result.usage.input_tokens),
            output_tokens: u64::from(result.usage.output_tokens),
            tool_calls: Vec::new(),
            // `ClaudeCliAgent::run` does not expose session ids in its result.
            session_id: None,
            duration: started.elapsed(),
            cancelled: false,
        })
    }

    #[cfg(test)]
    /// Clone the session for test assertions.
    fn clone_for_test(&self) -> Self {
        Self {
            workdir: self.workdir.clone(),
            model: self.model.clone(),
            model_selection: self.model_selection.clone(),
            effort: self.effort.clone(),
            system_prompt: self.system_prompt.clone(),
            allowed_tools_csv: self.allowed_tools_csv.clone(),
            mcp_config: self.mcp_config.clone(),
            session_id: self.session_id.clone(),
            api_history: self.api_history.clone(),
            http_client: reqwest::Client::new(),
            settings_json: self.settings_json.clone(),
            timeout: self.timeout,
        }
    }
}

/// Build a system prompt for interactive and one-shot chat using the shared
/// `SystemPromptBuilder`.
///
/// Workspace context is inferred from the working directory. If the composed
/// prompt ends up empty for any reason, fall back to a minimal role identity.
fn build_chat_system_prompt(workdir: &Path, config: &Config) -> String {
    let role_identity = "You are an expert software engineer working in an interactive chat session. You help inspect, understand, and edit the current repository. Stay concise, grounded in the workspace, and prefer existing code over inventing new abstractions.";

    let mut builder = SystemPromptBuilder::new(role_identity);

    if let Some(conventions) = gather_workspace_conventions(workdir) {
        builder = builder.with_conventions(conventions);
    }

    let project_name = project_name_for(workdir);
    builder = builder.with_domain(format!(
        "Working directory: {}\nProject: {}",
        workdir.display(),
        project_name
    ));

    if let Ok(context) = gather_workspace_context(workdir) {
        if !context.trim().is_empty() {
            builder = builder.with_context(context);
        }
    }

    let token_budget = config
        .prompt
        .token_budget
        .clamp(1, CHAT_SYSTEM_PROMPT_TOKEN_BUDGET);
    let prompt =
        builder
            .with_token_budget(token_budget)
            .build_with_counter(&TokenCounter::Heuristic {
                chars_per_token: 4.0,
            });

    if prompt.trim().is_empty() {
        role_identity.to_string()
    } else {
        prompt
    }
}

/// Gather lightweight workspace context: git branch and language hints.
///
/// The result is best-effort. Missing git metadata or workspace markers are
/// treated as empty context instead of a hard error.
fn gather_workspace_context(workdir: &Path) -> Result<String> {
    let mut parts = Vec::new();

    if let Some(branch) = capture_git_branch(workdir) {
        if !branch.is_empty() {
            parts.push(format!("Git branch: {branch}"));
        }
    }

    let language_hints = language_hints_for(workdir);
    if !language_hints.is_empty() {
        parts.push(format!("Language hints: {}", language_hints.join(", ")));
    }

    Ok(parts.join("\n"))
}

/// Default tools for interactive chat when no safety contract is found.
const DEFAULT_CHAT_TOOLS: &str = "Read,Glob,Grep,Bash,Edit,Write,NotebookEdit";

/// Resolve tool allowlist from safety contracts.
///
/// Looks for an `AgentContract` for the "chat" role at `.roko/safety/chat.yaml`.
/// If found, uses its `allowed_tools` field. If not found, falls back to a
/// read-oriented default set and logs a debug message.
fn resolve_tool_policy(workdir: &Path) -> String {
    let contract_path = workdir.join(".roko/safety/chat.yaml");
    match std::fs::read_to_string(&contract_path) {
        Ok(content) => match serde_yaml::from_str::<AgentContract>(&content) {
            Ok(contract) => {
                if let Some(ref allowlist) = contract.allowed_tools {
                    if !allowlist.is_empty() {
                        let tools = allowlist.join(",");
                        tracing::debug!("chat tool policy from contract: {}", tools);
                        return tools;
                    }
                }
                tracing::debug!("chat contract has no allowed_tools, using defaults");
                DEFAULT_CHAT_TOOLS.to_string()
            }
            Err(e) => {
                tracing::warn!(
                    "failed to parse chat contract at {}: {e}",
                    contract_path.display()
                );
                DEFAULT_CHAT_TOOLS.to_string()
            }
        },
        Err(_) => {
            tracing::debug!(
                "no chat safety contract at {}, using default tools",
                contract_path.display()
            );
            DEFAULT_CHAT_TOOLS.to_string()
        }
    }
}

fn gather_workspace_conventions(workdir: &Path) -> Option<String> {
    let cargo_toml = read_text_snippet(&workdir.join("Cargo.toml")).unwrap_or_default();
    let (source_samples, file_listing) = collect_workspace_samples(workdir);

    if cargo_toml.is_empty() && source_samples.is_empty() && file_listing.is_empty() {
        return None;
    }

    let source_refs = source_samples
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let file_refs = file_listing.iter().map(String::as_str).collect::<Vec<_>>();
    let conventions = detect_conventions(&cargo_toml, &source_refs, &file_refs);

    if conventions == ProjectConventions::default() {
        return None;
    }

    let fragment = conventions.to_prompt_fragment();
    if fragment.trim().is_empty() {
        None
    } else {
        Some(fragment)
    }
}

fn collect_workspace_samples(workdir: &Path) -> (Vec<String>, Vec<String>) {
    let mut source_samples = Vec::new();
    let mut file_listing = Vec::new();
    collect_workspace_samples_from_dir(workdir, workdir, 0, &mut source_samples, &mut file_listing);
    (source_samples, file_listing)
}

fn collect_workspace_samples_from_dir(
    dir: &Path,
    root: &Path,
    depth: usize,
    source_samples: &mut Vec<String>,
    file_listing: &mut Vec<String>,
) {
    if depth > MAX_WORKSPACE_SCAN_DEPTH || source_samples.len() >= MAX_WORKSPACE_SAMPLE_FILES {
        return;
    }

    let mut entries = match fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>(),
        Err(_) => return,
    };
    entries.sort_by(|left, right| left.path().cmp(&right.path()));

    for entry in entries {
        if source_samples.len() >= MAX_WORKSPACE_SAMPLE_FILES {
            break;
        }

        let path = entry.path();
        let file_name = path.file_name().and_then(|name| name.to_str());
        if path.is_dir() {
            if file_name.map_or(false, is_skipped_dir_name) {
                continue;
            }
            collect_workspace_samples_from_dir(
                &path,
                root,
                depth + 1,
                source_samples,
                file_listing,
            );
            continue;
        }

        if !path.is_file() || !is_workspace_source_file(&path) {
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .ok()
            .and_then(|relative| relative.to_str())
            .map(|relative| relative.to_string())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        file_listing.push(relative);

        if let Some(sample) = read_text_snippet(&path) {
            if !sample.trim().is_empty() {
                source_samples.push(sample);
            }
        }
    }
}

fn read_text_snippet(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut limited = file.take(MAX_WORKSPACE_SAMPLE_BYTES as u64);
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn capture_git_branch(workdir: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(workdir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

fn language_hints_for(workdir: &Path) -> Vec<String> {
    let mut hints = Vec::new();

    if workdir.join("Cargo.toml").is_file() || workdir.join("rust-toolchain.toml").is_file() {
        push_unique_hint(&mut hints, "Rust");
    }
    if workdir.join("package.json").is_file()
        || workdir.join("tsconfig.json").is_file()
        || workdir.join("deno.json").is_file()
        || workdir.join("deno.jsonc").is_file()
    {
        push_unique_hint(&mut hints, "TypeScript/JavaScript");
    }
    if workdir.join("pyproject.toml").is_file()
        || workdir.join("requirements.txt").is_file()
        || workdir.join("uv.lock").is_file()
    {
        push_unique_hint(&mut hints, "Python");
    }
    if workdir.join("go.mod").is_file() {
        push_unique_hint(&mut hints, "Go");
    }
    if workdir.join("pom.xml").is_file()
        || workdir.join("build.gradle").is_file()
        || workdir.join("build.gradle.kts").is_file()
    {
        push_unique_hint(&mut hints, "Java/Kotlin");
    }

    hints
}

fn push_unique_hint(hints: &mut Vec<String>, hint: &str) {
    if !hints.iter().any(|existing| existing == hint) {
        hints.push(hint.to_string());
    }
}

fn project_name_for(workdir: &Path) -> String {
    workdir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn is_workspace_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(
            "rs" | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "py"
                | "go"
                | "java"
                | "kt"
                | "swift"
                | "rb"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "cs"
                | "lua"
                | "sh"
        )
    )
}

fn is_skipped_dir_name(name: &str) -> bool {
    SKIP_DIR_NAMES.contains(&name)
}

/// Discover MCP config file using the same resolution order as orchestrate.rs.
///
/// Priority:
/// 1. Explicit path in `config.agent.mcp_config`
/// 2. Workspace `.roko/mcp.json`
/// 3. Global `~/.claude/mcp-config.json`
///
/// Returns `None` if no MCP config is found.
fn resolve_mcp_config(workdir: &Path, config: &Config) -> Option<PathBuf> {
    if let Some(ref path) = config.agent.mcp_config {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            workdir.join(path)
        };
        if resolved.exists() {
            tracing::debug!("MCP config from roko.toml: {}", resolved.display());
            return Some(resolved);
        }
        tracing::debug!(
            "MCP config in roko.toml does not exist: {}",
            resolved.display()
        );
    }

    let workspace_mcp = workdir.join(".roko/mcp.json");
    if workspace_mcp.exists() {
        tracing::debug!("MCP config from workspace: {}", workspace_mcp.display());
        return Some(workspace_mcp);
    }

    if let Some(home) = home_dir() {
        let global_mcp = home.join(".claude/mcp-config.json");
        if global_mcp.exists() {
            tracing::debug!("MCP config from global: {}", global_mcp.display());
            return Some(global_mcp);
        }
    }

    tracing::debug!("no MCP config found");
    None
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn shared_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new).clone()
}

fn preview_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        preview.push_str("...");
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;

    use roko_core::foundation::{ChatMessage, MessageRole};
    use tempfile::tempdir;

    fn test_model_selection() -> EffectiveModelSelection {
        EffectiveModelSelection {
            requested_model: Some("claude-sonnet-4-6".to_string()),
            effective_model_key: "claude-sonnet-4-6".to_string(),
            provider_key: "claude_cli".to_string(),
            provider_kind: "claude_cli".to_string(),
            backend_slug: "claude-sonnet-4-6".to_string(),
            source: crate::model_selection::SelectionSource::ProjectDefault,
            reason: "test selection".to_string(),
        }
    }

    /// Construct a minimal session for testing slash commands.
    fn test_session() -> ChatAgentSession {
        let model_selection = test_model_selection();
        let model = model_selection.effective_model_key.clone();
        ChatAgentSession {
            workdir: PathBuf::from("/tmp/test"),
            model,
            model_selection,
            effort: "medium".to_string(),
            system_prompt: String::new(),
            allowed_tools_csv: DEFAULT_CHAT_TOOLS.to_string(),
            mcp_config: None,
            session_id: None,
            api_history: Vec::new(),
            http_client: reqwest::Client::new(),
            settings_json: None,
            timeout: None,
        }
    }

    #[test]
    fn slash_system_shows_current() {
        let mut s = test_session().clone_for_test();
        s.system_prompt = "test prompt".to_string();
        match s.handle_slash_command("/system") {
            SlashResult::Updated(msg) => assert!(msg.contains("test prompt")),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_system_sets_new() {
        let mut s = test_session();
        match s.handle_slash_command("/system You are a Rust expert") {
            SlashResult::Updated(msg) => assert!(msg.contains("System prompt set")),
            other => panic!("expected Updated, got {other:?}"),
        }
        assert_eq!(s.system_prompt, "You are a Rust expert");
    }

    #[test]
    fn slash_system_preview_truncates_safely() {
        let mut s = test_session();
        s.system_prompt = "é".repeat(210);
        match s.handle_slash_command("/system") {
            SlashResult::Updated(msg) => {
                assert!(msg.contains("Current system prompt"));
                assert!(msg.ends_with("..."));
            }
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_model_shows_current() {
        let mut s = test_session().clone_for_test();
        match s.handle_slash_command("/model") {
            SlashResult::Updated(msg) => assert!(msg.contains("claude-sonnet-4-6")),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_model_sets_new() {
        let mut s = test_session();
        match s.handle_slash_command("/model claude-opus-4-5") {
            SlashResult::Updated(msg) => assert!(msg.contains("Model set to:")),
            other => panic!("expected Updated, got {other:?}"),
        }
        assert_eq!(s.model, "claude-opus-4-5");
    }

    #[test]
    fn slash_effort_valid_low() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/effort low"),
            SlashResult::Updated(_)
        ));
        assert_eq!(s.effort, "low");
    }

    #[test]
    fn slash_effort_valid_high() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/effort high"),
            SlashResult::Updated(_)
        ));
        assert_eq!(s.effort, "high");
    }

    #[test]
    fn slash_effort_valid_max() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/effort max"),
            SlashResult::Updated(_)
        ));
        assert_eq!(s.effort, "max");
    }

    #[test]
    fn slash_effort_shows_current_when_no_arg() {
        let mut s = test_session().clone_for_test();
        match s.handle_slash_command("/effort") {
            SlashResult::Updated(msg) => assert!(msg.contains("medium")),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_effort_invalid() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/effort turbo"),
            SlashResult::Error(_)
        ));
        assert_eq!(s.effort, "medium");
    }

    #[test]
    fn slash_reset_clears_session() {
        let mut s = test_session();
        s.session_id = Some("sess-123".to_string());
        s.api_history.push(ChatMessage {
            role: MessageRole::User,
            content: "hello".to_string(),
        });
        match s.handle_slash_command("/reset") {
            SlashResult::Updated(msg) => assert!(msg.contains("Session reset")),
            other => panic!("expected Updated, got {other:?}"),
        }
        assert!(s.session_id.is_none());
        assert!(s.api_history.is_empty());
    }

    #[test]
    fn slash_tools_shows_current() {
        let mut s = test_session().clone_for_test();
        match s.handle_slash_command("/tools") {
            SlashResult::Updated(msg) => assert!(msg.contains("Read")),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_tools_sets_new() {
        let mut s = test_session();
        match s.handle_slash_command("/tools Read,Edit") {
            SlashResult::Updated(msg) => assert!(msg.contains("Tools set to:")),
            other => panic!("expected Updated, got {other:?}"),
        }
        assert_eq!(s.allowed_tools_csv, "Read,Edit");
    }

    #[test]
    fn slash_mcp_shows_none() {
        let mut s = test_session().clone_for_test();
        match s.handle_slash_command("/mcp") {
            SlashResult::Updated(msg) => assert!(msg.contains("No MCP")),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn slash_mcp_sets_new() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("mcp.json");
        std::fs::write(&path, "{}").expect("write mcp config");

        let mut s = test_session();
        match s.handle_slash_command(&format!("/mcp {}", path.display())) {
            SlashResult::Updated(msg) => assert!(msg.contains("MCP config set to:")),
            other => panic!("expected Updated, got {other:?}"),
        }
        assert_eq!(s.mcp_config.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn slash_mcp_invalid_path() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/mcp /nonexistent/path/mcp.json"),
            SlashResult::Error(_)
        ));
        assert!(s.mcp_config.is_none());
    }

    #[test]
    fn regular_text_returns_not_a_command() {
        let mut s = test_session();
        assert_eq!(
            s.handle_slash_command("hello world"),
            SlashResult::NotACommand
        );
    }

    #[test]
    fn unknown_slash_returns_unknown() {
        let mut s = test_session();
        assert!(matches!(
            s.handle_slash_command("/banana"),
            SlashResult::Unknown(_)
        ));
    }
}
