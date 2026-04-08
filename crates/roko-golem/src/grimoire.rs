//! Grimoire subsystem scaffold.
//!
//! Placeholder API only; memory lineage/evolution behavior is not yet
//! implemented.

/// Placeholder grimoire engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GrimoireEngine;

impl GrimoireEngine {
    /// Construct a placeholder grimoire engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns a static marker describing scaffold behavior.
    #[must_use]
    pub const fn evolve(self) -> &'static str {
        "roko-golem scaffold: grimoire"
    }
}
