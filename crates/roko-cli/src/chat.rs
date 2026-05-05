//! `roko chat` REPL.
//!
//! Supports two backends:
//!   1. **Direct sidecar** — talks to the agent's HTTP sidecar at its bind address
//!      (looked up from `.roko/runtime/agents.json`). This is the default when a
//!      sidecar is running and no `--serve-url` override points elsewhere.
//!   2. **Via roko-serve** — routes messages through the control plane at `:6677`.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::auth;
use crate::chat_history::{SessionSummary, save_summary_nonblocking};

#[derive(Debug, Deserialize)]
struct SendMessageResponse {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    response: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RunStatusResponse {
    #[serde(default)]
    finished: bool,
    #[serde(default)]
    status: String,
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Runtime entry from `.roko/runtime/agents.json`.
#[derive(Debug, Deserialize)]
struct AgentEntry {
    name: String,
    #[allow(dead_code)]
    pid: u32,
    bind: String,
}

/// Which backend the REPL is talking to.
#[derive(Debug)]
enum ChatBackend {
    /// Direct to agent sidecar (e.g. `http://127.0.0.1:8081`).
    Sidecar(String),
    /// Via roko-serve control plane.
    Serve(String),
    /// Neither reachable at startup; will try serve_url on each message.
    Unreachable(String),
}

/// Construct the health-check URL for a roko-serve instance (`/api/health`).
fn serve_health_url(base: &str) -> String {
    format!("{}/api/health", base.trim_end_matches('/'))
}

/// Construct the health-check URL for an agent sidecar (`/health`).
fn sidecar_health_url(base: &str) -> String {
    format!("{}/health", base.trim_end_matches('/'))
}

/// Try to find the agent's sidecar bind address from `.roko/runtime/agents.json`.
///
/// Public alias for use by `chat_inline`.
pub fn lookup_sidecar_url(agent_id: &str, workdir: &Path) -> Option<String> {
    lookup_sidecar(agent_id, workdir)
}

/// Try to find the agent's sidecar bind address from `.roko/runtime/agents.json`.
fn lookup_sidecar(agent_id: &str, workdir: &Path) -> Option<String> {
    let path = workdir.join(".roko/runtime/agents.json");
    let contents = std::fs::read_to_string(path).ok()?;
    let entries: Vec<AgentEntry> = serde_json::from_str(&contents).ok()?;
    let entry = entries.iter().find(|e| e.name == agent_id)?;
    let bind = &entry.bind;
    // Ensure it's a full URL.
    if bind.starts_with("http") {
        Some(bind.clone())
    } else {
        Some(format!("http://{bind}"))
    }
}

/// Run the chat REPL against a roko-serve instance.
pub async fn run_chat_repl(agent_id: &str, serve_url: &str) -> Result<()> {
    println!("roko chat \u{2014} talking to agent '{agent_id}'");
    println!("Type a message. Press Ctrl-D to exit.\n");

    // Resolve API key from CLI flag / env / config (best-effort).
    let api_key =
        auth::resolve_api_key(&roko_core::config::ServeAuthConfig::default(), None).map(|r| r.key);

    // Build client with auth headers when a key is available.
    let mut client_builder = reqwest::Client::builder();
    if let Some(ref key) = api_key {
        client_builder = client_builder.default_headers(auth::auth_headers(key));
    }
    let client = client_builder.build().context("build HTTP client")?;

    let workdir = PathBuf::from(".");

    // Determine backend: prefer sidecar, fall back to roko-serve.
    let backend = resolve_backend(&client, agent_id, serve_url, &workdir).await;

    match &backend {
        ChatBackend::Sidecar(url) => {
            eprintln!("Connected directly to agent sidecar at {url}");
            eprintln!();
        }
        ChatBackend::Serve(url) => {
            eprintln!("Connected to roko-serve at {url}");
            eprintln!();
        }
        ChatBackend::Unreachable(url) => {
            // Warning already printed by resolve_backend.
            eprintln!("Will attempt roko-serve at {url} on each message.");
            eprintln!();
        }
    }

    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();

    loop {
        // Cyan prompt for user input.
        print!("\x1b[36myou>\x1b[0m ");
        io::stdout().flush().context("flush prompt")?;

        let mut line = String::new();
        if stdin_lock.read_line(&mut line).context("read chat input")? == 0 {
            break;
        }

        let message = line.trim();
        if message.is_empty() {
            continue;
        }

        // Build request based on backend.
        let (url, body) = match &backend {
            ChatBackend::Sidecar(base) => (
                format!("{}/message", base.trim_end_matches('/')),
                json!({ "prompt": message }),
            ),
            ChatBackend::Serve(base) | ChatBackend::Unreachable(base) => (
                format!(
                    "{}/api/agents/{agent_id}/message",
                    base.trim_end_matches('/')
                ),
                json!({ "message": message }),
            ),
        };

        let response = match client.post(&url).json(&body).send().await {
            Ok(resp) => resp,
            Err(err) => {
                eprintln!("[connection error] {err}");
                match &backend {
                    ChatBackend::Sidecar(_) => {
                        eprintln!(
                            "  Is the agent sidecar running? Try: roko agent serve --agent-id {agent_id}"
                        );
                    }
                    ChatBackend::Serve(_) | ChatBackend::Unreachable(_) => {
                        eprintln!("  Is roko-serve running? Try: roko serve");
                    }
                }
                println!();
                continue;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            eprintln!("[request failed: {status}] {body}");
            println!();
            continue;
        }

        // Yellow prompt for agent output.
        print!("\x1b[33m{agent_id}>\x1b[0m ");
        io::stdout().flush().context("flush agent prompt")?;
        let body: SendMessageResponse = response.json().await.context("decode chat response")?;

        // Prefer a direct response (already completed inline) over run_id
        // polling.  The serve proxy returns both `run_id` and `response`
        // when the sidecar answered synchronously — polling in that case
        // would 404 because no background run was created.
        if let Some(reply) = body
            .response
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            let cleaned = extract_clean_text(reply);
            println!("{cleaned}");
            if let Some(reasoning) = body
                .reasoning
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                println!();
                println!("[reasoning]");
                println!("{reasoning}");
            }
        } else if let Some(run_id) = body
            .run_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            // run_id polling only works via roko-serve (background run).
            match &backend {
                ChatBackend::Serve(base) | ChatBackend::Unreachable(base) => {
                    wait_for_run_completion(&client, base, run_id).await?;
                }
                _ => {
                    println!("[background run {run_id} — poll not supported in sidecar mode]");
                }
            }
        } else {
            bail!("agent message response did not include run_id or direct response");
        }
        println!();
    }

