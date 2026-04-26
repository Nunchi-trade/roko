//! F7 Inspect / Context view -- MCP servers, token sparklines, tool panels.
//!
//! Three-section layout: summary (top 25%), token sparklines (mid 25%),
//! detail panels (bottom 50% with 3-column layout for Server, Index, Tool).

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, List, ListItem, Paragraph, Row, Sparkline, Table, Wrap,
};
use ratatui::Frame;

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};

/// MCP server status entry.
#[derive(Debug, Clone)]
pub struct McpServerEntry {
    pub name: String,
    pub status: McpStatus,
    pub tool_count: usize,
    pub total_calls: u64,
    pub errors: u64,
}

/// MCP connection status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpStatus {
    Connected,
    Disconnected,
    Error,
}

impl McpStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Connected => "ok",
            Self::Disconnected => "off",
            Self::Error => "err",
        }
    }
}

/// Token burn data for sparkline rendering.
#[derive(Debug, Clone)]
pub struct TokenBurnData {
    pub agent_id: String,
    pub cumulative: Vec<u64>,
}

/// Code index status entry.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub name: String,
    pub files_indexed: usize,
    pub status: String,
}

/// Tool usage stats entry.
#[derive(Debug, Clone)]
pub struct ToolUsageEntry {
    pub tool_name: String,
    pub call_count: u64,
    pub avg_duration_ms: f64,
    pub error_count: u64,
}

/// Context view data container, populated externally.
#[derive(Debug, Clone, Default)]
pub struct ContextViewData {
    pub mcp_servers: Vec<McpServerEntry>,
    pub token_burns: Vec<TokenBurnData>,
    pub index_entries: Vec<IndexEntry>,
    pub tool_usage: Vec<ToolUsageEntry>,
}

/// Render the full context/inspect view.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    // Context data is not yet in DashboardData; build from available info.
    let ctx_data = build_context_data(data);
    render_with_context_data(frame, area, data, &ctx_data, view_state, theme);
}

/// Render the context view with explicit context data (for integration layer).
pub fn render_with_context_data(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    ctx_data: &ContextViewData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections = Layout::vertical([
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(50),
    ])
    .split(area);

    render_summary(frame, sections[0], data, ctx_data, theme);
    render_token_sparklines(frame, sections[1], ctx_data, view_state, theme);
    render_detail_panels(frame, sections[2], ctx_data, view_state, theme);
}

/// Top section: MCP summary counters.
fn render_summary(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    ctx_data: &ContextViewData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Context Summary ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let server_count = ctx_data.mcp_servers.len();
    let connected = ctx_data
        .mcp_servers
        .iter()
        .filter(|s| s.status == McpStatus::Connected)
        .count();
    let total_calls: u64 = ctx_data.mcp_servers.iter().map(|s| s.total_calls).sum();
    let total_errors: u64 = ctx_data.mcp_servers.iter().map(|s| s.errors).sum();

    let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // Left column: MCP stats
    let mcp_lines = vec![
        Line::from(vec![
            Span::styled("MCP servers:      ", theme.muted()),
            Span::styled(server_count.to_string(), theme.info()),
        ]),
        Line::from(vec![
            Span::styled("connected:        ", theme.muted()),
            Span::styled(connected.to_string(), theme.success()),
        ]),
        Line::from(vec![
            Span::styled("total tool calls: ", theme.muted()),
            Span::styled(total_calls.to_string(), theme.info()),
        ]),
        Line::from(vec![
            Span::styled("errors:           ", theme.muted()),
            if total_errors > 0 {
                Span::styled(total_errors.to_string(), theme.danger())
            } else {
                Span::styled("0", theme.success())
            },
        ]),
    ];
    let mcp_para = Paragraph::new(mcp_lines).wrap(Wrap { trim: false });
    frame.render_widget(mcp_para, cols[0]);

    // Right column: token/cost stats
    let eff = &data.efficiency;
    let token_lines = vec![
        Line::from(vec![
            Span::styled("input tokens:  ", theme.muted()),
            Span::styled(format_count(eff.total_input_tokens), theme.info()),
        ]),
        Line::from(vec![
            Span::styled("output tokens: ", theme.muted()),
            Span::styled(format_count(eff.total_output_tokens), theme.info()),
        ]),
        Line::from(vec![
            Span::styled("total cost:    ", theme.muted()),
            Span::styled(
                format!("${:.4}", eff.total_cost_usd),
                theme.warning(),
            ),
        ]),
        Line::from(vec![
            Span::styled("avg wall time: ", theme.muted()),
            Span::styled(
                format!("{:.0}ms", eff.average_wall_time_ms),
                theme.info(),
            ),
        ]),
    ];
    let token_para = Paragraph::new(token_lines).wrap(Wrap { trim: false });
    frame.render_widget(token_para, cols[1]);
}

