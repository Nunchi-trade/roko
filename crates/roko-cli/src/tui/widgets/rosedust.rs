//! ROSEDUST color constants and helpers — the Mori-style palette for roko's TUI.
//!
//! Centralises all the named colors that were previously on `MoriTheme` so
//! that every widget in this module can `use super::rosedust::MoriTheme;`
//! without duplicating RGB values.

use ratatui::style::{Color, Modifier, Style};

// ---------------------------------------------------------------------------
// MoriTheme — all named color constants + style helpers
// ---------------------------------------------------------------------------

/// The Mori/rosedust palette as associated constants.
pub(crate) struct MoriTheme;

impl MoriTheme {
    // -- Primaries --
    pub(crate) const VOID: Color = Color::Rgb(0, 0, 0);
    pub(crate) const ROSE: Color = Color::Rgb(185, 120, 148);
    pub(crate) const ROSE_BRIGHT: Color = Color::Rgb(220, 155, 180);
    pub(crate) const ROSE_DIM: Color = Color::Rgb(140, 96, 112);
    pub(crate) const BONE: Color = Color::Rgb(215, 198, 158);
    pub(crate) const BONE_DIM: Color = Color::Rgb(160, 142, 108);

    // -- Text --
    pub(crate) const TEXT: Color = Color::Rgb(165, 142, 158);
    pub(crate) const TEXT_DIM: Color = Color::Rgb(145, 120, 138);
    pub(crate) const TEXT_GHOST: Color = Color::Rgb(110, 85, 105);
    pub(crate) const TEXT_PHANTOM: Color = Color::Rgb(55, 42, 55);

    // -- Accents --
    pub(crate) const DREAM: Color = Color::Rgb(120, 115, 165);
    pub(crate) const SAGE: Color = Color::Rgb(125, 158, 140);
    pub(crate) const EMBER: Color = Color::Rgb(195, 110, 85);
    pub(crate) const WARNING: Color = Color::Rgb(195, 155, 95);

    // -- Backgrounds --
    pub(crate) const BG: Color = Color::Rgb(0, 0, 0);
    pub(crate) const BG_SECONDARY: Color = Color::Rgb(14, 12, 16);
    pub(crate) const BG_HIGHLIGHT: Color = Color::Rgb(34, 28, 36);

    // -- Foreground aliases --
    pub(crate) const FG: Color = Color::Rgb(165, 142, 158);
    pub(crate) const FG_DIM: Color = Color::Rgb(145, 120, 138);

    // -- Semantic status --
    pub(crate) const STATUS_OK: Color = Color::Rgb(125, 158, 140); // = SAGE
    pub(crate) const STATUS_ERROR: Color = Color::Rgb(195, 110, 85); // = EMBER

    // -- Block styling helpers -----------------------------------------------

    /// Default block background style — uses terminal default background
    /// for consistency across all panels.
    pub(crate) fn block_style() -> Style {
        Style::default()
    }

    /// Focused-panel border style.
    pub(crate) fn focused_border_style() -> Style {
        Style::default().fg(Self::ROSE)
    }

    /// Unfocused-panel border style — uses TEXT_DIM for visible borders.
    pub(crate) fn unfocused_border_style() -> Style {
        Style::default().fg(Self::TEXT_DIM)
    }

    /// Focused-panel title style.
    pub(crate) fn focused_title_style() -> Style {
        Style::default()
            .fg(Self::ROSE_BRIGHT)
            .add_modifier(Modifier::BOLD)
    }

    /// Unfocused-panel title style.
    pub(crate) fn unfocused_title_style() -> Style {
        Style::default().fg(Self::FG_DIM)
    }

    /// Default title style (dim, not bold).
    pub(crate) fn title_style() -> Style {
        Style::default().fg(Self::FG_DIM)
    }

    /// Error style (EMBER, bold).
    pub(crate) fn error_style() -> Style {
        Style::default()
            .fg(Self::EMBER)
            .add_modifier(Modifier::BOLD)
    }

