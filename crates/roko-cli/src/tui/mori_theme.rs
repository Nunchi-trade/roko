//! ROSEDUST color palette — ported from Mori's theme.rs.
//!
//! The canonical Mori-accurate palette for roko's TUI. All new widgets
//! use `MoriTheme::` constants. The old `dashboard::Theme` and
//! `RosedustTheme` remain for backward compatibility with existing views.

use ratatui::style::{Color, Modifier, Style};

/// Mori-accurate ROSEDUST color palette.
///
/// Uses associated constants (zero-cost) rather than instance fields.
pub struct MoriTheme;

impl MoriTheme {
    // ── Core palette ─────────────────────────────────────────────────────
    pub const VOID: Color = Color::Rgb(0, 0, 0);
    pub const ROSE: Color = Color::Rgb(185, 120, 148);
    pub const ROSE_BRIGHT: Color = Color::Rgb(220, 155, 180);
    pub const ROSE_DIM: Color = Color::Rgb(140, 96, 112);
    pub const BONE: Color = Color::Rgb(215, 198, 158);
    pub const BONE_DIM: Color = Color::Rgb(160, 142, 108);
    pub const TEXT: Color = Color::Rgb(165, 142, 158);
    pub const TEXT_DIM: Color = Color::Rgb(145, 120, 138);
    pub const TEXT_GHOST: Color = Color::Rgb(110, 85, 105);
    pub const DREAM: Color = Color::Rgb(120, 115, 165);
    pub const SAGE: Color = Color::Rgb(125, 158, 140);
    pub const EMBER: Color = Color::Rgb(195, 110, 85);
    pub const WARNING: Color = Color::Rgb(195, 155, 95);

    // ── Extended palette ─────────────────────────────────────────────────
    pub const BG_RAISED: Color = Color::Rgb(14, 12, 18);
    pub const ROSE_DEEP: Color = Color::Rgb(65, 36, 52);
    pub const ROSE_EMBER: Color = Color::Rgb(80, 45, 62);
    pub const TEXT_PHANTOM: Color = Color::Rgb(55, 42, 55);

    // ── Derived aliases ──────────────────────────────────────────────────
    pub const BG: Color = Self::VOID;
    pub const BG_SECONDARY: Color = Color::Rgb(14, 12, 16);
    pub const BG_HIGHLIGHT: Color = Color::Rgb(34, 28, 36);
    pub const FG: Color = Self::TEXT;
    pub const FG_DIM: Color = Self::TEXT_DIM;
    pub const FG_BRIGHT: Color = Self::BONE;

    // ── Status aliases ───────────────────────────────────────────────────
    pub const STATUS_OK: Color = Self::SAGE;
    pub const STATUS_ACTIVE: Color = Self::ROSE;
    pub const STATUS_WARN: Color = Self::WARNING;
    pub const STATUS_ERROR: Color = Self::EMBER;

    // ── Semantic styles ──────────────────────────────────────────────────

    pub fn default_style() -> Style {
        Style::default().fg(Self::FG).bg(Self::BG)
    }

    pub fn block_style() -> Style {
        Style::default().fg(Self::FG_DIM).bg(Self::BG)
    }

    pub fn selected_style() -> Style {
        Style::default().fg(Self::FG_BRIGHT).bg(Self::BG_HIGHLIGHT)
    }

    pub fn active_style() -> Style {
        Style::default()
            .fg(Self::ROSE)
            .add_modifier(Modifier::BOLD)
    }

    pub fn title_style() -> Style {
        Style::default()
            .fg(Self::BONE_DIM)
            .add_modifier(Modifier::BOLD)
    }

    pub fn tab_active_style() -> Style {
        Style::default().fg(Self::BG).bg(Self::ROSE)
    }

    pub fn tab_inactive_style() -> Style {
        Style::default().fg(Self::FG_DIM).bg(Self::BG_SECONDARY)
    }

    pub fn error_style() -> Style {
        Style::default().fg(Self::STATUS_ERROR)
    }

    pub fn success_style() -> Style {
        Style::default().fg(Self::STATUS_OK)
    }

    pub fn warning_style() -> Style {
        Style::default().fg(Self::STATUS_WARN)
    }

    pub fn dim_style() -> Style {
        Style::default().fg(Self::FG_DIM)
    }

    // ── Panel border styles ──────────────────────────────────────────────

    pub fn focused_border_style() -> Style {
        Style::default()
            .fg(Self::ROSE_BRIGHT)
            .add_modifier(Modifier::BOLD)
    }

    pub fn focused_title_style() -> Style {
        Style::default()
            .fg(Self::BONE)
            .add_modifier(Modifier::BOLD)
    }

    pub fn unfocused_border_style() -> Style {
        Style::default().fg(Self::TEXT_PHANTOM)
    }

    pub fn unfocused_title_style() -> Style {
        Style::default().fg(Self::TEXT_GHOST)
    }

    // ── Per-role accent ──────────────────────────────────────────────────

    pub fn role_accent(role: &str) -> Color {
        match role {
            "conductor" => Self::EMBER,
            "strategist" | "pre-planner" => Self::BONE_DIM,
            "implementer" => Self::ROSE,
            "architect" => Self::SAGE,
            "auditor" => Self::WARNING,
            "scribe" => Self::DREAM,
            "critic" | "reviewer" | "quick-reviewer" => Self::SAGE,
            "refactorer" => Self::SAGE,
            "researcher" => Self::DREAM,
            "tester" | "integration-tester" => Self::WARNING,
            "merge-resolver" => Self::EMBER,
            _ => Self::FG_DIM,
        }
    }

