//! Mori-ported atmosphere animation system.
//!
//! Provides frame-driven heartbeat, breathing, spinners, and timing
//! for all new TUI widgets. The old `atmosphere.rs` (Heartbeat/Spinner
//! structs) remains for backward compatibility with existing views.

use std::time::Instant;

/// Character sets for spinners.
const BRAILLE: &[char] = &[
    '\u{280B}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283C}', '\u{2834}', '\u{2826}', '\u{2827}',
    '\u{2807}', '\u{280F}',
];
const ETHEREAL: &[char] = &[
    '\u{2727}', '\u{00B7}', '\u{00B0}', '\u{2726}', '\u{2218}', '\u{22C6}', '\u{2736}', '\u{274B}',
];

/// Frame-driven atmospheric state for all TUI animations.
///
/// Unlike the old `Heartbeat`/`Spinner` types, this is a single struct
/// that advances per-frame and provides all animation values.
pub struct Atmosphere {
    frame_count: u64,
    elapsed: f64,
    dt: f64,
    last_frame: Instant,
    heartbeat_phase: f64,
    breathing_phase: f64,
    flash_intensity: f64,
    flash_hue: f64,
    fps: f64,
    fps_accum: f64,
    fps_frame_count: u32,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self::new()
    }
}

impl Atmosphere {
    const HEARTBEAT_PERIOD_FRAMES: f64 = 60.0;

    pub fn new() -> Self {
        Self {
            frame_count: 0,
            elapsed: 0.0,
            dt: 1.0 / 30.0,
            last_frame: Instant::now(),
            heartbeat_phase: 0.0,
            breathing_phase: 0.0,
            flash_intensity: 0.0,
            flash_hue: 325.0,
            fps: 30.0,
            fps_accum: 0.0,
            fps_frame_count: 0,
        }
    }

    /// Braille spinner character, cycling every 3 frames.
    pub fn spinner(&self) -> char {
        BRAILLE[(self.frame_count / 3) as usize % BRAILLE.len()]
    }

    /// Ethereal spinner character, cycling every 4 frames.
    pub fn spinner_ethereal(&self) -> char {
        ETHEREAL[(self.frame_count / 4) as usize % ETHEREAL.len()]
    }

    /// Heartbeat pulse: 0.95..1.05 sine wave.
    pub fn heartbeat(&self) -> f64 {
        1.0 + 0.05 * self.heartbeat_phase.sin()
    }

    /// Shimmer effect: 0.9..1.1 oscillation.
    pub fn shimmer(&self) -> f64 {
        let phase = (self.frame_count as f64 / 8.0).sin();
        1.0 + phase * 0.10
    }

    /// Current frame number.
    pub fn frame(&self) -> u64 {
        self.frame_count
    }

    /// Breathing brightness: 0.88..1.0 range, ~5.2s period.
    pub fn breathing_brightness(&self) -> f64 {
        0.94 + 0.06 * (self.breathing_phase).sin()
    }

    /// Total elapsed seconds since creation.
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// Current FPS (smoothed).
    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// Delta time of last frame in seconds.
    pub fn dt(&self) -> f64 {
        self.dt
    }

    /// Trigger a flash effect.
    pub fn trigger_flash(&mut self, hue: f64) {
        self.flash_intensity = 1.0;
        self.flash_hue = hue;
    }

    /// Current flash intensity (0.0 = none, 1.0 = peak).
    pub fn flash(&self) -> f64 {
        self.flash_intensity
    }

    /// Flash hue.
    pub fn flash_hue(&self) -> f64 {
        self.flash_hue
    }

    /// Exponential decay lerp: smoothly moves `current` toward `target`.
    pub fn lerp_toward(current: f64, target: f64, rate: f64) -> f64 {
        current + (target - current) * (1.0 - (-rate * 0.033).exp())
    }

    /// Phosphor decay brightness multiplier.
    pub fn phosphor_decay(frames_since_change: u64) -> f64 {
        let t = frames_since_change as f64 / 30.0;
        (-t * 4.0).exp()
    }

    /// Advance one frame. Call every render tick.
    pub fn tick(&mut self) {
        let now = Instant::now();
        self.dt = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;

        let dt = self.dt.min(0.1);

        self.frame_count = self.frame_count.wrapping_add(1);
        self.elapsed += dt;
        self.heartbeat_phase =
            (self.frame_count as f64 / Self::HEARTBEAT_PERIOD_FRAMES) * std::f64::consts::TAU;
        self.breathing_phase = self.elapsed * (std::f64::consts::TAU / 5.2);

        self.flash_intensity = (self.flash_intensity - dt * 3.0).max(0.0);

        self.fps_frame_count += 1;
        self.fps_accum += dt;
        if self.fps_accum >= 1.0 {
            self.fps = self.fps_frame_count as f64 / self.fps_accum;
            self.fps_frame_count = 0;
            self.fps_accum = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_range() {
        let atm = Atmosphere::new();
        let h = atm.heartbeat();
        assert!((0.9..=1.1).contains(&h));
    }

    #[test]
    fn breathing_range() {
        let atm = Atmosphere::new();
        let b = atm.breathing_brightness();
        assert!((0.8..=1.0).contains(&b));
    }

    #[test]
    fn spinner_valid_char() {
        let atm = Atmosphere::new();
        let ch = atm.spinner();
        assert!(BRAILLE.contains(&ch));
    }

    #[test]
    fn tick_advances_frame() {
        let mut atm = Atmosphere::new();
        assert_eq!(atm.frame(), 0);
        atm.tick();
        assert_eq!(atm.frame(), 1);
    }

    #[test]
    fn lerp_toward_converges() {
        let v = Atmosphere::lerp_toward(0.0, 1.0, 10.0);
        assert!(v > 0.0 && v < 1.0);
    }

    #[test]
    fn phosphor_decay_decreases() {
        let a = Atmosphere::phosphor_decay(0);
        let b = Atmosphere::phosphor_decay(30);
        assert!(a > b);
    }
}
