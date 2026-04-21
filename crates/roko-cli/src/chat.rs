//! `roko chat` REPL.

use std::io::{self, BufRead, Write};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

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

/// Run the chat REPL against a roko-serve instance.
pub async fn run_chat_repl(agent_id: &str, serve_url: &str) -> Result<()> {
    println!("roko chat — talking to agent '{agent_id}'");
    println!("Type a message. Press Ctrl-D to exit.\n");

    let client = reqwest::Client::new();
    let stdin = io::stdin();
    let mut stdin_lock = stdin.lock();

    loop {
        print!("you> ");
        io::stdout().flush().context("flush prompt")?;

        let mut line = String::new();
        if stdin_lock.read_line(&mut line).context("read chat input")? == 0 {
            break;
        }

        let message = line.trim();
        if message.is_empty() {
            continue;
        }

        let response = client
            .post(format!(
                "{}/api/agents/{agent_id}/message",
                serve_url.trim_end_matches('/')
            ))
            .json(&json!({ "message": message }))
            .send()
            .await
            .context("send chat message")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            eprintln!("[request failed: {status}] {body}");
            println!();
            continue;
        }

        print!("{agent_id}> ");
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
}
