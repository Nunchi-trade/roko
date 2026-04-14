//! Bottom status bar — ported from Mori.
//!
//! 4 sections: git info (branch + commit + age), heartbeat + pause indicator,
//! plan progress + health summary, context-sensitive keybind hints.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::state::TuiState;
use super::super::tabs::Tab;
use super::rosedust::MoriTheme;

const HEARTBEAT_FRAMES: [&str; 4] = ["\u{00b7}", "\u{00b0}", ".", "\u{25cf}"];

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the bottom status bar.
pub fn render_status_bar(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let bg = Style::default().bg(MoriTheme::BG_SECONDARY);

    let (done, total) = state.task_counts();
    let all_done = total > 0 && state.plans.iter().all(|p| !p.active);
    let has_failures = state.plans.iter().any(|p| p.tasks_failed > 0);

    let mut spans: Vec<Span<'static>> = vec![Span::styled(" ", bg)];

    // ── 1. Git info: branch, commit hash, last commit time ────────────
    if !state.git_branch.is_empty() {
        spans.push(Span::styled(
            state.git_branch.clone(),
            Style::default()
                .fg(MoriTheme::BONE)
                .bg(MoriTheme::BG_SECONDARY),
        ));
        if !state.git_commit_short.is_empty() {
            spans.push(Span::styled(
                format!(" {}", &state.git_commit_short),
                Style::default()
                    .fg(MoriTheme::TEXT_GHOST)
                    .bg(MoriTheme::BG_SECONDARY),
            ));
        }
        if !state.git_age.is_empty() {
            spans.push(Span::styled(
                format!(" {}", &state.git_age),
                Style::default()
                    .fg(MoriTheme::TEXT_GHOST)
                    .bg(MoriTheme::BG_SECONDARY),
            ));
        }
        spans.push(Span::styled(
            " \u{2502} ",
            Style::default()
                .fg(MoriTheme::ROSE_DIM)
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }

    // ── 2. Heartbeat + pause indicator ────────────────────────────────
    let hb_idx = (state.atmosphere.frame() / 8) as usize % HEARTBEAT_FRAMES.len();
    spans.push(Span::styled(
        HEARTBEAT_FRAMES[hb_idx],
        Style::default()
            .fg(MoriTheme::ROSE_DIM)
            .bg(MoriTheme::BG_SECONDARY),
    ));

    // ── 3. Plan progress + health summary ─────────────────────────────
    let progress_text = if all_done && !has_failures {
        "COMPLETE".to_string()
    } else if has_failures {
        let err_count = state.plans.iter().filter(|p| p.tasks_failed > 0).count();
        format!("ERR:{err_count}")
    } else {
        format!(" {done}/{total}")
    };
    let progress_style = if has_failures {
        MoriTheme::error_style()
    } else if all_done {
        MoriTheme::success_style()
    } else {
        Style::default().fg(MoriTheme::ROSE)
    };
    spans.push(Span::styled(
        format!(" {progress_text} "),
        progress_style.bg(MoriTheme::BG_SECONDARY),
    ));

    // Health summary: active plans, live agents, flailing, retries, failures
    let active_count = state.plans.iter().filter(|p| p.active).count();
    let live_agents = state.active_agent_count();
    let flailing_count = state.plans.iter().filter(|p| p.tasks_failed >= 3).count();
    let total_failures: usize = state.plans.iter().map(|p| p.tasks_failed).sum();

    if active_count > 0 || live_agents > 0 {
        spans.push(Span::styled(
            format!(" {active_count}\u{25b8} {live_agents}ag"),
            Style::default()
                .fg(MoriTheme::ROSE_DIM)
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }
    if flailing_count > 0 {
        spans.push(Span::styled(
            format!(" \u{26a0}{flailing_count}"),
            Style::default()
                .fg(MoriTheme::EMBER)
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }
    if total_failures > 0 {
        spans.push(Span::styled(
            format!(" \u{2717}{total_failures}"),
            Style::default()
                .fg(MoriTheme::EMBER)
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }

    spans.push(Span::styled(
        " \u{2502} ",
        Style::default()
            .fg(MoriTheme::ROSE_DIM)
            .bg(MoriTheme::BG_SECONDARY),
    ));

    // ── 4. Context-sensitive keybind hints ────────────────────────────
    let keys: &str = match state.active_tab {
        Tab::Dashboard => {
            if has_failures {
                "\u{2191}\u{2193}:nav  a/o/d/e/g:sub-tab  Tab:panel  ?:help"
            } else {
                "\u{2191}\u{2193}:nav  a/o/d/e/g:sub-tab  Tab:panel  ?:help"
            }
        }
        Tab::Plans => "\u{2191}\u{2193}:nav  Enter:detail  h/l:drill  /:filter  ?:help",
        Tab::Agents => "\u{2191}\u{2193}:nav  `:cycle  i:inject  y:approve  ?:help",
        Tab::Git => "\u{2191}\u{2193}:nav  h/l:drill  Enter:expand  ?:help",
        Tab::Logs => "\u{2191}\u{2193}/PgUp/PgDn:scroll  /:filter  ?:help",
        Tab::Config => "j/k:nav  Enter:toggle  ?:help",
        Tab::Inspect => "\u{2191}\u{2193}:nav  ?:help",
    };

    spans.push(Span::styled(
        format!(" {keys}"),
        Style::default()
            .fg(MoriTheme::FG_DIM)
            .bg(MoriTheme::BG_SECONDARY),
    ));

    let line = Line::from(spans);
    let p = Paragraph::new(line).style(bg);
    frame.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::super::super::dashboard::DashboardData;
    use super::super::super::state::TuiState;

    #[test]
    fn status_bar_renders_without_panic() {
        let data = DashboardData::default();
        let state = TuiState::from_dashboard_data(&data);
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_status_bar(frame, area, &state);
            })
            .unwrap();
    }
}
