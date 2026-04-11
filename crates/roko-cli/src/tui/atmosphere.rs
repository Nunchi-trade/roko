//! Animated UI elements for the dashboard TUI.
//!
//! Provides heartbeat pulsing, breathing effects, and spinner frames
//! for loading indicators.

use std::f64::consts::TAU;
use std::time::{Duration, Instant};

/// Heartbeat state for pulsing UI elements.
pub struct Heartbeat {
    start: Instant,
    period: Duration,
}

impl Heartbeat {
    /// Create a new heartbeat with the given period.
    #[must_use]
    pub fn new(period: Duration) -> Self {
        Self {
            start: Instant::now(),
            period,
        }
    }

    /// Returns a value in `[0.0, 1.0]` following a sine wave over the period.
    #[must_use]
    pub fn pulse(&self) -> f64 {
        let elapsed = self.start.elapsed().as_secs_f64();
        let period = self.period.as_secs_f64();
        if period == 0.0 {
            return 0.5;
        }
        let t = (elapsed / period).fract();
        (t * TAU).sin() * 0.5 + 0.5
    }
}

/// Compute a breathing alpha value from a normalized time `t` in `[0.0, 1.0]`.
///
/// Returns a `u8` in `[1, 255]` following a sine curve.
#[must_use]
pub fn breathing_alpha(t: f64) -> u8 {
    let t = t.clamp(0.0, 1.0);
    let v = (t * TAU).sin();
    // Map [-1, 1] to [1, 255].
    ((v * 127.0) + 128.0).clamp(1.0, 255.0) as u8
}

/// Spinner frames for loading indicators.
pub struct Spinner {
    frames: &'static [&'static str],
    start: Instant,
    interval: Duration,
}

impl Spinner {
    /// A dots-style spinner.
    #[must_use]
    pub fn dots() -> Self {
        Self {
            frames: &[".", "..", "...", ".."],
            start: Instant::now(),
            interval: Duration::from_millis(250),
        }
    }

    /// A braille-style spinner.
    #[must_use]
    pub fn braille() -> Self {
        Self {
            frames: &["\u{2800}", "\u{2801}", "\u{2803}", "\u{2807}", "\u{280f}", "\u{281f}", "\u{283f}", "\u{287f}"],
            start: Instant::now(),
            interval: Duration::from_millis(100),
        }
    }

    /// Return the current frame string.
    #[must_use]
    pub fn frame(&self) -> &str {
        let elapsed = self.start.elapsed();
        let interval_ms = self.interval.as_millis().max(1);
        let idx = (elapsed.as_millis() / interval_ms) as usize % self.frames.len();
        self.frames[idx]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_range() {
        let hb = Heartbeat::new(Duration::from_millis(100));
        let v = hb.pulse();
        assert!((0.0..=1.0).contains(&v));
    }

    #[test]
    fn breathing_alpha_range() {
        for i in 0..=100 {
            let t = i as f64 / 100.0;
            let a = breathing_alpha(t);
            assert!(a >= 1);
        }
    }

    #[test]
    fn spinner_dots_returns_frame() {
        let s = Spinner::dots();
        let f = s.frame();
        assert!(
            [".","..","...",".."].contains(&f),
            "unexpected frame: {f}"
        );
    }

    #[test]
    fn spinner_braille_returns_frame() {
        let s = Spinner::braille();
        let f = s.frame();
        assert!(!f.is_empty());
    }
}
