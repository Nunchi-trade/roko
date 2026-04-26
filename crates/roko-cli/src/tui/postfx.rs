//! Post-processing visual effects that operate directly on a ratatui Buffer.
//!
//! Every function takes `(area: Rect, buf: &mut Buffer, ...params)` and modifies
//! cell foreground/background colors in place.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Approximate perceptual luminance (0..255) from RGB.
fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((r as u16 * 77 + g as u16 * 150 + b as u16 * 29) >> 8) as u8
}

/// Scale a single channel by a float factor, clamping to [0, 255].
fn scale(c: u8, factor: f64) -> u8 {
    ((c as f64) * factor).round().clamp(0.0, 255.0) as u8
}

/// Additive blend: min(a + amount, 255).
fn add_channel(a: u8, amount: u8) -> u8 {
    a.saturating_add(amount)
}

/// Screen-blend two values: 1 - (1-a)*(1-b) mapped to 0..255.
fn screen_blend(a: u8, b: u8) -> u8 {
    let af = a as f64 / 255.0;
    let bf = b as f64 / 255.0;
    ((1.0 - (1.0 - af) * (1.0 - bf)) * 255.0).round() as u8
}

/// Extract RGB components from a Color, returning None for non-RGB indexed colors.
fn to_rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Black => Some((0, 0, 0)),
        Color::White => Some((255, 255, 255)),
        Color::Red => Some((205, 0, 0)),
        Color::Green => Some((0, 205, 0)),
        Color::Blue => Some((0, 0, 238)),
        Color::Yellow => Some((205, 205, 0)),
        Color::Magenta => Some((205, 0, 205)),
        Color::Cyan => Some((0, 205, 205)),
        Color::Gray => Some((128, 128, 128)),
        Color::DarkGray => Some((85, 85, 85)),
        Color::LightRed => Some((255, 85, 85)),
        Color::LightGreen => Some((85, 255, 85)),
        Color::LightBlue => Some((85, 85, 255)),
        Color::LightYellow => Some((255, 255, 85)),
        Color::LightMagenta => Some((255, 85, 255)),
        Color::LightCyan => Some((85, 255, 255)),
        _ => None,
    }
}

/// Extract RGB from an Option<Color> (as returned by style().fg / style().bg).
fn opt_to_rgb(color: Option<Color>) -> Option<(u8, u8, u8)> {
    color.and_then(to_rgb)
}

/// Read the foreground color of a cell as RGB.
fn cell_fg_rgb(cell: &ratatui::buffer::Cell) -> Option<(u8, u8, u8)> {
    opt_to_rgb(cell.style().fg)
}

/// Read the background color of a cell as RGB.
fn cell_bg_rgb(cell: &ratatui::buffer::Cell) -> Option<(u8, u8, u8)> {
    opt_to_rgb(cell.style().bg)
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

/// Bloom: brighten cells above a luminance threshold and screen-blend into neighbors.
///
/// `threshold` is the luminance cutoff (0..255). `radius` is the blur distance in cells.
/// `intensity` controls how much bloom is blended (0.0..1.0).
pub fn bloom(area: Rect, buf: &mut Buffer, threshold: u8, radius: u16, intensity: f64) {
    if area.width == 0 || area.height == 0 || intensity <= 0.0 {
        return;
    }

    let w = area.width as usize;
    let h = area.height as usize;

    // Collect bloom source contributions.
    let mut bloom_r = vec![0.0f64; w * h];
    let mut bloom_g = vec![0.0f64; w * h];
    let mut bloom_b = vec![0.0f64; w * h];

    for dy in 0..h {
        for dx in 0..w {
            let x = area.x + dx as u16;
            let y = area.y + dy as u16;
            if let Some(cell) = buf.cell((x, y)) {
                if let Some((r, g, b)) = cell_fg_rgb(cell) {
                    if luminance(r, g, b) > threshold {
                        // Spread to neighbors within radius (box kernel).
                        let r_start = (dy as i32 - radius as i32).max(0) as usize;
                        let r_end = ((dy as i32 + radius as i32 + 1) as usize).min(h);
                        let c_start = (dx as i32 - radius as i32).max(0) as usize;
                        let c_end = ((dx as i32 + radius as i32 + 1) as usize).min(w);

                        let kernel_area =
                            ((r_end - r_start) * (c_end - c_start)).max(1) as f64;
                        let contrib = intensity / kernel_area;

                        for ny in r_start..r_end {
                            for nx in c_start..c_end {
                                let idx = ny * w + nx;
                                bloom_r[idx] += r as f64 * contrib;
                                bloom_g[idx] += g as f64 * contrib;
                                bloom_b[idx] += b as f64 * contrib;
                            }
                        }
                    }
                }
            }
        }
    }

    // Apply bloom via screen blend.
    for dy in 0..h {
        for dx in 0..w {
            let idx = dy * w + dx;
            let br = bloom_r[idx];
            let bg = bloom_g[idx];
            let bb = bloom_b[idx];
            if br <= 0.0 && bg <= 0.0 && bb <= 0.0 {
                continue;
            }

            let x = area.x + dx as u16;
            let y = area.y + dy as u16;
            if let Some(cell) = buf.cell_mut((x, y)) {
                if let Some((cr, cg, cb)) = cell_fg_rgb(cell) {
                    let nr = screen_blend(cr, br.min(255.0) as u8);
                    let ng = screen_blend(cg, bg.min(255.0) as u8);
                    let nb = screen_blend(cb, bb.min(255.0) as u8);
                    cell.set_fg(Color::Rgb(nr, ng, nb));
                }
            }
        }
    }
}

/// Radial vignette: darken cells near the edges of the area.
///
/// `intensity` controls how dark the corners become (0.0..1.0).
pub fn vignette(area: Rect, buf: &mut Buffer, intensity: f64) {
    if area.width == 0 || area.height == 0 || intensity <= 0.0 {
        return;
    }

    let cx = area.x as f64 + area.width as f64 / 2.0;
    let cy = area.y as f64 + area.height as f64 / 2.0;
    let max_dist = ((cx - area.x as f64).powi(2) + (cy - area.y as f64).powi(2)).sqrt();

    if max_dist <= 0.0 {
        return;
    }

    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let dist =
                ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
            let factor = 1.0 - intensity * (dist / max_dist).powi(2);
            let factor = factor.clamp(0.0, 1.0);

            if let Some(cell) = buf.cell_mut((x, y)) {
                if let Some((r, g, b)) = cell_fg_rgb(cell) {
                    cell.set_fg(Color::Rgb(scale(r, factor), scale(g, factor), scale(b, factor)));
                }
                if let Some((r, g, b)) = cell_bg_rgb(cell) {
                    cell.set_bg(Color::Rgb(scale(r, factor), scale(g, factor), scale(b, factor)));
                }
            }
        }
    }
}

