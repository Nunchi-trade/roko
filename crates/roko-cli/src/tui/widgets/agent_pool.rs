//! Agent pool widget — combines Mori's agent_pool + parallel_pool into
//! a single compact roster with per-agent context gauges.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::super::mori_theme::{Gradient, MoriTheme, brighten, gradient_context};
use super::super::tui_state::TuiState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format a token count as compact string: 0, 1.2k, 45k, 120k, 1.2M
fn fmt_tokens(n: u64) -> String {
    if n == 0 {
        "0".to_string()
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

/// Truncate in the middle with `...` if too long.
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

// ---------------------------------------------------------------------------
// Gradient gauge bar
// ---------------------------------------------------------------------------

/// Render a per-cell gradient gauge bar.
fn gradient_bar(
    width: usize,
    fill_pct: f64,
    gradient: &Gradient,
    breathing: Option<f64>,
) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let pct = fill_pct.clamp(0.0, 1.0);
    let filled = (pct * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);

    let mut spans = Vec::with_capacity(filled + 1);
    for i in 0..filled {
        let t = if filled > 1 {
            i as f64 / (filled - 1) as f64
        } else {
            pct
        };
        let mut color = gradient.sample(t);
        if let Some(br) = breathing {
            color = brighten(color, br);
        }
        spans.push(Span::styled("\u{2588}", Style::default().fg(color)));
    }
    if empty > 0 {
        spans.push(Span::styled(
            "\u{2500}".repeat(empty),
            Style::default().fg(MoriTheme::TEXT_GHOST),
        ));
    }
    spans
}

// ---------------------------------------------------------------------------
// Public render
// ---------------------------------------------------------------------------

/// Render the agent pool widget.
pub fn render_agent_pool(frame: &mut Frame<'_>, area: Rect, state: &TuiState, focused: bool) {
    let gradient = gradient_context();
    let breathing = Some(state.atmosphere.breathing_brightness());

    let content_width = area.width.saturating_sub(2) as usize;
    let gauge_width = 11usize;

    let mut lines: Vec<Line> = Vec::new();

    // Header row
    if area.height > 3 {
        let role_w = 11usize;
        let plan_task_w = content_width.saturating_div(3).clamp(16, 30);
        lines.push(Line::from(vec![
            Span::styled("   ", Style::default()),
            Span::styled(
                format!("{:<role_w$}", "role"),
                Style::default().fg(MoriTheme::TEXT_GHOST),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(
                format!("{:<plan_task_w$}", "plan \u{00b7} task"),
                Style::default().fg(MoriTheme::TEXT_GHOST),
            ),
            Span::styled(" ctx", Style::default().fg(MoriTheme::TEXT_GHOST)),
        ]));
    }

    for (idx, agent) in state.agents.iter().enumerate() {
        if !agent.active && agent.input_tokens == 0 && agent.output_tokens == 0 {
            continue;
        }

        let selected = idx == state.selected_agent;
        let accent = MoriTheme::role_accent(&agent.role);
        let bg = if selected {
            MoriTheme::BG_HIGHLIGHT
        } else {
            MoriTheme::BG
        };

        // Context fill percentage
        let ctx_limit = agent.context_limit.max(1);
        let total_tokens = agent.input_tokens + agent.output_tokens;
        let fill_pct = (total_tokens as f64 / ctx_limit as f64).clamp(0.0, 1.0);

        // Role label + plan:task
        let role_w = 11usize;
        let plan_task_w = content_width.saturating_div(3).clamp(16, 30);
        let bar_w = if content_width > 88 {
            16usize
        } else {
            gauge_width
        };

        let role_label = truncate_middle(&agent.role, role_w);
        let short_model = shorten_model(&agent.model);

        // Plan + task columns
        let separator = " \u{00b7} ";
        let sep_len = separator.len();
        let plan_max = plan_task_w
            .saturating_sub(sep_len + 1)
            .min(plan_task_w / 2 + 2);
        let plan_part = truncate_middle(&agent.current_plan, plan_max);
        let task_budget = plan_task_w.saturating_sub(plan_part.chars().count() + sep_len);
        let task_part = truncate_middle(&agent.current_task, task_budget);
        let combined_len = plan_part.chars().count() + sep_len + task_part.chars().count();
        let pad = plan_task_w.saturating_sub(combined_len);

        // Token counts
        let ctx_str = format!("{:>4}k/{:>3}k", total_tokens / 1000, ctx_limit / 1000);

        // State chip
        let state_chip = if agent.active {
            format!(" {} LIVE ", state.atmosphere.spinner())
        } else if agent.input_tokens > 0 || agent.output_tokens > 0 {
            " \u{2713} DONE ".to_string()
        } else {
            " \u{00b7} idle ".to_string()
        };
        let state_bg = if agent.active {
            accent
        } else if agent.input_tokens > 0 || agent.output_tokens > 0 {
            MoriTheme::SAGE
        } else {
            MoriTheme::BG_SECONDARY
        };
        let state_fg = if agent.active || agent.input_tokens > 0 || agent.output_tokens > 0 {
            MoriTheme::BG
        } else {
            MoriTheme::TEXT_DIM
        };

        let mut spans = vec![
            Span::styled(
                if selected { " \u{25b6} " } else { "   " },
                Style::default().fg(MoriTheme::ROSE).bg(bg),
            ),
            Span::styled(
                format!("{:<role_w$}", role_label),
                Style::default()
                    .fg(accent)
                    .bg(bg)
                    .add_modifier(if agent.active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(
                format!(" {}", plan_part),
                Style::default().fg(MoriTheme::FG_BRIGHT).bg(bg),
            ),
            Span::styled(separator, Style::default().fg(MoriTheme::TEXT_GHOST).bg(bg)),
            Span::styled(
                format!("{}{}", task_part, " ".repeat(pad)),
                Style::default().fg(MoriTheme::BONE_DIM).bg(bg),
            ),
        ];

        // Token count
        spans.push(Span::styled(
            format!(" {}", ctx_str),
            Style::default().fg(MoriTheme::TEXT_DIM).bg(bg),
        ));
        spans.push(Span::styled(" ", Style::default().bg(bg)));

        // Gradient gauge bar
        let bar_spans = gradient_bar(bar_w, fill_pct, &gradient, breathing);
        for span in bar_spans {
            let mut s = span;
            s.style = s.style.bg(bg);
            spans.push(s);
        }

        spans.push(Span::styled(" ", Style::default().bg(bg)));

        // State chip
        spans.push(Span::styled(
            state_chip,
            Style::default()
                .fg(state_fg)
                .bg(state_bg)
                .add_modifier(Modifier::BOLD),
        ));

        // Model tag
        if content_width > 60 {
            spans.push(Span::styled(
                format!(" {}", short_model),
                Style::default().fg(MoriTheme::TEXT_GHOST).bg(bg),
            ));
        }

        lines.push(Line::from(spans));

        // Detail row for selected agent: last output snippet
        if selected && !agent.last_output_line.is_empty() {
            let snippet_max = content_width.saturating_sub(4);
            let snippet = truncate_middle(&agent.last_output_line, snippet_max);
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default().bg(bg)),
                Span::styled(snippet, Style::default().fg(MoriTheme::TEXT_GHOST).bg(bg)),
            ]));
        }
    }

    if lines.is_empty() || (lines.len() == 1 && area.height > 3) {
        lines.push(Line::from(Span::styled(
            "  no agents spawned",
            Style::default().fg(MoriTheme::TEXT_DIM),
        )));
    }

    let active_count = state.active_agent_count();
    let title = format!(" Agents ({} active) ", active_count);

    let (border_s, title_s) = if focused {
        (
            MoriTheme::focused_border_style(),
            MoriTheme::focused_title_style(),
        )
    } else if active_count > 0 {
        (
            Style::default().fg(MoriTheme::ROSE_DIM),
            Style::default()
                .fg(MoriTheme::ROSE)
                .add_modifier(Modifier::BOLD),
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
        .border_style(border_s)
        .title_style(title_s);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}
