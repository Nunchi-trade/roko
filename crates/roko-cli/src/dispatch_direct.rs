//! In-process agent dispatch — no HTTP intermediary required.
//!
//! **DEPRECATED** — This module is no longer on the happy path.
//!
//! `dispatch_prompt` and `dispatch_via_model_call_service` are deprecated.
//! Use `ChatAgentSession` (in `chat_session.rs`) for interactive and one-shot
//! dispatch. Use `ClaudeCliAgent` (in `roko-agent`) for plan execution.
//!
//! This module remains for backward compatibility. Types `DispatchResult`
//! and `ToolOutput` may still be imported by callers. No happy path
//! (interactive, one-shot, or plan-run) should call through it.
//!
//! # Migration
//!
//! - Interactive (`roko` no args): use `ChatAgentSession::send_turn_streaming`
//!   (wired in R3_D01)
//! - One-shot (`roko "prompt"`): use `ChatAgentSession::send_turn_oneshot`
//!   (wired in R3_D02)
//! - Plan execution (`roko plan run`): uses `ClaudeCliAgent` directly in
//!   `orchestrate.rs`

#![cfg(feature = "legacy-orchestrate")]

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use crate::auth_detect::AuthMethod;
use crate::chat::extract_clean_text;
pub use crate::dispatch_v2::{DispatchResult, ToolOutput};

/// Dispatch a prompt using the detected auth method.
///
/// **Deprecated** — use `ChatAgentSession` from `chat_session.rs` instead.
#[deprecated(
    since = "0.1.0",
    note = "Use ChatAgentSession::send_turn_oneshot or send_turn_streaming instead"
)]
pub async fn dispatch_prompt(auth: &AuthMethod, prompt: &str) -> Result<DispatchResult> {
    tracing::warn!(
        "dispatch_direct::dispatch_prompt called — this path is deprecated; \
         use ChatAgentSession instead (see crates/roko-cli/src/chat_session.rs)"
    );
    match auth {
        AuthMethod::ClaudeCli => dispatch_claude_cli(prompt).await,
        AuthMethod::AnthropicApi { key, model } => {
            dispatch_anthropic_api(key, model.as_deref(), prompt).await
        }
        AuthMethod::OpenAiCompat {
            key,
            base_url,
            model,
        } => dispatch_openai_compat(key, base_url, model.as_deref(), prompt).await,
        AuthMethod::NeedsSetup => bail!("no auth configured"),
    }
}

/// Dispatch a prompt through ModelCallService (v2 path).
///
/// Uses the ModelCaller trait that WorkflowEngine uses, giving cost tracking
/// and feedback recording for free.
///
/// **Deprecated** — use `ChatAgentSession` from `chat_session.rs` instead.
#[deprecated(
    since = "0.1.0",
    note = "Use ChatAgentSession::send_turn_oneshot or send_turn_streaming instead"
)]
pub async fn dispatch_via_model_call_service(prompt: &str) -> Result<DispatchResult> {
    tracing::warn!(
        "dispatch_direct::dispatch_via_model_call_service called — this path is deprecated; \
         use ChatAgentSession instead (see crates/roko-cli/src/chat_session.rs)"
    );
    crate::dispatch_v2::dispatch_via_model_call_service(prompt).await
}

// ---------------------------------------------------------------------------
// Claude CLI
// ---------------------------------------------------------------------------

async fn dispatch_claude_cli(prompt: &str) -> Result<DispatchResult> {
    let mut child = Command::new("claude")
        .args(["--print", "--output-format", "stream-json"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("spawn `claude` CLI — is it installed?")?;

    // Write prompt to stdin, then close so claude knows input is done.
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(prompt.as_bytes()).await?;
        drop(stdin);
    }

    let stdout = child.stdout.take().context("capture claude stdout")?;
    let stderr = child.stderr.take();
    let reader = tokio::io::BufReader::new(stdout);
    let mut lines = reader.lines();

    let mut raw_lines = Vec::new();
    let mut model = String::from("claude");
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut tool_outputs = Vec::new();
    let mut session_id: Option<String> = None;

    while let Some(line) = lines.next_line().await.context("read claude stdout")? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Try to extract metadata from stream-json events
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let event_type = event.get("type").and_then(serde_json::Value::as_str);

            match event_type {
                Some("tool") => {
                    // Tool event: carries the output of Bash/Read/Edit/etc tool calls.
                    // Extract from "content" or "output" field (Claude uses both).
                    let content = event
                        .get("content")
                        .and_then(|c| c.as_str())
                        .or_else(|| event.get("output").and_then(|o| o.as_str()));
                    if let Some(content) = content.filter(|s| !s.is_empty()) {
                        // Truncate very large outputs (like mori's 4KB limit)
                        let truncated = if content.len() > 4096 {
                            let mut end = 4096;
                            while !content.is_char_boundary(end) {
                                end -= 1;
                            }
                            format!("{}...[truncated]", &content[..end])
                        } else {
                            content.to_string()
                        };
                        let tool_name =
                            event.get("tool").and_then(|t| t.as_str()).map(String::from);
                        tool_outputs.push(ToolOutput {
                            tool_name,
                            content: truncated,
                        });
                    }
                }
                Some("result") => {
                    // Result event: carries session_id, cost, error flag.
                    if let Some(sid) = event.get("session_id").and_then(serde_json::Value::as_str) {
                        session_id = Some(sid.to_string());
                    }
                    // Usage from result event (often the final/accurate count)
                    if let Some(usage) = event.get("usage") {
                        if let Some(n) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                            input_tokens = n;
                        }
                        if let Some(n) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                            output_tokens = n;
                        }
                    }
                }
                _ => {
                    // Model from assistant event
                    if let Some(m) = event
                        .pointer("/message/model")
                        .or_else(|| event.get("model"))
                        .and_then(serde_json::Value::as_str)
                    {
                        model = m.to_string();
                    }
                    // Token usage from assistant event
                    if let Some(usage) = event
                        .pointer("/message/usage")
                        .or_else(|| event.get("usage"))
                    {
                        if let Some(n) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                            input_tokens = n;
                        }
                        if let Some(n) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                            output_tokens = n;
                        }
                    }
                }
            }
        }
        raw_lines.push(line);
    }

    let status = child.wait().await.context("wait for claude CLI")?;
    if !status.success() {
        // Include stderr in error message for diagnostics.
        let mut stderr_text = String::new();
        if let Some(se) = stderr {
            let stderr_reader = tokio::io::BufReader::new(se);
            let mut stderr_lines = stderr_reader.lines();
            while let Some(line) = stderr_lines.next_line().await.unwrap_or(None) {
                if !stderr_text.is_empty() {
                    stderr_text.push('\n');
                }
                stderr_text.push_str(&line);
            }
        }
        let context = if stderr_text.is_empty() {
            raw_lines.join("\n")
        } else {
            stderr_text
        };
        bail!("claude CLI exited with {status}: {context}");
    }

    let raw = raw_lines.join("\n");
    let text = extract_clean_text(&raw);

    Ok(DispatchResult {
        text,
        model,
        input_tokens,
        output_tokens,
        tool_outputs,
        session_id,
    })
}

