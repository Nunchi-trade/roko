//! `roko chat` REPL.

use std::io::{self, BufRead, Write};
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

/// Construct the health-check URL for a serve instance.
fn health_url(serve_url: &str) -> String {
    format!("{}/api/health", serve_url.trim_end_matches('/'))
}

/// Run the chat REPL against a roko-serve instance.
pub async fn run_chat_repl(agent_id: &str, serve_url: &str) -> Result<()> {
    println!("roko chat \u{2014} talking to agent '{agent_id}'");
    println!("Type a message. Press Ctrl-D to exit.\n");

    // Resolve API key from CLI flag / env / config (best-effort).
    let api_key = auth::resolve_api_key(
        &roko_core::config::ServeAuthConfig::default(),
        None,
    )
    .map(|r| r.key);

    // Build client with auth headers when a key is available.
    let mut client_builder = reqwest::Client::builder();
    if let Some(ref key) = api_key {
        client_builder = client_builder.default_headers(auth::auth_headers(key));
    }
    let client = client_builder.build().context("build HTTP client")?;

    // Health check: probe serve before entering the REPL.
    match client.get(health_url(serve_url)).send().await {
        Ok(resp) if resp.status().is_success() => {
            eprintln!("Connected to roko-serve at {serve_url}");
        }
        Ok(resp) => {
            eprintln!("\u{26a0} roko-serve at {serve_url} returned {}", resp.status());
            eprintln!("  Chat may not work correctly.");
        }
        Err(err) => {
            eprintln!("\u{26a0} Cannot reach roko-serve at {serve_url}: {err}");
            eprintln!("  Start it with: roko serve");
            eprintln!("  Or check --serve-url flag.");
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

        let response = match client
            .post(format!(
                "{}/api/agents/{agent_id}/message",
                serve_url.trim_end_matches('/')
            ))
            .json(&json!({ "message": message }))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                eprintln!("[connection error] {err}");
                eprintln!("  Is roko-serve running? Try: roko serve");
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
        if let Some(run_id) = body
            .run_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            wait_for_run_completion(&client, serve_url, run_id).await?;
        } else if let Some(reply) = body.response.as_deref() {
            println!("{reply}");
            if let Some(reasoning) = body
                .reasoning
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                println!();
                println!("[reasoning]");
                println!("{reasoning}");
            }
        } else {
            bail!("agent message response did not include run_id or direct response");
        }
        println!();
    }

    println!("\nbye.");
    Ok(())
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
                if let Some(error) = status.error.as_deref().filter(|value| !value.trim().is_empty())
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
    fn health_url_strips_trailing_slash() {
        assert_eq!(
            health_url("http://localhost:6677/"),
            "http://localhost:6677/api/health"
        );
        assert_eq!(
            health_url("http://localhost:6677"),
            "http://localhost:6677/api/health"
        );
    }
}
