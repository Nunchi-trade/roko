//! Observer hooks for LLM inference calls.

/// Receives lifecycle notifications around a model inference request.
///
/// Implementations must be non-blocking. If they need async work, they should
/// enqueue internally and return immediately.
pub trait InferenceObserver: Send + Sync {
    /// Called immediately before a backend inference request starts.
    fn on_start(&self, request_id: &str, model: &str, agent_id: &str, auto_routed: bool);

    /// Called after a backend inference request completes successfully.
    fn on_complete(
        &self,
        request_id: &str,
        model: &str,
        agent_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        duration_ms: u64,
    );

    /// Called after a backend inference request fails.
    fn on_error(&self, request_id: &str, model: &str, agent_id: &str, error: &str);
}

/// No-op observer for call sites that do not have an event pipeline.
#[derive(Debug, Default)]
pub struct NoopInferenceObserver;

impl InferenceObserver for NoopInferenceObserver {
    fn on_start(&self, _request_id: &str, _model: &str, _agent_id: &str, _auto_routed: bool) {}

    fn on_complete(
        &self,
        _request_id: &str,
        _model: &str,
        _agent_id: &str,
        _input_tokens: u64,
        _output_tokens: u64,
        _cost_usd: f64,
        _duration_ms: u64,
    ) {
    }

    fn on_error(&self, _request_id: &str, _model: &str, _agent_id: &str, _error: &str) {}
}
