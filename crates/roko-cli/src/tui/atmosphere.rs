//! Time-based animation state for TUI effects.

use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// Tracks elapsed time and frame count for driving animations.
#[derive(Debug, Clone)]
pub struct Atmosphere {
    start_time: Instant,
    /// Seconds since start.
    pub elapsed: f64,
    /// Total frames rendered.
    pub frame_count: u64,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            start_time: Instant::now(),
            elapsed: 0.0,
            frame_count: 0,
        }
    }
}

impl Atmosphere {
    /// Advance the clock. Call once per frame.
    pub fn tick(&mut self) {
        self.elapsed = self.start_time.elapsed().as_secs_f64();
        self.frame_count += 1;
    }

    /// Breathing brightness oscillation (sine wave, range 0.8..1.0).
    #[must_use]
    pub fn breathing_brightness(&self) -> f64 {
        let phase = (self.elapsed * std::f64::consts::PI * 0.5).sin();
        0.9 + 0.1 * phase
    }

    /// Double-pulse heartbeat pattern.
    /// Returns a value in 0.0..1.0 representing beat intensity.
    #[must_use]
    pub fn heartbeat(&self) -> f64 {
        // Two quick pulses per ~1.5s cycle
        let t = (self.elapsed % 1.5) / 1.5;
        if t < 0.1 {
            // First beat: quick rise
            (t / 0.1).min(1.0)
        } else if t < 0.2 {
            // First beat: quick fall
            1.0 - ((t - 0.1) / 0.1)
        } else if t < 0.3 {
            // Second beat: quick rise
            ((t - 0.2) / 0.1).min(1.0) * 0.7
        } else if t < 0.4 {
            // Second beat: quick fall
            0.7 * (1.0 - ((t - 0.3) / 0.1))
        } else {
            // Rest
            0.0
        }
    }

    /// Current frame count.
    #[must_use]
    pub const fn frame(&self) -> u64 {
        self.frame_count
    }

    /// Elapsed seconds since start.
    #[must_use]
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// Approximate FPS based on elapsed time and frame count.
    #[must_use]
    pub fn fps(&self) -> f64 {
        if self.elapsed > 0.0 {
            self.frame_count as f64 / self.elapsed
        } else {
            60.0
        }
    }

    /// Spinner character cycling through animation frames.
    #[must_use]
    pub fn spinner(&self) -> char {
        const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        SPINNER[(self.frame_count as usize / 4) % SPINNER.len()]
    }

    /// Ethereal spinner (slower, for subtle animations).
    #[must_use]
    pub fn spinner_ethereal(&self) -> char {
        const SPINNER: &[char] = &['◜', '◝', '◞', '◟'];
        SPINNER[(self.frame_count as usize / 8) % SPINNER.len()]
    }

    /// Apply a full-frame bloom post-processing pass.
    /// Brightens cells whose luminance exceeds `threshold` by `intensity`.
    pub fn apply(&self, area: Rect, buf: &mut Buffer) {
        let brightness = self.breathing_brightness();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    if let Some(Color::Rgb(r, g, b)) = cell.style().fg {
                        let lum = luminance(r, g, b);
                        if lum > 180 {
                            let factor = brightness;
                            let nr = scale_channel(r, factor);
                            let ng = scale_channel(g, factor);
                            let nb = scale_channel(b, factor);
                            cell.set_fg(Color::Rgb(nr, ng, nb));
                        }
                    }
                }
            }
        }
    }
}

/// Scale a color channel by a factor, clamping to 255.
fn scale_channel(c: u8, factor: f64) -> u8 {
    ((c as f64) * factor).round().min(255.0).max(0.0) as u8
}

/// Approximate perceptual luminance (0..255).
fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((r as u16 * 77 + g as u16 * 150 + b as u16 * 29) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breathing_in_range() {
        let atm = Atmosphere::default();
        let b = atm.breathing_brightness();
        assert!(b >= 0.79 && b <= 1.01, "breathing={b}");
    }

    #[test]
    fn heartbeat_in_range() {
        let atm = Atmosphere::default();
        let h = atm.heartbeat();
        assert!(h >= 0.0 && h <= 1.0, "heartbeat={h}");
    }

    #[test]
    fn luminance_black_is_zero() {
        assert_eq!(luminance(0, 0, 0), 0);
    }

    #[test]
    fn luminance_white_is_max() {
        let l = luminance(255, 255, 255);
        assert!(l > 250);
    }
}
