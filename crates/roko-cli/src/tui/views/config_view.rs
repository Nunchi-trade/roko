//! F6 Config view -- interactive single-panel config editor.
//!
//! Replaces the old two-panel read-only viewer with a scrollable list of
//! editable fields grouped by section, with inline value editing and a
//! save button. Runtime data (efficiency, cascade router, etc.) is shown
//! as read-only sections at the bottom.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::ViewState;
use crate::tui::config_meta::{
    self, ConfigFieldKind, ConfigItem, ConfigSource, build_flat_items, format_count, truncate,
};
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;

/// Render the full config editor view.
#[allow(clippy::too_many_lines)]
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    _view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Config ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 3 || inner.width < 20 {
        return;
    }

    // Build the flat item list (editable fields + runtime sections)
    let mut items = build_flat_items(data.root(), &tui_state.config_pending);

    // Append runtime data sections
    append_runtime_sections(&mut items, data);

    // Clamp cursor
    let cursor = tui_state.config_cursor.min(items.len().saturating_sub(1));
    let viewport_h = inner.height as usize;

    // Compute lines each item takes (field with description on selected = 2 lines)
    let mut line_offsets: Vec<usize> = Vec::with_capacity(items.len());
    let mut total_lines = 0usize;
    for (i, item) in items.iter().enumerate() {
        line_offsets.push(total_lines);
        total_lines += item_height(item, i == cursor);
    }

    // Scroll to keep cursor visible
    let cursor_top = line_offsets.get(cursor).copied().unwrap_or(0);
    let cursor_bottom = cursor_top + item_height(items.get(cursor).unwrap_or(&ConfigItem::SaveButton), true);
    let mut scroll = tui_state.config_scroll_offset;
    if cursor_top < scroll {
        scroll = cursor_top;
    }
    if cursor_bottom > scroll + viewport_h {
        scroll = cursor_bottom.saturating_sub(viewport_h);
    }

    // Render visible items
    let mut lines: Vec<Line<'_>> = Vec::new();
    let has_pending = !tui_state.config_pending.is_empty();

    for (i, item) in items.iter().enumerate() {
        let is_selected = i == cursor;
        match item {
            ConfigItem::Header(name) => {
                lines.push(render_header(name, inner.width, theme));
            }
            ConfigItem::Field {
                meta,
                value,
                source,
            } => {
                let is_modified = tui_state.config_pending.contains_key(meta.key);
                let is_editing = is_selected
                    && tui_state.config_editing
                    && tui_state.config_edit_key.as_deref() == Some(meta.key);

                let display_value = if is_editing {
                    &tui_state.config_edit_buffer
                } else {
                    value
                };

                lines.push(render_field_line(
                    meta.label,
                    display_value,
                    &meta.kind,
                    *source,
                    is_selected,
                    is_modified,
                    is_editing,
                    inner.width,
                    theme,
                ));

                // Show description for selected field
                if is_selected {
                    lines.push(render_description(meta.description, inner.width, theme));
                }
            }
            ConfigItem::SaveButton => {
                lines.push(render_save_button(is_selected, has_pending, inner.width, theme));
            }
        }
    }

    // Apply scroll offset: skip `scroll` lines from the top
    let visible_lines: Vec<Line<'_>> = lines
        .into_iter()
        .skip(scroll)
        .take(viewport_h.saturating_sub(1)) // leave room for hint bar
        .collect();

    let content = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
    frame.render_widget(content, inner);

    // Hint bar at the bottom
    let hint_area = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(1),
        width: inner.width,
        height: 1,
    };
    let hint = if tui_state.config_editing {
        Line::from(vec![
            Span::styled("Enter", theme.accent()),
            Span::styled(":confirm  ", theme.muted()),
            Span::styled("Esc", theme.accent()),
            Span::styled(":cancel", theme.muted()),
        ])
    } else {
        Line::from(vec![
            Span::styled("j/k", theme.accent()),
            Span::styled(":nav  ", theme.muted()),
            Span::styled("h/l", theme.accent()),
            Span::styled(":cycle  ", theme.muted()),
            Span::styled("Enter", theme.accent()),
            Span::styled(":edit  ", theme.muted()),
            Span::styled("Ctrl-S", theme.accent()),
            Span::styled(":save", theme.muted()),
        ])
    };
    frame.render_widget(Paragraph::new(hint), hint_area);
}