/// Middle section: per-agent token burn sparklines.
fn render_token_sparklines(
    frame: &mut Frame<'_>,
    area: Rect,
    ctx_data: &ContextViewData,
    _view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Token Burn (per agent) ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // TODO: use token_sparkline widget here when available
    if ctx_data.token_burns.is_empty() {
        let empty = Paragraph::new("no token data yet - run agents to see sparklines")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    // Allocate equal vertical space per agent (up to what fits)
    let max_agents = (inner.height as usize / 2).max(1);
    let visible = &ctx_data.token_burns[..ctx_data.token_burns.len().min(max_agents)];
    let constraints: Vec<Constraint> = visible
        .iter()
        .map(|_| Constraint::Length(2))
        .collect();
    let rows = Layout::vertical(constraints).split(inner);

    for (i, burn) in visible.iter().enumerate() {
        if i >= rows.len() {
            break;
        }
        let label = truncate(&burn.agent_id, 16);
        let sections =
            Layout::horizontal([Constraint::Length(18), Constraint::Min(0)]).split(rows[i]);

        let label_para = Paragraph::new(label).style(theme.muted());
        frame.render_widget(label_para, sections[0]);

        let data = &burn.cumulative;
        let sparkline = Sparkline::default()
            .data(data)
            .style(theme.info());
        frame.render_widget(sparkline, sections[1]);
    }
}

/// Bottom section: 3-column detail panels (Server, Index, Tool).
fn render_detail_panels(
    frame: &mut Frame<'_>,
    area: Rect,
    ctx_data: &ContextViewData,
    _view_state: &ViewState,
    theme: &Theme,
) {
    let cols = Layout::horizontal([
        Constraint::Percentage(34),
        Constraint::Percentage(33),
        Constraint::Percentage(33),
    ])
    .split(area);

    render_server_panel(frame, cols[0], ctx_data, theme);
    render_index_panel(frame, cols[1], ctx_data, theme);
    render_tool_panel(frame, cols[2], ctx_data, theme);
}

/// Server panel: connected MCP servers.
fn render_server_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    ctx_data: &ContextViewData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Servers ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if ctx_data.mcp_servers.is_empty() {
        let empty = Paragraph::new("no MCP servers")
            .style(theme.muted())
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem<'_>> = ctx_data
        .mcp_servers
        .iter()
        .map(|server| {
            let status_style = match server.status {
                McpStatus::Connected => theme.success(),
                McpStatus::Disconnected => theme.muted(),
                McpStatus::Error => theme.danger(),
            };
            ListItem::new(Line::from(vec![
                Span::styled(server.status.label(), status_style),
                Span::raw(" "),
                Span::styled(&server.name, theme.text()),
                Span::styled(
                    format!(" ({}t)", server.tool_count),
                    theme.muted(),
                ),
            ]))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Index panel: code index status.
fn render_index_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    ctx_data: &ContextViewData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Index ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if ctx_data.index_entries.is_empty() {
        let empty = Paragraph::new("no index data")
            .style(theme.muted())
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem<'_>> = ctx_data
        .index_entries
        .iter()
        .map(|entry| {
            ListItem::new(Line::from(vec![
                Span::styled(&entry.name, theme.text()),
                Span::raw(": "),
                Span::styled(
                    format!("{} files", entry.files_indexed),
                    theme.info(),
                ),
                Span::raw(" "),
                Span::styled(&entry.status, theme.muted()),
            ]))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Tool panel: tool usage stats.
fn render_tool_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    ctx_data: &ContextViewData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Tools ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if ctx_data.tool_usage.is_empty() {
        let empty = Paragraph::new("no tool usage data")
            .style(theme.muted())
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let rows: Vec<Row<'_>> = ctx_data
        .tool_usage
        .iter()
        .map(|tool| {
            let error_style = if tool.error_count > 0 {
                theme.danger()
            } else {
                theme.muted()
            };
            Row::new(vec![
                Cell::from(truncate(&tool.tool_name, 16)),
                Cell::from(tool.call_count.to_string()),
                Cell::from(format!("{:.0}ms", tool.avg_duration_ms)),
                Cell::from(Span::styled(tool.error_count.to_string(), error_style)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(10),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(4),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["tool", "calls", "avg", "err"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// Build context data from available dashboard data.
fn build_context_data(data: &DashboardData) -> ContextViewData {
    // Build token burn sparklines from efficiency events
    let mut burn_map: std::collections::HashMap<String, Vec<u64>> =
        std::collections::HashMap::new();
    for event in &data.efficiency_events {
        let id = event.agent_id.clone();
        let cumulative = burn_map.entry(id).or_default();
        let prev = cumulative.last().copied().unwrap_or(0);
        cumulative.push(prev + event.input_tokens + event.output_tokens);
    }

    let token_burns: Vec<TokenBurnData> = burn_map
        .into_iter()
        .map(|(agent_id, cumulative)| TokenBurnData {
            agent_id,
            cumulative,
        })
        .collect();

    ContextViewData {
        mcp_servers: Vec::new(), // MCP status not yet in DashboardData
        token_burns,
        index_entries: Vec::new(), // Index status not yet in DashboardData
        tool_usage: Vec::new(),    // Tool stats not yet in DashboardData
    }
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
