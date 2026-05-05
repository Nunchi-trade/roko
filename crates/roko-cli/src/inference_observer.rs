//! CLI inference observer that publishes canonical RuntimeEvents.

use roko_agent::InferenceObserver;
use roko_core::RuntimeEvent;

/// Emits inference lifecycle events onto the shared runtime event bus.
#[derive(Debug, Default)]
pub struct RuntimeEventInferenceObserver;

impl RuntimeEventInferenceObserver {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl InferenceObserver for RuntimeEventInferenceObserver {
    fn on_start(
        &self,
        run_id: &str,
        request_id: &str,
        model: &str,
        agent_id: &str,
        auto_routed: bool,
    ) {
        roko_runtime::event_bus::emit_runtime_event(RuntimeEvent::InferenceStarted {
            run_id: run_id.to_string(),
            request_id: request_id.to_string(),
            model: model.to_string(),
            agent_id: agent_id.to_string(),
            auto_routed,
        });
    }

    fn on_complete(
        &self,
        run_id: &str,
        request_id: &str,
        model: &str,
        agent_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        duration_ms: u64,
    ) {
        roko_runtime::event_bus::emit_runtime_event(RuntimeEvent::InferenceCompleted {
            run_id: run_id.to_string(),
            request_id: request_id.to_string(),
            model: model.to_string(),
            agent_id: agent_id.to_string(),
            input_tokens,
            output_tokens,
            cost_usd,
            duration_ms,
        });
    }

    fn on_error(&self, run_id: &str, request_id: &str, model: &str, agent_id: &str, error: &str) {
        roko_runtime::event_bus::emit_runtime_event(RuntimeEvent::InferenceFailed {
            run_id: run_id.to_string(),
            request_id: request_id.to_string(),
            model: model.to_string(),
            agent_id: agent_id.to_string(),
            error: error.to_string(),
        });
    }
}