    // ── Per-phase accent ─────────────────────────────────────────────────

    pub fn phase_accent(phase: &str) -> Color {
        match phase {
            "preflight" => Self::TEXT_DIM,
            "strategist" => Self::BONE_DIM,
            "implementer" => Self::ROSE,
            "gating" | "compile-gate" | "test-gate" => Self::WARNING,
            "verifying" | "verify-chain" => Self::DREAM,
            "reviewing" => Self::SAGE,
            "critic-review" | "doc-revision" => Self::DREAM,
            "verdict" | "committing" | "merging" => Self::BONE,
            "done" | "complete" => Self::SAGE,
            "failed" => Self::EMBER,
            _ => Self::FG_DIM,
        }
    }

    /// Semantic color by percentage: 0% = EMBER, 50% = WARNING, 100% = SAGE.
    pub fn semantic_color(pct: f64) -> Color {
        let pct = pct.clamp(0.0, 1.0);
        if pct < 0.5 {
            lerp_color(Self::EMBER, Self::WARNING, pct * 2.0)
        } else {
            lerp_color(Self::WARNING, Self::SAGE, (pct - 0.5) * 2.0)
        }
    }
}

// ── Gradient utilities ───────────────────────────────────────────────────

/// Brighten an RGB color by a factor (1.0 = no change, 1.5 = 50% brighter).
pub fn brighten(color: Color, factor: f64) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f64 * factor).min(255.0) as u8,
            (g as f64 * factor).min(255.0) as u8,
            (b as f64 * factor).min(255.0) as u8,
        ),
        other => other,
    }
}

/// Linearly interpolate between two RGB colors.
pub fn lerp_color(from: Color, to: Color, t: f64) -> Color {
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let t = t.clamp(0.0, 1.0);
            Color::Rgb(
                (r1 as f64 + (r2 as f64 - r1 as f64) * t) as u8,
                (g1 as f64 + (g2 as f64 - g1 as f64) * t) as u8,
                (b1 as f64 + (b2 as f64 - b1 as f64) * t) as u8,
            )
        }
        _ => to,
    }
}

/// Gradient with multiple color stops for smooth interpolation.
pub struct Gradient {
    stops: Vec<(f64, Color)>,
}

impl Gradient {
    pub fn from_stops(stops: Vec<(f64, Color)>) -> Self {
        Self { stops }
    }

    /// Sample the gradient at position t (0.0..=1.0).
    pub fn sample(&self, t: f64) -> Color {
        let t = t.clamp(0.0, 1.0);
        if self.stops.is_empty() {
            return Color::White;
        }
        if self.stops.len() == 1 {
            return self.stops[0].1;
        }

        let mut lower = &self.stops[0];
        let mut upper = &self.stops[self.stops.len() - 1];
        for i in 0..self.stops.len() - 1 {
            if t >= self.stops[i].0 && t <= self.stops[i + 1].0 {
                lower = &self.stops[i];
                upper = &self.stops[i + 1];
                break;
            }
        }

        let range = upper.0 - lower.0;
        if range <= 0.0 {
            return lower.1;
        }
        let local_t = (t - lower.0) / range;
        lerp_color(lower.1, upper.1, local_t)
    }
}

/// Fire gradient: dark red -> amber -> gold. For progress bars.
pub fn gradient_fire() -> Gradient {
    Gradient::from_stops(vec![
        (0.0, Color::Rgb(100, 30, 30)),
        (0.5, Color::Rgb(200, 100, 30)),
        (1.0, Color::Rgb(220, 180, 60)),
    ])
}

/// Context pressure gradient: sage -> warning -> ember. For context gauges.
pub fn gradient_context() -> Gradient {
    Gradient::from_stops(vec![
        (0.0, MoriTheme::SAGE),
        (0.5, MoriTheme::WARNING),
        (1.0, MoriTheme::EMBER),
    ])
}

/// Ocean gradient: deep blue -> teal -> cyan. For wave bars.
pub fn gradient_ocean() -> Gradient {
    Gradient::from_stops(vec![
        (0.0, Color::Rgb(40, 60, 90)),
        (0.5, Color::Rgb(60, 120, 160)),
        (1.0, Color::Rgb(80, 180, 200)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_color_endpoints() {
        let c0 = MoriTheme::semantic_color(0.0);
        assert_eq!(c0, MoriTheme::EMBER);
        let c1 = MoriTheme::semantic_color(1.0);
        assert_eq!(c1, MoriTheme::SAGE);
    }

    #[test]
    fn semantic_color_midpoint() {
        let c = MoriTheme::semantic_color(0.5);
        assert_eq!(c, MoriTheme::WARNING);
    }

    #[test]
    fn gradient_fire_samples() {
        let g = gradient_fire();
        let c0 = g.sample(0.0);
        assert_eq!(c0, Color::Rgb(100, 30, 30));
        let c1 = g.sample(1.0);
        assert_eq!(c1, Color::Rgb(220, 180, 60));
    }

    #[test]
    fn brighten_identity() {
        let c = Color::Rgb(100, 100, 100);
        assert_eq!(brighten(c, 1.0), c);
    }
}
