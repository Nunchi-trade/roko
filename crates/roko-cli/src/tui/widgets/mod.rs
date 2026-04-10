//! Reusable widgets for the dashboard TUI.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    BarChart, Block, Borders, Cell, Gauge, List, ListItem, Paragraph, Row, Sparkline, Table, Tabs,
    Wrap,
};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_learn::cfactor::trend_arrow as cfactor_trend_arrow;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::prompt_experiment::{ExperimentStatus, ExperimentStore};
use serde_json::Value;

use super::dashboard::{
    AgentActivitySnapshot, CascadeRouterState, DashboardData, DashboardScaffold, GateFailureRow,
    GateSummaryRow, GateThresholdRow, GateTrend, PlanExecutionSnapshot, SignalSummary, Theme,
    build_agent_activity_snapshot, operating_frequency_label, read_json_value, read_jsonl_values,
};
use super::pages::{PageId, PageRegistry};

/// Render the dashboard shell.
pub fn render_dashboard(
    frame: &mut Frame<'_>,
    dashboard: &DashboardScaffold,
    data: &DashboardData,
    pages: &PageRegistry,
    active_page: PageId,
    scroll: u16,
    signal_selected: usize,
    gate_failure_selected: usize,
    theme: &Theme,
) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(frame.area());

    render_header(frame, areas[0], dashboard, pages, active_page, theme);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(0)])
        .split(areas[1]);
    render_sidebar(frame, body[0], pages, active_page, theme);
    render_page(
        frame,
        body[1],
        dashboard,
        data,
        pages,
        active_page,
        scroll,
        signal_selected,
        gate_failure_selected,
        theme,
    );

    render_footer_themed(frame, areas[2], pages, active_page, theme);
}

/// Render the top shell header and page tabs.
pub fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    dashboard: &DashboardScaffold,
    pages: &PageRegistry,
    active_page: PageId,
    theme: &Theme,
) {
    let header = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(3)])
        .split(area);

    let summary = dashboard.summary();
    let title = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("roko ", theme.accent_bold()),
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
        .style(theme.muted())
        .highlight_style(theme.selection());
    frame.render_widget(tabs, header[1]);
}

