//! Reusable widgets for the dashboard TUI.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::{DateTime, Utc};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Gauge, List, ListItem, Paragraph, Row, Sparkline, Table, Tabs, Wrap,
};
use ratatui::Frame;
use serde_json::Value;
use roko_learn::efficiency::AgentEfficiencyEvent;

use super::dashboard::{read_json_value, read_jsonl_values, DashboardData, DashboardScaffold, CFactor};
use super::pages::{PageId, PageRegistry};

/// Render the dashboard shell.
pub fn render_dashboard(
    frame: &mut Frame<'_>,
    dashboard: &DashboardScaffold,
    data: &DashboardData,
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
    render_page(frame, body[1], dashboard, data, pages, active_page, scroll);

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
    data: &DashboardData,
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

    if active_page == PageId::Health {
        render_overview_page(frame, area, data);
        return;
    }

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

fn render_overview_page(frame: &mut Frame<'_>, area: Rect, data: &DashboardData) {
    let outer = Block::default().borders(Borders::ALL).title("Overview");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let body = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(inner);
    let columns = Layout::horizontal([
        Constraint::Percentage(44),
        Constraint::Percentage(30),
        Constraint::Percentage(26),
    ])
    .split(body[0]);

    render_plan_overview(frame, columns[0], data);
    render_health_indicators(frame, columns[1], data);
    render_alerts(frame, columns[2], data);
    render_summary_bar(frame, body[1], data);
}

fn render_plan_overview(frame: &mut Frame<'_>, area: Rect, data: &DashboardData) {
    let block = Block::default().borders(Borders::ALL).title("Plans");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 8 || inner.height < 3 {
        return;
    }

    let gauge_width = inner.width.saturating_div(3).clamp(12, 18).min(inner.width.saturating_sub(1));
    let table_width = inner.width.saturating_sub(gauge_width).max(1);
    let columns = Layout::horizontal([
        Constraint::Length(table_width),
        Constraint::Length(gauge_width),
    ])
    .split(inner);

    let rows = collect_plan_rows(data);
    if rows.is_empty() {
        let empty = Paragraph::new("no plans")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, columns[0]);
        return;
    }

    let visible = rows.len().min(columns[0].height.saturating_sub(1) as usize);
    let visible_rows = &rows[..visible];

    let table_rows: Vec<Row<'_>> = visible_rows
        .iter()
        .map(|row| {
            Row::new(vec![
                Cell::from(row.name.clone()),
                Cell::from(Span::styled(row.status.clone(), row.status_style)),
                Cell::from(row.elapsed.clone()),
            ])
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Min(10),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("plan"),
            Cell::from("status"),
            Cell::from("elapsed"),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1);
    frame.render_widget(table, columns[0]);

    let progress_block = Block::default().borders(Borders::ALL).title("Progress");
    let progress_inner = progress_block.inner(columns[1]);
    frame.render_widget(progress_block, columns[1]);

    if visible == 0 || progress_inner.height == 0 {
        return;
    }

    let gauge_rows = Layout::vertical(vec![Constraint::Length(1); visible]).split(progress_inner);
    for (row, area) in visible_rows.iter().zip(gauge_rows.iter()) {
        let label = format!("{:>3}%", (row.progress * 100.0).round() as u64);
        let gauge = Gauge::default()
            .ratio(row.progress)
            .label(Span::styled(label, row.status_style))
            .gauge_style(row.gauge_style);
        frame.render_widget(gauge, *area);
    }
}

fn render_health_indicators(frame: &mut Frame<'_>, area: Rect, data: &DashboardData) {
    let block = Block::default().borders(Borders::ALL).title("Health");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let panels = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Min(0),
    ])
    .split(inner);

    let gate_series = gate_pass_rate_series(data.root());
    let cost_series = cost_trend_series(data);
    let c_factor_series = cfactor_series(data);

    let gate = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title("gate pass rate (7d)"))
        .data(&gate_series)
        .style(Style::default().fg(Color::Green));
    frame.render_widget(gate, panels[0]);

    let cost = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title("cost trend"))
        .data(&cost_series)
        .style(Style::default().fg(Color::Yellow));
    frame.render_widget(cost, panels[1]);

    let cfactor = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title("C-Factor score"))
        .data(&c_factor_series)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(cfactor, panels[2]);
}

