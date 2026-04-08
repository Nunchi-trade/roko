//! Mortality subsystem scaffold.
//!
//! Placeholder API only; real lifecycle logic is intentionally not
//! implemented in this phase.

/// Placeholder mortality engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MortalityEngine;

impl MortalityEngine {
    /// Construct a placeholder mortality engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns a static marker describing scaffold behavior.
    #[must_use]
    pub const fn pulse(self) -> &'static str {
        "roko-golem scaffold: mortality"
    }
}
