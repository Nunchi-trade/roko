//! F7 Inspect / Context view -- token burn, cost breakdown, routing, health.
//!
//! Four-section layout:
//! - Top 20%: system health summary + C-Factor
//! - Mid-left 40%: token burn per role from efficiency events
//! - Mid-right 40%: cost breakdown per model
//! - Bottom 40%: cascade router decisions + conductor alerts

use std::collections::{BTreeMap, HashMap};

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap};

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;

/// Token burn data for sparkline rendering.
#[derive(Debug, Clone)]
struct TokenBurnData {
    /// Agent identifier for the burn series.
    pub agent_id: String,
    /// Cumulative token count over time.
    pub cumulative: Vec<u64>,
}

/// Context view data container, populated externally.
#[derive(Debug, Clone, Default)]
struct ContextViewData {
    /// Per-agent token burn series used by the sparkline panel.
    pub token_burns: Vec<TokenBurnData>,
}

/// Per-role token/cost aggregate.
#[derive(Debug, Clone, Default)]
struct RoleAggregate {
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
    turns: u64,
    cache_read_tokens: u64,
}

/// Per-model cost aggregate.
#[derive(Debug, Clone, Default)]
struct ModelCostAggregate {
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
    turns: u64,
    wall_time_ms: u64,
}

/// Render the full context/inspect view.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    _tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let ctx_data = build_context_data(data);
    render_with_context_data(frame, area, data, &ctx_data, view_state, theme);
}

/// Render the context view with explicit context data (for integration layer).
fn render_with_context_data(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    ctx_data: &ContextViewData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections = Layout::vertical([
        Constraint::Percentage(20), // Health summary
        Constraint::Percentage(40), // Token burn + cost breakdown side by side
        Constraint::Percentage(40), // Cascade router + alerts
    ])
    .split(area);

    render_health_summary(frame, sections[0], data, theme);

    let mid_panels = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(sections[1]);
    render_token_burn_by_role(frame, mid_panels[0], data, view_state, theme);
    render_cost_by_model(frame, mid_panels[1], data, theme);

    let bottom_panels =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(sections[2]);
    render_cascade_router(frame, bottom_panels[0], data, ctx_data, view_state, theme);
    render_alerts_and_health(frame, bottom_panels[1], data, theme);
}

