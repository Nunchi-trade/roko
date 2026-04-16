//! `roko chat` REPL.

use std::io::{self, BufRead, Write};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct SendMessageResponse {
    run_id: String,
}

#[derive(Debug, Deserialize)]
struct RunStatusResponse {
    #[serde(default)]
    finished: bool,
    #[serde(default)]
    status: String,
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

        let body: SendMessageResponse = response.json().await.context("decode chat response")?;
        if body.run_id.trim().is_empty() {
            bail!("agent message response did not include run_id");
        }

        print!("{agent_id}> ");
        io::stdout().flush().context("flush agent prompt")?;
        wait_for_run_completion(&client, serve_url, &body.run_id).await?;
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
                println!("[failed]");
            } else {
                println!("[completed]");
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
    }
}
