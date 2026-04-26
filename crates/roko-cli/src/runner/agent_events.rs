//! Agent event handler — updates `RunState` and `TuiBridge` in response
//! to `AgentEvent`s from the stream parser.

use tracing::debug;

use super::state::RunState;
use super::tui_bridge::TuiBridge;
use super::types::AgentEvent;

/// Process a single agent event, updating state and publishing to TUI.
pub fn handle_agent_event(event: &AgentEvent, state: &mut RunState, tui: &TuiBridge) {
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
        }

        AgentEvent::ToolCall { id: _, name } => {
            let marker = format!("\n[tool: {name}]\n");
            state.agent_output.push_str(&marker);
        }

        AgentEvent::ToolOutput { id: _, output } => {
            // Truncate tool output in the accumulated buffer.
            let truncated = if output.len() > 4096 {
                &output[..4096]
            } else {
                output.as_str()
            };
            state.agent_output.push_str(truncated);
            state.agent_output.push('\n');
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
        }

        AgentEvent::Error { message } => {
            state
                .agent_output
                .push_str(&format!("\n[error: {message}]\n"));
            tui.error(message);
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
