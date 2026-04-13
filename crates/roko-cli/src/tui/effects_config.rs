//! Per-tab visual effects configuration.

/// Controls which post-processing effects are enabled.
#[derive(Debug, Clone)]
pub struct EffectsConfig {
    /// Master switch for all screen-level post-processing.
    pub screen_postfx: bool,
    /// Enable bloom (glow bleed from bright cells). Off by default for performance.
    pub bloom_enabled: bool,
    /// Enable drop shadows behind panels.
    pub shadows_enabled: bool,
    /// Enable ambient VFX (orbs, atmosphere, color grading).
    pub vfx_enabled: bool,
    /// Bloom intensity multiplier (0.0..1.0).
    pub bloom_intensity: f64,
    /// Vignette intensity (0.0..1.0).
    pub vignette_intensity: f64,
}

impl Default for EffectsConfig {
    fn default() -> Self {
        Self {
            screen_postfx: false,
            bloom_enabled: false,
            shadows_enabled: false,
            vfx_enabled: false,
            bloom_intensity: 0.0,
            vignette_intensity: 0.0,
        }
    }
}

impl EffectsConfig {
    /// All effects disabled.
    #[must_use]
    pub fn none() -> Self {
        Self {
            screen_postfx: false,
            bloom_enabled: false,
            shadows_enabled: false,
            vfx_enabled: false,
            bloom_intensity: 0.0,
            vignette_intensity: 0.0,
        }
    }
}
