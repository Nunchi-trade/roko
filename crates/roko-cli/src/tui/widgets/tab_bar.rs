//! Tab bar widget with F-key labels.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::super::dashboard::Theme;

/// Definition of a single tab.
#[derive(Debug, Clone)]
pub struct TabDef {
    pub label: String,
    pub fkey: Option<String>,
}

/// Render a tab bar with F-key hints.
///
/// The active tab is highlighted with accent background.
/// F-key labels appear right-aligned in each tab cell.
pub fn render_tab_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    tabs: &[TabDef],
    active: usize,
    theme: &Theme,
) {
    if tabs.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }

    let constraints: Vec<Constraint> = tabs
        .iter()
        .map(|_| Constraint::Ratio(1, tabs.len() as u32))
        .collect();

    let columns = Layout::horizontal(constraints).split(area);

    for (i, (tab, col)) in tabs.iter().zip(columns.iter()).enumerate() {
        let is_active = i == active;

        let (label_style, border_style) = if is_active {
            (
                Style::default()
                    .fg(theme.accent_foreground)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(theme.accent),
            )
        } else {
            (
                Style::default().fg(theme.foreground),
                Style::default().fg(theme.muted),
            )
        };

        let mut spans = vec![Span::styled(tab.label.clone(), label_style)];

        if let Some(ref fkey) = tab.fkey {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!("[{fkey}]"),
                Style::default().fg(theme.muted),
            ));
        }

        let content = Paragraph::new(Line::from(spans))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(border_style),
            );

        frame.render_widget(content, *col);
    }
}
