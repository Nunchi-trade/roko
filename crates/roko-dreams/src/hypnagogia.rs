//! Hypnagogia subsystem placeholder.
//!
//! This keeps the public surface for liminal handoff behavior local to the
//! dreams crate while the real implementation is still pending.

use crate::{DreamsSubsystemId, DreamsSubsystemSummary};

/// Placeholder hypnagogia engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HypnagogiaEngine;

impl HypnagogiaEngine {
    /// Stable subsystem id.
    pub const ID: DreamsSubsystemId = DreamsSubsystemId::Hypnagogia;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Hypnagogia";
    /// Static placeholder marker string.
    pub const MARKER: &'static str = "roko-dreams subsystem: hypnagogia";

    /// Construct a placeholder hypnagogia engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Summary metadata for this subsystem placeholder.
    #[must_use]
    pub const fn summary(self) -> DreamsSubsystemSummary {
        DreamsSubsystemSummary::new(Self::ID, Self::LABEL, Self::MARKER)
    }

    /// Returns a static marker describing placeholder behavior.
    #[must_use]
    pub const fn interrupt(self) -> &'static str {
        Self::MARKER
    }
}
