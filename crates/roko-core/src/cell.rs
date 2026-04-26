//! The Cell trait — universal computation unit for all protocol implementations.

use std::time::Duration;

/// Stable identifier for a Cell instance.
pub type CellId = String;

/// Semantic version tuple for Cell implementations.
pub type CellVersion = (u32, u32, u32);

/// Universal computation unit. Every protocol trait (Store, Score, Verify,
/// Route, Compose, React) requires `Cell` as a supertrait, giving the
/// execution engine identity, cost estimation, and protocol introspection.
pub trait Cell: Send + Sync + 'static {
    /// Unique identifier for this cell instance.
    fn cell_id(&self) -> &str;
    /// Human-readable name for display and logging.
    fn cell_name(&self) -> &str;
    /// Semantic version of this cell's implementation.
    fn cell_version(&self) -> CellVersion {
        (0, 1, 0)
    }
    /// Protocol names this cell conforms to (e.g. `["Store", "Verify"]`).
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
}
