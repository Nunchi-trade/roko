//! Reusable widgets for the dashboard TUI.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use super::dashboard::DashboardScaffold;
use super::pages::{PageId, PageRegistry};

/// Render the dashboard shell.
pub fn render_dashboard(
    frame: &mut Frame<'_>,
    dashboard: &DashboardScaffold,
    pages: &PageRegistry,
    active_page: PageId,
    scroll: u16,
) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(frame.area());

    render_header(frame, areas[0], dashboard, pages, active_page);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(0)])
        .split(areas[1]);
    render_sidebar(frame, body[0], pages, active_page);
    render_page(frame, body[1], dashboard, pages, active_page, scroll);

    render_footer(frame, areas[2], pages, active_page);
}

/// Render the top shell header and page tabs.
pub fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    dashboard: &DashboardScaffold,
    pages: &PageRegistry,
    active_page: PageId,
) {
    let header = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(3)])
        .split(area);

    let summary = dashboard.summary();
    let title = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("roko ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("dashboard"),
        ]),
        Line::from(summary.to_string()),
    ])
    .block(Block::default().borders(Borders::ALL).title("status"));
    frame.render_widget(title, header[0]);

    let titles: Vec<Line<'_>> = pages
        .iter()
        .map(|page| Line::from(Span::raw(page.title)))
        .collect();
    let active_index = pages
        .ids()
        .iter()
        .position(|page| *page == active_page)
        .unwrap_or(0);
    let tabs = Tabs::new(titles)
        .select(active_index)
        .block(Block::default().borders(Borders::ALL).title("pages"))
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, header[1]);
}

/// Render the page list sidebar.
pub fn render_sidebar(
    frame: &mut Frame<'_>,
    area: Rect,
    pages: &PageRegistry,
    active_page: PageId,
) {
    let items: Vec<ListItem<'_>> = pages
        .iter()
        .map(|page| {
            ListItem::new(page.render_summary_line(page.id == active_page)).style(
                if page.id == active_page {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                },
            )
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("navigation"),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(list, area);
}

/// Render the active page content.
pub fn render_page(
    frame: &mut Frame<'_>,
    area: Rect,
    dashboard: &DashboardScaffold,
    pages: &PageRegistry,
    active_page: PageId,
    scroll: u16,
) {
    let Some(page) = pages.page(active_page) else {
        let placeholder = Paragraph::new("missing page")
            .block(Block::default().borders(Borders::ALL).title("content"));
        frame.render_widget(placeholder, area);
        return;
    };

    let rendered = page.render(dashboard);
    let content = Paragraph::new(rendered)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(page.title()),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(content, area);
}

/// Render the footer with keyboard shortcuts.
pub fn render_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    pages: &PageRegistry,
    active_page: PageId,
) {
    let page_count = pages.len();
    let footer = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("q", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" quit  "),
            Span::styled("r", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" refresh  "),
            Span::styled("←/→", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" page  "),
            Span::styled("↑/↓", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" scroll"),
        ]),
        Line::from(format!(
            "active: {} | pages: {}",
            active_page.slug(),
            page_count
        )),
    ])
    .block(Block::default().borders(Borders::ALL).title("controls"));

    frame.render_widget(footer, area);
}
