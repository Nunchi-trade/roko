//! F10 Learning view -- cascade router & model routing insights.
//!
//! Layout:
//!   Sub-view 1 (Route): cascade stage + per-model stats table
//!   Sub-view 2 (History): stage transition timeline
//!   Sub-view 3 (Efficiency): per-model cost/pass sparklines
//!
//! Data sources:
//!   - `TuiState.cascade_router` (CascadeRouterState from cascade-router.json)
//!   - `TuiState.efficiency_events` (AgentEfficiencyEvent from efficiency.jsonl)

use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Bar, BarChart, BarGroup, Block, Borders, Cell, Paragraph, Row, Table, Wrap,
};

use super::ViewState;
use crate::tui::dashboard::Theme;
use crate::tui::state::TuiState;
use crate::tui::tabs::Tab;

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

/// Render the full learning view.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &crate::tui::dashboard::DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    match view_state.active_sub_view(Tab::Learning) {
        super::SubView::LearningRouter => render_router(frame, area, tui_state, theme),
        super::SubView::LearningHistory => render_history(frame, area, tui_state, theme),
        super::SubView::LearningEfficiency => {
            render_efficiency(frame, area, tui_state, theme);
        }
        _ => render_router(frame, area, tui_state, theme),
    }
}

// ---------------------------------------------------------------------------
// Sub-view 1: Route overview
// ---------------------------------------------------------------------------

fn render_router(frame: &mut Frame<'_>, area: Rect, tui_state: &TuiState, theme: &Theme) {
    let router = &tui_state.cascade_router;

    if router.model_slugs.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Cascade Route ")
            .border_style(Style::default().fg(theme.muted));
        let msg = Paragraph::new("No cascade router data. Run tasks to populate.")
            .alignment(Alignment::Center)
            .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(5), // stage indicator
        Constraint::Min(6),    // model stats table
        Constraint::Length(6), // bar chart
    ])
    .split(area);

    render_stage_indicator(frame, chunks[0], tui_state, theme);
    render_model_table(frame, chunks[1], tui_state, theme);
    render_selection_bars(frame, chunks[2], tui_state, theme);
}

