//! F6 Config view -- key-value display with expandable sections.
//!
//! Shows config sections from roko.toml and effective runtime config
//! in a read-only tree-style display. Sections are collapsible.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;

/// A config section with key-value pairs.
#[derive(Debug, Clone)]
pub struct ConfigSection {
    pub name: String,
    pub entries: Vec<ConfigEntry>,
    pub expanded: bool,
}

/// A single config key-value pair.
#[derive(Debug, Clone)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub source: ConfigSource,
}

/// Where a config value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// From roko.toml.
    File,
    /// From environment variable.
    Env,
    /// Default value.
    Default,
}

impl ConfigSource {
    fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Env => "env",
            Self::Default => "default",
        }
    }
}

/// Render the full config view.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    _tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    // Config data is not yet in DashboardData; build from available info.
    let sections = build_config_sections(data);
    render_with_sections(frame, area, &sections, view_state, theme);
}

/// Render the config view with explicit sections (for integration layer).
pub fn render_with_sections(
    frame: &mut Frame<'_>,
    area: Rect,
    sections: &[ConfigSection],
    view_state: &ViewState,
    theme: &Theme,
) {
    let panels =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).split(area);

    render_section_list(frame, panels[0], sections, view_state, theme);
    render_section_detail(frame, panels[1], sections, view_state, theme);
}

