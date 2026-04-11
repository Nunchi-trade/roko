//! Scrollbar overlay for long lists.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

use super::super::dashboard::Theme;

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render a vertical scrollbar overlay on the right edge of `area`.
///
/// Only visible when `total_items > visible_items`.
pub fn render_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    total_items: usize,
    visible_items: usize,
    scroll_offset: usize,
    theme: &Theme,
) {
    if total_items <= visible_items {
        return;
    }

    let mut state = ScrollbarState::new(total_items).position(scroll_offset);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .thumb_style(Style::default().fg(theme.accent))
        .track_style(Style::default().fg(theme.muted))
        .begin_symbol(Some("\u{25b2}"))
        .end_symbol(Some("\u{25bc}"));

    frame.render_stateful_widget(scrollbar, area, &mut state);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn scrollbar_hidden_when_fits() {
        let backend = TestBackend::new(20, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        terminal
            .draw(|frame| {
                let area = frame.area();
                // 5 items fits in 10 rows -- no scrollbar drawn.
                render_scrollbar(frame, area, 5, 10, 0, &theme);
            })
            .unwrap();
    }

    #[test]
    fn scrollbar_shown_when_overflow() {
        let backend = TestBackend::new(20, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_scrollbar(frame, area, 50, 10, 5, &theme);
            })
            .unwrap();
    }
}
