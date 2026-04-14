//! Per-tab post-processing pipeline.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::effects_config::EffectsConfig;
use super::postfx;

/// Apply the full post-processing pipeline for the given tab.
///
/// `tab_idx`: 0 = Dashboard, 1 = Plans, 2 = Agents, etc.
/// `area`: the region to apply effects to.
/// `elapsed`: seconds since TUI start (drives animations).
/// `frame`: frame counter (used as seed for per-frame effects).
/// `fx`: effects configuration.
pub fn apply_pipeline(
    tab_idx: usize,
    area: Rect,
    buf: &mut Buffer,
    elapsed: f64,
    frame: u64,
    fx: &EffectsConfig,
) {
    if !fx.screen_postfx {
        return;
    }

    // Self-glow for Dashboard, Plans, Agents tabs.
    match tab_idx {
        0 | 1 | 2 => {
            self_glow(area, buf, 200, 0.12);
        }
        _ => {}
    }

    // Bloom (expensive, gated separately).
    if fx.bloom_enabled {
        postfx::bloom(area, buf, 220, 1, fx.bloom_intensity);
    }

    // Shadows.
    if fx.shadows_enabled {
        postfx::drop_shadow(buf, area);
    }

    // VFX: ambient orbs + atmosphere.
    if fx.vfx_enabled {
        postfx::ambient_orbs(area, buf, elapsed, 3, 40);
        postfx::dream_atmosphere(area, buf, elapsed, frame);
    }
}

/// Self-illumination: brightens cells whose foreground luminance exceeds a threshold.
///
/// This is a lighter-weight alternative to full bloom, applied per-cell without blur.
/// `threshold`: luminance cutoff (0..255).
/// `intensity`: how much to brighten (0.0..1.0).
pub fn self_glow(area: Rect, buf: &mut Buffer, threshold: u8, intensity: f64) {
    if area.width == 0 || area.height == 0 || intensity <= 0.0 {
        return;
    }

    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                if let Some(Color::Rgb(r, g, b)) = cell.style().fg {
                    let lum = luminance(r, g, b);
                    if lum > threshold {
                        let boost =
                            intensity * ((lum - threshold) as f64 / (255 - threshold) as f64);
                        let nr = add_bright(r, boost);
                        let ng = add_bright(g, boost);
                        let nb = add_bright(b, boost);
                        cell.set_fg(Color::Rgb(nr, ng, nb));
                    }
                }
            }
        }
    }
}

/// Approximate luminance.
fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((r as u16 * 77 + g as u16 * 150 + b as u16 * 29) >> 8) as u8
}

/// Brighten a channel additively by a fraction of its headroom.
fn add_bright(c: u8, fraction: f64) -> u8 {
    let headroom = 255.0 - c as f64;
    ((c as f64 + headroom * fraction).round().min(255.0)) as u8
}