fn render_alerts(frame: &mut Frame<'_>, area: Rect, data: &DashboardData) {
    let block = Block::default().borders(Borders::ALL).title("Conductor Alerts");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut alerts = data
        .conductor_alerts
        .iter()
        .rev()
        .take(10)
        .cloned()
        .collect::<Vec<_>>();
    if alerts.is_empty() {
        let empty = Paragraph::new("no alerts")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }
    alerts.reverse();

    let items: Vec<ListItem<'_>> = alerts
        .into_iter()
        .map(|alert| {
            let severity_style = severity_style(&alert.severity);
            let severity = Span::styled(format!("{:>8}", alert.severity), severity_style);
            let max_message = inner.width.saturating_sub(12) as usize;
            let message = Span::styled(
                truncate_alert_message(&alert.message, max_message),
                Style::default().fg(Color::White),
            );
            ListItem::new(Line::from(vec![severity, Span::raw(" "), message]))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_summary_bar(frame: &mut Frame<'_>, area: Rect, data: &DashboardData) {
    let summary = build_summary_bar(data);
    let paragraph = Paragraph::new(summary)
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(paragraph, area);
}

fn collect_plan_rows(data: &DashboardData) -> Vec<PlanRow> {
    let tracker_counts = load_task_tracker_completion_counts(data.root());
    let started_at = load_plan_started_at_map(data.root());
    let mut active_task_status = HashMap::new();
    for task in &data.active_tasks {
        active_task_status
            .entry(task.plan_id.clone())
            .or_insert_with(|| task.status.clone());
    }

    let mut rows = data
        .plans
        .iter()
        .map(|plan| {
            let completed = plan.completed;
            let completed_tasks = tracker_counts.get(&plan.id).copied().unwrap_or_default();
            let total_tasks = plan.task_count.max(1);
            let progress = if completed {
                1.0
            } else {
                (completed_tasks as f64 / total_tasks as f64).clamp(0.0, 1.0)
            };
            let status = if completed {
                String::from("done")
            } else {
                active_task_status
                    .get(&plan.id)
                    .cloned()
                    .unwrap_or_else(|| String::from("active"))
            };
            let status_style = status_style(&status, completed);
            let gauge_style = gauge_style(&status, completed);
            let elapsed = started_at
                .get(&plan.id)
                .map(|started_at_ms| format_elapsed_ms(now_ms().saturating_sub(*started_at_ms)))
                .unwrap_or_else(|| String::from("--"));

            PlanRow {
                name: plan.title.clone(),
                status,
                status_style,
                gauge_style,
                progress,
                elapsed,
                completed,
            }
        })
        .collect::<Vec<_>>();

    rows.sort_by(|a, b| b.completed.cmp(&a.completed).then_with(|| a.name.cmp(&b.name)));
    rows
}

fn build_summary_bar(data: &DashboardData) -> String {
    let active_plans = data.plans.iter().filter(|plan| !plan.completed).count();
    let done_plans = data.plans.len().saturating_sub(active_plans);
    let total_tasks = data.plans.iter().map(|plan| plan.task_count).sum::<usize>();
    let done_tasks = load_task_tracker_completion_total(data.root());
    let cost = data.efficiency.total_cost_usd;
    let cfactor = data
        .cfactor
        .as_ref()
        .map(|snapshot| snapshot.overall)
        .unwrap_or(0.0);
    let trend = cfactor_trend(data);
    format!(
        "Plans: {active_plans} active, {done_plans} done | Tasks: {done_tasks}/{total_tasks} | Cost: ${cost:.2} | C-Factor: {cfactor:.2} {trend}"
    )
}

fn load_task_tracker_completion_counts(root: &Path) -> HashMap<String, usize> {
    let path = root.join(".roko").join("state").join("task-trackers.json");
    let Some(value) = read_json_value(&path) else {
        return HashMap::new();
    };
    let Some(entries) = value.as_array() else {
        return HashMap::new();
    };

    let mut counts = HashMap::new();
    for entry in entries {
        let plan_id = entry.get("plan_id").and_then(Value::as_str).unwrap_or_default();
        if plan_id.is_empty() {
            continue;
        }
        let completed = entry
            .get("completed")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        counts.insert(plan_id.to_string(), completed);
    }
    counts
}

fn load_task_tracker_completion_total(root: &Path) -> usize {
    load_task_tracker_completion_counts(root).values().sum()
}

fn load_plan_started_at_map(root: &Path) -> HashMap<String, u64> {
    let path = root.join(".roko").join("state").join("executor.json");
    let Some(state) = read_json_value(&path) else {
        return HashMap::new();
    };
    let Some(plan_states) = state.get("plan_states").and_then(Value::as_object) else {
        return HashMap::new();
    };

    let mut started = HashMap::new();
    for (plan_id, plan_state) in plan_states {
        let started_at_ms = plan_state
            .get("started_at_ms")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if started_at_ms > 0 {
            started.insert(plan_id.clone(), started_at_ms);
        }
    }
    started
}

fn gate_pass_rate_series(root: &Path) -> Vec<u64> {
    let path = root.join(".roko").join("signals.jsonl");
    let entries = read_jsonl_values(&path);
    let today = Utc::now().date_naive();
    let mut buckets: BTreeMap<i64, (u64, u64)> = BTreeMap::new();

    for entry in entries {
        let Some(kind) = entry.get("kind").and_then(Value::as_str) else {
            continue;
        };
        if !is_gate_kind(kind) {
            continue;
        }
        let Some(ts_ms) = entry.get("created_at_ms").and_then(Value::as_i64) else {
            continue;
        };
        let Some(timestamp) = DateTime::<Utc>::from_timestamp_millis(ts_ms) else {
            continue;
        };
        let day = timestamp.date_naive();
        let age = today.signed_duration_since(day).num_days();
        if !(0..7).contains(&age) {
            continue;
        }
        let bucket = buckets.entry(age).or_default();
        bucket.1 += 1;
        if gate_passed_from_value(&entry) {
            bucket.0 += 1;
        }
    }

    (0..7)
        .rev()
        .map(|age| {
            let (passed, total) = buckets.get(&age).copied().unwrap_or_default();
            if total == 0 {
                0
            } else {
                ((passed as f64 / total as f64) * 100.0).round() as u64
            }
        })
        .collect()
}

fn cost_trend_series(data: &DashboardData) -> Vec<u64> {
    let today = Utc::now().date_naive();
    let mut buckets: BTreeMap<i64, f64> = BTreeMap::new();

    for event in load_efficiency_events(data.root()) {
        let Ok(timestamp) = DateTime::parse_from_rfc3339(&event.timestamp) else {
            continue;
        };
        let day = timestamp.with_timezone(&Utc).date_naive();
        let age = today.signed_duration_since(day).num_days();
        if !(0..7).contains(&age) {
            continue;
        }
        *buckets.entry(age).or_default() += event.cost_usd;
    }

    (0..7)
        .rev()
        .map(|age| {
            let value = buckets.get(&age).copied().unwrap_or_default();
            (value * 100.0).round().max(0.0) as u64
        })
        .collect()
}

fn load_efficiency_events(root: &Path) -> Vec<AgentEfficiencyEvent> {
    let path = root.join(".roko").join("learn").join("efficiency.jsonl");
    let entries = read_jsonl_values(&path);
    entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<AgentEfficiencyEvent>(entry).ok())
        .collect()
}

fn cfactor_series(data: &DashboardData) -> Vec<u64> {
    let path = data.root().join(".roko").join("learn").join("c-factor.jsonl");
    let history = read_jsonl_values(&path)
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<CFactor>(entry).ok())
        .collect::<Vec<_>>();
    let source = if history.is_empty() {
        data.cfactor.clone().into_iter().collect::<Vec<_>>()
    } else {
        history
    };

    let mut series = source
        .into_iter()
        .rev()
        .take(7)
        .map(|snapshot| (snapshot.overall * 100.0).round().max(0.0) as u64)
        .collect::<Vec<_>>();
    series.reverse();
    if series.is_empty() {
        series.push(0);
    }
    series
}

fn cfactor_trend(data: &DashboardData) -> &'static str {
    let path = data.root().join(".roko").join("learn").join("c-factor.jsonl");
    let history = read_jsonl_values(&path)
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<CFactor>(entry).ok())
        .collect::<Vec<_>>();
    if history.len() >= 2 {
        let latest = history[history.len() - 1].overall;
        let previous = history[history.len() - 2].overall;
        if latest > previous {
            "↑"
        } else if latest < previous {
            "↓"
        } else {
            "→"
        }
    } else if data
        .cfactor
        .as_ref()
        .is_some_and(|snapshot| snapshot.overall >= 0.5)
    {
        "↑"
    } else {
        "↓"
    }
}