/// Dim the entire area by multiplying all cell colors by `factor` (0.0..1.0).
/// Useful for modal background dimming.
pub fn dim_overlay(area: Rect, buf: &mut Buffer, factor: f64) {
    let factor = factor.clamp(0.0, 1.0);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                if let Some((r, g, b)) = cell_fg_rgb(cell) {
                    cell.set_fg(Color::Rgb(scale(r, factor), scale(g, factor), scale(b, factor)));
                }
                if let Some((r, g, b)) = cell_bg_rgb(cell) {
                    cell.set_bg(Color::Rgb(scale(r, factor), scale(g, factor), scale(b, factor)));
                }
            }
        }
    }
}

/// Colored halo around a modal area.
///
/// `modal_area` is the modal rect. `full_area` is the full screen. Cells in
/// `full_area` but outside `modal_area` receive a tinted glow that fades with
/// distance from the modal edge.
pub fn modal_glow(
    modal_area: Rect,
    buf: &mut Buffer,
    full_area: Rect,
    glow_color: Color,
    intensity: f64,
) {
    let Some((gr, gg, gb)) = to_rgb(glow_color) else {
        return;
    };

    let max_range = 6.0_f64; // max glow distance in cells

    for y in full_area.top()..full_area.bottom() {
        for x in full_area.left()..full_area.right() {
            // Skip cells inside the modal
            if x >= modal_area.left()
                && x < modal_area.right()
                && y >= modal_area.top()
                && y < modal_area.bottom()
            {
                continue;
            }

            // Distance to nearest modal edge
            let dx = if x < modal_area.left() {
                (modal_area.left() - x) as f64
            } else if x >= modal_area.right() {
                (x - modal_area.right() + 1) as f64
            } else {
                0.0
            };
            let dy = if y < modal_area.top() {
                (modal_area.top() - y) as f64
            } else if y >= modal_area.bottom() {
                (y - modal_area.bottom() + 1) as f64
            } else {
                0.0
            };
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > max_range {
                continue;
            }

            let falloff = intensity * (1.0 - dist / max_range);
            if falloff <= 0.0 {
                continue;
            }

            if let Some(cell) = buf.cell_mut((x, y)) {
                if let Some((r, g, b)) = cell_bg_rgb(cell) {
                    let nr = add_channel(r, (gr as f64 * falloff).min(255.0) as u8);
                    let ng = add_channel(g, (gg as f64 * falloff).min(255.0) as u8);
                    let nb = add_channel(b, (gb as f64 * falloff).min(255.0) as u8);
                    cell.set_bg(Color::Rgb(nr, ng, nb));
                }
            }
        }
    }
}