/// Render the page list sidebar.
pub fn render_sidebar(
    frame: &mut Frame<'_>,
    area: Rect,
    pages: &PageRegistry,
    active_page: PageId,
    theme: &Theme,
) {
    let items: Vec<ListItem<'_>> = pages
        .iter()
        .map(|page| {
            ListItem::new(page.render_summary_line(page.id == active_page)).style(
                if page.id == active_page {
                    theme.selection()
                } else {
                    theme.text()
                },
            )
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("navigation"))
        .highlight_style(theme.selection());

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
    signal_selected: usize,
    gate_failure_selected: usize,
    theme: &Theme,
) {
    let Some(page) = pages.page(active_page) else {
        let placeholder = Paragraph::new("missing page").style(theme.muted()).block(
            Block::default()
                .borders(Borders::ALL)
                .title("content")
                .border_style(theme.muted()),
        );
        frame.render_widget(placeholder, area);
        return;
    };

    if active_page == PageId::Health {
        render_overview_page(frame, area, data);
        return;
    }

    if active_page == PageId::PlanView {
        render_plan_execution_page(frame, area, data, scroll);
        return;
    }

    if active_page == PageId::AgentStatus {
        render_agent_activity_page(frame, area, data);
        return;
    }

    if active_page == PageId::GateResults {
        render_gate_results_page(frame, area, data, gate_failure_selected);
        return;
    }

    if active_page == PageId::Learning {
        render_learning_page(frame, area, data);
        return;
    }

    if active_page == PageId::Signals {
        render_signals_page(frame, area, data, signal_selected);
        return;
    }

    let rendered = page.render(dashboard);
    let content = Paragraph::new(rendered)
        .block(Block::default().borders(Borders::ALL).title(page.title()))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(content, area);
}

fn render_agent_activity_page(frame: &mut Frame<'_>, area: Rect, data: &DashboardData) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Agent Activity");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(snapshot) = build_agent_activity_snapshot(&data.agents, &data.efficiency_events)
    else {
        let empty = Paragraph::new("no active agents or efficiency history")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    };

    let sections = Layout::vertical([
        Constraint::Percentage(44),
        Constraint::Percentage(20),
        Constraint::Percentage(36),
    ])
    .split(inner);

    render_active_agents_table(frame, sections[0], &snapshot.active_agents, data);
    render_model_distribution_chart(frame, sections[1], &snapshot);
    render_model_cost_breakdown(frame, sections[2], &snapshot);
}

fn render_gate_results_page(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    gate_failure_selected: usize,
) {
    let block = Block::default().borders(Borders::ALL).title("Gate Results");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let sections = Layout::vertical([
        Constraint::Percentage(42),
        Constraint::Percentage(24),
        Constraint::Percentage(34),
    ])
    .split(inner);

    render_gate_summary_table(frame, sections[0], &data.gate_results_page.gate_rows);
    render_gate_thresholds_table(frame, sections[1], &data.gate_results_page.threshold_rows);
    render_gate_failures_list(
        frame,
        sections[2],
        &data.gate_results_page.failure_rows,
        gate_failure_selected,
    );
}

fn render_learning_page(frame: &mut Frame<'_>, area: Rect, data: &DashboardData) {
    let block = Block::default().borders(Borders::ALL).title("Learning");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let sections =
        Layout::vertical([Constraint::Percentage(56), Constraint::Percentage(44)]).split(inner);
    let top = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(sections[0]);

    render_cascade_router_table(frame, top[0], &data.cascade_router);
    render_active_experiments_table(frame, top[1], &data.experiment_store);
    render_learning_trends(frame, sections[1], &data.efficiency_events);
}

fn render_signals_page(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    signal_selected: usize,
) {
    let block = Block::default().borders(Borders::ALL).title("Signals");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let signals = prepare_signals(&data.recent_signals);
    if signals.is_empty() {
        let empty = Paragraph::new("no signals found in .roko/signals.jsonl")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let sections = Layout::vertical([
        Constraint::Percentage(46),
        Constraint::Length(8),
        Constraint::Min(0),
    ])
    .split(inner);
    let selected = signal_selected.min(signals.len().saturating_sub(1));

    render_signals_table(frame, sections[0], &signals, selected);
    render_signal_kind_chart(frame, sections[1], &signals);
    render_signal_tree(frame, sections[2], &signals, selected);
}

fn render_cascade_router_table(frame: &mut Frame<'_>, area: Rect, router: &CascadeRouterState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("cascade router");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = build_cascade_router_rows(router);
    if rows.is_empty() {
        let empty = Paragraph::new("no cascade-router data")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let table_rows: Vec<Row<'_>> = rows
        .iter()
        .map(|row| {
            Row::new(vec![
                Cell::from(truncate_text(&row.model, 18)),
                Cell::from(Span::styled(
                    format_pct(row.weight),
                    Style::default().fg(Color::Cyan),
                )),
                Cell::from(row.recommendations.to_string()),
                Cell::from(Span::styled(
                    format_float(row.ucb_score),
                    Style::default().fg(Color::Yellow),
                )),
            ])
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Min(16),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("model"),
            Cell::from("weight"),
            Cell::from("recs"),
            Cell::from("UCB"),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1);

    frame.render_widget(table, inner);
}

fn render_active_experiments_table(frame: &mut Frame<'_>, area: Rect, store: &ExperimentStore) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("active experiments");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = build_active_experiment_rows(store);
    if rows.is_empty() {
        let empty = Paragraph::new("no active experiments")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let table_rows: Vec<Row<'_>> = rows
        .iter()
        .map(|row| {
            Row::new(vec![
                Cell::from(truncate_text(&row.experiment, 18)),
                Cell::from(truncate_text(&row.variants, 18)),
                Cell::from(truncate_text(&row.sample_sizes, 18)),
                Cell::from(truncate_text(&row.winner, 14)),
                Cell::from(Span::styled(
                    truncate_text(&row.significance, 14),
                    significance_style(&row.significance),
                )),
            ])
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Min(16),
            Constraint::Min(14),
            Constraint::Min(14),
            Constraint::Length(14),
            Constraint::Length(16),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("experiment"),
            Cell::from("variants"),
            Cell::from("samples"),
            Cell::from("winner"),
            Cell::from("significance"),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1);

    frame.render_widget(table, inner);
}

fn render_learning_trends(frame: &mut Frame<'_>, area: Rect, events: &[AgentEfficiencyEvent]) {
    let blocks =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
    let top = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(blocks[0]);
    let bottom = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(blocks[1]);
    let series = build_learning_trend_series(events);

    render_learning_sparkline(
        frame,
        top[0],
        "cost / task (7d)",
        &series.cost_per_task,
        Color::Yellow,
    );
    render_learning_sparkline(
        frame,
        top[1],
        "tokens / task (7d)",
        &series.tokens_per_task,
        Color::Cyan,
    );
    render_learning_sparkline(
        frame,
        bottom[0],
        "success rate (7d)",
        &series.success_rate,
        Color::Green,
    );
    render_learning_sparkline(
        frame,
        bottom[1],
        "first-try rate (7d)",
        &series.first_try_rate,
        Color::Magenta,
    );
}

fn render_learning_sparkline(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &'static str,
    data: &[u64],
    color: Color,
) {
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let spark = Sparkline::default()
        .block(Block::default())
        .data(data)
        .style(Style::default().fg(color));
    frame.render_widget(spark, inner);
}

fn prepare_signals(signals: &[SignalSummary]) -> Vec<SignalSummary> {
    let mut rows = signals.to_vec();
    rows.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    rows
}

fn signal_relative_age(created_at_ms: i64) -> String {
    let created_at_ms = u64::try_from(created_at_ms).unwrap_or_default();
    let now = u64::try_from(Utc::now().timestamp_millis()).unwrap_or(u64::MAX);
    format_elapsed_ms(now.saturating_sub(created_at_ms))
}

fn signal_kind_prefix(kind: &str) -> String {
    kind.split(':').next().unwrap_or(kind).to_string()
}

fn signal_kind_distribution(signals: &[SignalSummary]) -> Vec<(String, u64)> {
    let mut counts = BTreeMap::<String, u64>::new();
    for signal in signals {
        *counts.entry(signal_kind_prefix(&signal.kind)).or_default() += 1;
    }

    let mut rows = counts.into_iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows
}

#[derive(Debug, Clone)]
struct SignalTreeEntry<'a> {
    hash: String,
    signal: Option<&'a SignalSummary>,
}

fn signal_parent_chain<'a>(
    signals: &'a [SignalSummary],
    selected: &'a SignalSummary,
) -> Vec<SignalTreeEntry<'a>> {
    let by_id = signals
        .iter()
        .map(|signal| (signal.id.as_str(), signal))
        .collect::<HashMap<_, _>>();

    let mut chain = Vec::new();
    chain.push(SignalTreeEntry {
        hash: selected.id.clone(),
        signal: Some(selected),
    });

    let ancestors: Vec<String> = if selected.lineage.is_empty() {
        selected.parent_hash.iter().cloned().collect()
    } else {
        selected.lineage.iter().rev().cloned().collect()
    };

    for hash in ancestors {
        chain.push(SignalTreeEntry {
            hash: hash.clone(),
            signal: by_id.get(hash.as_str()).copied(),
        });
    }

    chain
}

fn visible_signal_window(selected: usize, len: usize, visible: usize) -> (usize, usize) {
    if len <= visible {
        return (0, len);
    }

    let visible = visible.max(1).min(len);
    let mut start = selected.saturating_sub(visible / 2);
    if start + visible > len {
        start = len - visible;
    }
    (start, start + visible)
}

fn render_signals_table(
    frame: &mut Frame<'_>,
    area: Rect,
    signals: &[SignalSummary],
    selected: usize,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("recent signals");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let visible_rows = inner.height.saturating_sub(2) as usize;
    let (start, end) = visible_signal_window(selected, signals.len(), visible_rows);

    let rows: Vec<Row<'_>> = signals[start..end]
        .iter()
        .enumerate()
        .map(|(offset, signal)| {
            let index = start + offset;
            let is_selected = index == selected;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(signal_relative_age(signal.created_at_ms)),
                Cell::from(truncate_text(&signal.kind, 18)),
                Cell::from(truncate_text(
                    signal
                        .plan_id
                        .as_deref()
                        .or(signal.task_id.as_deref())
                        .unwrap_or("-"),
                    18,
                )),
                Cell::from(truncate_text(&signal.payload_preview, 60)),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Min(20),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("time"),
            Cell::from("kind"),
            Cell::from("plan/task"),
            Cell::from("payload preview"),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1);

    frame.render_widget(table, inner);
}

fn render_signal_kind_chart(frame: &mut Frame<'_>, area: Rect, signals: &[SignalSummary]) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("kind distribution");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = signal_kind_distribution(signals);
    if rows.is_empty() {
        let empty = Paragraph::new("no signal kinds")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let chart_data = rows
        .iter()
        .map(|(kind, count)| (kind.as_str(), *count))
        .collect::<Vec<_>>();
    let chart = BarChart::default()
        .data(&chart_data)
        .direction(Direction::Horizontal)
        .bar_width(1)
        .bar_gap(1)
        .max(rows.iter().map(|(_, count)| *count).max().unwrap_or(1))
        .bar_style(Style::default().fg(Color::Cyan))
        .value_style(Style::default().fg(Color::White))
        .label_style(Style::default().fg(Color::Gray))
        .bar_set(ratatui::symbols::bar::NINE_LEVELS);

    frame.render_widget(chart, inner);
}

fn render_signal_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    signals: &[SignalSummary],
    selected: usize,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("signal DAG explorer");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let selected = selected.min(signals.len().saturating_sub(1));
    let Some(selected_signal) = signals.get(selected) else {
        let empty = Paragraph::new("no selected signal")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    };

    let chain = signal_parent_chain(signals, selected_signal);
    let mut lines = Vec::with_capacity(chain.len());
    for (depth, entry) in chain.iter().enumerate() {
        let indent = "  ".repeat(depth);
        let line = if let Some(signal) = entry.signal {
            format!(
                "{indent}- {} [{}] {}",
                truncate_text(&signal.kind, 24),
                truncate_text(&signal.id, 16),
                signal_relative_age(signal.created_at_ms)
            )
        } else {
            format!("{indent}- {}", truncate_text(&entry.hash, 32))
        };
        lines.push(Line::from(line));
    }

    if lines.is_empty() {
        let empty = Paragraph::new("no parent chain")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((0, 0));
    frame.render_widget(paragraph, inner);
}

fn build_cascade_router_rows(router: &CascadeRouterState) -> Vec<LearningCascadeRow> {
    let mut rows = router
        .model_slugs
        .iter()
        .chain(router.confidence_stats.keys())
        .fold(Vec::<String>::new(), |mut acc, slug| {
            if !acc.iter().any(|seen| seen == slug) {
                acc.push(slug.clone());
            }
            acc
        })
        .into_iter()
        .map(|model| {
            let stats = router.confidence_stats.get(&model);
            let trials = stats.map(|stats| stats.trials).unwrap_or_default();
            let successes = stats.map(|stats| stats.successes).unwrap_or_default();
            let ucb_score = confidence_upper_bound(trials, successes);
            LearningCascadeRow {
                model,
                weight: ucb_score,
                recommendations: 0,
                ucb_score,
            }
        })
        .collect::<Vec<_>>();

    let total_weight = rows
        .iter()
        .map(|row| row.weight)
        .sum::<f64>()
        .max(f64::EPSILON);
    for row in &mut rows {
        row.weight /= total_weight;
    }

    let recommendations = cascade_recommendation_counts(&rows);
    for row in &mut rows {
        row.recommendations = recommendations.get(&row.model).copied().unwrap_or_default();
    }

    rows.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.model.cmp(&b.model))
    });
    rows
}

fn cascade_recommendation_counts(rows: &[LearningCascadeRow]) -> HashMap<String, u64> {
    let mut counts: HashMap<String, u64> = HashMap::new();
    if rows.is_empty() {
        return counts;
    }

    for category in [
        TaskCategory::Scaffolding,
        TaskCategory::Implementation,
        TaskCategory::Integration,
        TaskCategory::Verification,
        TaskCategory::Research,
        TaskCategory::Refactor,
        TaskCategory::Infra,
        TaskCategory::Docs,
    ] {
        let complexity = complexity_for_category(category);
        let tier = tier_for_complexity(complexity);
        let selected = select_model_for_tier(rows, tier)
            .or_else(|| rows.first())
            .map(|row| row.model.clone());
        if let Some(model) = selected {
            *counts.entry(model).or_default() += 1;
        }
    }

    counts
}

fn build_active_experiment_rows(store: &ExperimentStore) -> Vec<LearningExperimentRow> {
    let mut rows = store
        .iter()
        .filter(|experiment| experiment.status == ExperimentStatus::Running)
        .map(|experiment| {
            let mut variants = experiment
                .variants
                .iter()
                .filter(|variant| variant.active)
                .map(|variant| {
                    let stats = experiment
                        .stats
                        .get(&variant.id)
                        .cloned()
                        .unwrap_or_default();
                    (variant, stats)
                })
                .collect::<Vec<_>>();
            variants.sort_by(|(a, _), (b, _)| a.id.cmp(&b.id));

            let sample_sizes = variants
                .iter()
                .map(|(variant, stats)| format!("{}={}", variant.id, stats.trials))
                .collect::<Vec<_>>()
                .join(", ");
            let variant_names = variants
                .iter()
                .map(|(variant, _)| variant.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            let winner = experiment
                .winner_id
                .clone()
                .or_else(|| {
                    variants
                        .iter()
                        .max_by(|(_, a), (_, b)| {
                            a.success_rate()
                                .partial_cmp(&b.success_rate())
                                .unwrap_or(std::cmp::Ordering::Equal)
                                .then_with(|| b.trials.cmp(&a.trials))
                                .then_with(|| a.successes.cmp(&b.successes))
                        })
                        .map(|(variant, _)| variant.id.clone())
                })
                .unwrap_or_else(|| String::from("-"));
            let significance = experiment_significance_label(experiment, &variants);

            LearningExperimentRow {
                experiment: experiment.section_name.clone(),
                variants: if variant_names.is_empty() {
                    format!("{} variants", variants.len())
                } else {
                    format!("{} variants: {}", variants.len(), variant_names)
                },
                sample_sizes: if sample_sizes.is_empty() {
                    String::from("-")
                } else {
                    sample_sizes
                },
                winner,
                significance,
            }
        })
        .collect::<Vec<_>>();

    rows.sort_by(|a, b| a.experiment.cmp(&b.experiment));
    rows
}

fn experiment_significance_label(
    experiment: &roko_learn::prompt_experiment::PromptExperiment,
    variants: &[(
        &roko_learn::prompt_experiment::PromptVariant,
        roko_learn::prompt_experiment::VariantStats,
    )],
) -> String {
    let active = variants
        .iter()
        .map(|(variant, stats)| (variant.id.as_str(), stats))
        .collect::<Vec<_>>();

    if active.len() < 2 {
        return String::from("insufficient");
    }

    let best = active
        .iter()
        .max_by(|(_, a), (_, b)| {
            a.success_rate()
                .partial_cmp(&b.success_rate())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.trials.cmp(&b.trials))
        })
        .copied();
    let runner_up = active
        .iter()
        .filter(|(id, _)| Some(*id) != best.map(|(id, _)| id))
        .max_by(|(_, a), (_, b)| {
            a.success_rate()
                .partial_cmp(&b.success_rate())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.trials.cmp(&b.trials))
        })
        .copied();

    let Some((best_id, best_stats)) = best else {
        return String::from("insufficient");
    };
    let Some((runner_up_id, runner_up_stats)) = runner_up else {
        return format!("winner {best_id}");
    };

    let p_value = two_proportion_p_value(
        best_stats.successes,
        best_stats.trials,
        runner_up_stats.successes,
        runner_up_stats.trials,
    );
    let gap = best_stats.success_rate() - runner_up_stats.success_rate();
    let significant = p_value
        .map(|p| p < 0.05 && gap >= experiment.min_effect_size)
        .unwrap_or(false);

    match p_value {
        Some(p) if significant => format!("sig p={:.3}", p),
        Some(p) => format!("p={:.3}", p),
        None => format!("n.s. {best_id}/{runner_up_id}"),
    }
}

fn build_learning_trend_series(events: &[AgentEfficiencyEvent]) -> LearningTrendSeries {
    let today = Utc::now().date_naive();
    let mut tasks: HashMap<(String, String), LearningTaskAggregate> = HashMap::new();

    for event in events {
        let key = (event.plan_id.clone(), event.task_id.clone());
        tasks
            .entry(key)
            .or_insert_with(LearningTaskAggregate::default)
            .record(event);
    }

    let mut buckets: BTreeMap<i64, LearningDayAggregate> = BTreeMap::new();
    for aggregate in tasks.values() {
        let Some(day) = aggregate.latest_day() else {
            continue;
        };
        let age = today.signed_duration_since(day).num_days();
        if !(0..7).contains(&age) {
            continue;
        }
        let bucket = buckets.entry(age).or_default();
        bucket.tasks += 1;
        bucket.cost_usd += aggregate.cost_usd;
        bucket.tokens += aggregate.tokens;
        if aggregate.latest_passed {
            bucket.successes += 1;
        }
        if aggregate.first_try_passed() {
            bucket.first_try_successes += 1;
        }
    }

    let series = |extract: fn(&LearningDayAggregate) -> f64| -> Vec<u64> {
        (0..7)
            .rev()
            .map(|age| {
                let bucket = buckets.get(&age).cloned().unwrap_or_default();
                if bucket.tasks == 0 {
                    return 0;
                }
                (extract(&bucket) * 100.0).round().max(0.0) as u64
            })
            .collect::<Vec<_>>()
    };

    LearningTrendSeries {
        cost_per_task: (0..7)
            .rev()
            .map(|age| {
                let bucket = buckets.get(&age).cloned().unwrap_or_default();
                if bucket.tasks == 0 {
                    0
                } else {
                    ((bucket.cost_usd / bucket.tasks as f64) * 100.0)
                        .round()
                        .max(0.0) as u64
                }
            })
            .collect(),
        tokens_per_task: (0..7)
            .rev()
            .map(|age| {
                let bucket = buckets.get(&age).cloned().unwrap_or_default();
                if bucket.tasks == 0 {
                    0
                } else {
                    (bucket.tokens / bucket.tasks).max(0)
                }
            })
            .collect(),
        success_rate: series(|bucket| bucket.successes as f64 / bucket.tasks as f64),
        first_try_rate: series(|bucket| bucket.first_try_successes as f64 / bucket.tasks as f64),
    }
}

fn confidence_upper_bound(trials: u64, successes: u64) -> f64 {
    if trials == 0 {
        return 1.0;
    }

    let p = successes as f64 / trials as f64;
    let width = 1.96 * (p * (1.0 - p) / trials as f64).sqrt();
    (p + width).min(1.0)
}

fn select_model_for_tier<'a>(
    rows: &'a [LearningCascadeRow],
    tier: &str,
) -> Option<&'a LearningCascadeRow> {
    rows.iter()
        .filter(|row| tier_for_model(&row.model) == tier)
        .max_by(|a, b| {
            a.weight
                .partial_cmp(&b.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.model.cmp(&b.model))
        })
}