    println!("\nbye.");
    Ok(())
}

/// Run a chat REPL that talks directly to a provider adapter.
///
/// `provider_name` must be a key in `config.providers` (for example
/// `"anthropic_api"` or `"openai_compat"`). The model is resolved via
/// `config.agent.default_model` or the first model whose provider matches
/// `provider_name`.
pub async fn run_direct_provider_chat(
    agent_id: &str,
    provider_name: &str,
    config: &roko_core::config::schema::RokoConfig,
    workdir: &Path,
) -> Result<()> {
    use crate::learning_helpers::capture_runtime_model_slugs;
    use roko_agent::provider::{AgentOptions, create_agent_for_model};
    use roko_core::agent::resolve_model;
    use roko_core::{Body, Context, Engram, Kind};
    use roko_learn::model_call_feedback::{ModelCallFeedback, ModelCallFeedbackRecorder};

    let model_key = find_model_for_provider(config, provider_name).ok_or_else(|| {
        anyhow::anyhow!(
            "no model configured for provider '{provider_name}'; add a [[models]] entry with provider = \"{provider_name}\" in roko.toml"
        )
    })?;
    let model_slug = resolve_model(config, &model_key).slug;
    let cascade_model_slugs = capture_runtime_model_slugs(config, &model_slug);
    let feedback_recorder = ModelCallFeedbackRecorder::from_workdir(workdir, cascade_model_slugs);

    let llm_timeout_ms = config.timeouts.llm_call().as_millis() as u64;
    let options = AgentOptions {
        name: agent_id.to_string(),
        timeout_ms: Some(llm_timeout_ms),
        ..Default::default()
    };

    let agent = create_agent_for_model(config, &model_key, options)
        .map_err(|e| anyhow::anyhow!("create agent: {e}"))?;
    println!("roko chat (direct) — provider: {provider_name}, model: {model_key}");
    println!("Type a message. Press Ctrl-D to exit.\n");

    let started_at = Utc::now();
    let mut turn_count: u32 = 0;
    let mut first_message = String::new();
    let mut last_message = String::new();
    let mut total_tokens: u64 = 0;

    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();
    let mut history: Vec<serde_json::Value> = Vec::new();

    loop {
        print!("\x1b[36myou>\x1b[0m ");
        std::io::stdout().flush().context("flush prompt")?;

        let mut line = String::new();
        if stdin_lock.read_line(&mut line).context("read input")? == 0 {
            break;
        }
        let message = line.trim();
        if message.is_empty() {
            continue;
        }

        if first_message.is_empty() {
            first_message = message.chars().take(120).collect();
        }
        last_message = message.chars().take(120).collect();
        turn_count += 1;

        history.push(json!({
            "role": "user",
            "content": message,
        }));

        let prompt_text = render_history_prompt(&history);
        let engram = Engram::builder(Kind::Prompt)
            .body(Body::text(&prompt_text))
            .build();
        let ctx = Context::now();

        let indicator = if agent.supports_streaming() {
            "[streaming...]"
        } else {
            "[waiting...]"
        };
        print!("{indicator} ");
        std::io::stdout().flush().context("flush indicator")?;

        let turn_started = Instant::now();
        let result = agent.run(&engram, &ctx).await;
        let latency_ms = turn_started.elapsed().as_millis() as u64;
        total_tokens +=
            u64::from(result.usage.input_tokens) + u64::from(result.usage.output_tokens);
        if let Err(error) = feedback_recorder
            .record(ModelCallFeedback {
                run_id: None,
                request_id: Some(format!("direct-chat-{agent_id}-{turn_count}")),
                prompt_section_ids: Vec::new(),
                knowledge_ids: Vec::new(),
                model: model_slug.clone(),
                provider: provider_name.to_string(),
                role: "chat_direct".to_string(),
                input_tokens: u64::from(result.usage.input_tokens),
                output_tokens: u64::from(result.usage.output_tokens),
                cost_usd: f64::from(result.usage.cost_usd),
                latency_ms,
                success: result.success,
                provider_success: Some(result.success),
            })
            .await
        {
            tracing::warn!(
                provider = %provider_name,
                model = %model_slug,
                error = %error,
                "failed to record direct provider chat feedback"
            );
        }

        print!("\r\x1b[K");

        print!("\x1b[33m{agent_id}>\x1b[0m ");
        std::io::stdout().flush().context("flush agent prompt")?;

        let text = result.output.body.as_text().unwrap_or("[no text output]");
        history.push(json!({
            "role": "assistant",
            "content": text,
            "success": result.success,
        }));

        if result.success {
            println!("{text}");
        } else {
            eprintln!("[agent error] {text}");
        }
        println!();
    }

    println!("\nbye.");

    let ended_at = Utc::now();
    let session_id = format!("{}-{}", started_at.format("%Y-%m-%dT%H-%M-%S"), agent_id);
    let summary = SessionSummary {
        session_id,
        agent_id: agent_id.to_string(),
        provider: provider_name.to_string(),
        model_key: model_key.clone(),
        started_at: started_at.to_rfc3339(),
        ended_at: ended_at.to_rfc3339(),
        turn_count,
        first_message,
        last_message,
        total_tokens,
        total_cost_usd: 0.0,
    };
    save_summary_nonblocking(workdir.to_path_buf(), summary);
    tokio::task::yield_now().await;
    Ok(())
}