// ---------------------------------------------------------------------------
// Anthropic Messages API
// ---------------------------------------------------------------------------

async fn dispatch_anthropic_api(
    api_key: &str,
    model: Option<&str>,
    prompt: &str,
) -> Result<DispatchResult> {
    let client = reqwest::Client::new();
    let model_id = model.unwrap_or("claude-sonnet-4-6-20250514");

    let body = serde_json::json!({
        "model": model_id,
        "max_tokens": 8192,
        "messages": [{"role": "user", "content": prompt}]
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .context("POST api.anthropic.com/v1/messages")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        bail!("Anthropic API {status}: {body_text}");
    }

    let data: AnthropicResponse = resp.json().await.context("decode Anthropic response")?;

    let text = data
        .content
        .iter()
        .filter_map(|block| {
            if block.r#type == "text" {
                block.text.as_deref()
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");

    Ok(DispatchResult {
        text,
        model: data.model,
        input_tokens: data.usage.input_tokens,
        output_tokens: data.usage.output_tokens,
        tool_outputs: Vec::new(),
        session_id: None,
    })
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    model: String,
    content: Vec<AnthropicContentBlock>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    r#type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

// ---------------------------------------------------------------------------
// OpenAI-compatible
// ---------------------------------------------------------------------------

async fn dispatch_openai_compat(
    api_key: &str,
    base_url: &str,
    model: Option<&str>,
    prompt: &str,
) -> Result<DispatchResult> {
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let model_name = model.unwrap_or("gpt-5.4-mini");

    let body = serde_json::json!({
        "model": model_name,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 8192,
    });

    let resp = client
        .post(&url)
        .header("authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        bail!("OpenAI API {status}: {body_text}");
    }

    let data: OpenAiResponse = resp.json().await.context("decode OpenAI response")?;

    let text = data
        .choices
        .first()
        .and_then(|c| c.message.content.as_deref())
        .unwrap_or("")
        .to_string();

    let model = data.model.unwrap_or_default();
    let input_tokens = data.usage.as_ref().map_or(0, |u| u.prompt_tokens);
    let output_tokens = data.usage.as_ref().map_or(0, |u| u.completion_tokens);

    Ok(DispatchResult {
        text,
        model,
        input_tokens,
        output_tokens,
        tool_outputs: Vec::new(),
        session_id: None,
    })
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    #[serde(default)]
    model: Option<String>,
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_result_fields() {
        let r = DispatchResult {
            text: "hello".into(),
            model: "test".into(),
            input_tokens: 10,
            output_tokens: 5,
            tool_outputs: Vec::new(),
            session_id: None,
        };
        assert_eq!(r.text, "hello");
        assert_eq!(r.model, "test");
    }

    #[test]
    fn dispatch_result_from_model_call() {
        let result = DispatchResult {
            text: "hello".into(),
            model: "test-model".into(),
            input_tokens: 10,
            output_tokens: 5,
            tool_outputs: Vec::new(),
            session_id: None,
        };
        assert_eq!(result.text, "hello");
        assert!(result.tool_outputs.is_empty());
    }

    #[test]
    fn anthropic_response_deser() {
        let json = serde_json::json!({
            "model": "claude-sonnet-4-6-20250514",
            "content": [{"type": "text", "text": "Hello"}],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let resp: AnthropicResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.content.len(), 1);
        assert_eq!(resp.usage.input_tokens, 10);
    }

    #[test]
    fn openai_response_deser() {
        let json = serde_json::json!({
            "model": "gpt-4o",
            "choices": [{"message": {"content": "Hi"}, "index": 0}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 3}
        });
        let resp: OpenAiResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.choices[0].message.content.as_deref(), Some("Hi"));
    }
}
