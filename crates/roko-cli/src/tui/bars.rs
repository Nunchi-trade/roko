//! Custom bar widgets for the TUI.
//!
//! Provides gradient and semantic progress bars that use the ROSEDUST palette.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use super::color::gradient;
use super::theme::RosedustTheme;

/// A horizontal bar that renders a filled portion with a color gradient.
///
/// The fill interpolates from `from` to `to` across the filled width.
/// The unfilled portion is rendered with `bg_style`.
pub struct GradientBar {
    ratio: f64,
    from: Color,
    to: Color,
    bg_style: Style,
}

impl GradientBar {
    /// Create a new gradient bar.
    ///
    /// * `ratio` — fill fraction `[0.0, 1.0]`.
    /// * `from` — color at the left edge.
    /// * `to` — color at the right (filled) edge.
    #[must_use]
    pub fn new(ratio: f64, from: Color, to: Color) -> Self {
        Self {
            ratio: ratio.clamp(0.0, 1.0),
            from,
            to,
            bg_style: Style::default(),
        }
    }

    /// Set the background style for the unfilled portion.
    #[must_use]
    pub fn bg_style(mut self, style: Style) -> Self {
        self.bg_style = style;
        self
    }
}

impl Widget for GradientBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let filled_width = (self.ratio * area.width as f64).round() as u16;

        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let col = x - area.left();
                if col < filled_width {
                    // Interpolate color across the filled portion.
                    let t = if filled_width <= 1 {
                        0.0
                    } else {
                        col as f64 / (filled_width - 1) as f64
                    };
                    let color = gradient(self.from, self.to, t);
                    let cell = &mut buf[(x, y)];
                    cell.set_char(BLOCK_FULL);
                    cell.set_fg(color);
                } else {
                    let cell = &mut buf[(x, y)];
                    cell.set_char(BLOCK_LIGHT);
                    cell.set_style(self.bg_style);
                }
            }
        }
    }
}

/// A horizontal bar whose color reflects the ratio semantically:
///
/// * Low ratio → danger (warm red)
/// * Mid ratio → warning (gold)
/// * High ratio → success (teal)
///
/// Interpolation matches [`RosedustTheme::progress_style`].
pub struct SemanticBar {
    ratio: f64,
    theme: RosedustTheme,
    bg_style: Style,
}

impl SemanticBar {
    /// Create a new semantic bar.
    ///
    /// * `ratio` — fill fraction `[0.0, 1.0]`.
    /// * `theme` — the ROSEDUST theme to derive colors from.
    #[must_use]
    pub fn new(ratio: f64, theme: &RosedustTheme) -> Self {
        Self {
            ratio: ratio.clamp(0.0, 1.0),
            theme: *theme,
            bg_style: Style::default(),
        }
    }

    /// Set the background style for the unfilled portion.
    #[must_use]
    pub fn bg_style(mut self, style: Style) -> Self {
        self.bg_style = style;
        self
    }
}

impl Widget for SemanticBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let filled_width = (self.ratio * area.width as f64).round() as u16;
        let bar_color = self
            .theme
            .progress_style(self.ratio)
            .fg
            .unwrap_or(self.theme.fg);

        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let col = x - area.left();
                if col < filled_width {
                    let cell = &mut buf[(x, y)];
                    cell.set_char(BLOCK_FULL);
                    cell.set_fg(bar_color);
                } else {
                    let cell = &mut buf[(x, y)];
                    cell.set_char(BLOCK_LIGHT);
                    cell.set_style(self.bg_style);
                }
            }
        }
    }
}

// Unicode block characters used for bar rendering.
const BLOCK_FULL: char = '█';
const BLOCK_LIGHT: char = '░';

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    #[test]
    fn gradient_bar_zero() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        GradientBar::new(0.0, Color::Red, Color::Green).render(area, &mut buf);
        // All cells should be unfilled (light block).
        for x in 0..10 {
            assert_eq!(buf[(x, 0)].symbol(), "░");
        }
    }

    #[test]
    fn gradient_bar_full() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        GradientBar::new(1.0, Color::Red, Color::Green).render(area, &mut buf);
        for x in 0..10 {
            assert_eq!(buf[(x, 0)].symbol(), "█");
        }
    }

    #[test]
    fn semantic_bar_half() {
        let theme = RosedustTheme::new();
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        SemanticBar::new(0.5, &theme).render(area, &mut buf);
        // First 5 cells filled, last 5 unfilled.
        for x in 0..5 {
            assert_eq!(buf[(x, 0)].symbol(), "█");
        }
        for x in 5..10 {
            assert_eq!(buf[(x, 0)].symbol(), "░");
        }
    }
}
