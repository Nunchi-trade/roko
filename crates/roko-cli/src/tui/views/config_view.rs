//! F6 Config view -- key-value display with expandable sections.
//!
//! Shows the real `roko.toml` configuration parsed into sections,
//! plus runtime data (efficiency, cascade router, gate thresholds,
//! experiments, plans, agents). Each value is annotated with its
//! source: file, env override, or default.

use std::collections::HashMap;

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
    tui_state: &TuiState,
    _view_state: &ViewState,
    theme: &Theme,
) {
    let mut sections = build_config_sections(data);

    // Apply expansion state from TuiState
    for (i, section) in sections.iter_mut().enumerate() {
        section.expanded = tui_state.config_expanded.contains(&i);
    }

    let selected = tui_state.config_selected;

    let panels =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).split(area);

    render_section_list(frame, panels[0], &sections, selected, theme);
    render_section_detail(frame, panels[1], &sections, selected, theme);
}

/// Left panel: section list with selection cursor.
fn render_section_list(
    frame: &mut Frame<'_>,
    area: Rect,
    sections: &[ConfigSection],
    selected: usize,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Sections (j/k:nav Enter:toggle) ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if sections.is_empty() {
        let empty = Paragraph::new("no config loaded — run `roko init`")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem<'_>> = sections
        .iter()
        .enumerate()
        .map(|(i, section)| {
            let marker = if section.expanded { "\u{25bc}" } else { "\u{25b6}" }; // ▼ / ▶
            let is_selected = i == selected;
            let cursor = if is_selected { "\u{25b8} " } else { "  " }; // ▸
            let name_style = if is_selected {
                theme.selection()
            } else {
                theme.text()
            };
            ListItem::new(Line::from(vec![
                Span::styled(cursor, if is_selected { theme.accent() } else { theme.muted() }),
                Span::styled(format!("{marker} "), theme.muted()),
                Span::styled(&section.name, name_style),
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
    selected: usize,
    theme: &Theme,
) {
    let section_name = sections
        .get(selected)
        .map(|s| s.name.as_str())
        .unwrap_or("config");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {section_name} "))
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(section) = sections.get(selected) else {
        let empty = Paragraph::new("select a section with j/k")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    };

    if !section.expanded {
        let hint = Paragraph::new("press Enter to expand this section")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(hint, inner);
        return;
    }

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
                Cell::from(truncate(&entry.value, 50)),
                Cell::from(Span::styled(entry.source.label(), source_style)),
            ])
        })
        .collect();

    let widths = [Constraint::Min(22), Constraint::Min(20), Constraint::Length(8)];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["key", "value", "source"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

// ---------------------------------------------------------------------------
// TOML config loading
// ---------------------------------------------------------------------------

/// Known ROKO_* env vars and the config field they override.
const ENV_OVERRIDES: &[(&str, &str, &str)] = &[
    ("ROKO_MODEL", "agent", "default_model"),
    ("ROKO_BACKEND", "agent", "default_backend"),
    ("ROKO_EFFORT", "agent", "default_effort"),
    ("ROKO_CONTEXT_LIMIT_K", "agent", "context_limit_k"),
    ("ROKO_MAX_AGENTS", "conductor", "max_agents"),
    ("ROKO_BUDGET_USD", "budget", "max_plan_usd"),
    ("ROKO_PARALLEL", "conductor", "parallel_enabled"),
    ("ROKO_EXPRESS", "conductor", "express_mode"),
    ("ROKO_SKIP_TESTS", "gates", "skip_tests"),
    ("ROKO_CLIPPY", "gates", "clippy_enabled"),
    ("ROKO_PROVIDER", "agent", "provider_override"),
    ("ROKO_MODEL_SLUG", "agent", "model_slug_override"),
];

/// Build env override map: (section, key) -> env var name, only for vars
/// that are actually set in the current process environment.
fn active_env_overrides() -> HashMap<(String, String), String> {
    let mut map = HashMap::new();
    for &(env_var, section, key) in ENV_OVERRIDES {
        if std::env::var(env_var).is_ok() {
            map.insert(
                (section.to_string(), key.to_string()),
                env_var.to_string(),
            );
        }
    }
    map
}

/// Try to load and parse roko.toml from the workspace root.
fn load_toml_config(root: &std::path::Path) -> Option<toml::Value> {
    let config_path = root.join("roko.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    content.parse::<toml::Value>().ok()
}

/// Try to load default config for comparison.
fn default_toml_config() -> toml::Value {
    let default_cfg = roko_core::config::RokoConfig::default();
    let toml_str = default_cfg.to_toml().unwrap_or_default();
    toml_str.parse::<toml::Value>().unwrap_or(toml::Value::Table(toml::map::Map::new()))
}

/// Walk a TOML table and produce ConfigEntry items. Handles nested tables
/// by flattening keys with dot notation.
fn toml_table_to_entries(
    table: &toml::map::Map<String, toml::Value>,
    defaults: Option<&toml::map::Map<String, toml::Value>>,
    section_name: &str,
    env_overrides: &HashMap<(String, String), String>,
) -> Vec<ConfigEntry> {
    let mut entries = Vec::new();
    let mut keys: Vec<&String> = table.keys().collect();
    keys.sort();

    for key in keys {
        let value = &table[key];
        match value {
            toml::Value::Table(sub) => {
                // Flatten nested table with dot-prefix
                let sub_defaults = defaults.and_then(|d| d.get(key)).and_then(|v| v.as_table());
                for sub_entry in toml_table_to_entries(sub, sub_defaults, section_name, env_overrides) {
                    entries.push(ConfigEntry {
                        key: format!("{key}.{}", sub_entry.key),
                        value: sub_entry.value,
                        source: sub_entry.source,
                    });
                }
            }
            toml::Value::Array(arr) => {
                let display = if arr.len() <= 3 {
                    format_toml_value(value)
                } else {
                    format!("[{} items]", arr.len())
                };
                let source = determine_source(section_name, key, value, defaults, env_overrides);
                entries.push(ConfigEntry {
                    key: key.clone(),
                    value: display,
                    source,
                });
            }
            _ => {
                let display = format_toml_value(value);
                let source = determine_source(section_name, key, value, defaults, env_overrides);
                entries.push(ConfigEntry {
                    key: key.clone(),
                    value: display,
                    source,
                });
            }
        }
    }
    entries
}

/// Format a TOML value for display.
fn format_toml_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => {
            if s.is_empty() {
                "(empty)".to_string()
            } else {
                s.clone()
            }
        }
        toml::Value::Integer(n) => n.to_string(),
        toml::Value::Float(f) => {
            if *f == f.floor() && f.abs() < 1_000_000.0 {
                format!("{f:.1}")
            } else {
                format!("{f}")
            }
        }
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(format_toml_value).collect();
            format!("[{}]", items.join(", "))
        }
        toml::Value::Table(_) => "{...}".to_string(),
        toml::Value::Datetime(dt) => dt.to_string(),
    }
}

/// Determine whether a value comes from file, env, or is a default.
fn determine_source(
    section_name: &str,
    key: &str,
    value: &toml::Value,
    defaults: Option<&toml::map::Map<String, toml::Value>>,
    env_overrides: &HashMap<(String, String), String>,
) -> ConfigSource {
    // Check env overrides first
    let lookup = (section_name.to_string(), key.to_string());
    if env_overrides.contains_key(&lookup) {
        return ConfigSource::Env;
    }

    // Check if value differs from default
    if let Some(defaults) = defaults {
        if let Some(default_value) = defaults.get(key) {
            if value != default_value {
                return ConfigSource::File;
            }
        } else {
            // Key exists in file but not in defaults => custom
            return ConfigSource::File;
        }
    }

    ConfigSource::Default
}

/// Ordered list of top-level config sections to display.
const CONFIG_SECTION_ORDER: &[&str] = &[
    "project",
    "prd",
    "agent",
    "providers",
    "models",
    "gates",
    "routing",
    "pipeline",
    "budget",
    "conductor",
    "learning",
    "tui",
    "serve",
    "server",
    "deploy",
    "scheduler",
    "webhooks",
    "perplexity",
    "gemini",
];

/// Build sections from the real roko.toml + runtime data.
fn build_config_sections(data: &DashboardData) -> Vec<ConfigSection> {
    let mut sections = Vec::new();
    let env_overrides = active_env_overrides();

    // Load real config
    let toml_config = load_toml_config(data.root());
    let defaults = default_toml_config();
    let default_table = defaults.as_table();

    if let Some(toml::Value::Table(root_table)) = &toml_config {
        // Top-level scalars (config_version, schema_version)
        let mut top_entries = Vec::new();
        for key in &["config_version", "schema_version"] {
            if let Some(value) = root_table.get(*key) {
                let default_val = default_table.and_then(|d| d.get(*key));
                let source = if default_val.map_or(true, |d| d != value) {
                    ConfigSource::File
                } else {
                    ConfigSource::Default
                };
                top_entries.push(ConfigEntry {
                    key: key.to_string(),
                    value: format_toml_value(value),
                    source,
                });
            }
        }
        if !top_entries.is_empty() {
            sections.push(ConfigSection {
                name: String::from("roko.toml"),
                entries: top_entries,
                expanded: true,
            });
        }

        // Walk sections in defined order
        for &section_name in CONFIG_SECTION_ORDER {
            if let Some(toml::Value::Table(section_table)) = root_table.get(section_name) {
                let section_defaults = default_table
                    .and_then(|d| d.get(section_name))
                    .and_then(|v| v.as_table());

                let entries = toml_table_to_entries(
                    section_table,
                    section_defaults,
                    section_name,
                    &env_overrides,
                );

                if !entries.is_empty() {
                    sections.push(ConfigSection {
                        name: format!("[{section_name}]"),
                        entries,
                        expanded: true,
                    });
                }
            }
        }

        // Any sections in the file not in our known order
        let known: std::collections::HashSet<&str> =
            CONFIG_SECTION_ORDER.iter().copied().chain(["config_version", "schema_version"]).collect();
        let mut extra_keys: Vec<&String> = root_table.keys()
            .filter(|k| !known.contains(k.as_str()))
            .collect();
        extra_keys.sort();
        for key in extra_keys {
            if let Some(toml::Value::Table(section_table)) = root_table.get(key) {
                let entries = toml_table_to_entries(
                    section_table,
                    None,
                    key,
                    &env_overrides,
                );
                if !entries.is_empty() {
                    sections.push(ConfigSection {
                        name: format!("[{key}]"),
                        entries,
                        expanded: false,
                    });
                }
            }
        }
    } else {
        // No roko.toml found -- show a notice
        sections.push(ConfigSection {
            name: String::from("roko.toml"),
            entries: vec![ConfigEntry {
                key: String::from("status"),
                value: String::from("not found (run `roko init`)"),
                source: ConfigSource::Default,
            }],
            expanded: true,
        });
    }

    // Active env overrides section (show which ROKO_* vars are set)
    {
        let mut env_entries: Vec<ConfigEntry> = Vec::new();
        for &(env_var, section, key) in ENV_OVERRIDES {
            if let Ok(val) = std::env::var(env_var) {
                env_entries.push(ConfigEntry {
                    key: format!("{env_var} -> {section}.{key}"),
                    value: truncate(&val, 40),
                    source: ConfigSource::Env,
                });
            }
        }
        if !env_entries.is_empty() {
            sections.push(ConfigSection {
                name: String::from("env overrides"),
                entries: env_entries,
                expanded: true,
            });
        }
    }

    // -----------------------------------------------------------------------
    // Runtime data sections (below config sections)
    // -----------------------------------------------------------------------

    // Efficiency summary
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
            name: String::from("runtime: efficiency"),
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

    // Cascade router
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
            name: String::from("runtime: cascade_router"),
            entries,
            expanded: true,
        });
    }

    // Gate thresholds
    if !data.gate_results_page.threshold_rows.is_empty() {
        let entries: Vec<ConfigEntry> = data
            .gate_results_page
            .threshold_rows
            .iter()
            .map(|row| {
                let trend_icon = match row.trend {
                    crate::tui::dashboard::GateTrend::Up => "^",
                    crate::tui::dashboard::GateTrend::Down => "v",
                    crate::tui::dashboard::GateTrend::Flat => "-",
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
            name: String::from("runtime: gate_thresholds"),
            entries,
            expanded: true,
        });
    }

    // Gate results summary
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
            name: String::from("runtime: gate_results"),
            entries,
            expanded: true,
        });
    }

    // Experiments
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
            name: String::from("runtime: experiments"),
            entries,
            expanded: false,
        });
    }

    // Plans
    if !data.plans.is_empty() {
        sections.push(ConfigSection {
            name: String::from("runtime: plans"),
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

    // Agents
    if !data.agents.is_empty() {
        let entries: Vec<ConfigEntry> = data
            .agents
            .iter()
            .map(|agent| ConfigEntry {
                key: agent.id.clone(),
                value: format!("{} ({})", agent.label, agent.status),
                source: match agent.status.as_str() {
                    "running" | "active" => ConfigSource::Env,
                    _ => ConfigSource::Default,
                },
            })
            .collect();
        sections.push(ConfigSection {
            name: String::from("runtime: agents"),
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
