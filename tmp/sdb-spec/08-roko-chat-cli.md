# Checklist: Add `roko chat` REPL CLI command

## Implementation note (2026-04-15)

- `roko chat` is implemented in `crates/roko-cli/src/chat.rs` and exposed from `crates/roko-cli/src/main.rs`
- The shipped v1 uses `POST /api/agents/{id}/message` plus polling on `GET /api/run/{id}/status`
- The current REPL reports completion or failure status only; it does not yet stream token-by-token agent output

**Priority**: P1 — developer adoption, testing tool
**Estimated LOC**: ~80 lines
**Source**: [GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45) (issue body: "roko run doesn't print agent output to stdout")

## Problem

`roko run` stores output in binary signals, not stdout. For developer adoption, a `roko chat` REPL mode is high-impact. It also serves as a testing tool for the agent messaging pipeline (checklist 05).

## What already exists

- `crates/roko-cli/src/main.rs`: CLI with subcommands: run, serve, status, dream, config, inject, plan, research, neuro, subscription, event-sources, experiment.
- `crates/roko-serve/src/routes/run.rs`: `POST /api/run` spawns execution, `GET /api/run/{id}/status` polls.
- `crates/roko-serve/src/routes/sse.rs`: SSE endpoint for streaming events.
- After checklist 05 lands: `POST /api/agents/{id}/message` + WS `agent_output` events.

## Files to modify

### 1. `crates/roko-cli/src/main.rs`

- [ ] Add `Chat` variant to the CLI enum:
```rust
/// Interactive chat REPL with an agent
Chat {
    /// Agent ID to chat with (default: system agent)
    #[arg(long, default_value = "nunchi-intelligence")]
    agent: String,
    /// roko-serve URL
    #[arg(long, default_value = "http://localhost:6677")]
    serve_url: String,
},
```

- [ ] Add match arm in the main dispatch:
```rust
Command::Chat { agent, serve_url } => {
    chat::run_chat_repl(&agent, &serve_url).await?;
}
```

### 2. New file: `crates/roko-cli/src/chat.rs`

- [ ] Create the REPL:

```rust
use std::io::{self, Write, BufRead};

pub async fn run_chat_repl(agent_id: &str, serve_url: &str) -> anyhow::Result<()> {
    println!("roko chat — talking to agent '{agent_id}'");
    println!("Type your message. Ctrl-D to exit.\n");

    let client = reqwest::Client::new();
    let stdin = io::stdin();

    loop {
        print!("you> ");
        io::stdout().flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF
        }
        let line = line.trim();
        if line.is_empty() { continue; }

        // Send message via POST /api/agents/{id}/message
        let resp = client
            .post(format!("{serve_url}/api/agents/{agent_id}/message"))
            .json(&serde_json::json!({ "content": line }))
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        let run_id = body["run_id"].as_str().unwrap_or("");

        // Poll for completion via GET /api/run/{id}/status
        print!("{agent_id}> ");
        io::stdout().flush()?;

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let status_resp = client
                .get(format!("{serve_url}/api/run/{run_id}/status"))
                .send()
                .await?;
            let status: serde_json::Value = status_resp.json().await?;

            match status["status"].as_str() {
                Some("completed") => {
                    if let Some(output) = status["output"].as_str() {
                        println!("{output}");
                    } else {
                        println!("[completed]");
                    }
                    break;
                }
                Some("failed") => {
                    eprintln!("[failed: {}]", status["error"].as_str().unwrap_or("unknown"));
                    break;
                }
                _ => continue, // still running
            }
        }
        println!();
    }

    println!("\nbye.");
    Ok(())
}
```

**Note**: Once WS streaming (checklist 05) is wired, upgrade to stream response chunks via WS/SSE instead of polling. The polling version is a working v1.

### 3. `crates/roko-cli/src/main.rs`

- [ ] Add `mod chat;` to module declarations

## Usage

```bash
# Chat with default system agent
roko chat

# Chat with a specific agent
roko chat --agent golem-alpha-7f

# Chat against a different roko-serve instance
roko chat --serve-url http://remote:6677
```

## Testing

- [ ] `roko chat` with roko-serve running → enters REPL, can send/receive messages
- [ ] `roko chat --agent nonexistent` → sends message, gets error response gracefully
- [ ] Ctrl-D exits cleanly
- [ ] Empty lines are skipped (no API call)
