//! Top header bar widget showing branding, progress, and tab hints.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::super::dashboard::Theme;

// ---------------------------------------------------------------------------
// Tab hint labels
// ---------------------------------------------------------------------------

const TAB_HINTS: &[(&str, &str)] = &[
    ("F1", "Dashboard"),
    ("F2", "Plans"),
    ("F3", "Agents"),
    ("F4", "Logs"),
    ("F5", "Signals"),
    ("F6", "Config"),
];

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the top header bar.
///
/// Layout (inside the provided `area`):
/// ```text
/// ┌────────────────────────────────────────────────────────────┐
/// │ roko dashboard              2/5 plans  ·  3 tasks active   │
/// │ F1:Dashboard F2:Plans F3:Agents F4:Logs F5:Signals F6:Config│
/// └────────────────────────────────────────────────────────────┘
/// ```
pub fn render_header_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    plans_active: usize,
    plans_total: usize,
    tasks_active: usize,
    theme: &Theme,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner_rect(area));

    // --- Row 1: branding + progress summary ---
    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    let branding = Line::from(vec![
        Span::styled("roko ", theme.accent_bold()),
        Span::styled(title, theme.text()),
    ]);
    frame.render_widget(Paragraph::new(branding), top_cols[0]);

    let progress_text = format!(
        "{}/{} plans  \u{00b7}  {} task{} active",
        plans_active,
        plans_total,
        tasks_active,
        if tasks_active == 1 { "" } else { "s" },
    );
    let progress = Paragraph::new(Line::from(Span::styled(progress_text, theme.muted())))
        .alignment(Alignment::Right);
    frame.render_widget(progress, top_cols[1]);

    // --- Row 2: F-key tab hints ---
    let hints: Vec<Span<'_>> = TAB_HINTS
        .iter()
        .enumerate()
        .flat_map(|(i, (key, label))| {
            let mut spans = Vec::with_capacity(3);
            if i > 0 {
                spans.push(Span::styled(" ", theme.muted()));
            }
            spans.push(Span::styled(
                format!("{key}:"),
                theme.accent().add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(*label, theme.text()));
            spans
        })
        .collect();

    frame.render_widget(Paragraph::new(Line::from(hints)), rows[1]);

    // --- Outer border ---
    let border = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.muted())
        .title(Span::styled("status", theme.accent()));
    frame.render_widget(border, area);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Shrink `area` by one cell on each side (inside a `Borders::ALL` block).
fn inner_rect(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
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
    fn header_bar_renders_without_panic() {
        let backend = TestBackend::new(80, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_header_bar(frame, area, "dashboard", 2, 5, 3, &theme);
            })
            .unwrap();
    }
}
