//! Toast notification stack rendered at the bottom-right corner.

use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::super::dashboard::Theme;

/// Severity level for a notification toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    Info,
    Warn,
    Error,
    Debug,
}

/// A single toast notification.
#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub created: Instant,
    pub ttl_secs: u64,
    pub level: NotificationLevel,
}

impl Notification {
    /// Create a new notification with the given level and TTL.
    pub fn new(message: impl Into<String>, level: NotificationLevel, ttl_secs: u64) -> Self {
        Self {
            message: message.into(),
            created: Instant::now(),
            ttl_secs,
            level,
        }
    }

    /// Whether this notification has expired.
    pub fn is_expired(&self) -> bool {
        self.created.elapsed().as_secs() >= self.ttl_secs
    }
}

/// Render the notification stack in the bottom-right corner.
///
/// Notifications are stacked from the bottom up. Expired notifications are
/// skipped (caller should prune them separately).
pub fn render_notifications(
    frame: &mut Frame<'_>,
    area: Rect,
    notifications: &[Notification],
    theme: &Theme,
) {
    let active: Vec<&Notification> = notifications.iter().filter(|n| !n.is_expired()).collect();
    if active.is_empty() {
        return;
    }

    let toast_width: u16 = 40;
    let toast_height: u16 = 3; // border top + message + border bottom

    let max_visible = (area.height / toast_height).min(5) as usize;
    let visible = &active[active.len().saturating_sub(max_visible)..];

    for (i, notif) in visible.iter().enumerate() {
        let y_offset = area.height.saturating_sub((i as u16 + 1) * toast_height);
        let x_offset = area.width.saturating_sub(toast_width);

        let toast_area = Rect::new(
            area.x + x_offset,
            area.y + y_offset,
            toast_width.min(area.width),
            toast_height.min(area.height.saturating_sub(y_offset)),
        );

        if toast_area.height < 3 || toast_area.width < 6 {
            continue;
        }

        frame.render_widget(Clear, toast_area);

        let border_style = match notif.level {
            NotificationLevel::Info => theme.info(),
            NotificationLevel::Warn => theme.warning(),
            NotificationLevel::Error => theme.danger(),
            NotificationLevel::Debug => theme.muted(),
        };

        let level_tag = match notif.level {
            NotificationLevel::Info => "INFO",
            NotificationLevel::Warn => "WARN",
            NotificationLevel::Error => "ERR ",
            NotificationLevel::Debug => "DBG ",
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(toast_area);
        frame.render_widget(block, toast_area);

        // Truncate message to fit in one line.
        let max_msg_len = inner.width.saturating_sub(6) as usize; // "[TAG] " prefix
        let msg = if notif.message.len() > max_msg_len {
            format!("{}...", &notif.message[..max_msg_len.saturating_sub(3)])
        } else {
            notif.message.clone()
        };

        let line = Line::from(vec![
            Span::styled(
                format!("[{level_tag}] "),
                border_style.add_modifier(Modifier::BOLD),
            ),
            Span::styled(msg, theme.text()),
        ]);

        frame.render_widget(Paragraph::new(line), inner);
    }
}