fn render_history_prompt(history: &[serde_json::Value]) -> String {
    let mut prompt = String::new();

    for message in history {
        let role = message.get("role").and_then(serde_json::Value::as_str);
        let content = message
            .get("content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if content.trim().is_empty() {
            continue;
        }

        let label = match role {
            Some("assistant") => "Assistant",
            Some("system") => "System",
            _ => "User",
        };

        prompt.push_str(label);
        prompt.push_str(":\n");
        prompt.push_str(content);
        prompt.push_str("\n\n");
    }

    prompt.trim_end().to_string()
}

/// Find the first model key in `config` whose provider name matches `provider_name`.
///
/// Tries `config.agent.default_model` first; if that model's provider matches,
/// returns it. Otherwise scans `config.models` for the first match.
fn find_model_for_provider(
    config: &roko_core::config::schema::RokoConfig,
    provider_name: &str,
) -> Option<String> {
    if !config.agent.default_model.is_empty() {
        let default_model = &config.agent.default_model;
        if let Some(profile) = config.models.get(default_model.as_str()) {
            if profile.provider == provider_name {
                return Some(default_model.clone());
            }
        }
    }

    config
        .models
        .iter()
        .find(|(_, profile)| profile.provider == provider_name)
        .map(|(key, _)| key.clone())
}