    /// Success style (SAGE, bold).
    pub(crate) fn success_style() -> Style {
        Style::default().fg(Self::SAGE).add_modifier(Modifier::BOLD)
    }

    // -- Semantic helpers ----------------------------------------------------

    /// Per-role accent color.
    pub(crate) fn role_accent(role: &str) -> Color {
        match role {
            r if r.contains("implement") => Self::ROSE,
            r if r.contains("strateg") => Self::DREAM,
            r if r.contains("architect") => Self::BONE,
            r if r.contains("audit") => Self::SAGE,
            r if r.contains("critic") => Self::EMBER,
            r if r.contains("conduct") => Self::WARNING,
            r if r.contains("research") => Self::DREAM,
            _ => Self::TEXT_DIM,
        }
    }

    /// Phase-based accent color.
    pub(crate) fn phase_accent(phase: &str) -> Color {
        match phase {
            p if p.contains("preflight") => Self::TEXT_GHOST,
            p if p.contains("implement") => Self::ROSE,
            p if p.contains("strateg") => Self::DREAM,
            p if p.contains("compil") || p.contains("test") => Self::WARNING,
            p if p.contains("review") || p.contains("critic") => Self::BONE_DIM,
            p if p.contains("gate") || p.contains("verify") => Self::SAGE,
            p if p.contains("fail") => Self::EMBER,
            p if p.contains("done") || p.contains("complete") => Self::SAGE,
            _ => Self::TEXT_DIM,
        }
    }

    /// Semantic color on a 0..1 progress scale: red -> amber -> green.
    pub(crate) fn semantic_color(t: f64) -> Color {
        if t >= 0.8 {
            Self::SAGE
        } else if t >= 0.4 {
            Self::WARNING
        } else {
            Self::EMBER
        }
    }
}

// ---------------------------------------------------------------------------
// Gradient
// ---------------------------------------------------------------------------

/// A three-stop linear gradient.
#[derive(Clone, Debug)]
pub(crate) struct Gradient {
    start: (f64, f64, f64),
    mid: (f64, f64, f64),
    end: (f64, f64, f64),
}

impl Gradient {
    /// Sample the gradient at `t` in `0.0..=1.0`.
    pub(crate) fn sample(&self, t: f64) -> Color {
        let t = t.clamp(0.0, 1.0);
        let (r, g, b) = if t < 0.5 {
            let lt = t * 2.0;
            (
                self.start.0 + (self.mid.0 - self.start.0) * lt,
                self.start.1 + (self.mid.1 - self.start.1) * lt,
                self.start.2 + (self.mid.2 - self.start.2) * lt,
            )
        } else {
            let lt = (t - 0.5) * 2.0;
            (
                self.mid.0 + (self.end.0 - self.mid.0) * lt,
                self.mid.1 + (self.end.1 - self.mid.1) * lt,
                self.mid.2 + (self.end.2 - self.mid.2) * lt,
            )
        };
        Color::Rgb(
            r.clamp(0.0, 255.0) as u8,
            g.clamp(0.0, 255.0) as u8,
            b.clamp(0.0, 255.0) as u8,
        )
    }
}

/// Fire gradient: dark red -> amber -> gold.
pub(crate) fn gradient_fire() -> Gradient {
    Gradient {
        start: (120.0, 30.0, 20.0),
        mid: (195.0, 110.0, 45.0),
        end: (215.0, 198.0, 80.0),
    }
}

/// Ocean gradient: deep blue -> teal -> cyan.
pub(crate) fn gradient_ocean() -> Gradient {
    Gradient {
        start: (30.0, 40.0, 120.0),
        mid: (40.0, 120.0, 150.0),
        end: (80.0, 190.0, 210.0),
    }
}

/// Brighten (or dim) an RGB color by a multiplier.
pub(crate) fn brighten(color: Color, factor: f64) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(
            ((r as f64) * factor).clamp(0.0, 255.0) as u8,
            ((g as f64) * factor).clamp(0.0, 255.0) as u8,
            ((b as f64) * factor).clamp(0.0, 255.0) as u8,
        ),
        other => other,
    }
}
