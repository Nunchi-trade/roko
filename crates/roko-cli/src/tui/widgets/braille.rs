//! Braille sparkline rendering — maps pairs of data points to single braille
//! characters for 2x horizontal density in a single terminal row.
//!
//! Ported from Mori's braille.rs.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

/// Braille dot bit patterns for the left column (dots 1,2,3,7 from bottom).
const LEFT_COL_BITS: [u8; 4] = [0x40, 0x04, 0x02, 0x01];
/// Braille dot bit patterns for the right column (dots 4,5,6,8 from bottom).
const RIGHT_COL_BITS: [u8; 4] = [0x80, 0x20, 0x10, 0x08];

fn filled_bits(bits: &[u8; 4], n_dots: usize) -> u8 {
    bits[4_usize.saturating_sub(n_dots.min(4))..]
        .iter()
        .fold(0u8, |acc, &bit| acc | bit)
}

/// Render a single-row braille sparkline from `f64` data.
///
/// Each terminal cell encodes two adjacent samples (left/right columns).
/// Returns a `Vec<Span>` ready for inclusion in a `Line`.
///
/// `max`: scale ceiling. 0.0 triggers auto-scaling from visible data.
pub fn braille_spans_f64(data: &[f64], max: f64, width: usize, color: Color) -> Vec<Span<'static>> {
    if data.is_empty() || width == 0 {
        return vec![Span::styled(
            " ".repeat(width),
            Style::default().fg(Color::DarkGray),
        )];
    }

    let capacity = width * 2;
    let offset = data.len().saturating_sub(capacity);
    let visible = &data[offset..];

    let max = if max > 0.0 {
        max
    } else {
        visible.iter().copied().fold(1.0_f64, f64::max)
    };

    let mut s = String::with_capacity(width * 3);
    for cell in 0..width {
        let li = cell * 2;
        let ri = li + 1;
        let lv = visible.get(li).copied().unwrap_or(0.0);
        let rv = visible.get(ri).copied().unwrap_or(0.0);
        let ld = ((lv / max) * 4.0).round().clamp(0.0, 4.0) as usize;
        let rd = ((rv / max) * 4.0).round().clamp(0.0, 4.0) as usize;
        let bits = filled_bits(&LEFT_COL_BITS, ld) | filled_bits(&RIGHT_COL_BITS, rd);
        let ch = char::from_u32(0x2800 + bits as u32).unwrap_or(' ');
        s.push(ch);
    }

    vec![Span::styled(s, Style::default().fg(color))]
}

/// Same as `braille_spans_f64` but accepts `f32` data.
pub fn braille_spans_f32(data: &[f32], max: f32, width: usize, color: Color) -> Vec<Span<'static>> {
    let f64_data: Vec<f64> = data.iter().map(|&v| v as f64).collect();
    braille_spans_f64(&f64_data, max as f64, width, color)
}

/// Convenience: render u64 data as braille (auto-scaled, delta-normalized).
pub fn braille_spans_u64(data: &[u64], width: usize, color: Color) -> Vec<Span<'static>> {
    if data.len() < 2 {
        return vec![Span::styled(
            " ".repeat(width),
            Style::default().fg(Color::DarkGray),
        )];
    }
    let min_val = *data.iter().min().unwrap_or(&0);
    let max_val = *data.iter().max().unwrap_or(&1);
    let range = (max_val - min_val).max(1) as f64;
    let normalized: Vec<f64> = data.iter().map(|&v| (v - min_val) as f64 / range).collect();
    braille_spans_f64(&normalized, 1.0, width, color)
}
