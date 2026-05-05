//! Agent event handler — updates `RunState` and `TuiBridge` in response
//! to `AgentEvent`s from the stream parser.
//!
//! When streaming output is enabled, key events are rendered through the
//! runner's inline terminal so the operator sees what the agent is doing
//! instead of a static spinner.

use tracing::{debug, info};

use super::inline_output::RunnerInlineTerminal;
use super::state::RunState;
use super::tui_bridge::TuiBridge;
use super::types::AgentEvent;

/// Maximum bytes retained in `agent_output`. When exceeded, the buffer is
/// trimmed to keep the tail (most recent output), which is what replan
/// context and diagnostics need.
const MAX_AGENT_OUTPUT: usize = 32_768;

/// Buffered inline streamer for agent text output.
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
        Self { buf: String::new() }
    }

    /// Append a text delta to the buffer.
    pub fn push(&mut self, text: &str) {
        self.buf.push_str(text);
    }

    /// Drain the last N non-empty lines, truncating each to `max_chars`.
    pub fn drain_lines(&mut self, max_lines: usize, max_chars: usize) -> Vec<String> {
        if self.buf.trim().is_empty() {
            self.buf.clear();
            return Vec::new();
        }

        let lines: Vec<&str> = self.buf.lines().filter(|l| !l.trim().is_empty()).collect();
        let start = lines.len().saturating_sub(max_lines);
        let drained = lines[start..]
            .iter()
            .map(|line| truncate_chars(line.trim(), max_chars))
            .filter(|line| !line.is_empty())
            .collect();
        self.buf.clear();
        drained
    }

    /// Discard accumulated text without printing.
    pub fn clear(&mut self) {
        self.buf.clear();
    }
}

/// Process a single agent event, updating state and publishing to TUI.
///
/// When streaming output is enabled, structural events are rendered through
/// the inline terminal. `stream_buf` accumulates `MessageDelta` text and is
/// flushed on tool calls / turn completion.
pub(crate) fn handle_agent_event(
    event: &AgentEvent,
    state: &mut RunState,
    tui: &TuiBridge,
    inline: &mut RunnerInlineTerminal,
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
            inline.agent_started(provider, model, *pid);
        }

        AgentEvent::SystemInit { session_id, model } => {
            state.agent_active = true;
            state.agent_model = model.clone();
            state.session_id = Some(session_id.clone());
            debug!(model = %model, session_id = %session_id, "agent initialized");
        }

        AgentEvent::MessageDelta { text } => {
            state.agent_output.push_str(text);
            if state.agent_output.len() > MAX_AGENT_OUTPUT {
                let trim_point = state.agent_output.len() - MAX_AGENT_OUTPUT / 2;
                let boundary = state.agent_output.ceil_char_boundary(trim_point);
                state.agent_output = format!(
                    "[...truncated {}B...]\n{}",
                    boundary,
                    &state.agent_output[boundary..],
                );
                debug!(
                    trimmed_to = state.agent_output.len(),
                    "agent_output exceeded cap, trimmed to tail"
                );
            }
            let agent_id = agent_id_for_state(state);
            tui.agent_output(&agent_id, text);

            if inline.is_enabled() {
                stream_buf.push(text);
            }
        }

        AgentEvent::ToolCall { id, name } => {
            let marker = format!("\n[tool: {name}]\n");
            state.agent_output.push_str(&marker);

            inline.agent_text(stream_buf.drain_lines(3, 120));
            inline.tool_call_started(id, name);
        }

        AgentEvent::ToolOutput { id, output } => {
            // Truncate tool output in the accumulated buffer.
            let limit = roko_core::defaults::DEFAULT_TOOL_OUTPUT_TRUNCATE_AT;
            let truncated = if output.len() > limit {
                &output[..limit]
            } else {
                output.as_str()
            };
            state.agent_output.push_str(truncated);
            state.agent_output.push('\n');

            inline.tool_output(id, output);
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

            inline.token_usage(
                *input_tokens,
                *output_tokens,
                *cache_read_tokens,
                *cache_write_tokens,
                &state.agent_model,
            );
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
            let cost_display = format!("{:.4}", state.cost_usd);
            info!(
                task = %state.current_task,
                plan_id = %state.plan_id,
                tokens_in = state.tokens_in,
                tokens_out = state.tokens_out,
                cache_read = state.cache_read_tokens,
                cache_write = state.cache_write_tokens,
                cost_usd = %cost_display,
                model = %state.agent_model,
                is_error = *is_error,
                "agent turn completed"
            );

            inline.agent_text(stream_buf.drain_lines(3, 120));
            inline.agent_turn_completed(
                *total_cost_usd,
                *is_error,
                &state.agent_model,
                state.tokens_in,
                state.tokens_out,
            );
        }

        AgentEvent::Error { message } => {
            state
                .agent_output
                .push_str(&format!("\n[error: {message}]\n"));
            tui.error(message);

            inline.agent_text(stream_buf.drain_lines(3, 120));
            inline.agent_error(message);
        }

        AgentEvent::Exited { exit_code } => {
            state.agent_active = false;
            state.agent_pid = None;
            debug!(exit_code = ?exit_code, task = %state.current_task, "agent process exited");

            stream_buf.clear();
        }
    }
}

/// Derive an agent identifier from the current state.
fn agent_id_for_state(state: &RunState) -> String {
    format!("{}/{}", state.plan_id, state.current_task)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}
