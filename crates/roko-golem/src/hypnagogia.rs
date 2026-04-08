//! Hypnagogia subsystem scaffold.
//!
//! Placeholder API only; liminal-interrupt behavior is not yet implemented.

/// Placeholder hypnagogia engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HypnagogiaEngine;

impl HypnagogiaEngine {
    /// Construct a placeholder hypnagogia engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns a static marker describing scaffold behavior.
    #[must_use]
    pub const fn interrupt(self) -> &'static str {
        "roko-golem scaffold: hypnagogia"
    }
}
