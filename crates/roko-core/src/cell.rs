//! The Cell trait — universal computation unit for all protocol implementations.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::bus_backends::BusErased;
use crate::error::{Result, RokoError};
use crate::traits::Substrate;
use crate::{Engram, Kind};

/// Stable identifier for a Cell instance.
pub type CellId = String;

/// Semantic version tuple for Cell implementations.
pub type CellVersion = (u32, u32, u32);

// ─── CellContext ────────────────────────────────────────────────────────────

/// Runtime context passed to Cell::execute(). Provides access to
/// shared infrastructure without cells needing to manage their own.
pub struct CellContext {
    /// Pub/sub transport for ephemeral Pulses (type-erased).
    pub bus: Arc<dyn BusErased>,
    /// Durable storage for Engrams.
    pub store: Arc<dyn Substrate>,
    /// Cancellation token for cooperative shutdown.
    pub cancel: CancellationToken,
    /// Trace context for observability.
    pub trace_id: Option<String>,
    /// Run identifier (if executing within a Graph/Flow).
    pub run_id: Option<String>,
    /// Remaining budget for this execution (USD).
    pub budget_remaining: Option<f64>,
}

impl CellContext {
    /// Construct a new `CellContext` with the required infrastructure handles.
    #[must_use]
    pub fn new(
        bus: Arc<dyn BusErased>,
        store: Arc<dyn Substrate>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            bus,
            store,
            cancel,
            trace_id: None,
            run_id: None,
            budget_remaining: None,
        }
    }
}

// ─── TypeSchema ─────────────────────────────────────────────────────────────

/// Describes the input or output type contract of a Cell for edge validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeSchema {
    /// Accepts any input.
    Any,
    /// Accepts signals of a specific Kind.
    OfKind(Kind),
    /// Accepts signals matching a JSON schema string.
    JsonSchema(String),
}

impl TypeSchema {
    /// Check if an output of type `self` is compatible as input to a cell expecting `target`.
    #[must_use]
    pub fn is_compatible_with(&self, target: &TypeSchema) -> bool {
        match (self, target) {
            (_, TypeSchema::Any) => true,
            (TypeSchema::Any, _) => true,
            (TypeSchema::OfKind(a), TypeSchema::OfKind(b)) => a == b,
            _ => false,
        }
    }
}

// ─── Cell trait ─────────────────────────────────────────────────────────────

/// Universal computation unit. Every protocol trait (Substrate, Scorer, Gate,
/// Router, Composer, Policy) requires `Cell` as a supertrait, giving the
/// execution engine identity, cost estimation, and protocol introspection.
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
    /// Protocol names this cell conforms to (e.g. `["Substrate", "Gate"]`).
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

    // ─── v2 additions ───────────────────────────────────────────────────

    /// Describes the input type this cell expects. `None` means untyped.
    fn input_schema(&self) -> Option<&TypeSchema> {
        None
    }

    /// Describes the output type this cell produces. `None` means untyped.
    fn output_schema(&self) -> Option<&TypeSchema> {
        None
    }

    /// Execute this cell. Default returns an error — override in implementations.
    async fn execute(&self, input: Vec<Engram>, ctx: &CellContext) -> Result<Vec<Engram>> {
        let _ = (input, ctx);
        Err(RokoError::Invalid(format!(
            "{}: execute() not implemented",
            self.cell_name()
        )))
    }
}