/// Drifting ambient orbs that breathe in brightness.
///
/// `elapsed` is seconds since start. `count` is number of orbs.
/// `brightness` is base orb brightness (0..255).
pub fn ambient_orbs(area: Rect, buf: &mut Buffer, elapsed: f64, count: usize, brightness: u8) {
    if area.width < 4 || area.height < 4 {
        return;
    }

    for i in 0..count {
        // Deterministic drift per orb using index as seed.
        let seed = i as f64 * 1.618;
        let drift_x = ((elapsed * 0.3 + seed * 7.0).sin() * 0.5 + 0.5) * (area.width as f64 - 2.0);
        let drift_y =
            ((elapsed * 0.2 + seed * 13.0).cos() * 0.5 + 0.5) * (area.height as f64 - 2.0);
        let ox = area.x + 1 + drift_x as u16;
        let oy = area.y + 1 + drift_y as u16;

        // Breathing per orb
        let breath = ((elapsed * 1.5 + seed * 3.0).sin() * 0.5 + 0.5) as f64;
        let b = (brightness as f64 * (0.3 + 0.7 * breath)).min(255.0) as u8;

        // Paint the orb center + immediate neighbors with additive blend.
        for dy in -1i16..=1 {
            for dx in -1i16..=1 {
                let px = ox as i32 + dx as i32;
                let py = oy as i32 + dy as i32;
                if px < area.left() as i32
                    || px >= area.right() as i32
                    || py < area.top() as i32
                    || py >= area.bottom() as i32
                {
                    continue;
                }

                let dist_factor = if dx == 0 && dy == 0 {
                    1.0
                } else {
                    0.4
                };
                let amount = (b as f64 * dist_factor).min(255.0) as u8;

                if let Some(cell) = buf.cell_mut((px as u16, py as u16)) {
                    if let Some((r, g, bb)) = cell_bg_rgb(cell) {
                        let nr = add_channel(r, amount / 3);
                        let ng = add_channel(g, amount / 4);
                        let nb = add_channel(bb, amount / 2);
                        cell.set_bg(Color::Rgb(nr, ng, nb));
                    }
                }
            }
        }
    }
}

/// Dream atmosphere: combines vignette + subtle grain + breathing brightness.
///
/// `elapsed` drives animation. `frame_seed` provides per-frame variation.
pub fn dream_atmosphere(area: Rect, buf: &mut Buffer, elapsed: f64, frame_seed: u64) {
    // Subtle vignette
    vignette(area, buf, 0.25);

    // Breathing brightness modulation
    let breath = (elapsed * std::f64::consts::PI * 0.5).sin() * 0.05 + 1.0;

    // Grain + breathing combined in one pass
    let mut rng = frame_seed;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            // Simple LCG for grain noise
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let noise = ((rng >> 33) as i8).wrapping_abs() as f64 / 127.0; // 0.0..1.0
            let grain_offset = (noise - 0.5) * 6.0; // -3..+3

            if let Some(cell) = buf.cell_mut((x, y)) {
                if let Some((r, g, b)) = cell_fg_rgb(cell) {
                    let nr = ((r as f64 * breath + grain_offset).clamp(0.0, 255.0)) as u8;
                    let ng = ((g as f64 * breath + grain_offset).clamp(0.0, 255.0)) as u8;
                    let nb = ((b as f64 * breath + grain_offset).clamp(0.0, 255.0)) as u8;
                    cell.set_fg(Color::Rgb(nr, ng, nb));
                }
            }
        }
    }
}

/// Amber/warm color grade: boost red channel, slightly boost blue, reduce green.
///
/// `intensity` is how strong the grade is (0.0..1.0).
pub fn amber_color_grade(area: Rect, buf: &mut Buffer, intensity: f64) {
    let intensity = intensity.clamp(0.0, 1.0);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                if let Some((r, g, b)) = cell_fg_rgb(cell) {
                    let nr = add_channel(r, (20.0 * intensity) as u8);
                    let ng = scale(g, 1.0 - 0.1 * intensity);
                    let nb = add_channel(b, (8.0 * intensity) as u8);
                    cell.set_fg(Color::Rgb(nr, ng, nb));
                }
                if let Some((r, g, b)) = cell_bg_rgb(cell) {
                    let nr = add_channel(r, (10.0 * intensity) as u8);
                    let ng = scale(g, 1.0 - 0.05 * intensity);
                    let nb = add_channel(b, (4.0 * intensity) as u8);
                    cell.set_bg(Color::Rgb(nr, ng, nb));
                }
            }
        }
    }
}

/// Drop shadow: paint 1-cell right and bottom edges of `area` with a dark style.
pub fn drop_shadow(buf: &mut Buffer, area: Rect) {
    let shadow = Color::Rgb(30, 30, 30);

    // Right edge (1 cell wide, from area.top+1 to area.bottom+1)
    let right_x = area.right();
    for y in (area.top() + 1)..=area.bottom() {
        if let Some(cell) = buf.cell_mut((right_x, y)) {
            cell.set_bg(shadow);
            cell.set_fg(Color::Rgb(50, 50, 50));
        }
    }

    // Bottom edge (1 cell tall, from area.left+1 to area.right+1)
    let bottom_y = area.bottom();
    for x in (area.left() + 1)..=area.right() {
        if let Some(cell) = buf.cell_mut((x, bottom_y)) {
            cell.set_bg(shadow);
            cell.set_fg(Color::Rgb(50, 50, 50));
        }
    }
}