/// Left panel: section list.
fn render_section_list(
    frame: &mut Frame<'_>,
    area: Rect,
    sections: &[ConfigSection],
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Sections ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if sections.is_empty() {
        let empty = Paragraph::new("no config loaded")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem<'_>> = sections
        .iter()
        .enumerate()
        .map(|(i, section)| {
            let marker = if section.expanded { "[-]" } else { "[+]" };
            let style = if i == view_state.selected {
                theme.selection()
            } else {
                theme.text()
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{marker} ")),
                Span::styled(&section.name, style),
                Span::styled(
                    format!("  ({} keys)", section.entries.len()),
                    theme.muted(),
                ),
            ]))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Right panel: key-value detail for selected section.
fn render_section_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    sections: &[ConfigSection],
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Config Values ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(section) = sections.get(view_state.selected) else {
        let empty = Paragraph::new("select a section from the left panel")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    };

    if section.entries.is_empty() {
        let empty = Paragraph::new("no entries in this section")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let rows: Vec<Row<'_>> = section
        .entries
        .iter()
        .map(|entry| {
            let source_style = match entry.source {
                ConfigSource::File => theme.accent(),
                ConfigSource::Env => theme.warning(),
                ConfigSource::Default => theme.muted(),
            };
            Row::new(vec![
                Cell::from(entry.key.as_str()),
                Cell::from(truncate(&entry.value, 40)),
                Cell::from(Span::styled(entry.source.label(), source_style)),
            ])
        })
        .collect();

    let widths = [Constraint::Min(18), Constraint::Min(20), Constraint::Length(8)];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["key", "value", "source"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// Build config sections from available dashboard data.
fn build_config_sections(data: &DashboardData) -> Vec<ConfigSection> {
    let mut sections = Vec::new();

    // Efficiency summary section
    {
        let eff = &data.efficiency;
        let pass_rate = if eff.event_count > 0 {
            format!(
                "{:.1}%",
                eff.passed_count as f64 / eff.event_count as f64 * 100.0
            )
        } else {
            "-".to_string()
        };
        sections.push(ConfigSection {
            name: String::from("efficiency"),
            entries: vec![
                ConfigEntry {
                    key: String::from("total_cost_usd"),
                    value: format!("${:.4}", eff.total_cost_usd),
                    source: ConfigSource::Default,
                },
                ConfigEntry {
                    key: String::from("event_count"),
                    value: eff.event_count.to_string(),
                    source: ConfigSource::Default,
                },
                ConfigEntry {
                    key: String::from("avg_wall_time_ms"),
                    value: format!("{:.0}", eff.average_wall_time_ms),
                    source: ConfigSource::Default,
                },
                ConfigEntry {
                    key: String::from("total_input_tokens"),
                    value: format_count(eff.total_input_tokens),
                    source: ConfigSource::Default,
                },
                ConfigEntry {
                    key: String::from("total_output_tokens"),
                    value: format_count(eff.total_output_tokens),
                    source: ConfigSource::Default,
                },
                ConfigEntry {
                    key: String::from("pass_rate"),
                    value: pass_rate,
                    source: ConfigSource::Default,
                },
            ],
            expanded: true,
        });
    }

    // Cascade router section — model routing state
    if !data.cascade_router.model_slugs.is_empty() {
        let mut entries: Vec<ConfigEntry> = data
            .cascade_router
            .model_slugs
            .iter()
            .map(|slug| {
                let stats = data.cascade_router.confidence_stats.get(slug);
                let trials = stats.map_or(0, |s| s.trials);
                let successes = stats.map_or(0, |s| s.successes);
                let rate = if trials > 0 {
                    format!("{:.0}%", successes as f64 / trials as f64 * 100.0)
                } else {
                    "-".to_string()
                };
                ConfigEntry {
                    key: slug.clone(),
                    value: format!("{successes}/{trials} ({rate})"),
                    source: ConfigSource::File,
                }
            })
            .collect();

        // Add summary row
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
        entries.insert(
            0,
            ConfigEntry {
                key: String::from("_total"),
                value: format!(
                    "{} models, {total_success}/{total_trials} total",
                    data.cascade_router.model_slugs.len()
                ),
                source: ConfigSource::Default,
            },
        );

        sections.push(ConfigSection {
            name: String::from("cascade_router"),
            entries,
            expanded: true,
        });
    }

    // Gate thresholds section — adaptive thresholds per rung
    if !data.gate_results_page.threshold_rows.is_empty() {
        let entries: Vec<ConfigEntry> = data
            .gate_results_page
            .threshold_rows
            .iter()
            .map(|row| {
                let trend_icon = match row.trend {
                    crate::tui::dashboard::GateTrend::Up => "\u{2191}",   // arrow up
                    crate::tui::dashboard::GateTrend::Down => "\u{2193}", // arrow down
                    crate::tui::dashboard::GateTrend::Flat => "\u{2194}", // arrow left-right
                };
                ConfigEntry {
                    key: format!("rung_{}", row.rung),
                    value: format!(
                        "threshold={} pass_rate={:.1}% {}",
                        row.current_threshold,
                        row.ema_pass_rate * 100.0,
                        trend_icon,
                    ),
                    source: ConfigSource::File,
                }
            })
            .collect();
        sections.push(ConfigSection {
            name: String::from("gate_thresholds"),
            entries,
            expanded: true,
        });
    }

    // Gate results summary section
    if !data.gate_results_page.gate_rows.is_empty() {
        let entries: Vec<ConfigEntry> = data
            .gate_results_page
            .gate_rows
            .iter()
            .map(|row| ConfigEntry {
                key: row.gate_name.clone(),
                value: format!(
                    "{} runs, {:.0}% pass, avg {:.0}ms",
                    row.total_runs,
                    row.pass_rate * 100.0,
                    row.avg_duration_ms,
                ),
                source: ConfigSource::Default,
            })
            .collect();
        sections.push(ConfigSection {
            name: String::from("gate_results"),
            entries,
            expanded: true,
        });
    }

    // Experiments section
    if !data.experiments.is_empty() {
        let entries: Vec<ConfigEntry> = data
            .experiments
            .iter()
            .map(|exp| ConfigEntry {
                key: exp.experiment_id.clone(),
                value: format!(
                    "{} ({} variants, {} trials, {})",
                    exp.section_name,
                    exp.active_variants,
                    exp.total_trials,
                    exp.status,
                ),
                source: if exp.winner_id.is_some() {
                    ConfigSource::File
                } else {
                    ConfigSource::Default
                },
            })
            .collect();
        sections.push(ConfigSection {
            name: String::from("experiments"),
            entries,
            expanded: false,
        });
    }

    // Plans section
    if !data.plans.is_empty() {
        sections.push(ConfigSection {
            name: String::from("plans"),
            entries: data
                .plans
                .iter()
                .map(|plan| ConfigEntry {
                    key: plan.id.clone(),
                    value: format!(
                        "{} ({} tasks, {})",
                        plan.title,
                        plan.task_count,
                        if plan.completed { "done" } else { "pending" }
                    ),
                    source: ConfigSource::File,
                })
                .collect(),
            expanded: false,
        });
    }

    // Agents section
    if !data.agents.is_empty() {
        let entries: Vec<ConfigEntry> = data
            .agents
            .iter()
            .map(|agent| ConfigEntry {
                key: agent.id.clone(),
                value: format!(
                    "{} ({})",
                    agent.label,
                    agent.status,
                ),
                source: match agent.status.as_str() {
                    "running" | "active" => ConfigSource::Env,
                    _ => ConfigSource::Default,
                },
            })
            .collect();
        sections.push(ConfigSection {
            name: String::from("agents"),
            entries,
            expanded: false,
        });
    }

    sections
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
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
