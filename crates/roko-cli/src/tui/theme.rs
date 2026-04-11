//! ROSEDUST color palette for the dashboard TUI.
//!
//! A warm, muted palette inspired by dusty rose and dark terminals.
//! This is the new theme system that replaces the basic `Theme` in
//! `dashboard.rs` — the old one stays for backward compatibility.

use ratatui::style::{Color, Modifier, Style};

use super::color::gradient;

/// ROSEDUST color palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RosedustTheme {
    // Base colors
    pub bg: Color,
    pub bg_alt: Color,
    pub fg: Color,
    pub fg_muted: Color,

    // Accent colors
    pub rose: Color,
    pub rose_muted: Color,
    pub gold: Color,
    pub teal: Color,
    pub blue: Color,
    pub lavender: Color,
    pub coral: Color,

    // Semantic colors
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub info: Color,

    // UI elements
    pub border: Color,
    pub border_active: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub header_bg: Color,
    pub status_bg: Color,
}

impl RosedustTheme {
    /// The default ROSEDUST palette.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bg: Color::Rgb(26, 21, 32),
            bg_alt: Color::Rgb(34, 29, 42),
            fg: Color::Rgb(232, 223, 213),
            fg_muted: Color::Rgb(138, 127, 142),

            rose: Color::Rgb(212, 119, 140),
            rose_muted: Color::Rgb(160, 92, 110),
            gold: Color::Rgb(212, 168, 87),
            teal: Color::Rgb(93, 184, 163),
            blue: Color::Rgb(107, 143, 189),
            lavender: Color::Rgb(160, 140, 196),
            coral: Color::Rgb(196, 122, 92),

            success: Color::Rgb(93, 184, 163),   // teal
            warning: Color::Rgb(212, 168, 87),    // gold
            danger: Color::Rgb(196, 92, 80),      // warm red
            info: Color::Rgb(107, 143, 189),       // blue

            border: Color::Rgb(58, 51, 69),
            border_active: Color::Rgb(212, 119, 140), // rose
            selection_bg: Color::Rgb(45, 40, 56),
            selection_fg: Color::Rgb(232, 223, 213),   // fg
            header_bg: Color::Rgb(30, 25, 40),
            status_bg: Color::Rgb(34, 29, 42),        // bg_alt
        }
    }

    /// An uncolored palette for `NO_COLOR` environments.
    #[must_use]
    pub const fn no_color() -> Self {
        Self {
            bg: Color::Reset,
            bg_alt: Color::Reset,
            fg: Color::Reset,
            fg_muted: Color::Reset,

            rose: Color::Reset,
            rose_muted: Color::Reset,
            gold: Color::Reset,
            teal: Color::Reset,
            blue: Color::Reset,
            lavender: Color::Reset,
            coral: Color::Reset,

            success: Color::Reset,
            warning: Color::Reset,
            danger: Color::Reset,
            info: Color::Reset,

            border: Color::Reset,
            border_active: Color::Reset,
            selection_bg: Color::Reset,
            selection_fg: Color::Reset,
            header_bg: Color::Reset,
            status_bg: Color::Reset,
        }
    }

    // -- Semantic style helpers --

    /// Normal text.
    #[must_use]
    pub const fn text(&self) -> Style {
        Style::new().fg(self.fg)
    }

    /// Muted / secondary text.
    #[must_use]
    pub const fn muted(&self) -> Style {
        Style::new().fg(self.fg_muted)
    }

    /// Primary accent (rose).
    #[must_use]
    pub const fn accent(&self) -> Style {
        Style::new().fg(self.rose)
    }

    /// Bold accent.
    #[must_use]
    pub fn accent_bold(&self) -> Style {
        self.accent().add_modifier(Modifier::BOLD)
    }

    /// Success state.
    #[must_use]
    pub fn success_style(&self) -> Style {
        Style::default().fg(self.success).add_modifier(Modifier::BOLD)
    }

    /// Warning state.
    #[must_use]
    pub fn warning_style(&self) -> Style {
        Style::default().fg(self.warning).add_modifier(Modifier::BOLD)
    }

    /// Danger / error state.
    #[must_use]
    pub fn danger_style(&self) -> Style {
        Style::default().fg(self.danger).add_modifier(Modifier::BOLD)
    }

    /// Informational state.
    #[must_use]
    pub fn info_style(&self) -> Style {
        Style::default().fg(self.info).add_modifier(Modifier::BOLD)
    }

    /// Selection highlight.
    #[must_use]
    pub const fn selection(&self) -> Style {
        Style::new().fg(self.selection_fg).bg(self.selection_bg)
    }

    /// Normal border.
    #[must_use]
    pub const fn border_style(&self) -> Style {
        Style::new().fg(self.border)
    }

    /// Active / focused border.
    #[must_use]
    pub const fn border_active_style(&self) -> Style {
        Style::new().fg(self.border_active)
    }

    /// Header bar background.
    #[must_use]
    pub fn header(&self) -> Style {
        Style::default()
            .fg(self.fg)
            .bg(self.header_bg)
            .add_modifier(Modifier::BOLD)
    }

    /// Status bar background.
    #[must_use]
    pub const fn status(&self) -> Style {
        Style::new().fg(self.fg_muted).bg(self.status_bg)
    }

    /// Map a plan phase name to an accent color.
    #[must_use]
    pub fn phase_accent(&self, phase: &str) -> Style {
        let color = match phase {
            "plan" | "planning" => self.lavender,
            "build" | "building" | "implement" | "implementation" => self.blue,
            "test" | "testing" | "validate" | "validation" => self.gold,
            "gate" | "gating" | "review" => self.coral,
            "deploy" | "deploying" | "done" | "complete" => self.teal,
            _ => self.fg_muted,
        };
        Style::default().fg(color)
    }

    /// Map an agent role name to an accent color.
    #[must_use]
    pub fn role_accent(&self, role: &str) -> Style {
        let color = match role {
            "architect" | "planner" => self.lavender,
            "implementer" | "coder" | "developer" => self.blue,
            "reviewer" | "critic" => self.coral,
            "researcher" => self.gold,
            "tester" => self.teal,
            "orchestrator" | "conductor" => self.rose,
            _ => self.fg_muted,
        };
        Style::default().fg(color)
    }

    /// Progress bar style that interpolates danger -> warning -> success.
    ///
    /// * `ratio` in `[0.0, 1.0]` where 0 = danger, 0.5 = warning, 1.0 = success.
    #[must_use]
    pub fn progress_style(&self, ratio: f64) -> Style {
        let ratio = ratio.clamp(0.0, 1.0);
        let color = if ratio < 0.5 {
            gradient(self.danger, self.warning, ratio * 2.0)
        } else {
            gradient(self.warning, self.success, (ratio - 0.5) * 2.0)
        };
        Style::default().fg(color)
    }
}

