//! Agent event handler — updates `RunState` and `TuiBridge` in response
//! to `AgentEvent`s from the stream parser.
//!
//! When streaming output is enabled, key events are rendered through the
//! runner's output sink (`RunOutputSink`) so the operator sees what the
//! agent is doing instead of a static spinner.

use tracing::{debug, info};

use super::output_sink::{RunOutputSink, TokenUsage};
use super::state::RunState;
use super::tui_bridge::TuiBridge;
use super::types::AgentEvent;

/// Maximum bytes retained in `agent_output`. When exceeded, the buffer is
/// trimmed to keep the tail (most recent output), which is what replan
/// context and diagnostics need.
const MAX_AGENT_OUTPUT: usize = 32_768;

/// Process a single agent event, updating state and publishing to TUI.
///
/// Output rendering is delegated to the provided `sink`. The sink handles
/// text buffering internally (e.g., `StderrSink` accumulates deltas and
/// flushes on structural boundaries).
pub(crate) fn handle_agent_event(
    event: &AgentEvent,
    state: &mut RunState,
    tui: &TuiBridge,
    sink: &dyn RunOutputSink,
) {
    let plan_id = &state.plan_id;
    let task_id = &state.current_task;

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
            sink.agent_started(plan_id, task_id, provider, model, *pid);
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

            sink.agent_text_delta(plan_id, task_id, text);
        }

        AgentEvent::ToolCall { id, name } => {
            let marker = format!("\n[tool: {name}]\n");
            state.agent_output.push_str(&marker);

            sink.tool_call(plan_id, task_id, id, name);
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

            sink.tool_output(plan_id, task_id, id, output);
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

            sink.token_usage(
                plan_id,
                task_id,
                TokenUsage {
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    cache_read_tokens: *cache_read_tokens,
                    cache_write_tokens: *cache_write_tokens,
                },
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

            sink.agent_turn_completed(
                plan_id,
                task_id,
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

            sink.agent_error(plan_id, task_id, message);
        }

        AgentEvent::Exited { exit_code } => {
            state.agent_active = false;
            state.agent_pid = None;
            debug!(exit_code = ?exit_code, task = %state.current_task, "agent process exited");
        }
    }
}

/// Derive an agent identifier from the current state.
fn agent_id_for_state(state: &RunState) -> String {
    format!("{}/{}", state.plan_id, state.current_task)
}
