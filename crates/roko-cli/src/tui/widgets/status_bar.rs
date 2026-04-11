//! Bottom status bar widget showing keybind hints and event count.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::dashboard::Theme;

// ---------------------------------------------------------------------------
// Key-bind hint text
// ---------------------------------------------------------------------------

const KEYBINDS: &[(&str, &str)] = &[
    ("q", "quit"),
    ("Tab", "next"),
    ("?", "help"),
    ("Enter", "detail"),
    ("r", "refresh"),
];

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the bottom status bar.
///
/// ```text
/// q:quit Tab:next ?:help Enter:detail r:refresh       42 events  12:34:56
/// ```
pub fn render_status_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    event_count: u64,
    theme: &Theme,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // --- Left: keybind hints ---
    let hints: Vec<Span<'_>> = KEYBINDS
        .iter()
        .enumerate()
        .flat_map(|(i, (key, action))| {
            let mut spans = Vec::with_capacity(3);
            if i > 0 {
                spans.push(Span::styled(" ", theme.muted()));
            }
            spans.push(Span::styled(format!("{key}:"), theme.accent()));
            spans.push(Span::styled(*action, theme.muted()));
            spans
        })
        .collect();

    frame.render_widget(Paragraph::new(Line::from(hints)), cols[0]);

    // --- Right: event count + timestamp ---
    let now = chrono::Local::now().format("%H:%M:%S");
    let right_text = format!("{event_count} events  {now}");
    let right = Paragraph::new(Line::from(Span::styled(right_text, theme.muted())))
        .alignment(Alignment::Right);
    frame.render_widget(right, cols[1]);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn status_bar_renders_without_panic() {
        let backend = TestBackend::new(80, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_status_bar(frame, area, 42, &theme);
            })
            .unwrap();
    }
}