fn complexity_for_category(category: TaskCategory) -> TaskComplexityBand {
    match category {
        TaskCategory::Scaffolding | TaskCategory::Docs => TaskComplexityBand::Fast,
        TaskCategory::Research | TaskCategory::Refactor => TaskComplexityBand::Complex,
        TaskCategory::Implementation
        | TaskCategory::Integration
        | TaskCategory::Verification
        | TaskCategory::Infra
        | _ => TaskComplexityBand::Standard,
    }
}

fn tier_for_complexity(complexity: TaskComplexityBand) -> &'static str {
    match complexity {
        TaskComplexityBand::Fast => "fast",
        TaskComplexityBand::Complex => "premium",
        _ => "standard",
    }
}

fn tier_for_model(model: &str) -> &'static str {
    let lower = model.to_ascii_lowercase();
    if lower.contains("haiku") {
        "fast"
    } else if lower.contains("opus") || lower.contains("premium") {
        "premium"
    } else {
        "standard"
    }
}

fn significance_style(significance: &str) -> Style {
    if significance.starts_with("sig") {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else if significance.starts_with('p') {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn format_float(value: f64) -> String {
    format!("{value:.2}")
}

fn parse_efficiency_timestamp(timestamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn two_proportion_p_value(
    successes_a: u64,
    trials_a: u64,
    successes_b: u64,
    trials_b: u64,
) -> Option<f64> {
    let z = two_proportion_z_score(successes_a, trials_a, successes_b, trials_b)?;
    Some(2.0 * (1.0 - standard_normal_cdf(z.abs())))
}

fn two_proportion_z_score(
    successes_a: u64,
    trials_a: u64,
    successes_b: u64,
    trials_b: u64,
) -> Option<f64> {
    if trials_a == 0 || trials_b == 0 {
        return None;
    }

    let p1 = successes_a as f64 / trials_a as f64;
    let p2 = successes_b as f64 / trials_b as f64;
    let pooled = (successes_a + successes_b) as f64 / (trials_a + trials_b) as f64;
    let standard_error =
        (pooled * (1.0 - pooled) * (1.0 / trials_a as f64 + 1.0 / trials_b as f64)).sqrt();
    if standard_error == 0.0 {
        return None;
    }

    Some((p1 - p2) / standard_error)
}

fn standard_normal_cdf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.231_641_9 * x.abs());
    let d = 0.398_942_3 * (-0.5 * x * x).exp();
    let prob = d
        * t
        * (0.319_381_5 + t * (-0.356_563_8 + t * (1.781_478 + t * (-1.821_256 + t * 1.330_274))));
    if x >= 0.0 { 1.0 - prob } else { prob }
}

#[derive(Debug, Clone)]
struct LearningCascadeRow {
    model: String,
    weight: f64,
    recommendations: u64,
    ucb_score: f64,
}

#[derive(Debug, Clone)]
struct LearningExperimentRow {
    experiment: String,
    variants: String,
    sample_sizes: String,
    winner: String,
    significance: String,
}

#[derive(Debug, Clone, Default)]
struct LearningTrendSeries {
    cost_per_task: Vec<u64>,
    tokens_per_task: Vec<u64>,
    success_rate: Vec<u64>,
    first_try_rate: Vec<u64>,
}

#[derive(Debug, Clone, Default)]
struct LearningTaskAggregate {
    cost_usd: f64,
    tokens: u64,
    first_timestamp: Option<DateTime<Utc>>,
    first_iteration: u32,
    first_passed: bool,
    latest_timestamp: Option<DateTime<Utc>>,
    latest_passed: bool,
}

impl LearningTaskAggregate {
    fn record(&mut self, event: &AgentEfficiencyEvent) {
        self.cost_usd += event.cost_usd;
        self.tokens += event.total_tokens();

        let Some(timestamp) = parse_efficiency_timestamp(&event.timestamp) else {
            return;
        };

        if self.first_timestamp.map_or(true, |first| timestamp < first) {
            self.first_timestamp = Some(timestamp);
            self.first_iteration = event.iteration;
            self.first_passed = event.gate_passed;
        }

        if self
            .latest_timestamp
            .map_or(true, |latest| timestamp > latest)
        {
            self.latest_timestamp = Some(timestamp);
            self.latest_passed = event.gate_passed;
        }
    }

    fn latest_day(&self) -> Option<chrono::NaiveDate> {
        self.latest_timestamp
            .map(|timestamp| timestamp.date_naive())
    }

    fn first_try_passed(&self) -> bool {
        self.first_iteration == 1 && self.first_passed
    }
}

#[derive(Debug, Clone, Default)]
struct LearningDayAggregate {
    tasks: u64,
    cost_usd: f64,
    tokens: u64,
    successes: u64,
    first_try_successes: u64,
}

fn render_gate_summary_table(frame: &mut Frame<'_>, area: Rect, rows: &[GateSummaryRow]) {
    let block = Block::default().borders(Borders::ALL).title("gate summary");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if rows.is_empty() {
        let empty = Paragraph::new("no gate results")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let table_rows: Vec<Row<'_>> = rows
        .iter()
        .map(|row| {
            Row::new(vec![
                Cell::from(truncate_text(&row.gate_name, 18)),
                Cell::from(row.total_runs.to_string()),
                Cell::from(Span::styled(
                    format_pct(row.pass_rate),
                    gate_pass_rate_style(row.pass_rate),
                )),
                Cell::from(format!("{:.0} ms", row.avg_duration_ms.round())),
                Cell::from(truncate_text(&row.last_run, 16)),
            ])
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Min(12),
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Min(12),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("gate name"),
            Cell::from("runs"),
            Cell::from("pass rate"),
            Cell::from("avg duration"),
            Cell::from("last run"),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1);

    frame.render_widget(table, inner);
}

fn render_gate_thresholds_table(frame: &mut Frame<'_>, area: Rect, rows: &[GateThresholdRow]) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("adaptive thresholds");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if rows.is_empty() {
        let empty = Paragraph::new("no threshold data")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let table_rows: Vec<Row<'_>> = rows
        .iter()
        .map(|row| {
            let (arrow, color) = match row.trend {
                GateTrend::Up => ("↑", Color::Green),
                GateTrend::Flat => ("→", Color::Yellow),
                GateTrend::Down => ("↓", Color::Red),
            };
            Row::new(vec![
                Cell::from(row.rung.to_string()),
                Cell::from(row.current_threshold.to_string()),
                Cell::from(Span::styled(
                    format_pct(row.ema_pass_rate),
                    Style::default().fg(Color::Cyan),
                )),
                Cell::from(Span::styled(
                    arrow,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
            ])
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(6),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(6),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("rung"),
            Cell::from("threshold"),
            Cell::from("EMA"),
            Cell::from("trend"),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1);

    frame.render_widget(table, inner);
}

fn render_gate_failures_list(
    frame: &mut Frame<'_>,
    area: Rect,
    rows: &[GateFailureRow],
    selected: usize,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("recent failures");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if rows.is_empty() {
        let empty = Paragraph::new("no gate failures")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let max_excerpt = inner.width.saturating_sub(28) as usize;
    let selected = selected.min(rows.len().saturating_sub(1));
    let items: Vec<ListItem<'_>> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let is_selected = index == selected;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    truncate_text(&row.task_id, 10),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(" "),
                Span::styled(
                    truncate_text(&row.gate_name, 12),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(" "),
                Span::raw(truncate_text(&row.error_excerpt, max_excerpt)),
            ]))
            .style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_active_agents_table(
    frame: &mut Frame<'_>,
    area: Rect,
    rows: &[super::dashboard::AgentActivityRow],
    data: &DashboardData,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("active agents");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if rows.is_empty() {
        let empty = Paragraph::new("no active agents")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let table_rows: Vec<Row<'_>> = rows
        .iter()
        .map(|row| {
            let affect = row
                .plan_id
                .as_deref()
                .map(|plan_id| data.affect_indicator(plan_id))
                .unwrap_or_else(|| data.affect_indicator(&row.agent_id));
            Row::new(vec![
                Cell::from(affect),
                Cell::from(truncate_text(&row.agent_id, 20)),
                Cell::from(truncate_text(&row.model, 14)),
                Cell::from(truncate_text(&row.task, 16)),
                Cell::from(truncate_text(&row.role, 12)),
                Cell::from(row.turns.to_string()),
                Cell::from(row.tokens_used.to_string()),
                Cell::from(format!("${:.4}", row.cost_usd)),
                Cell::from(format_elapsed_ms(row.uptime_ms)),
            ])
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(3),
            Constraint::Length(20),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Length(12),
            Constraint::Length(5),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("aff"),
            Cell::from("agent ID"),
            Cell::from("model"),
            Cell::from("task"),
            Cell::from("role"),
            Cell::from("turns"),
            Cell::from("tokens used"),
            Cell::from("cost"),
            Cell::from("uptime"),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1);

    frame.render_widget(table, inner);
}

fn render_model_distribution_chart(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &AgentActivitySnapshot,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("model distribution");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let data = [
        ("haiku", snapshot.model_usage[0].count),
        ("sonnet", snapshot.model_usage[1].count),
        ("opus", snapshot.model_usage[2].count),
    ];
    let chart = BarChart::default()
        .data(&data)
        .direction(Direction::Horizontal)
        .bar_width(1)
        .bar_gap(1)
        .max(
            snapshot
                .model_usage
                .iter()
                .map(|row| row.count)
                .max()
                .unwrap_or(1),
        )
        .bar_style(Style::default().fg(Color::Cyan))
        .value_style(Style::default().fg(Color::White))
        .label_style(Style::default().fg(Color::Gray))
        .bar_set(ratatui::symbols::bar::NINE_LEVELS);

    frame.render_widget(chart, inner);
}

fn render_model_cost_breakdown(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &AgentActivitySnapshot,
) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title("cost breakdown");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let sections = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);

    if snapshot.cost_rows.is_empty() {
        let empty = Paragraph::new("no cost history")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, sections[0]);
        return;
    }

    let rows: Vec<Row<'_>> = snapshot
        .cost_rows
        .iter()
        .map(|row| {
            Row::new(vec![
                Cell::from(truncate_text(&row.model, 24)),
                Cell::from(row.input_tokens.to_string()),
                Cell::from(row.output_tokens.to_string()),
                Cell::from(format!("${:.4}", row.cost_usd)),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(18),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("model"),
            Cell::from("input tokens"),
            Cell::from("output tokens"),
            Cell::from("cost"),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1);
    frame.render_widget(table, sections[0]);

    let footer = Paragraph::new(format!(
        "total session cost: ${:.4}",
        snapshot.total_session_cost
    ))
    .alignment(Alignment::Right)
    .style(Style::default().fg(Color::Yellow));
    frame.render_widget(footer, sections[1]);
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

fn render_plan_execution_page(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    scroll: u16,
) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title("Plan Execution");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let Some(execution) = data.current_plan_execution.as_ref() else {
        let empty = Paragraph::new("no active plan")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    };

    let top_height = 3.min(inner.height);
    let rest = inner.height.saturating_sub(top_height);
    let top = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: top_height,
    };
    let body = Rect {
        x: inner.x,
        y: inner.y.saturating_add(top_height),
        width: inner.width,
        height: rest,
    };

    let title_label = format!(
        "{}  {}  [{}/{}]",
        execution.plan_title,
        data.affect_indicator(&execution.plan_id),
        execution.tasks_done,
        execution.tasks_total
    );
    let progress = if execution.tasks_total == 0 {
        0.0
    } else {
        (execution.tasks_done as f64 / execution.tasks_total as f64).clamp(0.0, 1.0)
    };
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(title_label))
        .ratio(progress)
        .label(Span::styled(
            format!("{}/{}", execution.tasks_done, execution.tasks_total),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black));
    frame.render_widget(gauge, top);

    let split =
        Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)]).split(body);
    let left = split[0];
    let right = split[1];

    let left_parts = Layout::vertical([
        Constraint::Min(8),
        Constraint::Length(8.max(body.height / 3)),
    ])
    .split(left);

    render_plan_execution_table(frame, left_parts[0], execution);
    render_plan_execution_output(frame, left_parts[1], execution, scroll);
    render_plan_execution_sidebar(frame, right, execution, data);
}

fn render_plan_execution_table(
    frame: &mut Frame<'_>,
    area: Rect,
    execution: &PlanExecutionSnapshot,
) {
    let block = Block::default().borders(Borders::ALL).title("tasks");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows: Vec<Row<'_>> = execution
        .tasks
        .iter()
        .map(|task| {
            let phase_style = phase_style(&task.phase);
            let mut row_style = Style::default();
            if task.is_current {
                row_style = row_style.add_modifier(Modifier::BOLD);
            }
            Row::new(vec![
                Cell::from(task.task_id.clone()),
                Cell::from(truncate_text(&task.title, 40)),
                Cell::from(Span::styled(task.phase.clone(), phase_style)),
                Cell::from(operating_frequency_label(task.frequency)),
                Cell::from(task.model.clone()),
                Cell::from(task.duration.clone()),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Min(24),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Length(18),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("task id"),
            Cell::from("title"),
            Cell::from("phase"),
            Cell::from("frequency"),
            Cell::from("model"),
            Cell::from("duration"),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1);

    frame.render_widget(table, inner);
}

fn render_plan_execution_output(
    frame: &mut Frame<'_>,
    area: Rect,
    execution: &PlanExecutionSnapshot,
    scroll: u16,
) {
    let block = Block::default().borders(Borders::ALL).title("agent stderr");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = if execution.agent_output_tail.is_empty() {
        String::from("<no agent stderr captured>")
    } else {
        execution.agent_output_tail.join("\n")
    };

    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, inner);
}

fn render_plan_execution_sidebar(
    frame: &mut Frame<'_>,
    area: Rect,
    execution: &PlanExecutionSnapshot,
    data: &DashboardData,
) {
    let block = Block::default().borders(Borders::ALL).title("current task");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(task) = execution.current_task.as_ref() else {
        let empty = Paragraph::new("no current task")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    };

    let gate_rows = data
        .gate_results
        .iter()
        .filter(|gate| gate.plan_id == execution.plan_id)
        .collect::<Vec<_>>();

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            "task",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": "),
        Span::raw(&task.task_id),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "plan affect",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": "),
        Span::raw(data.affect_indicator(&execution.plan_id)),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "frequency",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": "),
        Span::raw(operating_frequency_label(task.frequency)),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "description",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(": "),
        Span::raw(truncate_text(&task.description, 200)),
    ]));
    lines.push(Line::from(" "));
    lines.push(Line::from(Span::styled(
        "read_files",
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    )));
    if task.read_files.is_empty() {
        lines.push(Line::from("  <none>"));
    } else {
        for file in &task.read_files {
            let mut text = format!("  - {}", file.path);
            if let Some(lines_range) = file.lines.as_deref() {
                text.push_str(&format!(" ({lines_range})"));
            }
            if !file.why.trim().is_empty() {
                text.push_str(&format!(" — {}", file.why));
            }
            lines.push(Line::from(text));
        }
    }
    lines.push(Line::from(" "));
    lines.push(Line::from(Span::styled(
        "write_files",
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    )));
    if task.write_files.is_empty() {
        lines.push(Line::from("  <none>"));
    } else {
        for file in &task.write_files {
            lines.push(Line::from(format!("  - {file}")));
        }
    }
    lines.push(Line::from(" "));
    lines.push(Line::from(Span::styled(
        "gate_results",
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    )));
    if gate_rows.is_empty() {
        lines.push(Line::from("  <none>"));
    } else {
        for gate in gate_rows {
            lines.push(Line::from(vec![
                Span::raw("  - "),
                Span::styled(
                    gate.gate_name.clone(),
                    phase_style(if gate.passed { "done" } else { "failed" }),
                ),
                Span::raw(" ["),
                Span::raw(if gate.passed { "pass" } else { "fail" }),
                Span::raw("] "),
                Span::raw(format!("{}ms", gate.duration_ms)),
                if gate.summary.trim().is_empty() {
                    Span::raw("")
                } else {
                    Span::raw(format!(" — {}", truncate_text(&gate.summary, 120)))
                },
            ]));
        }
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn render_plan_overview(frame: &mut Frame<'_>, area: Rect, data: &DashboardData) {
    let block = Block::default().borders(Borders::ALL).title("Plans");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 8 || inner.height < 3 {
        return;
    }

    let gauge_width = inner
        .width
        .saturating_div(3)
        .clamp(12, 18)
        .min(inner.width.saturating_sub(1));
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
        Constraint::Length(5),
        Constraint::Min(0),
    ])
    .split(inner);

    let gate_series = gate_pass_rate_series(data.root());
    let cost_series = cost_trend_series(data);
    let c_factor_series = cfactor_series(data);
    let current_cfactor = data
        .cfactor
        .as_ref()
        .map(|snapshot| snapshot.overall)
        .unwrap_or(0.0);
    let trend = cfactor_trend(data);

    let gate = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("gate pass rate (7d)"),
        )
        .data(&gate_series)
        .style(Style::default().fg(Color::Green));
    frame.render_widget(gate, panels[0]);

    let cost = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title("cost trend"))
        .data(&cost_series)
        .style(Style::default().fg(Color::Yellow));
    frame.render_widget(cost, panels[1]);

    let cfactor = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("C-Factor 7d: {:.2} {}", current_cfactor, trend)),
        )
        .data(&c_factor_series)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(cfactor, panels[2]);

    render_cfactor_breakdown(frame, panels[3], data, trend);
}

