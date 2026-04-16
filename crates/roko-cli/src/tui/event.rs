//! Crossterm event polling for the TUI shell.

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, KeyEventKind, MouseEvent};

/// High-level terminal events consumed by the TUI app loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Keyboard input.
    Key(KeyEvent),
    /// Mouse input.
    Mouse(MouseEvent),
    /// Terminal resize.
    Resize(u16, u16),
    /// Tick fired when no input arrives before the configured timeout.
    Tick,
}

/// Polls crossterm for keyboard, resize, and tick events.
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

    /// Wait for the next keyboard, mouse, resize, or tick event.
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
                    CrosstermEvent::Mouse(mouse) => {
                        return Ok(Event::Mouse(mouse));
                    }
                    CrosstermEvent::Resize(width, height) => {
                        return Ok(Event::Resize(width, height));
                    }
                    _ => continue,
                }
            }
            self.last_tick = Instant::now();
            return Ok(Event::Tick);
        }
    }
}
