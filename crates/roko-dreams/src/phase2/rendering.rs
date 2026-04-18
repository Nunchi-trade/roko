//! Phase 2 dream-rendering stubs.

use serde::{Deserialize, Serialize};

use crate::phase2::cycle::DreamPhase;
use crate::phase2::shared::ColorPalette;

/// Dream rendering configuration for the TUI portal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamRenderConfig {
    /// Maximum frames per second for dream animations.
    pub target_fps: u16,
    /// Phosphene pattern rotation speed in degrees per second.
    pub phosphene_rotation_speed: f64,
    /// Fragment surface duration in milliseconds.
    pub fragment_surface_ms: u64,
    /// Connection flash duration in milliseconds.
    pub connection_flash_ms: u64,
    /// Whether to render braille phosphenes in hypnagogia.
    pub braille_phosphenes: bool,
    /// Dream phase transition duration in milliseconds.
    pub phase_transition_ms: u64,
    /// Opacity for dream portal mode.
    pub dream_opacity: f64,
}

impl Default for DreamRenderConfig {
    fn default() -> Self {
        Self {
            target_fps: 10,
            phosphene_rotation_speed: 15.0,
            fragment_surface_ms: 3_000,
            connection_flash_ms: 200,
            braille_phosphenes: true,
            phase_transition_ms: 1_500,
            dream_opacity: 0.85,
        }
    }
}

impl DreamRenderConfig {
    /// Construct the documented default render configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            target_fps: 10,
            phosphene_rotation_speed: 15.0,
            fragment_surface_ms: 3_000,
            connection_flash_ms: 200,
            braille_phosphenes: true,
            phase_transition_ms: 1_500,
            dream_opacity: 0.85,
        }
    }
}

/// Dream phase visual treatment specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseVisualSpec {
    /// Dream phase being rendered.
    pub phase: DreamPhase,
    /// Border treatment.
    pub border_style: BorderStyle,
    /// Content opacity.
    pub content_opacity: f64,
    /// Animation treatment.
    pub animation_type: AnimationType,
    /// Palette used for the phase.
    pub color_palette: ColorPalette,
}

impl PhaseVisualSpec {
    /// Construct a default visual specification for a phase.
    #[must_use]
    pub fn new(phase: DreamPhase) -> Self {
        Self {
            border_style: BorderStyle::default_for_phase(&phase),
            content_opacity: 1.0,
            animation_type: AnimationType::default_for_phase(&phase),
            color_palette: ColorPalette::default(),
            phase,
        }
    }
}

/// Border style used for dream rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderStyle {
    /// Stable double-line borders for NREM.
    StableDouble,
    /// Dashed borders for REM counterfactuals.
    Dashed,
    /// Oscillating borders for hypnagogia.
    Oscillating {
        /// Oscillation frequency applied to the border treatment.
        frequency_hz: f64,
    },
    /// No borders for integration.
    None,
}

impl BorderStyle {
    /// Select a reasonable border style for a phase.
    #[must_use]
    pub const fn default_for_phase(phase: &DreamPhase) -> Self {
        match phase {
            DreamPhase::Idle => Self::Oscillating { frequency_hz: 2.0 },
            DreamPhase::NremReplay { .. } => Self::StableDouble,
            DreamPhase::RemImagination { .. } => Self::Dashed,
            DreamPhase::Integration { .. } => Self::None,
        }
    }
}

impl Default for BorderStyle {
    fn default() -> Self {
        Self::StableDouble
    }
}

/// Animation style used for dream rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationType {
    /// Static with periodic updates.
    StaticPeriodic {
        /// Delay between periodic frame updates.
        update_interval_ms: u64,
    },
    /// Continuously drifting.
    Drift {
        /// Character drift speed for the rendered content.
        speed_chars_per_sec: f64,
    },
    /// Typing animation.
    TypeWriter {
        /// Character emission speed for the crystallization effect.
        chars_per_sec: f64,
    },
    /// Decision-tree expansion.
    TreeGrowth {
        /// Delay between branch expansions.
        branch_delay_ms: u64,
    },
}

impl AnimationType {
    /// Select a reasonable animation for a phase.
    #[must_use]
    pub const fn default_for_phase(phase: &DreamPhase) -> Self {
        match phase {
            DreamPhase::Idle => Self::Drift {
                speed_chars_per_sec: 2.5,
            },
            DreamPhase::NremReplay { .. } => Self::StaticPeriodic {
                update_interval_ms: 1_000,
            },
            DreamPhase::RemImagination { .. } => Self::TreeGrowth {
                branch_delay_ms: 120,
            },
            DreamPhase::Integration { .. } => Self::TypeWriter {
                chars_per_sec: 24.0,
            },
        }
    }
}

impl Default for AnimationType {
    fn default() -> Self {
        Self::StaticPeriodic {
            update_interval_ms: 1_000,
        }
    }
}
