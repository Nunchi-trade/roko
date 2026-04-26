//! Layout helper functions for the TUI.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Return a centered rectangle using percentage-based constraints.
#[must_use]
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

/// Apply a 1-cell outer margin when the terminal is large enough (>=120w x >=50h).
/// Returns the inner area unchanged if the terminal is too small.
#[must_use]
pub fn responsive_outer_margin(area: Rect) -> Rect {
    if area.width >= 120 && area.height >= 50 {
        Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        }
    } else {
        area
    }
}

/// Split an area horizontally (left/right) by percentage for the left pane.
#[must_use]
pub fn split_horizontal(area: Rect, left_pct: u16) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Percentage(100 - left_pct),
        ])
        .split(area);
    (chunks[0], chunks[1])
}

/// Split an area vertically (top/bottom) by percentage for the top pane.
#[must_use]
pub fn split_vertical(area: Rect, top_pct: u16) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(top_pct),
            Constraint::Percentage(100 - top_pct),
        ])
        .split(area);
    (chunks[0], chunks[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_rect_fits_inside() {
        let area = Rect::new(0, 0, 100, 50);
        let r = centered_rect(50, 50, area);
        assert!(r.x >= area.x);
        assert!(r.y >= area.y);
        assert!(r.right() <= area.right());
        assert!(r.bottom() <= area.bottom());
    }

    #[test]
    fn responsive_margin_applied_for_large_terminal() {
        let area = Rect::new(0, 0, 120, 50);
        let inner = responsive_outer_margin(area);
        assert_eq!(inner.x, 1);
        assert_eq!(inner.y, 1);
        assert_eq!(inner.width, 118);
        assert_eq!(inner.height, 48);
    }

    #[test]
    fn responsive_margin_skipped_for_small_terminal() {
        let area = Rect::new(0, 0, 80, 24);
        let inner = responsive_outer_margin(area);
        assert_eq!(inner, area);
    }

    #[test]
    fn split_horizontal_sums_to_whole() {
        let area = Rect::new(0, 0, 100, 50);
        let (left, right) = split_horizontal(area, 30);
        assert!(left.width + right.width <= area.width);
        assert_eq!(left.y, right.y);
    }

    #[test]
    fn split_vertical_sums_to_whole() {
        let area = Rect::new(0, 0, 100, 50);
        let (top, bottom) = split_vertical(area, 40);
        assert!(top.height + bottom.height <= area.height);
        assert_eq!(top.x, bottom.x);
    }
}
