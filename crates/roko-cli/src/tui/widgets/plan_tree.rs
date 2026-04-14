//! Collapsible plan tree widget with wave grouping, inline progress bars,
//! fixed-column layout, scrollbar, and data-rain fill for empty space.
//!
//! Ported from Mori's `plan_tree.rs` (~1078 LOC) adapted to roko's
//! `TuiState`, `PlanEntry`, and `Wave` types.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::super::state::{PlanEntry, TuiState};
use super::rosedust::{MoriTheme, gradient_ocean};

// ---------------------------------------------------------------------------
// Fixed column widths (chars)
// ---------------------------------------------------------------------------

const COL_PROGRESS: usize = 6;
const COL_BAR: usize = 8;
const COL_DELTA: usize = 8;
const COL_VERIFY: usize = 3;
const COL_AGE: usize = 6;
/// Total reserved width for columns + 5 separator chars.
const RESERVED: u16 = (COL_PROGRESS + COL_BAR + COL_DELTA + COL_VERIFY + COL_AGE + 5) as u16;

// ---------------------------------------------------------------------------
// Status icons
// ---------------------------------------------------------------------------

/// Status icon and style for a plan entry.
fn plan_icon(plan: &PlanEntry) -> (&'static str, Style) {
    if !plan.active && (plan.phase == "done" || plan.phase == "completed") {
        (
            "\u{2713}", // ✓
            Style::default().fg(MoriTheme::SAGE),
        )
    } else if !plan.active && plan.phase == "failed" {
        (
            "\u{2717}", // ✗
            Style::default()
                .fg(MoriTheme::EMBER)
                .add_modifier(Modifier::BOLD),
        )
    } else if plan.active {
        (
            "\u{25b6}", // ▶
            Style::default()
                .fg(MoriTheme::WARNING)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            "\u{25cb}", // ○
            Style::default().fg(MoriTheme::TEXT_GHOST),
        )
    }
}

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the collapsible plan tree: Wave -> Plan hierarchy.
/// Falls back to flat list when no waves are configured.
pub fn render_plan_tree(frame: &mut Frame<'_>, area: Rect, state: &TuiState, focused: bool) {
    let total = state.plans.len();
    let completed = state
        .plans
        .iter()
        .filter(|p| !p.active && p.tasks_failed == 0)
        .count();
    let active = state.plans.iter().filter(|p| p.active).count();
    let failed = state
        .plans
        .iter()
        .filter(|p| !p.active && p.phase == "failed")
        .count();

    let health_suffix = {
        let mut parts = Vec::new();
        if active > 0 {
            parts.push(format!("{active}\u{25b8}")); // ▸
        }
        if failed > 0 {
            parts.push(format!("{failed}\u{2717}")); // ✗
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!(" {}", parts.join(" "))
        }
    };

    let active_filter = active_filter_text(state);
    let filtered_plan_indices = filtered_plan_indices(state, active_filter.as_deref());
    let filtered_total = filtered_plan_indices.len();
    let selected_plan_idx = clamped_selected_plan_idx(state.selected_plan_idx, filtered_total);
    let selected_plan_id = filtered_plan_indices
        .get(selected_plan_idx)
        .and_then(|&idx| state.plans.get(idx))
        .map(|plan| plan.id.as_str());
    let filtered_suffix = if active_filter.is_some() {
        format!(", {filtered_total}/{total} filtered")
    } else {
        String::new()
    };

    let title = if focused {
        format!(
            "Plans ({completed}/{total}{health_suffix}{filtered_suffix}) [Enter:detail h/l:tree]"
        )
    } else {
        format!("Plans ({completed}/{total}{health_suffix}{filtered_suffix})")
    };

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Filter indicator
    if active_filter.is_some() {
        lines.push(Line::from(vec![
            Span::styled(" /", Style::default().fg(MoriTheme::DREAM)),
            Span::styled(state.filter.clone(), Style::default().fg(MoriTheme::BONE)),
            Span::styled("/ ", Style::default().fg(MoriTheme::DREAM)),
        ]));
    }

    // Column header
    if area.width >= 32 {
        lines.push(render_column_header(area));
    }

    if state.execution_waves.is_empty() {
        render_flat_plans(
            &mut lines,
            state,
            focused,
            area,
            selected_plan_id,
            active_filter.as_deref(),
        );
    } else {
        render_wave_tree(
            &mut lines,
            state,
            focused,
            area,
            selected_plan_id,
            active_filter.as_deref(),
        );
    }

    // Border styling
    let (border_style, title_style) = if focused {
        (
            MoriTheme::focused_border_style(),
            MoriTheme::focused_title_style(),
        )
    } else {
        (
            MoriTheme::unfocused_border_style(),
            MoriTheme::unfocused_title_style(),
        )
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(MoriTheme::block_style())
        .border_style(border_style)
        .title_style(title_style);

    let visible_height = area.height.saturating_sub(2) as usize;
    let total_lines = lines.len();

    // Scroll to keep selected visible
    let scroll_offset = state
        .plan_scroll
        .min(total_lines.saturating_sub(visible_height));
    let visible: Vec<Line> = lines
        .into_iter()
        .skip(scroll_offset)
        .take(visible_height)
        .collect();

    let paragraph = Paragraph::new(visible).block(block);
    frame.render_widget(paragraph, area);

    // (data-rain visualization removed — kept empty space clean)

    // Scrollbar
    if total_lines > visible_height {
        let inner = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );
        render_scrollbar(frame, inner, total_lines, visible_height, scroll_offset);
    }
}

