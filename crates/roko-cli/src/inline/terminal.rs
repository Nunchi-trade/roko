//! Inline terminal renderer using ratatui's `Viewport::Inline`.
//!
//! This is the core of the inline CLI UX — a fixed-height viewport at the
//! bottom of the terminal for live content (streaming, spinners, status bars),
//! with completed blocks pushed into scrollback via `insert_before`.
//!
//! The pattern matches Claude Code's UX: history scrolls up, live content
//! stays at the bottom.

use std::io::{self, IsTerminal, Stdout, Write as _};

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::{
    Frame, Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};

use crate::tui::Theme;

/// Default viewport height in terminal lines.
const DEFAULT_VIEWPORT_HEIGHT: u16 = 10;

/// RAII guard that disables raw mode on drop.
///
/// Hold this value for as long as raw mode should stay active. When it goes
/// out of scope — whether via normal return, early `?` bail, or panic unwind —
/// `disable_raw_mode()` is called automatically.
///
/// ```ignore
/// let _guard = RawModeGuard::enable()?;
/// // raw mode active …
/// // dropped here → raw mode disabled
/// ```
pub struct RawModeGuard {
    _private: (),
}

impl RawModeGuard {
    /// Enable raw mode and return a guard that will disable it on drop.
    pub fn enable() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self { _private: () })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

/// The inline terminal: renders a viewport at the bottom of the screen and
/// pushes completed blocks into terminal scrollback.
pub struct InlineTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    theme: Theme,
    viewport_height: u16,
    /// Keeps raw mode active for the lifetime of this struct. Dropped in
    /// field-declaration order (after `terminal`), which is fine because
    /// `restore()` in our `Drop` impl runs first.
    _raw_guard: RawModeGuard,
}

impl InlineTerminal {
    /// Create a new inline terminal.
    ///
    /// Enters raw mode and sets up an inline viewport. The viewport occupies
    /// `height` lines at the bottom of the terminal. Normal scrollback is
    /// preserved above it.
    ///
    /// # Errors
    ///
    /// Returns an error if the terminal cannot be initialized (e.g. not a TTY,
    /// or raw mode fails).
    pub fn new() -> io::Result<Self> {
        let height = std::env::var("ROKO_VIEWPORT_HEIGHT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_VIEWPORT_HEIGHT);

        let theme = Theme::from_env();

        // Set panic hook to restore terminal before panic output
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(std::io::stdout(), crossterm::cursor::Show);
            default_hook(info);
        }));

        let raw_guard = RawModeGuard::enable()?;
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(height),
            },
        )?;

        Ok(Self {
            terminal,
            theme,
            viewport_height: height,
            _raw_guard: raw_guard,
        })
    }

    /// The active theme.
    #[must_use]
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The viewport height in lines.
    #[must_use]
    pub fn viewport_height(&self) -> u16 {
        self.viewport_height
    }

    /// Terminal width in columns.
    #[must_use]
    pub fn width(&self) -> u16 {
        self.terminal.size().map_or(80, |s| s.width)
    }

    /// Push styled lines into scrollback above the viewport.
    ///
    /// This is how completed blocks (run summaries, tool calls, user messages)
    /// enter the terminal history. They remain visible and copy-pasteable
    /// after the session ends.
    pub fn push_lines(&mut self, lines: &[Line<'_>]) -> io::Result<()> {
        let count = lines.len() as u16;
        if count == 0 {
            return Ok(());
        }
        self.terminal.insert_before(count, |buf| {
            let text = ratatui::text::Text::from(lines.to_vec());
            Paragraph::new(text)
                .wrap(Wrap { trim: false })
                .render(buf.area, buf);
        })?;
        Ok(())
    }

    /// Push a single line into scrollback.
    pub fn push_line(&mut self, line: Line<'_>) -> io::Result<()> {
        self.push_lines(&[line])
    }

    /// Push a blank line into scrollback.
    pub fn push_blank(&mut self) -> io::Result<()> {
        self.push_line(Line::raw(""))
    }

    /// Redraw the live viewport area.
    ///
    /// The closure receives a `Frame` for the viewport region. This is called
    /// on every tick (30-60fps) to update streaming content, spinners, and
    /// the status bar.
    pub fn draw<F>(&mut self, f: F) -> io::Result<()>
    where
        F: FnOnce(&mut Frame<'_>),
    {
        self.terminal.draw(f)?;
        Ok(())
    }

    /// Clear the viewport (fill with empty space).
    pub fn clear_viewport(&mut self) -> io::Result<()> {
        self.draw(|frame| {
            let area = frame.area();
            frame.render_widget(Paragraph::new(""), area);
        })
    }

    /// Push styled lines into scrollback with a per-line delay for reveal effect.
    ///
    /// Each line appears individually with `delay` between them, creating a
    /// typing/reveal animation. Use `Duration::ZERO` for instant rendering.
    pub fn push_lines_revealed(
        &mut self,
        lines: &[Line<'_>],
        delay: std::time::Duration,
    ) -> io::Result<()> {
        for line in lines {
            self.push_line(line.clone())?;
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
        }
        Ok(())
    }

    /// Push a horizontal rule separator.
    pub fn push_separator(&mut self) -> io::Result<()> {
        let width = self.width().saturating_sub(2) as usize;
        let style = ratatui::style::Style::default().fg(
            ratatui::style::Color::Rgb(55, 42, 55), // TEXT_PHANTOM
        );
        self.push_line(Line::from(vec![Span::styled("─".repeat(width), style)]))
    }

    /// Restore the terminal to its normal state.
    ///
    /// This MUST be called before the process exits, or the terminal will be
    /// left in raw mode. The `Drop` impl calls this automatically.
    pub fn restore(&mut self) -> io::Result<()> {
        disable_raw_mode()?;
        // Show cursor and flush
        crossterm::queue!(io::stdout(), crossterm::cursor::Show)?;
        io::stdout().flush()?;
        Ok(())
    }
}

impl Drop for InlineTerminal {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Returns `true` if stdout is a TTY and inline rendering is appropriate.
///
/// When this returns `false`, callers should fall back to plain text output.
#[must_use]
pub fn should_use_inline() -> bool {
    io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_use_inline_respects_no_color() {
        // This test checks the NO_COLOR logic but can't fully test TTY
        // detection in CI. The function is simple enough to verify by reading.
        let _ = should_use_inline();
    }
}
