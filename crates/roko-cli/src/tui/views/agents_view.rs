//! F3 Agents view -- Mori-style agent roster + output panel.
//!
//! Layout: left 32% (agent roster, summary line, token sparkline),
//! right 68% (role tabs + scrollable agent output).
//!
//! Renders rich gradient progress bars, context gauges, role-colored
//! tabs, and status chips matching the Mori Agents screen (F3).

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;

// ---------------------------------------------------------------------------
// Role tab labels (fixed order, matching Mori)
// ---------------------------------------------------------------------------

const ROLE_TABS: &[(&str, &str)] = &[
    ("implementer", "1:impl"),
    ("strategist", "2:strat"),
    ("architect", "3:arch"),
    ("auditor", "4:audit"),
    ("critic", "5:crit"),
    ("conductor", "6:cond"),
    ("researcher", "7:res"),
];

// ---------------------------------------------------------------------------
// Public render
// ---------------------------------------------------------------------------

/// Render the full agents view.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let panels =
        Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)]).split(area);

    render_left_panel(frame, panels[0], data, tui_state, view_state, theme);
    render_right_panel(frame, panels[1], data, tui_state, view_state, theme);
}

// ---------------------------------------------------------------------------
// Left panel: agent roster + summary + token sparkline
// ---------------------------------------------------------------------------

fn render_left_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    // Compute how much space to allocate.
    // Summary = 2 lines, sparkline = 6 lines (if token data exists), rest = roster.
    let has_token_data = tui_state.cumulative_input_tokens > 0
        || tui_state.cumulative_output_tokens > 0
        || !data.efficiency_events.is_empty();

    let sparkline_height = if has_token_data { 6u16 } else { 0u16 };

    let sections = Layout::vertical([
        Constraint::Min(4),                        // agent roster (flexible)
        Constraint::Length(2),                      // summary line
        Constraint::Length(sparkline_height),       // token sparkline
    ])
    .split(area);

    render_agent_roster(frame, sections[0], data, tui_state, view_state, theme);
    render_summary_line(frame, sections[1], data, tui_state, theme);
    if has_token_data {
        render_token_sparkline(frame, sections[2], data, tui_state, theme);
    }
}

// ---------------------------------------------------------------------------
// Agent roster
// ---------------------------------------------------------------------------