/// Determine which backend to use. Prefers sidecar when available.
async fn resolve_backend(
    client: &reqwest::Client,
    agent_id: &str,
    serve_url: &str,
    workdir: &Path,
) -> ChatBackend {
    // 1. Try sidecar from agents.json (local runtime registry).
    if let Some(sidecar_url) = lookup_sidecar(agent_id, workdir) {
        if probe_sidecar(client, &sidecar_url).await {
            return ChatBackend::Sidecar(sidecar_url);
        }
        eprintln!("\u{26a0} Agent '{agent_id}' registered at {sidecar_url} but not reachable.");
    }

    // 2. Query roko-serve's agent registry for the sidecar URL.
    //    This covers agents that registered via auto-port and are only
    //    known to the control plane.
    if let Some(sidecar_url) = lookup_sidecar_from_serve(client, agent_id, serve_url).await {
        if probe_sidecar(client, &sidecar_url).await {
            return ChatBackend::Sidecar(sidecar_url);
        }
    }

    // 3. Try roko-serve (which proxies to the agent if it's registered).
    if probe_health(client, &serve_health_url(serve_url)).await {
        return ChatBackend::Serve(serve_url.to_string());
    }

    eprintln!("\u{26a0} Neither agent sidecar nor roko-serve is reachable.");
    eprintln!("  Start the agent:  roko agent serve --agent-id {agent_id}");
    eprintln!("  Or start serve:   roko serve");
    eprintln!();

    ChatBackend::Unreachable(serve_url.to_string())
}

