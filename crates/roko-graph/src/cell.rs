//! The Cell trait -- universal computation unit for graph nodes.
//!
//! Every node in a `Graph` is backed by a Cell implementation. Cells are
//! instantiated from TOML config via the `CellRegistry` and executed by the
//! graph engine in topological order.

use std::time::Duration;

use async_trait::async_trait;
use roko_core::{Engram, error::Result};

/// Semantic version tuple for Cell implementations.
pub type CellVersion = (u32, u32, u32);

/// Runtime context passed to `Cell::execute()`.
///
/// Provides the cell with access to shared infrastructure (cancel tokens,
/// budgets, trace context) without cells needing to manage their own handles.
#[derive(Debug, Clone)]
pub struct CellContext {
    /// Trace context for observability.
    pub trace_id: Option<String>,
    /// Run identifier (if executing within a Graph/Flow).
    pub run_id: Option<String>,
    /// Remaining budget for this execution (USD).
    pub budget_remaining: Option<f64>,
}

impl CellContext {
    /// Construct a new `CellContext` with no trace or budget info.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            trace_id: None,
            run_id: None,
            budget_remaining: None,
        }
    }

    /// Builder: set the trace ID.
    #[must_use]
    pub fn with_trace_id(mut self, trace_id: String) -> Self {
        self.trace_id = Some(trace_id);
        self
    }

    /// Builder: set the run ID.
    #[must_use]
    pub fn with_run_id(mut self, run_id: String) -> Self {
        self.run_id = Some(run_id);
        self
    }

    /// Builder: set the remaining budget.
    #[must_use]
    pub const fn with_budget(mut self, budget: f64) -> Self {
        self.budget_remaining = Some(budget);
        self
    }
}

impl Default for CellContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Universal computation unit. Every graph node is backed by a Cell implementation.
///
/// The Cell trait provides identity, cost estimation, and an async execute method.
/// Implementations include gates (compile, test, clippy), agent dispatch, compose
/// steps, and user-defined cells registered via `CellRegistry`.
#[async_trait]
pub trait Cell: Send + Sync + 'static {
    /// Unique identifier for this cell instance.
    fn cell_id(&self) -> &str;

    /// Human-readable name for display and logging.
    fn cell_name(&self) -> &str;

    /// Semantic version of this cell's implementation.
    fn cell_version(&self) -> CellVersion {
        (0, 1, 0)
    }

    /// Protocol names this cell conforms to (e.g. `["Gate", "Scorer"]`).
    fn protocols(&self) -> &[&str] {
        &[]
    }

    /// Estimated USD cost per invocation, when known.
    fn estimated_cost(&self) -> Option<f64> {
        None
    }

    /// Estimated wall-clock duration per invocation, when known.
    fn estimated_duration(&self) -> Option<Duration> {
        None
    }

    /// Execute this cell with the given input engrams, producing output engrams.
    ///
    /// The graph engine calls this in topological order, feeding outputs from
    /// upstream cells as inputs to downstream cells.
    async fn execute(&self, input: Vec<Engram>, ctx: &CellContext) -> Result<Vec<Engram>>;
}
