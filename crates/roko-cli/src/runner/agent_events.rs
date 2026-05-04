//! Agent event handler — updates `RunState` and `TuiBridge` in response
//! to `AgentEvent`s from the stream parser.
//!
//! When `stream_to_stderr` is true, key events are printed to stderr in
//! real time so the operator sees what the agent is doing instead of a
//! static spinner.

use tracing::debug;

use super::state::RunState;
use super::tui_bridge::TuiBridge;
use super::types::AgentEvent;

/// Buffered stderr streamer for agent text output.
///
/// Accumulates `MessageDelta` text and flushes the last few lines when a
/// structural event (tool call, turn completion) arrives. This avoids
/// flooding the terminal while still showing recent context.
#[derive(Debug, Default)]
pub struct AgentStreamBuffer {
    buf: String,
}

impl AgentStreamBuffer {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
        }
    }

    /// Append a text delta to the buffer.
    pub fn push(&mut self, text: &str) {
        self.buf.push_str(text);
    }

    /// Flush the buffer, printing the last N non-empty lines to stderr.
    /// Each line is truncated to `max_chars` and prefixed with `prefix`.
    pub fn flush(&mut self, max_lines: usize, max_chars: usize) {
        if self.buf.trim().is_empty() {
            self.buf.clear();
            return;
        }

        let lines: Vec<&str> = self
            .buf
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let start = lines.len().saturating_sub(max_lines);
        for line in &lines[start..] {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.len() > max_chars {
                eprintln!("     \u{2502} {}...", &trimmed[..max_chars]);
            } else {
                eprintln!("     \u{2502} {trimmed}");
            }
        }
        self.buf.clear();
    }

    /// Discard accumulated text without printing.
    pub fn clear(&mut self) {
        self.buf.clear();
    }
}

/// Process a single agent event, updating state and publishing to TUI.
///
/// When `stream_to_stderr` is true, structural events are printed to
/// stderr for real-time operator feedback. `stream_buf` accumulates
/// `MessageDelta` text and is flushed on tool calls / turn completion.
pub fn handle_agent_event(
    event: &AgentEvent,
    state: &mut RunState,
    tui: &TuiBridge,
    stream_to_stderr: bool,
    stream_buf: &mut AgentStreamBuffer,
) {
    match event {
        AgentEvent::Started {
            agent_id: _,
            provider,
            model,
            pid,
        } => {
            state.agent_active = true;
            state.agent_model = model.clone();
            state.agent_provider = provider.clone();
            state.agent_pid = *pid;
        }

        AgentEvent::SystemInit { session_id, model } => {
            state.agent_active = true;
            state.agent_model = model.clone();
            state.session_id = Some(session_id.clone());
            debug!(model = %model, session_id = %session_id, "agent initialized");
        }

        AgentEvent::MessageDelta { text } => {
            state.agent_output.push_str(text);
            let agent_id = agent_id_for_state(state);
            tui.agent_output(&agent_id, text);

            if stream_to_stderr {
                stream_buf.push(text);
            }
        }

        AgentEvent::ToolCall { id: _, name } => {
            let marker = format!("\n[tool: {name}]\n");
            state.agent_output.push_str(&marker);

            if stream_to_stderr {
                // Flush buffered text before showing tool call.
                stream_buf.flush(3, 120);
                eprintln!("     \u{2502} \u{1f527} {name}");
            }
        }

        AgentEvent::ToolOutput { id: _, output } => {
            // Truncate tool output in the accumulated buffer.
            let limit = roko_core::defaults::DEFAULT_TOOL_OUTPUT_TRUNCATE_AT;
            let truncated = if output.len() > limit {
                &output[..limit]
            } else {
                output.as_str()
            };
            state.agent_output.push_str(truncated);
            state.agent_output.push('\n');

            if stream_to_stderr {
                // Show abbreviated tool output — first line, up to 80 chars.
                let first_line = output.lines().next().unwrap_or("").trim();
                if !first_line.is_empty() {
                    if first_line.len() > 80 {
                        eprintln!("     \u{2502}   {}...", &first_line[..80]);
                    } else {
                        eprintln!("     \u{2502}   {first_line}");
                    }
                }
            }
        }

        AgentEvent::TokenUsage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        } => {
            state.tokens_in += input_tokens;
            state.tokens_out += output_tokens;
            state.cache_read_tokens += cache_read_tokens;
            state.cache_write_tokens += cache_write_tokens;
            // Token counts are accumulated here; authoritative cost comes from
            // TurnCompleted.total_cost_usd which overwrites state.cost_usd.

            if stream_to_stderr {
                let total = input_tokens + output_tokens;
                eprintln!(
                    "     \u{2502} tokens: {total} (in:{input_tokens} out:{output_tokens})"
                );
            }
        }

        AgentEvent::TurnCompleted {
            session_id,
            total_cost_usd,
            num_turns: _,
            is_error,
        } => {
            state.agent_active = false;
            state.agent_turn_completed = true;
            if let Some(sid) = session_id {
                state.session_id = Some(sid.clone());
            }
            if let Some(cost) = total_cost_usd {
                // Use the authoritative cost from the result event.
                state.cost_usd = *cost;
            }
            if *is_error {
                state.agent_output.push_str("\n[agent error]\n");
            }
            let agent_id = agent_id_for_state(state);
            tui.agent_completed(&agent_id);
            debug!(
                task = %state.current_task,
                tokens_in = state.tokens_in,
                tokens_out = state.tokens_out,
                cost = state.cost_usd,
                "agent turn completed"
            );

            if stream_to_stderr {
                // Flush any remaining buffered text.
                stream_buf.flush(3, 120);
                if *is_error {
                    eprintln!("     \u{2717} Agent turn completed with error");
                } else {
                    let cost_str = total_cost_usd
                        .map(|c| format!(", ${c:.2}"))
                        .unwrap_or_default();
                    eprintln!("     \u{2713} Agent turn complete{cost_str}");
                }
            }
        }

        AgentEvent::Error { message } => {
            state
                .agent_output
                .push_str(&format!("\n[error: {message}]\n"));
            tui.error(message);

            if stream_to_stderr {
                stream_buf.flush(3, 120);
                let msg = if message.len() > 120 {
                    format!("{}...", &message[..120])
                } else {
                    message.clone()
                };
                eprintln!("     \u{2717} Error: {msg}");
            }
        }

        AgentEvent::Exited { exit_code } => {
            state.agent_active = false;
            state.agent_pid = None;
            debug!(exit_code = ?exit_code, task = %state.current_task, "agent process exited");

            if stream_to_stderr {
                stream_buf.clear();
            }
        }
    }
}

/// Derive an agent identifier from the current state.
fn agent_id_for_state(state: &RunState) -> String {
    format!("{}/{}", state.plan_id, state.current_task)
}
