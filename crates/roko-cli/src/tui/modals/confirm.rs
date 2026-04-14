//! Generic destructive action confirmation modal.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::super::dashboard::Theme;

/// The kind of action requiring confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Merge the given source branch into target.
    Merge {
        source: String,
        target: String,
        /// Whether the merge is feasible (no conflicts detected).
        feasible: bool,
    },
    /// Kill an agent by role name.
    KillAgent { role: String },
    /// Reset the execution state.
    ResetState,
    /// Cancel a running plan.
    CancelPlan { plan_id: String },
    /// Generic confirmation with custom message.
    Custom { message: String },
}

impl ConfirmAction {
    /// A short title for the confirmation dialog.
    fn title(&self) -> &str {
        match self {
            Self::Merge { .. } => "Confirm Merge",
            Self::KillAgent { .. } => "Confirm Kill Agent",
            Self::ResetState => "Confirm Reset",
            Self::CancelPlan { .. } => "Confirm Cancel",
            Self::Custom { .. } => "Confirm",
        }
    }
}

/// Render a confirmation dialog for a destructive action.
///
/// Centered ~50x15 rectangle. Shows action-specific messaging with
/// `[y/Enter]` confirm / `[n/Esc]` cancel hints.
pub fn render_confirm(frame: &mut Frame<'_>, area: Rect, action: &ConfirmAction, theme: &Theme) {
    let popup = centered_rect(50, 30, area);
    frame.render_widget(Clear, popup);

    let title = format!(" {} ", action.title());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_alignment(Alignment::Center)
        .border_style(theme.warning());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines = build_confirm_lines(action, theme);

    let paragraph = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn build_confirm_lines<'a>(action: &'a ConfirmAction, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from("")];

    match action {
        ConfirmAction::Merge {
            source,
            target,
            feasible,
        } => {
            lines.push(Line::from(Span::styled("Branch merge:", theme.text())));
            lines.push(Line::from(""));
            // Branch flow graphic
            lines.push(Line::from(vec![
                Span::styled("  ", theme.text()),
                Span::styled(source.as_str(), theme.accent_bold()),
                Span::styled(" -> ", theme.muted()),
                Span::styled(target.as_str(), theme.accent_bold()),
            ]));
            lines.push(Line::from(""));
            let (badge, badge_style) = if *feasible {
                ("FEASIBLE", theme.success())
            } else {
                ("CONFLICTS", theme.danger())
            };
            lines.push(Line::from(vec![
                Span::styled("  Status: ", theme.muted()),
                Span::styled(badge, badge_style),
            ]));
        }
        ConfirmAction::KillAgent { role } => {
            lines.push(Line::from(vec![
                Span::styled("Kill agent ", theme.text()),
                Span::styled(role.as_str(), theme.danger()),
                Span::styled("?", theme.text()),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "This will terminate the agent process.",
                theme.muted(),
            )));
        }
        ConfirmAction::ResetState => {
            lines.push(Line::from(Span::styled(
                "Reset all execution state?",
                theme.text(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "This will clear .roko/state/ and cannot be undone.",
                theme.danger(),
            )));
        }
        ConfirmAction::CancelPlan { plan_id } => {
            lines.push(Line::from(vec![
                Span::styled("Cancel plan ", theme.text()),
                Span::styled(plan_id.as_str(), theme.accent_bold()),
                Span::styled("?", theme.text()),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Running agents will be terminated.",
                theme.warning(),
            )));
        }
        ConfirmAction::Custom { message } => {
            lines.push(Line::from(Span::styled(message.as_str(), theme.text())));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("[y/Enter]", theme.success()),
        Span::styled(" confirm   ", theme.text()),
        Span::styled("[n/Esc]", theme.danger()),
        Span::styled(" cancel", theme.text()),
    ]));

    lines
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