impl Default for RosedustTheme {
    fn default() -> Self {
        Self::new()
    }
}

/// Return the active theme, respecting `NO_COLOR`.
#[must_use]
pub fn active_theme() -> RosedustTheme {
    if std::env::var_os("NO_COLOR").is_some() {
        RosedustTheme::no_color()
    } else {
        RosedustTheme::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_equals_new() {
        assert_eq!(RosedustTheme::default(), RosedustTheme::new());
    }

    #[test]
    fn no_color_all_reset() {
        let t = RosedustTheme::no_color();
        assert_eq!(t.bg, Color::Reset);
        assert_eq!(t.fg, Color::Reset);
        assert_eq!(t.rose, Color::Reset);
        assert_eq!(t.success, Color::Reset);
        assert_eq!(t.border, Color::Reset);
    }

    #[test]
    fn phase_accent_known() {
        let t = RosedustTheme::new();
        // "plan" maps to lavender
        let s = t.phase_accent("plan");
        assert_eq!(s.fg, Some(t.lavender));
    }

    #[test]
    fn phase_accent_unknown() {
        let t = RosedustTheme::new();
        let s = t.phase_accent("unknown-phase");
        assert_eq!(s.fg, Some(t.fg_muted));
    }

    #[test]
    fn role_accent_known() {
        let t = RosedustTheme::new();
        let s = t.role_accent("tester");
        assert_eq!(s.fg, Some(t.teal));
    }

    #[test]
    fn progress_endpoints() {
        let t = RosedustTheme::new();
        let s0 = t.progress_style(0.0);
        assert_eq!(s0.fg, Some(t.danger));
        let s1 = t.progress_style(1.0);
        assert_eq!(s1.fg, Some(t.success));
    }

    #[test]
    fn progress_midpoint_is_warning() {
        let t = RosedustTheme::new();
        let s = t.progress_style(0.5);
        assert_eq!(s.fg, Some(t.warning));
    }
}
