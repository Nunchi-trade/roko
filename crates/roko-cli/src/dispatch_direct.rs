//! In-process agent dispatch — no HTTP intermediary required.
//!
//! Dispatches prompts directly via:
//! - Claude CLI subprocess (`claude --print --output-format stream-json`)
//! - Anthropic Messages API (`POST api.anthropic.com/v1/messages`)
//! - OpenAI-compatible chat completions
//!
//! Returns a unified [`DispatchResult`] regardless of backend.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use crate::auth_detect::AuthMethod;
use crate::chat::extract_clean_text;

/// Result of dispatching a prompt to an LLM backend.
#[derive(Debug, Clone)]
pub struct DispatchResult {
    /// The model's text response.
    pub text: String,
    /// Which model answered.
    pub model: String,
    /// Approximate input tokens.
    pub input_tokens: u64,
    /// Approximate output tokens.
    pub output_tokens: u64,
}

/// Dispatch a prompt using the detected auth method.
pub async fn dispatch_prompt(auth: &AuthMethod, prompt: &str) -> Result<DispatchResult> {
    match auth {
        AuthMethod::ClaudeCli => dispatch_claude_cli(prompt).await,
        AuthMethod::AnthropicApi { key } => dispatch_anthropic_api(key, prompt).await,
        AuthMethod::OpenAiCompat {
            key,
            base_url,
            model,
        } => dispatch_openai_compat(key, base_url, model.as_deref(), prompt).await,
        AuthMethod::NeedsSetup => bail!("no auth configured"),
    }
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

    while let Some(line) = lines.next_line().await.context("read claude stdout")? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Try to extract metadata from stream-json events
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(trimmed) {
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
    })
}

// ---------------------------------------------------------------------------
// Anthropic Messages API
// ---------------------------------------------------------------------------

async fn dispatch_anthropic_api(api_key: &str, prompt: &str) -> Result<DispatchResult> {
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "claude-sonnet-4-6-20250514",
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
    let model_name = model.unwrap_or("gpt-4o");

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

    let model = data.model.unwrap_or_else(|| "unknown".to_string());
    let input_tokens = data.usage.as_ref().map_or(0, |u| u.prompt_tokens);
    let output_tokens = data.usage.as_ref().map_or(0, |u| u.completion_tokens);

    Ok(DispatchResult {
        text,
        model,
        input_tokens,
        output_tokens,
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
        };
        assert_eq!(r.text, "hello");
        assert_eq!(r.model, "test");
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