// ---------------------------------------------------------------------------
// Wave tree rendering
// ---------------------------------------------------------------------------

fn render_wave_tree(
    lines: &mut Vec<Line<'static>>,
    state: &TuiState,
    focused: bool,
    area: Rect,
    selected_plan_id: Option<&str>,
    filter_lower: Option<&str>,
) {
    for wave in &state.execution_waves {
        let wave_plans: Vec<&PlanEntry> = wave
            .plans
            .iter()
            .filter_map(|plan_id| state.plans.iter().find(|p| p.id == *plan_id))
            .filter(|plan| matches_filter(plan, filter_lower))
            .collect();
        if wave_plans.is_empty() {
            continue;
        }

        let all_done = wave.done == wave.total && wave.total > 0;
        let any_active = wave
            .plans
            .iter()
            .any(|plan_id| state.plans.iter().any(|p| p.id == *plan_id && p.active));

        // Wave header icon and style
        let (wave_icon, wave_style) = if all_done {
            (
                "\u{2713}", // ✓
                Style::default().fg(MoriTheme::SAGE),
            )
        } else if any_active {
            (
                "\u{25b6}", // ►
                Style::default()
                    .fg(MoriTheme::ROSE)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                "\u{00b7}", // ·
                Style::default().fg(MoriTheme::TEXT_GHOST),
            )
        };

        let collapse_icon = if wave.expanded {
            "\u{25be}"
        } else {
            "\u{25b8}"
        }; // ▾ / ▸

        let mut wave_spans = vec![
            Span::styled(
                format!(" {collapse_icon} "),
                Style::default().fg(MoriTheme::FG_DIM),
            ),
            Span::styled(format!("{wave_icon} "), wave_style),
            Span::styled(
                format!("Wave {} ", wave.index),
                Style::default()
                    .fg(MoriTheme::BONE_DIM)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("({}/{}) ", wave.done, wave.total),
                Style::default().fg(MoriTheme::FG_DIM),
            ),
        ];

        // Wave progress bar (8-char gradient bar)
        let wave_fill = wave.done as f64 / wave.total.max(1) as f64;
        wave_spans.extend(render_gradient_bar(
            8,
            wave_fill,
            if any_active {
                Some(state.atmosphere.heartbeat())
            } else {
                None
            },
        ));

        // Count failed plans in this wave
        let wave_failed = wave
            .plans
            .iter()
            .filter(|plan_id| {
                state
                    .plans
                    .iter()
                    .any(|p| p.id == **plan_id && !p.active && p.phase == "failed")
            })
            .count();
        if wave_failed > 0 {
            wave_spans.push(Span::styled(
                format!(" \u{2717}{wave_failed}"),
                Style::default().fg(MoriTheme::EMBER),
            ));
        }

        // Fill remaining width with horizontal line
        let used: usize = wave_spans.iter().map(|s| s.content.chars().count()).sum();
        let avail = (area.width.saturating_sub(2)) as usize;
        if avail > used + 1 {
            wave_spans.push(Span::styled(
                format!(" {}", "\u{2500}".repeat(avail - used - 1)),
                Style::default().fg(MoriTheme::TEXT_GHOST),
            ));
        }
        lines.push(Line::from(wave_spans));

        if !wave.expanded {
            continue;
        }

        // Plans within wave
        for plan in wave_plans {
            render_plan_line(lines, plan, focused, area, true, selected_plan_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Flat plan list (no waves)
// ---------------------------------------------------------------------------

fn render_flat_plans(
    lines: &mut Vec<Line<'static>>,
    state: &TuiState,
    focused: bool,
    area: Rect,
    selected_plan_id: Option<&str>,
    filter_lower: Option<&str>,
) {
    for plan in &state.plans {
        if matches_filter(plan, filter_lower) {
            render_plan_line(lines, plan, focused, area, false, selected_plan_id);
        }
    }
}

// ---------------------------------------------------------------------------
// Single plan line
// ---------------------------------------------------------------------------

fn render_plan_line(
    lines: &mut Vec<Line<'static>>,
    plan: &PlanEntry,
    focused: bool,
    area: Rect,
    indented: bool,
    selected_plan_id: Option<&str>,
) {
    let is_selected = focused && selected_plan_id == Some(plan.id.as_str());

    let (icon, icon_style) = plan_icon(plan);

    // Text styling by plan status
    let text_style = if !plan.active && (plan.phase == "done" || plan.phase == "completed") {
        Style::default().fg(MoriTheme::SAGE)
    } else if plan.active {
        Style::default()
            .fg(MoriTheme::ROSE_BRIGHT)
            .add_modifier(Modifier::BOLD)
    } else if plan.phase == "failed" {
        Style::default().fg(MoriTheme::EMBER)
    } else {
        Style::default().fg(MoriTheme::TEXT_DIM)
    };

    let bg = if is_selected {
        MoriTheme::BG_HIGHLIGHT
    } else {
        MoriTheme::BG
    };
    let icon_s = if is_selected {
        icon_style.bg(bg)
    } else {
        icon_style
    };
    let text_s = if is_selected {
        text_style.bg(bg)
    } else {
        text_style
    };

    let indent = if indented { "   " } else { " " };

    // -- Task progress fraction --
    let fill_pct = if plan.tasks_total > 0 {
        plan.tasks_done as f64 / plan.tasks_total as f64
    } else {
        0.0
    };

    // Progress cell (6 chars): e.g. " 3/10"
    let progress_cell = if plan.tasks_total > 0 {
        let color = if !plan.active && plan.tasks_failed == 0 {
            MoriTheme::SAGE
        } else if plan.active {
            if fill_pct >= 0.999 {
                MoriTheme::WARNING
            } else {
                MoriTheme::semantic_color(fill_pct)
            }
        } else if plan.phase == "failed" {
            MoriTheme::EMBER
        } else if plan.tasks_done == 0 {
            MoriTheme::TEXT_GHOST
        } else {
            MoriTheme::TEXT_DIM
        };
        (
            format!(
                "{:>width$}",
                format!("{}/{}", plan.tasks_done.min(99), plan.tasks_total.min(99)),
                width = COL_PROGRESS
            ),
            color,
        )
    } else {
        (
            format!("{:>width$}", "\u{00b7}", width = COL_PROGRESS),
            MoriTheme::TEXT_PHANTOM,
        )
    };

    // Bar cell (8 chars): filled blocks + dashes
    let bar_cell = if plan.tasks_total > 0 {
        let filled = ((fill_pct.clamp(0.0, 1.0)) * COL_BAR as f64).round() as usize;
        let empty = COL_BAR.saturating_sub(filled);
        let bar_color = if !plan.active && plan.tasks_failed == 0 {
            MoriTheme::SAGE
        } else if plan.active && fill_pct >= 0.999 {
            MoriTheme::WARNING
        } else if plan.phase == "failed" {
            MoriTheme::EMBER
        } else if plan.tasks_done == 0 && !plan.active {
            MoriTheme::TEXT_PHANTOM
        } else {
            MoriTheme::semantic_color(fill_pct)
        };
        (
            format!(
                "{}{}",
                "\u{2588}".repeat(filled.min(COL_BAR)),
                "\u{2500}".repeat(empty)
            ),
            bar_color,
        )
    } else {
        (
            format!("{:>width$}", "\u{00b7}", width = COL_BAR),
            MoriTheme::TEXT_PHANTOM,
        )
    };

    // Delta cell (8 chars): git dirty or health indicator
    let delta_cell = if plan.tasks_failed > 0 {
        (
            format!(
                "{:>width$}",
                truncate_middle(&format!("\u{2717}{}", plan.tasks_failed), COL_DELTA),
                width = COL_DELTA
            ),
            MoriTheme::EMBER,
        )
    } else {
        (
            format!("{:>width$}", "\u{00b7}", width = COL_DELTA),
            MoriTheme::TEXT_PHANTOM,
        )
    };

    // Verify cell (3 chars): placeholder
    let verify_cell = (
        format!("{:>width$}", "\u{00b7}", width = COL_VERIFY),
        MoriTheme::TEXT_PHANTOM,
    );

    // Age cell (6 chars): elapsed time
    let age_cell = if plan.elapsed_secs > 0.0 {
        (
            format!(
                "{:>width$}",
                truncate_middle(&format_duration(plan.elapsed_secs), COL_AGE),
                width = COL_AGE
            ),
            MoriTheme::TEXT_DIM,
        )
    } else {
        (
            format!("{:>width$}", "\u{00b7}", width = COL_AGE),
            MoriTheme::TEXT_PHANTOM,
        )
    };

    // Phase abbreviation for active plans
    let phase_suffix = if plan.active && !plan.phase.is_empty() {
        format!(" {}", phase_abbrev(&plan.phase))
    } else {
        String::new()
    };

    let prefix_plain = format!("{indent}{icon} ");
    let available_title = area
        .width
        .saturating_sub(prefix_plain.len() as u16)
        .saturating_sub(RESERVED)
        .max(10) as usize;
    let name_budget = available_title.saturating_sub(phase_suffix.len());
    let plan_name = truncate_middle(&plan.name, name_budget);
    let title_text = format!("{plan_name}{phase_suffix}");

    // Title span — color phase suffix differently for active plans
    let title_span = if phase_suffix.is_empty() {
        Span::styled(
            format!("{title_text:<width$}", width = available_title),
            text_s.bg(bg),
        )
    } else {
        let phase_color = MoriTheme::phase_accent(&plan.phase);
        let padded = format!("{title_text:<width$}", width = available_title);
        let style = if plan.active {
            text_s.fg(phase_color).bg(bg)
        } else {
            text_s.bg(bg)
        };
        Span::styled(padded, style)
    };

    let sep = |bg_c: Color| {
        Span::styled(
            "\u{2502}",
            Style::default().fg(MoriTheme::TEXT_PHANTOM).bg(bg_c),
        )
    };

    let mut spans = vec![Span::styled(prefix_plain, icon_s.bg(bg)), title_span];
    spans.extend([
        sep(bg),
        Span::styled(progress_cell.0, Style::default().fg(progress_cell.1).bg(bg)),
        sep(bg),
        Span::styled(bar_cell.0, Style::default().fg(bar_cell.1).bg(bg)),
        sep(bg),
        Span::styled(delta_cell.0, Style::default().fg(delta_cell.1).bg(bg)),
        sep(bg),
        Span::styled(verify_cell.0, Style::default().fg(verify_cell.1).bg(bg)),
        sep(bg),
        Span::styled(age_cell.0, Style::default().fg(age_cell.1).bg(bg)),
    ]);

    lines.push(Line::from(spans));

    // Selected plan detail row (expanded info)
    if is_selected {
        let mut detail_spans = vec![Span::styled(format!("{indent}  "), Style::default().bg(bg))];

        // Mini progress bar in the detail row
        if plan.tasks_total > 0 {
            detail_spans.push(Span::styled(
                compact_progress_glyphs(8, fill_pct),
                Style::default()
                    .fg(MoriTheme::semantic_color(fill_pct))
                    .bg(bg),
            ));
            detail_spans.push(Span::styled(
                "  ",
                Style::default().fg(MoriTheme::TEXT_PHANTOM).bg(bg),
            ));
        }

        let mut detail_parts: Vec<(String, Color)> = Vec::new();
        if !plan.phase.is_empty() {
            detail_parts.push((format!("phase {}", plan.phase), MoriTheme::ROSE_DIM));
        }
        if plan.tasks_failed > 0 {
            detail_parts.push((format!("{} failed", plan.tasks_failed), MoriTheme::EMBER));
        }
        if plan.elapsed_secs > 0.0 {
            detail_parts.push((
                format!("elapsed {}", format_duration(plan.elapsed_secs)),
                MoriTheme::TEXT_GHOST,
            ));
        }

        for (idx, (text, color)) in detail_parts.iter().enumerate() {
            if idx > 0 {
                detail_spans.push(Span::styled(
                    " \u{00b7} ",
                    Style::default().fg(MoriTheme::TEXT_PHANTOM).bg(bg),
                ));
            }
            detail_spans.push(Span::styled(
                truncate_middle(text, area.width.saturating_sub(8) as usize),
                Style::default().fg(*color).bg(bg),
            ));
        }
        if detail_spans.len() > 1 {
            lines.push(Line::from(detail_spans));
        }
    }
}

// ---------------------------------------------------------------------------
// Column header
// ---------------------------------------------------------------------------

fn render_column_header(area: Rect) -> Line<'static> {
    let reserved: u16 = (COL_PROGRESS + COL_BAR + COL_DELTA + COL_VERIFY + COL_AGE + 5) as u16;
    let title_width = area.width.saturating_sub(reserved + 3).max(10) as usize;
    let sep = Span::styled("\u{2502}", Style::default().fg(MoriTheme::TEXT_PHANTOM));
    Line::from(vec![
        Span::styled(
            format!(" {:<width$}", "plan", width = title_width),
            Style::default().fg(MoriTheme::TEXT_GHOST),
        ),
        sep.clone(),
        Span::styled(
            format!("{:>6}", "prog"),
            Style::default().fg(MoriTheme::TEXT_PHANTOM),
        ),
        sep.clone(),
        Span::styled(
            format!("{:>8}", "bar"),
            Style::default().fg(MoriTheme::TEXT_PHANTOM),
        ),
        sep.clone(),
        Span::styled(
            format!("{:>8}", "delta"),
            Style::default().fg(MoriTheme::TEXT_PHANTOM),
        ),
        sep.clone(),
        Span::styled(
            format!("{:>3}", "vfy"),
            Style::default().fg(MoriTheme::TEXT_PHANTOM),
        ),
        sep,
        Span::styled(
            format!("{:>6}", "age"),
            Style::default().fg(MoriTheme::TEXT_PHANTOM),
        ),
    ])
}

// ---------------------------------------------------------------------------
// Gradient bar spans (ocean gradient)
// ---------------------------------------------------------------------------

fn render_gradient_bar(width: usize, fill_pct: f64, heartbeat: Option<f64>) -> Vec<Span<'static>> {
    let grad = gradient_ocean();
    let filled = ((fill_pct.clamp(0.0, 1.0)) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);

    let mut spans = Vec::with_capacity(width);
    for i in 0..filled.min(width) {
        let t = if filled <= 1 {
            0.5
        } else {
            i as f64 / (filled - 1).max(1) as f64
        };
        let mut color = grad.sample(t);
        if let Some(hb) = heartbeat {
            color = super::rosedust::brighten(color, hb);
        }
        spans.push(Span::styled(
            "\u{2588}".to_string(),
            Style::default().fg(color),
        ));
    }
    if empty > 0 {
        spans.push(Span::styled(
            "\u{2500}".repeat(empty),
            Style::default().fg(MoriTheme::TEXT_PHANTOM),
        ));
    }
    spans
}

// ---------------------------------------------------------------------------
// Data-rain fill for empty space
// ---------------------------------------------------------------------------

fn render_data_rain(frame: &mut Frame<'_>, area: Rect, elapsed: f64, progress: f64) {
    // Subtle animated rain effect — density increases with progress
    let base_density = 0.02 + progress * 0.08;
    let buf = frame.buffer_mut();

    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            // Pseudo-random using position and time
            let seed = (x as f64 * 13.37 + y as f64 * 7.31 + elapsed * 2.0).sin();
            if seed.abs() < base_density {
                let ch = match ((seed * 1000.0).abs() as usize) % 4 {
                    0 => '\u{00b7}', // ·
                    1 => '\u{2502}', // │
                    2 => '\u{2500}', // ─
                    _ => '\u{00b0}', // °
                };
                let brightness = 0.3 + (seed.abs() * 0.7);
                let color = super::rosedust::brighten(MoriTheme::TEXT_PHANTOM, brightness);
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(ch);
                    cell.set_fg(color);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scrollbar (lightweight, buffer-direct)
// ---------------------------------------------------------------------------

fn render_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    total: usize,
    visible: usize,
    offset: usize,
) {
    if total <= visible || area.height == 0 {
        return;
    }

    let track_height = area.height as usize;
    let thumb_height = ((visible as f64 / total as f64) * track_height as f64)
        .ceil()
        .max(1.0) as usize;
    let thumb_top = if total > visible {
        ((offset as f64 / (total - visible) as f64) * (track_height - thumb_height) as f64).round()
            as usize
    } else {
        0
    };

    let x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();

    for i in 0..track_height {
        let y = area.y + i as u16;
        let in_thumb = i >= thumb_top && i < thumb_top + thumb_height;
        let (ch, color) = if in_thumb {
            ('\u{2588}', MoriTheme::ROSE_DIM) // █
        } else {
            ('\u{2502}', MoriTheme::TEXT_PHANTOM) // │
        };
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char(ch);
            cell.set_fg(color);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn active_filter_text(state: &TuiState) -> Option<String> {
    let filter = state.filter.trim();
    if state.filter_active && !filter.is_empty() {
        Some(filter.to_lowercase())
    } else {
        None
    }
}

fn filtered_plan_indices(state: &TuiState, filter_lower: Option<&str>) -> Vec<usize> {
    state
        .plans
        .iter()
        .enumerate()
        .filter_map(|(idx, plan)| matches_filter(plan, filter_lower).then_some(idx))
        .collect()
}

fn clamped_selected_plan_idx(selected_plan_idx: usize, filtered_total: usize) -> usize {
    if filtered_total == 0 {
        0
    } else {
        selected_plan_idx.min(filtered_total - 1)
    }
}

fn matches_filter(plan: &PlanEntry, filter_lower: Option<&str>) -> bool {
    let Some(filter_lower) = filter_lower else {
        return true;
    };
    plan.name.to_lowercase().contains(filter_lower)
}

fn compact_progress_glyphs(width: usize, fill_pct: f64) -> String {
    if width == 0 {
        return String::new();
    }
    let filled = ((fill_pct.clamp(0.0, 1.0)) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!(
        " {}{}",
        "\u{2588}".repeat(filled.min(width)),
        "\u{2500}".repeat(empty)
    )
}

fn phase_abbrev(phase: &str) -> &'static str {
    match phase {
        p if p.contains("preflight") => "prfl",
        p if p.contains("strateg") => "strt",
        p if p.contains("implement") => "impl",
        p if p.contains("verify") => "vfy ",
        p if p.contains("merge") => "mrge",
        p if p.contains("complete") || p.contains("done") => "done",
        p if p.contains("fail") => "fail",
        p if p.contains("compile") => "comp",
        p if p.contains("test") => "test",
        p if p.contains("review") => "revw",
        p if p.contains("critic") => "crit",
        p if p.contains("verdict") => "vdct",
        p if p.contains("doc") => "docs",
        p if p.contains("commit") => "cmit",
        p if p.contains("gate") || p.contains("gating") => "gate",
        _ => "run ",
    }
}

/// Format seconds as compact duration: "30m", "2h", "1d", "45s".
fn format_duration(secs: f64) -> String {
    let s = secs as u64;
    if s >= 86400 {
        format!("{}d", s / 86400)
    } else if s >= 3600 {
        format!("{}h", s / 3600)
    } else if s >= 60 {
        format!("{}m", s / 60)
    } else {
        format!("{}s", s)
    }
}

fn truncate_middle(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 3 {
        return "\u{2026}".repeat(max);
    }
    let keep_left = (max - 1) / 2;
    let keep_right = max - keep_left - 1;
    let left: String = s.chars().take(keep_left).collect();
    let right: String = s
        .chars()
        .rev()
        .take(keep_right)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{left}\u{2026}{right}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::tui::state::Wave;

    fn sample_state() -> TuiState {
        use crate::tui::dashboard::DashboardData;
        use crate::tui::state::TuiState;

        let data = DashboardData::default();
        let mut state = TuiState::from_dashboard_data(&data);
        state.plans = vec![
            PlanEntry {
                id: "plan-alpha".into(),
                name: "plan-alpha".into(),
                wave: Some(0),
                tasks_total: 5,
                tasks_done: 3,
                tasks_failed: 0,
                active: true,
                phase: "implementing".into(),
                elapsed_secs: 120.0,
                ..Default::default()
            },
            PlanEntry {
                id: "plan-beta".into(),
                name: "plan-beta".into(),
                wave: Some(0),
                tasks_total: 4,
                tasks_done: 4,
                tasks_failed: 0,
                active: false,
                phase: "done".into(),
                elapsed_secs: 300.0,
                ..Default::default()
            },
            PlanEntry {
                id: "plan-gamma".into(),
                name: "plan-gamma".into(),
                wave: Some(1),
                tasks_total: 3,
                tasks_done: 1,
                tasks_failed: 2,
                active: false,
                phase: "failed".into(),
                elapsed_secs: 60.0,
                ..Default::default()
            },
        ];
        state.execution_waves = vec![
            Wave {
                index: 0,
                plans: vec!["plan-alpha".into(), "plan-beta".into()],
                done: 1,
                total: 2,
                expanded: true,
            },
            Wave {
                index: 1,
                plans: vec!["plan-gamma".into()],
                done: 0,
                total: 1,
                expanded: true,
            },
        ];
        state
    }

    #[test]
    fn plan_tree_renders_without_panic() {
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = sample_state();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_plan_tree(frame, area, &state, true);
            })
            .unwrap();
    }

    #[test]
    fn plan_tree_flat_fallback() {
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = sample_state();
        state.execution_waves.clear();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_plan_tree(frame, area, &state, false);
            })
            .unwrap();
    }

    #[test]
    fn plan_tree_empty() {
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = sample_state();
        state.plans.clear();
        state.execution_waves.clear();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_plan_tree(frame, area, &state, false);
            })
            .unwrap();
    }

    #[test]
    fn truncate_middle_edge_cases() {
        assert_eq!(truncate_middle("hello", 10), "hello");
        assert_eq!(truncate_middle("hello world", 5), "he\u{2026}ld");
        assert_eq!(truncate_middle("abc", 0), "");
        assert_eq!(truncate_middle("abcdef", 3), "\u{2026}\u{2026}\u{2026}");
    }

    #[test]
    fn phase_abbrev_known() {
        assert_eq!(phase_abbrev("implementing"), "impl");
        assert_eq!(phase_abbrev("compile-gate"), "comp");
        assert_eq!(phase_abbrev("verifying"), "vfy ");
    }

    #[test]
    fn format_duration_ranges() {
        assert_eq!(format_duration(30.0), "30s");
        assert_eq!(format_duration(120.0), "2m");
        assert_eq!(format_duration(7200.0), "2h");
        assert_eq!(format_duration(90000.0), "1d");
    }

    #[test]
    fn progress_glyphs_bounds() {
        let empty = compact_progress_glyphs(0, 0.5);
        assert!(empty.is_empty());
        let full = compact_progress_glyphs(4, 1.0);
        assert!(full.contains('\u{2588}'));
    }
}