// ---------------------------------------------------------------------------
// Item rendering helpers
// ---------------------------------------------------------------------------

const fn item_height(item: &ConfigItem, selected: bool) -> usize {
    match item {
        ConfigItem::Field { .. } if selected => 2,
        ConfigItem::Header(_) | ConfigItem::Field { .. } | ConfigItem::SaveButton => 1,
    }
}

fn render_header<'a>(name: &str, width: u16, theme: &Theme) -> Line<'a> {
    let w = width as usize;
    let label = format!(" {name} ");
    let dashes = w.saturating_sub(label.len() + 2);
    let line_str = format!("──{label}{}", "─".repeat(dashes));
    Line::from(Span::styled(
        truncate(&line_str, w),
        theme.accent().add_modifier(Modifier::BOLD),
    ))
}

#[allow(clippy::too_many_arguments)]
fn render_field_line<'a>(
    label: &str,
    value: &str,
    kind: &ConfigFieldKind,
    source: ConfigSource,
    selected: bool,
    modified: bool,
    editing: bool,
    width: u16,
    theme: &Theme,
) -> Line<'a> {
    let w = width as usize;
    let label_w = 28.min(w / 2);
    let source_tag = source.label();
    let source_w = source_tag.len() + 2; // padding

    let padded_label = format!("  {label:<lw$}", lw = label_w.saturating_sub(2));

    // Format value based on kind
    let formatted_value = if editing {
        format!("{value}_") // cursor indicator
    } else {
        match kind {
            ConfigFieldKind::Bool => {
                if value == "true" {
                    "[x]".to_string()
                } else {
                    "[ ]".to_string()
                }
            }
            ConfigFieldKind::Enum(_)
            | ConfigFieldKind::Int { .. }
            | ConfigFieldKind::Float { .. }
            | ConfigFieldKind::Str => {
                format!("< {value} >")
            }
            ConfigFieldKind::ReadOnly => value.to_string(),
        }
    };

    // Compute available space for value
    let value_max = w.saturating_sub(label_w + source_w + 2);
    let displayed_value = truncate(&formatted_value, value_max);

    // Pad to push source tag to the right
    let gap = value_max.saturating_sub(displayed_value.len());
    let source_str = format!("{}{source_tag}", " ".repeat(gap + 1));

    // Styles
    let label_style = if selected {
        theme.selection().add_modifier(Modifier::BOLD)
    } else if modified {
        theme.text().add_modifier(Modifier::BOLD)
    } else {
        theme.text()
    };

    let value_style = if editing {
        theme.accent().add_modifier(Modifier::UNDERLINED)
    } else if modified {
        theme.accent().add_modifier(Modifier::BOLD)
    } else {
        theme.text()
    };

    let source_style = match source {
        ConfigSource::File => theme.accent(),
        ConfigSource::Env => theme.warning(),
        ConfigSource::Default => theme.muted(),
    };

    let bg = if selected {
        theme.selection()
    } else {
        Style::default()
    };

    Line::from(vec![
        Span::styled(padded_label, label_style.patch(bg)),
        Span::styled(displayed_value, value_style.patch(bg)),
        Span::styled(source_str, source_style.patch(bg)),
    ])
}

fn render_description<'a>(desc: &str, _width: u16, theme: &Theme) -> Line<'a> {
    Line::from(Span::styled(
        format!("      {desc}"),
        theme.muted(),
    ))
}

fn render_save_button<'a>(
    selected: bool,
    has_pending: bool,
    width: u16,
    theme: &Theme,
) -> Line<'a> {
    let label = if has_pending {
        "[ Apply & Save * ]"
    } else {
        "[ Apply & Save ]"
    };

    // Center the button
    let w = width as usize;
    let pad = w.saturating_sub(label.len()) / 2;
    let padded = format!("{}{label}", " ".repeat(pad));

    let style = if selected {
        theme.accent().add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else if has_pending {
        theme.accent().add_modifier(Modifier::BOLD)
    } else {
        theme.muted()
    };

    Line::from(Span::styled(padded, style))
}