fn render_stage_indicator(frame: &mut Frame<'_>, area: Rect, tui_state: &TuiState, theme: &Theme) {
    let router = &tui_state.cascade_router;
    let total_trials: u64 = router.confidence_stats.values().map(|s| s.trials).sum();

    let (stage_label, stage_color, next_threshold) = if total_trials < 10 {
        ("Static", Color::Yellow, 10u64)
    } else if total_trials < 30 {
        ("Confidence", Color::Cyan, 30)
    } else {
        ("UCB (LinUCB)", Color::Green, u64::MAX)
    };

    let progress_line = if next_threshold == u64::MAX {
        format!("  Observations: {total_trials} (fully adaptive)")
    } else {
        format!(
            "  Observations: {total_trials} / {next_threshold} (next stage at {next_threshold})"
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Cascade Stage ")
        .border_style(Style::default().fg(theme.muted));

    let text = vec![
        Line::from(vec![
            Span::raw("  Stage: "),
            Span::styled(
                stage_label,
                Style::default()
                    .fg(stage_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(progress_line),
        Line::from(format!("  Models: {}", router.model_slugs.len())),
    ];

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}

fn render_model_table(frame: &mut Frame<'_>, area: Rect, tui_state: &TuiState, theme: &Theme) {
    let router = &tui_state.cascade_router;

    let header = Row::new(vec![
        Cell::from("Model").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Trials").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Successes").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Pass Rate").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Sparkline").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(theme.accent));

    let mut rows = Vec::new();
    for slug in &router.model_slugs {
        let stats = router.confidence_stats.get(slug);
        let (trials, successes) = stats.map(|s| (s.trials, s.successes)).unwrap_or((0, 0));
        let pass_rate = if trials > 0 {
            format!("{:.1}%", successes as f64 / trials as f64 * 100.0)
        } else {
            "\u{2014}".to_string()
        };

        let spark = model_sparkline(slug, &tui_state.efficiency_events);

        let rate_color = if trials == 0 {
            theme.muted
        } else if successes * 100 >= trials * 80 {
            Color::Green
        } else if successes * 100 >= trials * 50 {
            Color::Yellow
        } else {
            Color::Red
        };

        rows.push(Row::new(vec![
            Cell::from(slug.as_str()),
            Cell::from(trials.to_string()),
            Cell::from(successes.to_string()),
            Cell::from(pass_rate).style(Style::default().fg(rate_color)),
            Cell::from(spark),
        ]));
    }

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(12),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(30),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Per-Model Stats ")
            .border_style(Style::default().fg(theme.muted)),
    );

    frame.render_widget(table, area);
}

/// Build a mini sparkline string from efficiency events for a given model.
fn model_sparkline(
    model_slug: &str,
    events: &[roko_learn::efficiency::AgentEfficiencyEvent],
) -> String {
    let blocks = [
        '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}',
        '\u{2588}',
    ];
    let window: usize = 5;

    let model_events: Vec<bool> = events
        .iter()
        .filter(|e| event_model_slug(e) == model_slug)
        .map(|e| e.gate_passed)
        .collect();

    if model_events.is_empty() {
        return "\u{2014}".to_string();
    }

    let mut pass_windows: Vec<f64> = Vec::with_capacity(model_events.len());
    for i in 0..model_events.len() {
        let start = i.saturating_sub(window.saturating_sub(1));
        let slice = &model_events[start..=i];
        let rate = slice.iter().filter(|&&p| p).count() as f64 / slice.len() as f64;
        pass_windows.push(rate);
    }

    // Take last 20 data points
    let tail: Vec<f64> = pass_windows
        .into_iter()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    tail.iter()
        .map(|&v| {
            let idx = (v * 7.0).round() as usize;
            blocks[idx.min(7)]
        })
        .collect()
}

use crate::tui::display_utils::{display_model, event_model_slug, shorten_model};

fn render_selection_bars(frame: &mut Frame<'_>, area: Rect, tui_state: &TuiState, theme: &Theme) {
    let router = &tui_state.cascade_router;
    let colors = [
        Color::Blue,
        Color::Cyan,
        Color::Green,
        Color::Yellow,
        Color::Magenta,
        Color::Red,
    ];

    let bars: Vec<Bar> = router
        .model_slugs
        .iter()
        .enumerate()
        .map(|(i, slug)| {
            let trials = router
                .confidence_stats
                .get(slug)
                .map(|s| s.trials)
                .unwrap_or(0);
            let label = display_model(Some(slug.as_str()));
            let label = if label.len() > 12 {
                label[..12].to_string()
            } else {
                label
            };
            Bar::default()
                .value(trials)
                .label(Line::from(label))
                .style(Style::default().fg(colors[i % colors.len()]))
        })
        .collect();

    let bar_chart = BarChart::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Selection Frequency ")
                .border_style(Style::default().fg(theme.muted)),
        )
        .data(BarGroup::default().bars(&bars))
        .bar_width(
            area.width
                .saturating_sub(4)
                .checked_div(bars.len().max(1) as u16)
                .unwrap_or(5)
                .min(12)
                .max(3),
        )
        .bar_gap(1);

    frame.render_widget(bar_chart, area);
}

// ---------------------------------------------------------------------------
// Sub-view 2: Stage transition history
// ---------------------------------------------------------------------------

fn render_history(frame: &mut Frame<'_>, area: Rect, tui_state: &TuiState, theme: &Theme) {
    let router = &tui_state.cascade_router;
    let total_trials: u64 = router.confidence_stats.values().map(|s| s.trials).sum();

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    if total_trials == 0 {
        lines.push(Line::from(
            "  No observations yet. Run tasks to see transitions.",
        ));
    } else {
        lines.push(Line::from(vec![
            Span::raw("  Current observations: "),
            Span::styled(
                total_trials.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));

        let stages = [
            ("Static", 0u64, 10u64, Color::Yellow),
            ("Confidence", 10, 30, Color::Cyan),
            ("UCB (LinUCB)", 30, u64::MAX, Color::Green),
        ];

        for (label, from, to, color) in &stages {
            let active = if *to == u64::MAX {
                total_trials >= *from
            } else {
                total_trials >= *from && total_trials < *to
            };
            let marker = if active { "\u{25b6} " } else { "  " };
            let range_str = if *to == u64::MAX {
                format!("{from}+")
            } else {
                format!("{from}-{}", to - 1)
            };

            let style = if active {
                Style::default().fg(*color).add_modifier(Modifier::BOLD)
            } else if total_trials >= *from {
                Style::default().fg(*color)
            } else {
                Style::default().fg(theme.muted)
            };

            lines.push(Line::from(vec![
                Span::raw(format!("  {marker}")),
                Span::styled(format!("{label:<16}"), style),
                Span::styled(
                    format!("  ({range_str} obs)"),
                    Style::default().fg(theme.muted),
                ),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from("  Stage Progression:"));
        lines.push(Line::from(""));

        let bar_width = area.width.saturating_sub(8) as usize;
        let scale = if total_trials > 0 {
            bar_width as f64 / total_trials.max(30) as f64
        } else {
            1.0
        };

        let static_w = ((10.min(total_trials) as f64) * scale).round() as usize;
        let confidence_w = if total_trials > 10 {
            ((total_trials.min(30) - 10) as f64 * scale).round() as usize
        } else {
            0
        };
        let ucb_w = if total_trials > 30 {
            ((total_trials - 30) as f64 * scale).round() as usize
        } else {
            0
        };

        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "\u{2588}".repeat(static_w),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                "\u{2588}".repeat(confidence_w),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled("\u{2588}".repeat(ucb_w), Style::default().fg(Color::Green)),
        ]));

        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("\u{25a0}", Style::default().fg(Color::Yellow)),
            Span::raw(" Static  "),
            Span::styled("\u{25a0}", Style::default().fg(Color::Cyan)),
            Span::raw(" Confidence  "),
            Span::styled("\u{25a0}", Style::default().fg(Color::Green)),
            Span::raw(" UCB"),
        ]));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Stage Transition History ")
        .border_style(Style::default().fg(theme.muted));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

// ---------------------------------------------------------------------------
// Sub-view 3: Efficiency by model
// ---------------------------------------------------------------------------

fn render_efficiency(frame: &mut Frame<'_>, area: Rect, tui_state: &TuiState, theme: &Theme) {
    let events = &tui_state.efficiency_events;

    if events.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Model Efficiency ")
            .border_style(Style::default().fg(theme.muted));
        let msg = Paragraph::new("No efficiency events recorded yet.")
            .alignment(Alignment::Center)
            .block(block);
        frame.render_widget(msg, area);
        return;
    }

    let mut model_stats: HashMap<String, ModelEffStats> = HashMap::new();
    for event in events {
        let model = event_model_slug(event).to_string();
        let entry = model_stats.entry(model).or_default();
        entry.count += 1;
        if event.gate_passed {
            entry.passed += 1;
        }
        entry.total_cost += event.cost_usd;
        entry.total_latency_ms += event.wall_time_ms;
    }

    let chunks = Layout::vertical([
        Constraint::Min(6),    // stats table
        Constraint::Length(8), // cost bar chart
    ])
    .split(area);

    // ── Stats table ──
    let header = Row::new(vec![
        Cell::from("Model").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Events").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Passed").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Pass %").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Avg Cost").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Avg Latency").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(theme.accent));

    let mut sorted_models: Vec<_> = model_stats.iter().collect();
    sorted_models.sort_by(|a, b| b.1.count.cmp(&a.1.count));

    let rows: Vec<Row> = sorted_models
        .iter()
        .map(|(model, stats)| {
            let pass_pct = if stats.count > 0 {
                format!("{:.1}%", stats.passed as f64 / stats.count as f64 * 100.0)
            } else {
                "\u{2014}".to_string()
            };
            let avg_cost = if stats.count > 0 {
                format!("${:.4}", stats.total_cost / stats.count as f64)
            } else {
                "\u{2014}".to_string()
            };
            let avg_latency = if stats.count > 0 {
                format!("{}ms", stats.total_latency_ms / stats.count as u64)
            } else {
                "\u{2014}".to_string()
            };

            let rate_color = if stats.count == 0 {
                theme.muted
            } else if stats.passed * 100 >= stats.count * 80 {
                Color::Green
            } else if stats.passed * 100 >= stats.count * 50 {
                Color::Yellow
            } else {
                Color::Red
            };

            Row::new(vec![
                Cell::from(display_model(Some(model.as_str()))),
                Cell::from(stats.count.to_string()),
                Cell::from(stats.passed.to_string()),
                Cell::from(pass_pct).style(Style::default().fg(rate_color)),
                Cell::from(avg_cost),
                Cell::from(avg_latency),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(13),
            Constraint::Percentage(18),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Model Efficiency Stats ")
            .border_style(Style::default().fg(theme.muted)),
    );

    frame.render_widget(table, chunks[0]);

    // ── Cost bar chart ──
    let colors = [
        Color::Blue,
        Color::Cyan,
        Color::Green,
        Color::Yellow,
        Color::Magenta,
        Color::Red,
    ];
    let bars: Vec<Bar> = sorted_models
        .iter()
        .enumerate()
        .filter(|(_, (_, stats))| stats.count > 0)
        .map(|(i, (model, stats))| {
            let avg = stats.total_cost / stats.count as f64;
            let value = (avg * 10000.0).round() as u64;
            let label = display_model(Some(model.as_str()));
            let label = if label.len() > 12 {
                label[..12].to_string()
            } else {
                label
            };
            Bar::default()
                .value(value)
                .label(Line::from(label))
                .style(Style::default().fg(colors[i % colors.len()]))
        })
        .collect();

    if !bars.is_empty() {
        let bar_chart = BarChart::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Avg Cost (\u{00d7}10\u{207b}\u{2074} $) ")
                    .border_style(Style::default().fg(theme.muted)),
            )
            .data(BarGroup::default().bars(&bars))
            .bar_width(
                chunks[1]
                    .width
                    .saturating_sub(4)
                    .checked_div(bars.len().max(1) as u16)
                    .unwrap_or(5)
                    .min(12)
                    .max(3),
            )
            .bar_gap(1);

        frame.render_widget(bar_chart, chunks[1]);
    }
}

#[derive(Debug, Default)]
struct ModelEffStats {
    count: usize,
    passed: usize,
    total_cost: f64,
    total_latency_ms: u64,
}