fn render_alerts(frame: &mut Frame<'_>, area: Rect, data: &DashboardData) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Conductor Alerts");
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

fn render_cfactor_breakdown(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    trend: &str,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("C-Factor breakdown");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(snapshot) = data.cfactor.as_ref() else {
        let empty = Paragraph::new("no C-Factor snapshot")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                "current",
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(": "),
            Span::styled(
                format!("{:.2} {}", snapshot.overall, trend),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(format!(
            "gate pass rate: {}",
            format_pct(snapshot.components.gate_pass_rate)
        )),
        Line::from(format!(
            "cost efficiency: {}",
            format_pct(snapshot.components.cost_efficiency)
        )),
        Line::from(format!("speed: {}", format_pct(snapshot.components.speed))),
        Line::from(format!(
            "first-try rate: {}",
            format_pct(snapshot.components.first_try_rate)
        )),
        Line::from(format!(
            "knowledge growth: {}",
            format_pct(snapshot.components.knowledge_growth)
        )),
    ];

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
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

    rows.sort_by(|a, b| {
        b.completed
            .cmp(&a.completed)
            .then_with(|| a.name.cmp(&b.name))
    });
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
        let plan_id = entry
            .get("plan_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
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
    let today = Utc::now().date_naive();
    let mut buckets: BTreeMap<i64, (f64, u64)> = BTreeMap::new();

    for snapshot in &data.cfactor_history {
        let day = snapshot.computed_at.date_naive();
        let age = today.signed_duration_since(day).num_days();
        if !(0..7).contains(&age) {
            continue;
        }
        let bucket = buckets.entry(age).or_default();
        bucket.0 += snapshot.overall;
        bucket.1 += 1;
    }

    (0..7)
        .rev()
        .map(|age| {
            let (sum, count) = buckets.get(&age).copied().unwrap_or_default();
            if count == 0 {
                0
            } else {
                ((sum / count as f64) * 100.0).round().max(0.0) as u64
            }
        })
        .collect()
}

fn cfactor_trend(data: &DashboardData) -> &'static str {
    cfactor_trend_arrow(&data.cfactor_history, std::time::Duration::from_secs(7 * 24 * 60 * 60))
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
    let theme = Theme::from_env();
    if completed {
        return theme.success();
    }

    match status.to_ascii_lowercase().as_str() {
        "done" | "complete" | "completed" => theme.success(),
        "failed" | "error" => theme.danger(),
        "gating" => theme.warning(),
        "implementing" | "running" | "active" => theme.info(),
        "queued" | "pending" => theme.muted(),
        _ => theme.text(),
    }
}

fn gauge_style(status: &str, completed: bool) -> Style {
    let theme = Theme::from_env();
    if completed {
        return Style::default().fg(theme.success).bg(theme.background);
    }

    match status.to_ascii_lowercase().as_str() {
        "failed" | "error" => Style::default().fg(theme.danger).bg(theme.background),
        "gating" => Style::default().fg(theme.warning).bg(theme.background),
        "implementing" | "running" | "active" => {
            Style::default().fg(theme.info).bg(theme.background)
        }
        "queued" | "pending" => Style::default().fg(theme.muted).bg(theme.background),
        _ => Style::default().fg(theme.foreground).bg(theme.background),
    }
}

fn phase_style(phase: &str) -> Style {
    let theme = Theme::from_env();
    match phase.to_ascii_lowercase().as_str() {
        "implementing" => theme.info(),
        "gating" => theme.warning(),
        "done" => theme.success(),
        "failed" => theme.danger(),
        _ => theme.muted(),
    }
}

fn severity_style(severity: &str) -> Style {
    let theme = Theme::from_env();
    match severity.to_ascii_lowercase().as_str() {
        "critical" => theme.danger(),
        "warning" => theme.warning(),
        _ => theme.muted(),
    }
}

fn gate_pass_rate_style(pass_rate: f64) -> Style {
    let theme = Theme::from_env();
    if pass_rate > 0.9 {
        theme.success()
    } else if pass_rate >= 0.7 {
        theme.warning()
    } else {
        theme.danger()
    }
}

fn format_pct(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
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

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out = String::new();
    for _ in 0..max_chars {
        if let Some(ch) = chars.next() {
            out.push(ch);
        } else {
            return out;
        }
    }
    if chars.next().is_some() && max_chars > 3 {
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
pub fn render_footer(frame: &mut Frame<'_>, area: Rect, pages: &PageRegistry, active_page: PageId) {
    render_footer_themed(frame, area, pages, active_page, &Theme::from_env());
}

fn render_footer_themed(
    frame: &mut Frame<'_>,
    area: Rect,
    pages: &PageRegistry,
    active_page: PageId,
    theme: &Theme,
) {
    let page_count = pages.len();
    let footer = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("q", theme.warning()),
            Span::raw(" quit  "),
            Span::styled("r", theme.warning()),
            Span::raw(" refresh  "),
            Span::styled("←/→", theme.warning()),
            Span::raw(" page  "),
            Span::styled("↑/↓", theme.warning()),
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