fn render_agent_roster(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    _tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let active_count = data
        .agents
        .iter()
        .filter(|a| a.status == "running" || a.status == "active")
        .count();
    let title = format!(" Agents ({} active) ", active_count);

    let border_style = if active_count > 0 {
        Style::default().fg(theme.accent)
    } else {
        theme.muted()
    };
    let title_style = if active_count > 0 {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        theme.muted()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, title_style))
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if data.agents.is_empty() {
        let v_pad = inner.height / 2;
        let mut empty_lines: Vec<Line<'_>> = Vec::new();
        for _ in 0..v_pad.saturating_sub(1) {
            empty_lines.push(Line::from(""));
        }
        empty_lines.push(Line::from(Span::styled(
            "no agents spawned",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        )));
        empty_lines.push(Line::from(""));
        empty_lines.push(Line::from(Span::styled(
            "agents will appear here when",
            Style::default().fg(theme.muted),
        )));
        empty_lines.push(Line::from(Span::styled(
            "plan execution begins",
            Style::default().fg(theme.muted),
        )));
        let empty = Paragraph::new(empty_lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let content_width = inner.width as usize;

    // Build activity snapshot for token/cost columns
    let activity =
        crate::tui::dashboard::build_agent_activity_snapshot(&data.agents, &data.efficiency_events);

    // Column header
    let mut lines: Vec<Line<'_>> = Vec::new();
    if inner.height > 3 {
        let role_w = 11usize.min(content_width / 3);
        lines.push(Line::from(vec![
            Span::styled("   ", Style::default()),
            Span::styled(
                format!("{:<role_w$}", "role"),
                Style::default().fg(theme.muted),
            ),
            Span::styled("  ", Style::default()),
            Span::styled("tokens", Style::default().fg(theme.muted)),
            Span::styled("  ", Style::default()),
            Span::styled("cost", Style::default().fg(theme.muted)),
            Span::styled("   ", Style::default()),
            Span::styled("ctx", Style::default().fg(theme.muted)),
        ]));
    }

    for (idx, agent) in data.agents.iter().enumerate() {
        let is_selected = idx == view_state.selected;
        let is_active = agent.status == "running" || agent.status == "active";
        let is_done = agent.status == "done" || agent.status == "completed";
        let is_failed = agent.status == "error" || agent.status == "failed";

        // Status icon
        let (icon, icon_style) = if is_active {
            (
                "\u{25b6}", // ▶
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            )
        } else if is_done {
            ("\u{2713}", Style::default().fg(theme.success)) // ✓
        } else if is_failed {
            (
                "\u{2717}", // ✗
                Style::default()
                    .fg(theme.danger)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("\u{00b7}", Style::default().fg(theme.muted)) // ·
        };

        // Role accent color
        let accent = role_accent(&agent.label, theme);
        let bg = if is_selected {
            theme.selection_background
        } else {
            Color::Reset
        };

        // Cursor indicator
        let cursor = if is_selected { " \u{25b6} " } else { "   " };

        // Role label
        let role_w = 11usize.min(content_width / 3);
        let role_label = truncate_middle(&agent.label, role_w);

        // Activity data
        let activity_row = activity.as_ref().and_then(|snap| {
            snap.active_agents
                .iter()
                .find(|r| r.agent_id == agent.id)
        });

        let tokens_str = activity_row
            .map(|r| format_tokens(r.tokens_used))
            .unwrap_or_else(|| "-".to_string());

        let cost_str = activity_row
            .map(|r| {
                if r.cost_usd > 0.001 {
                    format!("${:.2}", r.cost_usd)
                } else {
                    "-".to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string());

        // Context gauge — use tokens / 200k as proxy fill
        let total_tokens = activity_row.map_or(0u64, |r| r.tokens_used);
        let ctx_limit = 200_000u64; // default context window
        let fill_pct = (total_tokens as f64 / ctx_limit as f64).clamp(0.0, 1.0);
        let gauge_width = 8usize.min(content_width.saturating_sub(30));

        // State chip
        let (state_label, state_fg, state_bg) = if is_active {
            (" LIVE ", Color::Black, accent)
        } else if is_done {
            (" DONE ", Color::Black, theme.success)
        } else if is_failed {
            (" FAIL ", Color::Black, theme.danger)
        } else {
            (" idle ", theme.muted, Color::Reset)
        };

        let mut spans = vec![
            Span::styled(cursor, Style::default().fg(theme.accent).bg(bg)),
            Span::styled(icon.to_string(), icon_style.bg(bg)),
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                format!("{:<role_w$}", role_label),
                Style::default()
                    .fg(accent)
                    .bg(bg)
                    .add_modifier(if is_active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                format!(" {:>6}", tokens_str),
                Style::default().fg(theme.foreground).bg(bg),
            ),
            Span::styled(
                format!(" {:>6}", cost_str),
                Style::default().fg(theme.muted).bg(bg),
            ),
            Span::styled(" ", Style::default().bg(bg)),
        ];

        // Gradient gauge bar
        spans.extend(gradient_bar(gauge_width, fill_pct, is_active, theme));

        spans.push(Span::styled(" ", Style::default().bg(bg)));
        spans.push(Span::styled(
            state_label.to_string(),
            Style::default()
                .fg(state_fg)
                .bg(state_bg)
                .add_modifier(Modifier::BOLD),
        ));

        // Model tag (compact)
        if content_width > 50 {
            let model = activity_row
                .map(|r| shorten_model(&r.model))
                .unwrap_or_default();
            if !model.is_empty() {
                spans.push(Span::styled(
                    format!(" {}", model),
                    Style::default().fg(theme.muted).bg(bg),
                ));
            }
        }

        lines.push(Line::from(spans));

        // Detail row for selected agent
        if is_selected {
            if let Some(row) = activity_row {
                if !row.task.is_empty() {
                    let snippet_max = content_width.saturating_sub(6);
                    let snippet = truncate_middle(&row.task, snippet_max);
                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default().bg(bg)),
                        Span::styled(
                            format!("task: {snippet}"),
                            Style::default().fg(theme.muted).bg(bg),
                        ),
                    ]));
                }
            }
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

// ---------------------------------------------------------------------------
// Summary line (2 lines)
// ---------------------------------------------------------------------------

fn render_summary_line(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let active_count = data
        .agents
        .iter()
        .filter(|a| a.status == "running" || a.status == "active")
        .count();
    let total_agents = data.agents.len();
    let total_tokens = tui_state.cumulative_input_tokens + tui_state.cumulative_output_tokens;
    let cost = tui_state.cumulative_cost_usd;

    let line1 = Line::from(vec![
        Span::styled(" agents: ", Style::default().fg(theme.muted)),
        Span::styled(
            format!("{active_count}/{total_agents}"),
            Style::default()
                .fg(if active_count > 0 {
                    theme.accent
                } else {
                    theme.foreground
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  tokens: ", Style::default().fg(theme.muted)),
        Span::styled(
            format_tokens(total_tokens),
            Style::default().fg(theme.foreground),
        ),
        Span::styled("  cost: ", Style::default().fg(theme.muted)),
        Span::styled(
            if cost > 0.001 {
                format!("${:.2}", cost)
            } else {
                "-".to_string()
            },
            Style::default().fg(if cost > 1.0 {
                theme.warning
            } else {
                theme.foreground
            }),
        ),
    ]);

    let sep = "\u{2500}".repeat(area.width.saturating_sub(2) as usize);
    let line2 = Line::from(Span::styled(
        format!(" {sep}"),
        Style::default().fg(Color::Rgb(40, 35, 42)),
    ));

    let para = Paragraph::new(vec![line1, line2]);
    frame.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Token sparkline (6 lines, braille-style inline rendering)
// ---------------------------------------------------------------------------

fn render_token_sparkline(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let total_tokens = tui_state.cumulative_input_tokens + tui_state.cumulative_output_tokens;
    let total_str = format_tokens(total_tokens);

    let border_style = if total_tokens > 0 {
        Style::default().fg(Color::Rgb(100, 65, 85))
    } else {
        theme.muted()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Token Burn ",
            Style::default().fg(theme.accent),
        ))
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 10 || inner.height < 2 {
        return;
    }

    let inner_w = inner.width as usize;

    // Build sparkline from efficiency events (aggregate by 10-event buckets)
    let mut buckets: Vec<u64> = Vec::new();
    let bucket_size = 10usize.max(1);
    let mut bucket_accum: u64 = 0;
    let mut bucket_count: usize = 0;
    for event in &data.efficiency_events {
        bucket_accum += event.input_tokens + event.output_tokens;
        bucket_count += 1;
        if bucket_count >= bucket_size {
            buckets.push(bucket_accum);
            bucket_accum = 0;
            bucket_count = 0;
        }
    }
    if bucket_count > 0 {
        buckets.push(bucket_accum);
    }

    let mut lines: Vec<Line<'_>> = Vec::new();

    if buckets.len() >= 2 {
        // Simple block chart sparkline
        let max_val = buckets.iter().copied().max().unwrap_or(1).max(1);
        let display: Vec<u64> = if buckets.len() > inner_w {
            buckets[buckets.len() - inner_w..].to_vec()
        } else {
            buckets.clone()
        };

        let spark_chars = [' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}'];

        let label_w = total_str.len() + 2;
        let bar_w = inner_w.saturating_sub(label_w);

        let mut spans: Vec<Span<'_>> = vec![Span::styled(
            format!(" {} ", total_str),
            Style::default().fg(theme.foreground),
        )];

        for i in 0..bar_w {
            let idx = if display.len() > bar_w {
                display.len() - bar_w + i
            } else if i < display.len() {
                i
            } else {
                continue;
            };
            if idx < display.len() {
                let val = display[idx];
                let frac = val as f64 / max_val as f64;
                let ch_idx = (frac * 8.0).round() as usize;
                let ch = spark_chars[ch_idx.min(8)];
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default().fg(theme.accent),
                ));
            }
        }

        lines.push(Line::from(spans));
    } else {
        lines.push(Line::from(Span::styled(
            format!(" {} total tokens  waiting for data...", total_str),
            Style::default().fg(theme.muted),
        )));
    }

    // Per-role token breakdown
    let mut role_tokens: Vec<(String, u64)> = Vec::new();
    for event in &data.efficiency_events {
        let role = event.role.clone();
        let tokens = event.input_tokens + event.output_tokens;
        if let Some(existing) = role_tokens.iter_mut().find(|(r, _)| *r == role) {
            existing.1 += tokens;
        } else {
            role_tokens.push((role, tokens));
        }
    }
    role_tokens.sort_by(|a, b| b.1.cmp(&a.1));

    let remaining_rows = (inner.height as usize).saturating_sub(lines.len());
    for (role, tokens) in role_tokens.iter().take(remaining_rows) {
        let accent = role_accent(role, theme);
        let label = format!(" {:>5} ", &role[..role.len().min(5)]);
        let pct = if total_tokens > 0 {
            (*tokens as f64 / total_tokens as f64 * 100.0).round() as u64
        } else {
            0
        };

        let bar_budget = inner_w.saturating_sub(label.len() + 12);
        let fill_pct = if total_tokens > 0 {
            (*tokens as f64 / total_tokens as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let filled = (fill_pct * bar_budget as f64).round() as usize;
        let empty = bar_budget.saturating_sub(filled);

        lines.push(Line::from(vec![
            Span::styled(label, Style::default().fg(accent)),
            Span::styled(
                "\u{2588}".repeat(filled.min(bar_budget)),
                Style::default().fg(accent),
            ),
            Span::styled(
                "\u{2500}".repeat(empty),
                Style::default().fg(Color::Rgb(40, 35, 42)),
            ),
            Span::styled(
                format!(" {} ({}%)", format_tokens(*tokens), pct),
                Style::default().fg(theme.muted),
            ),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

// ---------------------------------------------------------------------------
// Right panel: role tabs + agent output
// ---------------------------------------------------------------------------

fn render_right_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    _tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let layout = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    // -- Tab bar --
    render_role_tabs(frame, layout[0], data, view_state, theme);

    // -- Output body --
    render_output_body(frame, layout[1], data, view_state, theme);
}

// ---------------------------------------------------------------------------
// Role tabs
// ---------------------------------------------------------------------------

fn render_role_tabs(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    // Determine which roles have active agents
    let agent_roles: Vec<&str> = data.agents.iter().map(|a| a.label.as_str()).collect();

    // Selected role from sub_tab
    let selected_role = ROLE_TABS
        .get(view_state.sub_tab)
        .map(|(role, _)| *role)
        .unwrap_or("");

    let mut spans: Vec<Span<'_>> = Vec::new();
    spans.push(Span::styled(" ", Style::default()));

    for &(role, label) in ROLE_TABS {
        let is_active = role == selected_role;
        let has_agent = agent_roles.iter().any(|r| *r == role);

        let accent = role_accent(role, theme);
        let style = if is_active {
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD)
        } else if has_agent {
            Style::default().fg(accent)
        } else {
            Style::default().fg(theme.muted)
        };

        spans.push(Span::styled(format!(" {label} "), style));
        spans.push(Span::styled(" ", Style::default()));
    }

    let line = Paragraph::new(Line::from(spans));
    frame.render_widget(line, area);
}

// ---------------------------------------------------------------------------
// Output body
// ---------------------------------------------------------------------------

fn render_output_body(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    // Get selected agent's output
    let selected_agent = data.agents.get(view_state.selected);
    let selected_role = selected_agent.map(|a| a.label.as_str()).unwrap_or("");
    let accent = role_accent(selected_role, theme);

    let title_label = if selected_role.is_empty() {
        "Agent Output".to_string()
    } else {
        format!("Output \u{00b7} {}", selected_role)
    };

    let tail_indicator = if view_state.auto_tail {
        " TAIL"
    } else {
        " PINNED"
    };

    let (border_style, title_style) = if selected_agent.map_or(false, |a| {
        a.status == "running" || a.status == "active"
    }) {
        (
            Style::default().fg(accent),
            Style::default()
                .fg(accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (theme.muted(), theme.muted())
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(vec![
            Span::styled(format!(" {title_label}"), title_style),
            Span::styled(
                format!(" [{tail_indicator}] "),
                if view_state.auto_tail {
                    Style::default().fg(theme.success)
                } else {
                    Style::default().fg(theme.warning)
                },
            ),
        ])
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Gather output lines
    let output_lines: Vec<&str> = data
        .current_plan_execution
        .as_ref()
        .map(|exec| exec.agent_output_tail.iter().map(String::as_str).collect())
        .unwrap_or_default();

    if output_lines.is_empty() {
        // Centered empty state
        let v_pad = inner.height / 2;
        let mut empty_lines: Vec<Line<'_>> = Vec::new();
        for _ in 0..v_pad.saturating_sub(2) {
            empty_lines.push(Line::from(""));
        }
        empty_lines.push(Line::from(Span::styled(
            "waiting for agent output...",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        )));
        empty_lines.push(Line::from(""));
        empty_lines.push(Line::from(Span::styled(
            "output will stream here when agents are active",
            Style::default().fg(theme.muted),
        )));
        let empty = Paragraph::new(empty_lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let text: Vec<Line<'_>> = output_lines
        .iter()
        .map(|line| {
            Line::from(vec![
                Span::raw(" "),
                Span::styled(*line, Style::default().fg(theme.foreground)),
            ])
        })
        .collect();

    let scroll = if view_state.auto_tail {
        text.len().saturating_sub(inner.height as usize) as u16
    } else {
        view_state.scroll
    };

    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, inner);
}

// ---------------------------------------------------------------------------
// Gradient gauge bar (inline, matching Mori's per-cell gradient)
// ---------------------------------------------------------------------------

fn gradient_bar<'a>(width: usize, fill_pct: f64, active: bool, theme: &Theme) -> Vec<Span<'a>> {
    if width == 0 {
        return Vec::new();
    }
    let pct = fill_pct.clamp(0.0, 1.0);
    let filled = (pct * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);

    let mut spans = Vec::with_capacity(filled + 1);

    // Gradient from theme.success -> theme.accent -> theme.warning -> theme.danger
    for i in 0..filled {
        let t = if filled > 1 {
            i as f64 / (filled - 1) as f64
        } else {
            pct
        };
        let color = gradient_sample(t, active, theme);
        spans.push(Span::styled(
            "\u{2588}".to_string(),
            Style::default().fg(color),
        ));
    }
    if empty > 0 {
        spans.push(Span::styled(
            "\u{2500}".repeat(empty),
            Style::default().fg(Color::Rgb(40, 35, 42)),
        ));
    }
    spans
}

/// Sample a gradient color from teal -> accent -> warning based on position.
fn gradient_sample(t: f64, active: bool, theme: &Theme) -> Color {
    let (r0, g0, b0) = color_to_rgb(theme.info);
    let (r1, g1, b1) = color_to_rgb(theme.accent);
    let (r2, g2, b2) = color_to_rgb(theme.warning);

    let (r, g, b) = if t < 0.5 {
        let s = t * 2.0;
        lerp_rgb((r0, g0, b0), (r1, g1, b1), s)
    } else {
        let s = (t - 0.5) * 2.0;
        lerp_rgb((r1, g1, b1), (r2, g2, b2), s)
    };

    // Breathing effect for active agents
    let (r, g, b) = if active {
        let br = 1.1;
        (
            (r as f64 * br).min(255.0) as u8,
            (g as f64 * br).min(255.0) as u8,
            (b as f64 * br).min(255.0) as u8,
        )
    } else {
        (r, g, b)
    };

    Color::Rgb(r, g, b)
}

fn lerp_rgb(from: (u8, u8, u8), to: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    (
        (from.0 as f64 + (to.0 as f64 - from.0 as f64) * t) as u8,
        (from.1 as f64 + (to.1 as f64 - from.1 as f64) * t) as u8,
        (from.2 as f64 + (to.2 as f64 - from.2 as f64) * t) as u8,
    )
}

fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (128, 128, 128),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Role-specific accent color.
fn role_accent(role: &str, theme: &Theme) -> Color {
    match role.to_lowercase().as_str() {
        "implementer" | "impl" => Color::Rgb(185, 120, 148), // rose
        "strategist" | "strat" => Color::Rgb(120, 115, 165), // indigo
        "architect" | "arch" => Color::Rgb(125, 158, 140),   // sage
        "auditor" | "audit" => Color::Rgb(195, 155, 95),     // amber
        "critic" | "crit" => Color::Rgb(195, 110, 85),       // ember
        "conductor" | "cond" => Color::Rgb(155, 130, 175),   // lavender
        "researcher" | "res" => Color::Rgb(100, 150, 170),   // teal
        _ => theme.accent,
    }
}

/// Format a token count as compact string.
fn format_tokens(n: u64) -> String {
    if n == 0 {
        "-".to_string()
    } else if n < 1_000 {
        format!("{n}")
    } else if n < 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else if n < 1_000_000 {
        format!("{}k", n / 1_000)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// Shorten a model slug for compact display.
fn shorten_model(slug: &str) -> String {
    slug.replace("claude-", "")
        .replace("gpt-", "")
        .replace("-codex", "c")
        .replace("-mini", "m")
        .replace("sonnet-", "s")
        .replace("opus-", "o")
        .replace("haiku-", "h")
}

/// Truncate in the middle with ellipsis if too long.
fn truncate_middle(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max <= 3 {
        return chars[..max].iter().collect();
    }
    let keep_left = (max - 1) / 2;
    let keep_right = max - keep_left - 1;
    let left: String = chars[..keep_left].iter().collect();
    let right: String = chars[chars.len() - keep_right..].iter().collect();
    format!("{left}\u{2026}{right}")
}
