//! Scroll acceleration for keyboard-driven scrolling.

use std::time::{Duration, Instant};

/// Tracks scroll velocity and accelerates on repeated same-direction keys.
///
/// If the user presses the same direction within 300ms, velocity ramps
/// 1x -> 2x -> 4x -> 8x. Direction change or timeout resets to 1x.
#[derive(Debug, Clone)]
pub struct ScrollAccel {
    velocity: i16,
    last_direction: i16,
    last_key_time: Instant,
    acceleration_threshold: Duration,
}

impl Default for ScrollAccel {
    fn default() -> Self {
        Self {
            velocity: 1,
            last_direction: 0,
            last_key_time: Instant::now(),
            acceleration_threshold: Duration::from_millis(300),
        }
    }
}

impl ScrollAccel {
    /// Push a scroll event in the given direction (+1 or -1).
    /// Returns the accelerated scroll amount (signed).
    pub fn push(&mut self, direction: i16) -> i16 {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_key_time);

        if direction == self.last_direction && elapsed < self.acceleration_threshold {
            // Same direction within threshold: accelerate
            self.velocity = (self.velocity * 2).min(8);
        } else {
            // Direction change or timeout: reset
            self.velocity = 1;
        }

        self.last_direction = direction;
        self.last_key_time = now;

        direction * self.velocity
    }

    /// Reset velocity to 1x.
    pub fn reset(&mut self) {
        self.velocity = 1;
        self.last_direction = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_push_returns_base_velocity() {
        let mut accel = ScrollAccel::default();
        assert_eq!(accel.push(1), 1);
    }

    #[test]
    fn repeated_same_direction_accelerates() {
        let mut accel = ScrollAccel::default();
        assert_eq!(accel.push(1), 1);
        assert_eq!(accel.push(1), 2);
        assert_eq!(accel.push(1), 4);
        assert_eq!(accel.push(1), 8);
        // Caps at 8x
        assert_eq!(accel.push(1), 8);
    }

    #[test]
    fn direction_change_resets() {
        let mut accel = ScrollAccel::default();
        assert_eq!(accel.push(1), 1);
        assert_eq!(accel.push(1), 2);
        // Reverse direction
        assert_eq!(accel.push(-1), -1);
        assert_eq!(accel.push(-1), -2);
    }

    #[test]
    fn reset_clears_velocity() {
        let mut accel = ScrollAccel::default();
        accel.push(1);
        accel.push(1);
        accel.reset();
        assert_eq!(accel.push(1), 1);
    }
}