/// Query roko-serve's `GET /api/agents/{id}` to discover a sidecar URL.
async fn lookup_sidecar_from_serve(
    client: &reqwest::Client,
    agent_id: &str,
    serve_url: &str,
) -> Option<String> {
    let url = format!(
        "{}/api/agents/{}",
        serve_url.trim_end_matches('/'),
        agent_id,
    );
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    body.get("rest_endpoint")
        .or_else(|| body.get("sidecar_url"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Probe whether a sidecar URL is reachable (tries `/health` then `/api/health`).
async fn probe_sidecar(client: &reqwest::Client, base: &str) -> bool {
    probe_health(client, &sidecar_health_url(base)).await
        || probe_health(client, &serve_health_url(base)).await
}

async fn probe_health(client: &reqwest::Client, url: &str) -> bool {
    client
        .get(url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

async fn wait_for_run_completion(
    client: &reqwest::Client,
    serve_url: &str,
    run_id: &str,
) -> Result<()> {
    let status_url = format!(
        "{}/api/run/{run_id}/status",
        serve_url.trim_end_matches('/')
    );

    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let response = client
            .get(&status_url)
            .send()
            .await
            .context("poll run status")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("run status request failed: {status} {body}");
        }

        let status: RunStatusResponse = response
            .json()
            .await
            .context("decode run status response")?;
        if status.finished {
            if status.status.eq_ignore_ascii_case("failed") {
                if let Some(error) = status
                    .error
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    println!("[failed] {error}");
                } else {
                    println!("[failed]");
                }
            } else {
                match status
                    .output_text
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    Some(output) => println!("{output}"),
                    None => println!("[completed]"),
                }
            }
            break;
        }
    }

    Ok(())
}

/// Extract clean text from a response that might contain raw JSON.
///
/// Used by both the chat REPL and `ServingAgentDispatcher` to strip
/// Claude CLI streaming protocol JSON from agent responses.
///
/// Handles three cases:
///   1. Plain text — returned as-is.
///   2. JSON object with a `result` or `content` field (Claude CLI streaming
///      protocol) — the text value of that field is returned.
///   3. JSON array of content blocks (`[{"type":"text","text":"..."}]`) —
///      all text blocks are concatenated.
pub fn extract_clean_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return raw.to_string();
    }

    // Try parsing as a single JSON object.
    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(trimmed) {
        // Claude CLI streaming protocol: `{ "result": "..." }`
        if let Some(text) = obj
            .get("result")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return text.to_string();
        }
        // Sidecar wrapping: `{ "content": "..." }`
        if let Some(text) = obj
            .get("content")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return text.to_string();
        }
        // Content blocks array inside object: `{ "content": [{"type":"text","text":"..."}] }`
        if let Some(blocks) = obj.get("content").and_then(serde_json::Value::as_array) {
            let texts = extract_text_blocks(blocks);
            if !texts.is_empty() {
                return texts;
            }
        }
    }

    // Try parsing as a JSON array of content blocks.
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
        let texts = extract_text_blocks(&arr);
        if !texts.is_empty() {
            return texts;
        }
    }

    // Multi-line JSONL: Claude CLI streaming protocol produces one JSON
    // object per line with types like `system`, `assistant`, `result`, etc.
    // Extract text from assistant message content blocks and result events.
    if trimmed.contains('\n') {
        let mut parts = Vec::new();
        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let event_type = obj.get("type").and_then(serde_json::Value::as_str);
            match event_type {
                // Claude CLI result event: `{"type":"result","result":"the text",...}`
                Some("result") => {
                    if let Some(text) = obj
                        .get("result")
                        .and_then(serde_json::Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        parts.push(text.to_string());
                    }
                }
                // Claude CLI assistant event: `{"type":"assistant","message":{"content":[...]}}`
                Some("assistant") => {
                    if let Some(blocks) = obj
                        .pointer("/message/content")
                        .and_then(serde_json::Value::as_array)
                    {
                        let text = extract_text_blocks(blocks);
                        if !text.is_empty() {
                            parts.push(text);
                        }
                    }
                }
                // Claude CLI tool event: carries output of tool calls (Bash, Read, etc.)
                Some("tool") => {
                    let content = obj
                        .get("content")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| obj.get("output").and_then(serde_json::Value::as_str));
                    if let Some(content) = content.filter(|s| !s.is_empty()) {
                        let tool_name = obj
                            .get("tool")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("tool");
                        // Include tool output inline, truncated for sanity
                        let truncated = if content.len()
                            > roko_core::defaults::DEFAULT_TOOL_OUTPUT_TRUNCATE_AT
                        {
                            let mut end = roko_core::defaults::DEFAULT_TOOL_OUTPUT_TRUNCATE_AT;
                            while !content.is_char_boundary(end) {
                                end -= 1;
                            }
                            format!("[{tool_name}] {}...[truncated]", &content[..end])
                        } else {
                            format!("[{tool_name}] {content}")
                        };
                        parts.push(truncated);
                    }
                }
                // Generic: look for top-level `result` or `content` string fields.
                _ => {
                    if let Some(text) = obj
                        .get("result")
                        .and_then(serde_json::Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        parts.push(text.to_string());
                    } else if let Some(text) = obj
                        .get("content")
                        .and_then(serde_json::Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        parts.push(text.to_string());
                    }
                }
            }
        }
        if !parts.is_empty() {
            return parts.join("");
        }
    }

    // Fallback: return raw text.
    raw.to_string()
}