fn gate_passed_from_value(value: &Value) -> bool {
    if let Some(passed) = value
        .pointer("/tags/passed")
        .and_then(Value::as_str)
        .and_then(|value| match value {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
    {
        return passed;
    }

    value
        .pointer("/body/data/passed")
        .and_then(Value::as_bool)
        .or_else(|| value.pointer("/body/passed").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn is_gate_kind(kind: &str) -> bool {
    kind == "gate_verdict" || kind.starts_with("gate:") || kind.starts_with("gate_")
}

fn status_style(status: &str, completed: bool) -> Style {
    if completed {
        return Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD);
    }

    match status.to_ascii_lowercase().as_str() {
        "done" | "complete" | "completed" => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        "failed" | "error" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        "gating" => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        "implementing" | "running" | "active" => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        "queued" | "pending" => Style::default().fg(Color::Gray),
        _ => Style::default().fg(Color::White),
    }
}

fn gauge_style(status: &str, completed: bool) -> Style {
    if completed {
        return Style::default().fg(Color::Green).bg(Color::Black);
    }

    match status.to_ascii_lowercase().as_str() {
        "failed" | "error" => Style::default().fg(Color::Red).bg(Color::Black),
        "gating" => Style::default().fg(Color::Yellow).bg(Color::Black),
        "implementing" | "running" | "active" => Style::default().fg(Color::Cyan).bg(Color::Black),
        "queued" | "pending" => Style::default().fg(Color::Gray).bg(Color::Black),
        _ => Style::default().fg(Color::White).bg(Color::Black),
    }
}

fn severity_style(severity: &str) -> Style {
    match severity.to_ascii_lowercase().as_str() {
        "critical" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        "warning" => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        _ => Style::default().fg(Color::Gray),
    }
}

fn truncate_alert_message(message: &str, max: usize) -> String {
    let mut chars = message.chars();
    let mut out = String::new();
    for _ in 0..max {
        if let Some(ch) = chars.next() {
            out.push(ch);
        } else {
            return out;
        }
    }
    if chars.next().is_some() && max > 3 {
        out.truncate(out.len().saturating_sub(3));
        out.push_str("...");
    }
    out
}

fn now_ms() -> u64 {
    u64::try_from(Utc::now().timestamp_millis()).unwrap_or(u64::MAX)
}

fn format_elapsed_ms(ms: u64) -> String {
    let secs = ms / 1000;
    if secs == 0 {
        return String::from("<1s");
    }
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h {}m", mins % 60);
    }
    format!("{hours}h {}m", mins % 60)
}

#[derive(Debug, Clone)]
struct PlanRow {
    name: String,
    status: String,
    status_style: Style,
    gauge_style: Style,
    progress: f64,
    elapsed: String,
    completed: bool,
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
