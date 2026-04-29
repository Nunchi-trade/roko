//! Unified agent session for interactive and one-shot CLI modes.
//!
//! This module owns the session state that will later be passed to the Claude
//! CLI adapter or to API-backed provider adapters.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;
use roko_agent::safety::contract::AgentContract;
use roko_compose::system_prompt_builder::SystemPromptBuilder;
use roko_compose::{detect_conventions, ProjectConventions, TokenCounter};
use roko_core::foundation::ChatMessage;

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

/// Unified agent session for interactive and one-shot CLI modes.
///
/// Delegates to `ClaudeCliAgent` for Claude CLI turns and to provider
/// adapters for API turns, instead of duplicating command construction.
pub struct ChatAgentSession {
    /// Working directory for the agent.
    pub workdir: PathBuf,
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

        Ok(Self {
            workdir,
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
