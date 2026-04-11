//! Color math utilities for the TUI.
//!
//! Provides HSV conversion, gradient interpolation, and brightness helpers
//! used by the ROSEDUST theme and custom bar widgets.

use ratatui::style::Color;

/// Convert HSV to an RGB [`Color`].
///
/// * `h` — hue in degrees `[0, 360)`.
/// * `s` — saturation `[0.0, 1.0]`.
/// * `v` — value/brightness `[0.0, 1.0]`.
#[must_use]
pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> Color {
    let h = h.rem_euclid(360.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);

    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = match h as u16 {
        0..60 => (c, x, 0.0),
        60..120 => (x, c, 0.0),
        120..180 => (0.0, c, x),
        180..240 => (0.0, x, c),
        240..300 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    Color::Rgb(
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

/// Linearly interpolate between two RGB colors.
///
/// `t` is clamped to `[0.0, 1.0]`. Non-RGB colors are returned as-is (favoring `from`).
#[must_use]
pub fn gradient(from: Color, to: Color, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (from, to) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let lerp = |a: u8, b: u8| -> u8 {
                let a = a as f64;
                let b = b as f64;
                (a + (b - a) * t).round() as u8
            };
            Color::Rgb(lerp(r1, r2), lerp(g1, g2), lerp(b1, b2))
        }
        _ => from,
    }
}

/// Darken an RGB color by `amount` (0.0 = no change, 1.0 = black).
#[must_use]
pub fn darken(color: Color, amount: f64) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    match color {
        Color::Rgb(r, g, b) => {
            let scale = 1.0 - amount;
            Color::Rgb(
                (r as f64 * scale).round() as u8,
                (g as f64 * scale).round() as u8,
                (b as f64 * scale).round() as u8,
            )
        }
        other => other,
    }
}

/// Lighten an RGB color by `amount` (0.0 = no change, 1.0 = white).
#[must_use]
pub fn lighten(color: Color, amount: f64) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    match color {
        Color::Rgb(r, g, b) => {
            let lift = |c: u8| -> u8 {
                let c = c as f64;
                (c + (255.0 - c) * amount).round() as u8
            };
            Color::Rgb(lift(r), lift(g), lift(b))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_red() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), Color::Rgb(255, 0, 0));
    }

    #[test]
    fn hsv_green() {
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), Color::Rgb(0, 255, 0));
    }

    #[test]
    fn hsv_blue() {
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), Color::Rgb(0, 0, 255));
    }

    #[test]
    fn gradient_midpoint() {
        let c = gradient(Color::Rgb(0, 0, 0), Color::Rgb(100, 200, 100), 0.5);
        assert_eq!(c, Color::Rgb(50, 100, 50));
    }

    #[test]
    fn gradient_endpoints() {
        let from = Color::Rgb(10, 20, 30);
        let to = Color::Rgb(200, 100, 50);
        assert_eq!(gradient(from, to, 0.0), from);
        assert_eq!(gradient(from, to, 1.0), to);
    }

    #[test]
    fn darken_half() {
        let c = darken(Color::Rgb(100, 200, 50), 0.5);
        assert_eq!(c, Color::Rgb(50, 100, 25));
    }

    #[test]
    fn lighten_half() {
        let c = lighten(Color::Rgb(100, 0, 200), 0.5);
        // 100 + (255-100)*0.5 = 177.5 -> 178
        // 0 + 255*0.5 = 127.5 -> 128
        // 200 + (255-200)*0.5 = 227.5 -> 228
        assert_eq!(c, Color::Rgb(178, 128, 228));
    }

    #[test]
    fn non_rgb_passthrough() {
        assert_eq!(
            gradient(Color::White, Color::Rgb(0, 0, 0), 0.5),
            Color::White
        );
        assert_eq!(darken(Color::Red, 0.5), Color::Red);
        assert_eq!(lighten(Color::Blue, 0.5), Color::Blue);
    }
}
