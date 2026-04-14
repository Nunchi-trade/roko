//! Chain witness subsystem placeholder.
//!
//! This keeps the witness placeholder in the chain crate until the real
//! chain-facing witness loop is implemented.

/// Placeholder chain witness engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChainWitnessEngine;

impl ChainWitnessEngine {
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Chain Witness";
    /// Static placeholder marker string.
    pub const MARKER: &'static str = "roko-chain subsystem: witness";

    /// Construct a placeholder chain witness engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns a static marker describing placeholder behavior.
    #[must_use]
    pub const fn observe(self) -> &'static str {
        Self::MARKER
    }
}
