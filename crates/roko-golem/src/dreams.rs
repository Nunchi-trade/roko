//! Dreams subsystem scaffold.
//!
//! Placeholder API only; replay/imagination behavior is not yet implemented.

/// Placeholder dreams engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DreamsEngine;

impl DreamsEngine {
    /// Construct a placeholder dreams engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns a static marker describing scaffold behavior.
    #[must_use]
    pub const fn replay(self) -> &'static str {
        "roko-golem scaffold: dreams"
    }
}
