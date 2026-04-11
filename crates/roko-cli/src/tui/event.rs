//! Async-compatible terminal event handling for the TUI shell.
//!
//! Provides both a sync [`EventHandler`] (for backward compat) and an
//! async [`EventStream`](crossterm::event::EventStream) integration used
//! by the new `tokio::select!`-based render loop.

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, KeyEventKind};

/// High-level terminal events consumed by the TUI app loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Keyboard input.
    Key(KeyEvent),
    /// Terminal resize.
    Resize(u16, u16),
    /// Tick fired when no input arrives before the configured timeout.
    Tick,
}

/// Polls crossterm for keyboard, resize, and tick events (synchronous API).
///
/// Retained for the standalone `App::run()` path. The new async path uses
/// `crossterm::event::EventStream` directly via `tokio::select!`.
#[derive(Debug, Clone)]
pub struct EventHandler {
    tick_rate: Duration,
    last_tick: Instant,
}

impl EventHandler {
    /// Create a new handler with the given tick rate.
    #[must_use]
    pub fn new(tick_rate: Duration) -> Self {
        Self {
            tick_rate,
            last_tick: Instant::now(),
        }
    }

    /// Current tick rate.
    #[must_use]
    pub const fn tick_rate(&self) -> Duration {
        self.tick_rate
    }

    /// Update the tick rate.
    pub fn set_tick_rate(&mut self, tick_rate: Duration) {
        self.tick_rate = tick_rate;
    }

    /// Wait for the next keyboard, resize, or tick event.
    pub fn next(&mut self) -> io::Result<Event> {
        loop {
            let elapsed = self.last_tick.elapsed();
            let timeout = if elapsed >= self.tick_rate {
                Duration::ZERO
            } else {
                self.tick_rate - elapsed
            };

            if event::poll(timeout)? {
                match event::read()? {
                    CrosstermEvent::Key(key)
                        if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                    {
                        return Ok(Event::Key(key));
                    }
                    CrosstermEvent::Resize(width, height) => {
                        return Ok(Event::Resize(width, height));
                    }
                    _ => continue,
                }
            } else {
                self.last_tick = Instant::now();
                return Ok(Event::Tick);
            }
        }
    }
}

/// Actions the TUI can perform in response to events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    /// Quit the application.
    Quit,
    /// Navigate to a specific tab by index (0-based).
    TabSelect(usize),
    /// Navigate to the next tab.
    TabNext,
    /// Navigate to the previous tab.
    TabPrev,
    /// Scroll up by the given amount.
    ScrollUp(u16),
    /// Scroll down by the given amount.
    ScrollDown(u16),
    /// Scroll to the top.
    ScrollHome,
    /// Page up (scroll by 8).
    PageUp,
    /// Page down (scroll by 8).
    PageDown,
    /// Move selection up.
    SelectUp,
    /// Move selection down.
    SelectDown,
    /// Open detail view for selected item.
    OpenDetail,
    /// Close detail view or modal.
    CloseOverlay,
    /// Toggle help overlay.
    ToggleHelp,
    /// Force refresh data from disk.
    ForceRefresh,
    /// Render tick (redraw the screen).
    Render,
    /// Snapshot changed (StateHub update).
    SnapshotChanged,
}