// ---------------------------------------------------------------------------
// Runtime data sections (read-only, appended after editable fields)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
fn append_runtime_sections(items: &mut Vec<ConfigItem>, data: &DashboardData) {
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

        items.push(ConfigItem::Header("Runtime: Efficiency".to_string()));
        for (key, value) in [
            ("total_cost_usd", format!("${:.4}", eff.total_cost_usd)),
            ("event_count", eff.event_count.to_string()),
            ("avg_wall_time_ms", format!("{:.0}", eff.average_wall_time_ms)),
            ("total_input_tokens", format_count(eff.total_input_tokens)),
            ("total_output_tokens", format_count(eff.total_output_tokens)),
            ("pass_rate", pass_rate),
        ] {
            items.push(ConfigItem::Field {
                meta: config_meta::ConfigFieldMeta {
                    key: "runtime.efficiency",
                    label: key,
                    description: "",
                    kind: ConfigFieldKind::ReadOnly,
                    group: "Runtime",
                },
                value,
                source: ConfigSource::Default,
            });
        }
    }

    // Cascade router
    if !data.cascade_router.model_slugs.is_empty() {
        items.push(ConfigItem::Header("Runtime: Cascade Router".to_string()));

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

        items.push(ConfigItem::Field {
            meta: config_meta::ConfigFieldMeta {
                key: "runtime.cascade_router",
                label: "_total",
                description: "",
                kind: ConfigFieldKind::ReadOnly,
                group: "Runtime",
            },
            value: format!(
                "{} models, {total_success}/{total_trials} total",
                data.cascade_router.model_slugs.len()
            ),
            source: ConfigSource::Default,
        });

        for slug in &data.cascade_router.model_slugs {
            let stats = data.cascade_router.confidence_stats.get(slug);
            let trials = stats.map_or(0, |s| s.trials);
            let successes = stats.map_or(0, |s| s.successes);
            let rate = if trials > 0 {
                format!("{:.0}%", successes as f64 / trials as f64 * 100.0)
            } else {
                "-".to_string()
            };
            items.push(ConfigItem::Field {
                meta: config_meta::ConfigFieldMeta {
                    key: "runtime.cascade_router",
                    label: "model",
                    description: "",
                    kind: ConfigFieldKind::ReadOnly,
                    group: "Runtime",
                },
                value: format!("{slug}: {successes}/{trials} ({rate})"),
                source: ConfigSource::File,
            });
        }
    }

    // Gate thresholds
    if !data.gate_results_page.threshold_rows.is_empty() {
        items.push(ConfigItem::Header("Runtime: Gate Thresholds".to_string()));
        for row in &data.gate_results_page.threshold_rows {
            let trend_icon = match row.trend {
                crate::tui::dashboard::GateTrend::Up => "^",
                crate::tui::dashboard::GateTrend::Down => "v",
                crate::tui::dashboard::GateTrend::Flat => "-",
            };
            items.push(ConfigItem::Field {
                meta: config_meta::ConfigFieldMeta {
                    key: "runtime.gate_thresholds",
                    label: "rung",
                    description: "",
                    kind: ConfigFieldKind::ReadOnly,
                    group: "Runtime",
                },
                value: format!(
                    "rung_{}: threshold={} pass_rate={:.1}% {}",
                    row.rung,
                    row.current_threshold,
                    row.ema_pass_rate * 100.0,
                    trend_icon,
                ),
                source: ConfigSource::File,
            });
        }
    }

    // Gate results summary
    if !data.gate_results_page.gate_rows.is_empty() {
        items.push(ConfigItem::Header("Runtime: Gate Results".to_string()));
        for row in &data.gate_results_page.gate_rows {
            items.push(ConfigItem::Field {
                meta: config_meta::ConfigFieldMeta {
                    key: "runtime.gate_results",
                    label: "gate",
                    description: "",
                    kind: ConfigFieldKind::ReadOnly,
                    group: "Runtime",
                },
                value: format!(
                    "{}: {} runs, {:.0}% pass, avg {:.0}ms",
                    row.gate_name,
                    row.total_runs,
                    row.pass_rate * 100.0,
                    row.avg_duration_ms,
                ),
                source: ConfigSource::Default,
            });
        }
    }

    // Experiments
    if !data.experiments.is_empty() {
        items.push(ConfigItem::Header("Runtime: Experiments".to_string()));
        for exp in &data.experiments {
            items.push(ConfigItem::Field {
                meta: config_meta::ConfigFieldMeta {
                    key: "runtime.experiments",
                    label: "experiment",
                    description: "",
                    kind: ConfigFieldKind::ReadOnly,
                    group: "Runtime",
                },
                value: format!(
                    "{}: {} ({} variants, {} trials, {})",
                    exp.experiment_id,
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
            });
        }
    }
}
