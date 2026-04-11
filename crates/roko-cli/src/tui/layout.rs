//! Root layout for the TUI.
//!
//! Splits the terminal area into a fixed header, a flexible content area,
//! and a fixed status bar.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// The top-level layout structure for the dashboard TUI.
///
/// ```text
/// ┌──────────────── header (1 line) ────────────────┐
/// ├─────────────── content (flex) ──────────────────┤
/// │                                                   │
/// │                                                   │
/// ├──────────────── status (1 line) ────────────────┤
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootLayout {
    /// 1-line header at the top (tab bar, title).
    pub header: Rect,
    /// Full-width content area (pages render here).
    pub content: Rect,
    /// 1-line status bar at the bottom.
    pub status: Rect,
}

impl RootLayout {
    /// Compute the root layout from a terminal area.
    ///
    /// If the area has fewer than 3 rows, header and status collapse
    /// and all space goes to content.
    #[must_use]
    pub fn compute(area: Rect) -> Self {
        if area.height < 3 {
            return Self {
                header: Rect::new(area.x, area.y, area.width, 0),
                content: area,
                status: Rect::new(area.x, area.bottom(), area.width, 0),
            };
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(area);

        Self {
            header: chunks[0],
            content: chunks[1],
            status: chunks[2],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_layout() {
        let area = Rect::new(0, 0, 80, 24);
        let layout = RootLayout::compute(area);
        assert_eq!(layout.header.height, 1);
        assert_eq!(layout.content.height, 22);
        assert_eq!(layout.status.height, 1);
        assert_eq!(layout.header.y, 0);
        assert_eq!(layout.content.y, 1);
        assert_eq!(layout.status.y, 23);
    }

    #[test]
    fn tiny_terminal() {
        let area = Rect::new(0, 0, 80, 2);
        let layout = RootLayout::compute(area);
        assert_eq!(layout.header.height, 0);
        assert_eq!(layout.content, area);
        assert_eq!(layout.status.height, 0);
    }

    #[test]
    fn minimum_viable() {
        let area = Rect::new(0, 0, 80, 3);
        let layout = RootLayout::compute(area);
        assert_eq!(layout.header.height, 1);
        assert_eq!(layout.content.height, 1);
        assert_eq!(layout.status.height, 1);
    }
}
