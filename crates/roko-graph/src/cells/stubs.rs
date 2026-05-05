//! Stub cell implementations for graph nodes that don't have real implementations yet.
//!
//! These are registered in `default_registry()` so that graphs referencing
//! cell types like `signal-reader`, `relevance-scorer`, etc. can load and
//! validate without error. Each stub passes input signals through unchanged
//! and logs a trace message.

use std::time::Duration;

use roko_core::{Engram, error::Result};

use crate::cell::{Cell, CellContext, CellVersion};

/// Stub cell that passes input engrams through unchanged.
///
/// Used as a placeholder until the real implementation is built.
/// Each instance carries a name so logs indicate which stub was invoked.
pub struct PassthroughCell {
    /// Cell type name this stub represents.
    pub name: String,
}

impl PassthroughCell {
    /// Create a new passthrough stub with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait::async_trait]
impl Cell for PassthroughCell {
    fn cell_id(&self) -> &str {
        &self.name
    }

    fn cell_name(&self) -> &str {
        &self.name
    }

    fn cell_version(&self) -> CellVersion {
        (0, 1, 0)
    }

    fn protocols(&self) -> &[&str] {
        &[]
    }

    fn estimated_cost(&self) -> Option<f64> {
        None
    }

    fn estimated_duration(&self) -> Option<Duration> {
        Some(Duration::from_millis(1))
    }

    async fn execute(&self, input: Vec<Engram>, _ctx: &CellContext) -> Result<Vec<Engram>> {
        tracing::info!(
            cell = %self.name,
            input_count = input.len(),
            "PassthroughCell '{}' -- {} input engrams (stub)",
            self.name,
            input.len()
        );
        Ok(input)
    }
}

/// Names of stub cells registered in the default registry for the cognitive loop.
pub const COGNITIVE_LOOP_STUBS: &[&str] = &[
    "signal-reader",
    "relevance-scorer",
    "system-prompt-builder",
    "claude-agent",
    "gate-pipeline",
    "store-writer",
    "event-publisher",
];
