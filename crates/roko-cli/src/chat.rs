//! `roko chat` REPL.
//!
//! Supports two backends:
//!   1. **Direct sidecar** — talks to the agent's HTTP sidecar at its bind address
//!      (looked up from `.roko/runtime/agents.json`). This is the default when a
//!      sidecar is running and no `--serve-url` override points elsewhere.
//!   2. **Via roko-serve** — routes messages through the control plane at `:6677`.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

use crate::auth;

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
}
