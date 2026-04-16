//! Per-tab visual effects configuration.

use std::path::Path;

/// Presets for the visual effects stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EffectsPreset {
    /// Disable the new visual effects stack.
    #[default]
    Off,
    /// Enable floating particles only.
    Minimal,
    /// Enable NervViz and floating particles.
    Full,
}

impl EffectsPreset {
    /// Return the next preset in the cycle order.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Minimal,
            Self::Minimal => Self::Full,
            Self::Full => Self::Off,
        }
    }

    /// Short user-facing label for notifications and logs.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Minimal => "Minimal",
            Self::Full => "Full",
        }
    }

    /// String value written to `roko.toml`.
    #[must_use]
    pub const fn as_toml_value(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Full => "full",
        }
    }

    /// Parse a preset from a TOML string.
    #[must_use]
    pub fn from_str(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "off" => Some(Self::Off),
            "minimal" => Some(Self::Minimal),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

/// Controls which post-processing effects are enabled.
#[derive(Debug, Clone)]
pub struct EffectsConfig {
    /// Master switch for all screen-level post-processing.
    pub screen_postfx: bool,
    /// Preset driving the new state-driven visual effects.
    pub preset: EffectsPreset,
    /// Enable NervViz guide-line/rain overlays.
    pub nerv_viz: bool,
    /// Enable floating particle dots.
    pub particles: bool,
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
        Self::from_preset(EffectsPreset::Off)
    }
}

impl EffectsConfig {
    /// All effects disabled.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Build a config from a preset.
    #[must_use]
    pub fn from_preset(preset: EffectsPreset) -> Self {
        let mut config = Self {
            screen_postfx: false,
            preset,
            nerv_viz: false,
            particles: false,
            bloom_enabled: false,
            shadows_enabled: false,
            vfx_enabled: false,
            bloom_intensity: 0.0,
            vignette_intensity: 0.0,
        };
        config.apply_preset(preset);
        config
    }

    /// Update the preset and derived effect flags in place.
    pub fn set_preset(&mut self, preset: EffectsPreset) {
        self.apply_preset(preset);
    }

    /// Cycle to the next preset and return the newly selected preset.
    pub fn cycle_preset(&mut self) -> EffectsPreset {
        let next = self.preset.next();
        self.apply_preset(next);
        next
    }

    /// Persist the current preset into `roko.toml`.
    pub fn save_preset(&self, root: &Path) -> Result<(), String> {
        save_preset_to_root(root, self.preset)
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

        if let Some(preset) = string_at_path(&value, &["tui", "effects", "preset"])
            .and_then(|preset| EffectsPreset::from_str(&preset))
        {
            config.apply_preset(preset);
        }
        config.screen_postfx =
            bool_at_path(&value, &["tui", "effects", "screen_postfx"]).unwrap_or(false);

        config
    }

    fn apply_preset(&mut self, preset: EffectsPreset) {
        self.preset = preset;
        self.nerv_viz = matches!(preset, EffectsPreset::Full);
        self.particles = matches!(preset, EffectsPreset::Minimal | EffectsPreset::Full);
    }
}

/// Save the selected effects preset into `roko.toml`.
pub fn save_preset_to_root(root: &Path, preset: EffectsPreset) -> Result<(), String> {
    let config_path = root.join("roko.toml");
    let content = match std::fs::read_to_string(&config_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("read roko.toml: {err}")),
    };

    let mut root_val = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        content
            .parse::<toml::Value>()
            .map_err(|e| format!("parse roko.toml: {e}"))?
    };

    set_toml_path(
        &mut root_val,
        "tui.effects.preset",
        toml::Value::String(preset.as_toml_value().to_string()),
    )?;

    let toml_str =
        toml::to_string_pretty(&root_val).map_err(|e| format!("serialize roko.toml: {e}"))?;
    std::fs::write(&config_path, toml_str).map_err(|e| format!("write roko.toml: {e}"))?;
    Ok(())
}

fn bool_at_path(value: &toml::Value, path: &[&str]) -> Option<bool> {
    let mut current = value;
    for segment in path {
        current = current.as_table()?.get(*segment)?;
    }
    current.as_bool()
}

fn string_at_path(value: &toml::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.as_table()?.get(*segment)?;
    }
    current.as_str().map(|s| s.to_string())
}

fn set_toml_path(root: &mut toml::Value, key: &str, val: toml::Value) -> Result<(), String> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return Err("empty TOML path".to_string());
    }

    let mut current = root;
    for part in &parts[..parts.len() - 1] {
        if !current.is_table() {
            *current = toml::Value::Table(toml::map::Map::new());
        }

        let table = current
            .as_table_mut()
            .ok_or_else(|| format!("config path {key}: not a table"))?;
        current = table
            .entry((*part).to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    }

    if !current.is_table() {
        *current = toml::Value::Table(toml::map::Map::new());
    }

    let table = current
        .as_table_mut()
        .ok_or_else(|| format!("config path {key}: not a table"))?;
    table.insert(parts[parts.len() - 1].to_string(), val);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn preset_cycles_and_derives_flags() {
        let mut config = EffectsConfig::default();
        assert_eq!(config.preset, EffectsPreset::Off);
        assert!(!config.screen_postfx);
        assert!(!config.nerv_viz);
        assert!(!config.particles);

        assert_eq!(config.cycle_preset(), EffectsPreset::Minimal);
        assert!(!config.screen_postfx);
        assert!(!config.nerv_viz);
        assert!(config.particles);

        assert_eq!(config.cycle_preset(), EffectsPreset::Full);
        assert!(!config.screen_postfx);
        assert!(config.nerv_viz);
        assert!(config.particles);
    }

    #[test]
    fn load_prefers_preset_and_save_writes_preset() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("roko.toml"),
            "[tui.effects]\npreset = \"minimal\"\nscreen_postfx = true\n",
        )
        .expect("write roko.toml");

        let config = EffectsConfig::load_from_root(dir.path());
        assert_eq!(config.preset, EffectsPreset::Minimal);
        assert!(config.screen_postfx);
        assert!(!config.nerv_viz);
        assert!(config.particles);

        config
            .save_preset(dir.path())
            .expect("save preset to roko.toml");
        let saved = std::fs::read_to_string(dir.path().join("roko.toml")).expect("read back");
        assert!(saved.contains("preset = \"minimal\""));
    }
}
