//! Token context capacity gauge with threshold markers.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Gauge};

use super::super::dashboard::Theme;

/// Render a horizontal context window usage gauge with color thresholds.
///
/// Colors: green below 80%, yellow 80-90%, red above 90%.
pub fn render_context_gauge(
    frame: &mut Frame<'_>,
    area: Rect,
    used: u64,
    total: u64,
    theme: &Theme,
) {
    let ratio = if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    };

    let pct = (ratio * 100.0).round() as u64;

    let fill_color = if ratio >= 0.9 {
        theme.danger
    } else if ratio >= 0.8 {
        theme.warning
    } else {
        theme.success
    };

    let label_text = format!("{used}/{total} tokens ({pct}%)");
    let label = Span::styled(
        label_text,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    let title = if ratio >= 0.9 {
        "context [CRITICAL]"
    } else if ratio >= 0.8 {
        "context [WARNING]"
    } else {
        "context"
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(if ratio >= 0.9 {
                    theme.danger()
                } else {
                    theme.muted()
                }),
        )
        .ratio(ratio)
        .label(label)
        .gauge_style(Style::default().fg(fill_color).bg(theme.background));

    frame.render_widget(gauge, area);
}
