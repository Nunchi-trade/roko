//! Per-tab visual effects configuration.

use std::path::Path;

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

    /// Load TUI effects from `roko.toml`, falling back to defaults on error.
    #[must_use]
    pub fn load_from_root(root: &Path) -> Self {
        let mut config = Self::default();
        let Ok(content) = std::fs::read_to_string(root.join("roko.toml")) else {
            return config;
        };
        let Ok(value) = content.parse::<toml::Value>() else {
            return config;
        };

        config.screen_postfx =
            bool_at_path(&value, &["tui", "effects", "screen_postfx"]).unwrap_or(false);
        config
    }
}

fn bool_at_path(value: &toml::Value, path: &[&str]) -> Option<bool> {
    let mut current = value;
    for segment in path {
        current = current.as_table()?.get(*segment)?;
    }
    current.as_bool()
}