/// Top section: system health summary with C-Factor and key metrics.
fn render_health_summary(frame: &mut Frame<'_>, area: Rect, data: &DashboardData, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" System Health ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols = Layout::horizontal([
        Constraint::Percentage(33),
        Constraint::Percentage(34),
        Constraint::Percentage(33),
    ])
    .split(inner);

    // Left column: token/cost summary
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
            Span::styled(format!("${:.4}", eff.total_cost_usd), theme.warning()),
        ]),
        Line::from(vec![
            Span::styled("avg wall time: ", theme.muted()),
            Span::styled(format!("{:.0}ms", eff.average_wall_time_ms), theme.info()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(token_lines).wrap(Wrap { trim: false }),
        cols[0],
    );

    // Middle column: pass rate and event counts
    let pass_rate = if eff.event_count > 0 {
        eff.passed_count as f64 / eff.event_count as f64 * 100.0
    } else {
        0.0
    };
    let pass_style = if pass_rate >= 80.0 {
        theme.success()
    } else if pass_rate >= 50.0 {
        theme.warning()
    } else {
        theme.danger()
    };

    let rate_lines = vec![
        Line::from(vec![
            Span::styled("pass rate:     ", theme.muted()),
            Span::styled(format!("{pass_rate:.1}%"), pass_style),
        ]),
        Line::from(vec![
            Span::styled("events:        ", theme.muted()),
            Span::raw(eff.event_count.to_string()),
        ]),
        Line::from(vec![
            Span::styled("agents:        ", theme.muted()),
            Span::raw(data.agents.len().to_string()),
        ]),
        Line::from(vec![
            Span::styled("plans:         ", theme.muted()),
            Span::raw(data.plans.len().to_string()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(rate_lines).wrap(Wrap { trim: false }),
        cols[1],
    );

    // Right column: C-Factor or cascade router summary
    let right_lines = if let Some(ref cf) = data.cfactor {
        let cf_style = if cf.overall >= 0.7 {
            theme.success()
        } else if cf.overall >= 0.4 {
            theme.warning()
        } else {
            theme.danger()
        };
        vec![
            Line::from(vec![
                Span::styled("C-Factor:      ", theme.muted()),
                Span::styled(format!("{:.3}", cf.overall), cf_style),
            ]),
            Line::from(vec![
                Span::styled("  gate pass:   ", theme.muted()),
                Span::raw(format!("{:.2}", cf.components.gate_pass_rate)),
            ]),
            Line::from(vec![
                Span::styled("  cost eff:    ", theme.muted()),
                Span::raw(format!("{:.2}", cf.components.cost_efficiency)),
            ]),
            Line::from(vec![
                Span::styled("  first try:   ", theme.muted()),
                Span::raw(format!("{:.2}", cf.components.first_try_rate)),
            ]),
        ]
    } else {
        let router_models = data.cascade_router.model_slugs.len();
        let total_trials: u64 = data
            .cascade_router
            .confidence_stats
            .values()
            .map(|s| s.trials)
            .sum();
        vec![
            Line::from(vec![
                Span::styled("C-Factor:      ", theme.muted()),
                Span::styled("(not computed)", theme.muted()),
            ]),
            Line::from(vec![
                Span::styled("router models: ", theme.muted()),
                Span::raw(router_models.to_string()),
            ]),
            Line::from(vec![
                Span::styled("router trials: ", theme.muted()),
                Span::raw(total_trials.to_string()),
            ]),
            Line::from(vec![
                Span::styled("gate types:    ", theme.muted()),
                Span::raw(data.gate_results_page.gate_rows.len().to_string()),
            ]),
        ]
    };
    frame.render_widget(
        Paragraph::new(right_lines).wrap(Wrap { trim: false }),
        cols[2],
    );
}

/// Token burn per role from efficiency events.
fn render_token_burn_by_role(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    _view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Token Burn by Role ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if data.efficiency_events.is_empty() {
        let empty = Paragraph::new("no efficiency data -- run agents to see token burn")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    // Aggregate by role
    let mut role_agg: BTreeMap<String, RoleAggregate> = BTreeMap::new();
    for event in &data.efficiency_events {
        let role = if event.role.is_empty() {
            "unknown"
        } else {
            event.role.as_str()
        };
        let entry = role_agg.entry(role.to_string()).or_default();
        entry.input_tokens += event.input_tokens;
        entry.output_tokens += event.output_tokens;
        entry.cost_usd += event.cost_usd;
        entry.turns += 1;
        entry.cache_read_tokens += event.cache_read_tokens;
    }

    let rows: Vec<Row<'_>> = role_agg
        .iter()
        .map(|(role, agg)| {
            let total_tokens = agg.input_tokens + agg.output_tokens;
            let cache_pct = if agg.input_tokens > 0 {
                format!(
                    "{:.0}%",
                    agg.cache_read_tokens as f64 / agg.input_tokens as f64 * 100.0
                )
            } else {
                "-".to_string()
            };
            Row::new(vec![
                Cell::from(truncate(role, 14)),
                Cell::from(format_count(total_tokens)),
                Cell::from(format!("${:.3}", agg.cost_usd)),
                Cell::from(agg.turns.to_string()),
                Cell::from(cache_pct),
            ])
        })
        .collect();

    // Total row
    let total_tokens: u64 = role_agg
        .values()
        .map(|a| a.input_tokens + a.output_tokens)
        .sum();
    let total_cost: f64 = role_agg.values().map(|a| a.cost_usd).sum();
    let total_turns: u64 = role_agg.values().map(|a| a.turns).sum();
    let total_cache: u64 = role_agg.values().map(|a| a.cache_read_tokens).sum();
    let total_input: u64 = role_agg.values().map(|a| a.input_tokens).sum();
    let total_cache_pct = if total_input > 0 {
        format!("{:.0}%", total_cache as f64 / total_input as f64 * 100.0)
    } else {
        "-".to_string()
    };

    let mut all_rows = rows;
    all_rows.push(
        Row::new(vec![
            Cell::from(Span::styled("TOTAL", theme.accent_bold())),
            Cell::from(Span::styled(format_count(total_tokens), theme.accent())),
            Cell::from(Span::styled(format!("${:.3}", total_cost), theme.warning())),
            Cell::from(Span::styled(total_turns.to_string(), theme.accent())),
            Cell::from(total_cache_pct),
        ])
        .style(theme.accent()),
    );

    let widths = [
        Constraint::Min(10),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(6),
    ];
    let table = Table::new(all_rows, widths)
        .header(
            Row::new(["role", "tokens", "cost", "turns", "cache"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// Cost breakdown per model from efficiency events.
fn render_cost_by_model(frame: &mut Frame<'_>, area: Rect, data: &DashboardData, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Cost by Model ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if data.efficiency_events.is_empty() {
        let empty = Paragraph::new("no cost data")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    // Aggregate by model
    let mut model_agg: BTreeMap<String, ModelCostAggregate> = BTreeMap::new();
    for event in &data.efficiency_events {
        let model = if event.model.is_empty() {
            "unknown"
        } else {
            event.model.as_str()
        };
        let entry = model_agg.entry(model.to_string()).or_default();
        entry.input_tokens += event.input_tokens;
        entry.output_tokens += event.output_tokens;
        entry.cost_usd += event.cost_usd;
        entry.turns += 1;
        entry.wall_time_ms += event.wall_time_ms;
    }

    // Sort by cost descending
    let mut sorted: Vec<(&String, &ModelCostAggregate)> = model_agg.iter().collect();
    sorted.sort_by(|a, b| {
        b.1.cost_usd
            .partial_cmp(&a.1.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let rows: Vec<Row<'_>> = sorted
        .iter()
        .map(|(model, agg)| {
            let avg_time = if agg.turns > 0 {
                format!("{:.0}ms", agg.wall_time_ms as f64 / agg.turns as f64)
            } else {
                "-".to_string()
            };
            let cost_style = if agg.cost_usd > 1.0 {
                theme.danger()
            } else if agg.cost_usd > 0.1 {
                theme.warning()
            } else {
                theme.text()
            };
            Row::new(vec![
                Cell::from(truncate(model, 20)),
                Cell::from(Span::styled(format!("${:.4}", agg.cost_usd), cost_style)),
                Cell::from(format_count(agg.input_tokens)),
                Cell::from(format_count(agg.output_tokens)),
                Cell::from(avg_time),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(12),
        Constraint::Length(9),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["model", "cost", "in", "out", "avg"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// Cascade router decisions and model routing info.
fn render_cascade_router(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    ctx_data: &ContextViewData,
    _view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Cascade Router ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::vertical([Constraint::Min(0), Constraint::Length(4)]).split(inner);

    // Router model stats
    if data.cascade_router.model_slugs.is_empty() && ctx_data.token_burns.is_empty() {
        let empty = Paragraph::new("no routing decisions -- run agents to populate")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, sections[0]);
    } else {
        let rows: Vec<Row<'_>> = data
            .cascade_router
            .model_slugs
            .iter()
            .map(|slug| {
                let stats = data.cascade_router.confidence_stats.get(slug);
                let trials = stats.map_or(0, |s| s.trials);
                let successes = stats.map_or(0, |s| s.successes);
                let rate = if trials > 0 {
                    successes as f64 / trials as f64 * 100.0
                } else {
                    0.0
                };
                let rate_style = if rate >= 80.0 {
                    theme.success()
                } else if rate >= 50.0 {
                    theme.warning()
                } else if trials > 0 {
                    theme.danger()
                } else {
                    theme.muted()
                };
                Row::new(vec![
                    Cell::from(truncate(slug, 20)),
                    Cell::from(trials.to_string()),
                    Cell::from(successes.to_string()),
                    Cell::from(Span::styled(format!("{rate:.0}%"), rate_style)),
                ])
            })
            .collect();

        if rows.is_empty() {
            // Show sparklines from token burns instead
            let burn_lines: Vec<Line<'_>> = ctx_data
                .token_burns
                .iter()
                .take(sections[0].height as usize)
                .map(|burn| {
                    let total: u64 = burn.cumulative.last().copied().unwrap_or(0);
                    Line::from(vec![
                        Span::styled(
                            format!("{:>16}", truncate(&burn.agent_id, 16)),
                            theme.muted(),
                        ),
                        Span::raw(" "),
                        Span::styled(format_count(total), theme.info()),
                        Span::styled(" tokens", theme.muted()),
                    ])
                })
                .collect();
            frame.render_widget(
                Paragraph::new(burn_lines).wrap(Wrap { trim: false }),
                sections[0],
            );
        } else {
            let widths = [
                Constraint::Min(14),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(6),
            ];
            let table = Table::new(rows, widths)
                .header(
                    Row::new(["model", "tries", "wins", "rate"])
                        .style(theme.accent().add_modifier(Modifier::BOLD)),
                )
                .column_spacing(1);
            frame.render_widget(table, sections[0]);
        }
    }

    // Summary line at the bottom
    let total_trials: u64 = data
        .cascade_router
        .confidence_stats
        .values()
        .map(|s| s.trials)
        .sum();
    let total_success: u64 = data
        .cascade_router
        .confidence_stats
        .values()
        .map(|s| s.successes)
        .sum();
    let overall_rate = if total_trials > 0 {
        format!("{:.1}%", total_success as f64 / total_trials as f64 * 100.0)
    } else {
        "-".to_string()
    };

    let summary = Paragraph::new(vec![Line::from(vec![
        Span::styled("models: ", theme.muted()),
        Span::raw(data.cascade_router.model_slugs.len().to_string()),
        Span::styled("  trials: ", theme.muted()),
        Span::raw(total_trials.to_string()),
        Span::styled("  success: ", theme.muted()),
        Span::raw(overall_rate),
    ])])
    .wrap(Wrap { trim: false });
    frame.render_widget(summary, sections[1]);
}

/// Conductor alerts and gate threshold health.
fn render_alerts_and_health(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Alerts & Gates ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(inner);

    // Conductor alerts
    if data.conductor_alerts.is_empty() {
        let empty = Paragraph::new("no conductor alerts")
            .style(theme.success())
            .alignment(Alignment::Center);
        frame.render_widget(empty, sections[0]);
    } else {
        let items: Vec<ListItem<'_>> = data
            .conductor_alerts
            .iter()
            .take(sections[0].height as usize)
            .map(|alert| {
                let severity_style = match alert.severity.as_str() {
                    "error" | "critical" => theme.danger(),
                    "warning" | "warn" => theme.warning(),
                    _ => theme.info(),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(&alert.severity, severity_style),
                    Span::raw(": "),
                    Span::styled(truncate(&alert.message, 30), theme.text()),
                ]))
            })
            .collect();
        frame.render_widget(List::new(items), sections[0]);
    }

    // Gate threshold summary
    if data.gate_results_page.threshold_rows.is_empty()
        && data.gate_results_page.gate_rows.is_empty()
    {
        let empty = Paragraph::new("no gate data")
            .style(theme.muted())
            .alignment(Alignment::Center);
        frame.render_widget(empty, sections[1]);
    } else {
        let rows: Vec<Row<'_>> = data
            .gate_results_page
            .gate_rows
            .iter()
            .map(|row| {
                let rate_style = if row.pass_rate >= 0.8 {
                    theme.success()
                } else if row.pass_rate >= 0.5 {
                    theme.warning()
                } else {
                    theme.danger()
                };
                Row::new(vec![
                    Cell::from(truncate(&row.gate_name, 14)),
                    Cell::from(row.total_runs.to_string()),
                    Cell::from(Span::styled(
                        format!("{:.0}%", row.pass_rate * 100.0),
                        rate_style,
                    )),
                    Cell::from(format!("{:.0}ms", row.avg_duration_ms)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Min(10),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(8),
        ];
        let table = Table::new(rows, widths)
            .header(
                Row::new(["gate", "runs", "pass", "avg"])
                    .style(theme.accent().add_modifier(Modifier::BOLD)),
            )
            .column_spacing(1);
        frame.render_widget(table, sections[1]);
    }
}

/// Build context data from available dashboard data.
fn build_context_data(data: &DashboardData) -> ContextViewData {
    // Build token burn sparklines from efficiency events
    let mut burn_map: HashMap<String, Vec<u64>> = HashMap::new();
    for event in &data.efficiency_events {
        let id = event.agent_id.clone();
        let cumulative = burn_map.entry(id).or_default();
        let prev = cumulative.last().copied().unwrap_or(0);
        cumulative.push(prev + event.input_tokens + event.output_tokens);
    }

    let mut token_burns: Vec<TokenBurnData> = burn_map
        .into_iter()
        .map(|(agent_id, cumulative)| TokenBurnData {
            agent_id,
            cumulative,
        })
        .collect();
    token_burns.sort_by(|a, b| {
        let a_total = a.cumulative.last().copied().unwrap_or(0);
        let b_total = b.cumulative.last().copied().unwrap_or(0);
        b_total.cmp(&a_total)
    });

    ContextViewData {
        token_burns,
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
