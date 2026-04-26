//! Colored status badge renderer.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::dashboard::Theme;

/// Status badge variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusBadge {
    Active,
    Done,
    Error,
    Warning,
    Revision,
    Paused,
    Idle,
    Pending,
}

impl StatusBadge {
    /// Label text for this badge.
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Done => "DONE",
            Self::Error => "ERROR",
            Self::Warning => "WARN",
            Self::Revision => "REVISION",
            Self::Paused => "PAUSED",
            Self::Idle => "IDLE",
            Self::Pending => "PENDING",
        }
    }

    /// Icon prefix for this badge.
    fn icon(self) -> &'static str {
        match self {
            Self::Active => ">> ",
            Self::Done => "OK ",
            Self::Error => "!! ",
            Self::Warning => "?! ",
            Self::Revision => "<> ",
            Self::Paused => "|| ",
            Self::Idle => "-- ",
            Self::Pending => ".. ",
        }
    }

    /// Primary color for this badge.
    fn color(self, theme: &Theme) -> Color {
        match self {
            Self::Active => theme.accent,
            Self::Done => theme.success,
            Self::Error => theme.danger,
            Self::Warning => theme.warning,
            Self::Revision => theme.info,
            Self::Paused => theme.warning,
            Self::Idle => theme.muted,
            Self::Pending => theme.muted,
        }
    }

    /// Whether this badge should pulse (blink modifier).
    fn should_pulse(self) -> bool {
        matches!(self, Self::Active | Self::Error)
    }
}

/// Render a compact colored status badge.
///
/// Renders as colored text with an icon prefix in a single line.
pub fn render_status_badge(
    frame: &mut Frame<'_>,
    area: Rect,
    status: StatusBadge,
    theme: &Theme,
) {
    let color = status.color(theme);
    let mut style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    if status.should_pulse() {
        style = style.add_modifier(Modifier::SLOW_BLINK);
    }

    let line = Line::from(vec![
        Span::styled(status.icon().to_string(), style),
        Span::styled(status.label().to_string(), style),
    ]);

    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);
}

/// Create a styled Span for inline use in other widgets.
pub fn status_badge_span(status: StatusBadge, theme: &Theme) -> Span<'static> {
    let color = status.color(theme);
    let style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    Span::styled(
        format!("{}{}", status.icon(), status.label()),
        style,
    )
}