/// Extract text from an array of Anthropic content blocks.
fn extract_text_blocks(blocks: &[serde_json::Value]) -> String {
    blocks
        .iter()
        .filter_map(|block| {
            if block.get("type").and_then(serde_json::Value::as_str) == Some("text") {
                block.get("text").and_then(serde_json::Value::as_str)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_status_defaults_deserialize() {
        let status: RunStatusResponse =
            serde_json::from_value(json!({ "finished": true })).expect("decode run status");
        assert!(status.finished);
        assert!(status.status.is_empty());
        assert!(status.output_text.is_none());
        assert!(status.error.is_none());
    }

    #[test]
    fn send_message_response_accepts_background_run_shape() {
        let response: SendMessageResponse =
            serde_json::from_value(json!({ "run_id": "run-123" })).expect("decode run response");
        assert_eq!(response.run_id.as_deref(), Some("run-123"));
        assert!(response.response.is_none());
    }

    #[test]
    fn send_message_response_accepts_direct_sidecar_shape() {
        let response: SendMessageResponse = serde_json::from_value(json!({
            "response": "done",
            "reasoning": "looked at the diff"
        }))
        .expect("decode direct response");
        assert!(response.run_id.is_none());
        assert_eq!(response.response.as_deref(), Some("done"));
        assert_eq!(response.reasoning.as_deref(), Some("looked at the diff"));
    }

    #[test]
    fn serve_health_url_strips_trailing_slash() {
        assert_eq!(
            serve_health_url("http://localhost:6677/"),
            "http://localhost:6677/api/health"
        );
        assert_eq!(
            serve_health_url("http://localhost:6677"),
            "http://localhost:6677/api/health"
        );
    }

    #[test]
    fn sidecar_health_url_uses_plain_health() {
        assert_eq!(
            sidecar_health_url("http://127.0.0.1:8081"),
            "http://127.0.0.1:8081/health"
        );
        assert_eq!(
            sidecar_health_url("http://127.0.0.1:8081/"),
            "http://127.0.0.1:8081/health"
        );
    }

    #[test]
    fn extract_clean_text_plain() {
        assert_eq!(extract_clean_text("hello world"), "hello world");
    }

    #[test]
    fn extract_clean_text_json_result() {
        assert_eq!(
            extract_clean_text(r#"{"result": "The answer is 42"}"#),
            "The answer is 42"
        );
    }

    #[test]
    fn extract_clean_text_json_content_string() {
        assert_eq!(extract_clean_text(r#"{"content": "Hi there"}"#), "Hi there");
    }

    #[test]
    fn extract_clean_text_content_blocks() {
        let input = r#"[{"type":"text","text":"Hello"},{"type":"text","text":" world"}]"#;
        assert_eq!(extract_clean_text(input), "Hello world");
    }

    #[test]
    fn extract_clean_text_jsonl() {
        let input = "{\"result\": \"part1\"}\n{\"result\": \"part2\"}";
        assert_eq!(extract_clean_text(input), "part1part2");
    }

    #[test]
    fn extract_clean_text_claude_cli_streaming_protocol() {
        // Simulates the Claude CLI --output-format stream-json output.
        let input = r#"{"type":"system","subtype":"init","cwd":"/tmp","session_id":"abc"}
{"type":"assistant","message":{"model":"claude-sonnet-4-6","content":[{"type":"text","text":"Hello! How can I help?"}]}}
{"input_tokens":10,"cache_creation_input_tokens":0}
{"type":"result","subtype":"success","is_error":false,"result":"Hello! How can I help?","duration_ms":2000}"#;
        assert_eq!(
            extract_clean_text(input),
            "Hello! How can I help?Hello! How can I help?"
        );
    }

    #[test]
    fn extract_clean_text_claude_cli_result_only() {
        // When only the result event has text.
        let input = r#"{"type":"system","subtype":"init","cwd":"/tmp"}
{"type":"result","subtype":"success","result":"Hi there."}"#;
        assert_eq!(extract_clean_text(input), "Hi there.");
    }

    #[test]
    fn extract_clean_text_fallback() {
        // JSON that doesn't match any known pattern returns raw.
        assert_eq!(extract_clean_text(r#"{"foo": "bar"}"#), r#"{"foo": "bar"}"#);
    }

    #[test]
    fn sidecar_url_gets_http_prefix() {
        // Simulate what lookup_sidecar would return for a bare bind address.
        let bind = "127.0.0.1:8081";
        let url = if bind.starts_with("http") {
            bind.to_string()
        } else {
            format!("http://{bind}")
        };
        assert_eq!(url, "http://127.0.0.1:8081");
    }

    #[test]
    fn render_history_prompt_formats_turns() {
        let history = vec![
            json!({ "role": "user", "content": "hello" }),
            json!({ "role": "assistant", "content": "world" }),
            json!({ "role": "assistant", "content": "" }),
        ];

        let prompt = render_history_prompt(&history);

        assert!(prompt.contains("User:\nhello"));
        assert!(prompt.contains("Assistant:\nworld"));
        assert!(!prompt.contains("Assistant:\n\n"));
    }

    fn model(provider: &str, slug: &str) -> roko_core::config::schema::ModelProfile {
        roko_core::config::schema::ModelProfile {
            provider: provider.to_string(),
            slug: slug.to_string(),
            context_window: 128_000,
            max_output: Some(1_024),
            supports_tools: true,
            supports_thinking: false,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            supports_grounding: false,
            supports_code_execution: false,
            supports_caching: false,
            provider_routing: None,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: None,
            cost_output_per_m: None,
            cost_input_per_m_high: None,
            cost_output_per_m_high: None,
            cost_cache_read_per_m: None,
            cost_cache_write_per_m: None,
            thinking_level: None,
            max_tools: None,
            tokenizer_ratio: None,
            ..Default::default()
        }
    }

    #[test]
    fn find_model_for_provider_prefers_default_model() {
        let mut config = roko_core::config::schema::RokoConfig::default();
        config.agent.default_model = "default-model".to_string();
        config.models.insert(
            "default-model".to_string(),
            model("anthropic_api", "default-model"),
        );
        config.models.insert(
            "fallback-model".to_string(),
            model("anthropic_api", "fallback-model"),
        );

        assert_eq!(
            find_model_for_provider(&config, "anthropic_api"),
            Some("default-model".to_string())
        );
    }

    #[test]
    fn find_model_for_provider_scans_for_matching_provider() {
        let mut config = roko_core::config::schema::RokoConfig::default();
        config.agent.default_model = "unrelated".to_string();
        config.models.insert(
            "anthropic-model".to_string(),
            model("anthropic_api", "anthropic-model"),
        );
        config.models.insert(
            "openai-model".to_string(),
            model("openai_compat", "openai-model"),
        );

        assert_eq!(
            find_model_for_provider(&config, "openai_compat"),
            Some("openai-model".to_string())
        );
        assert_eq!(find_model_for_provider(&config, "missing"), None);
    }
}
