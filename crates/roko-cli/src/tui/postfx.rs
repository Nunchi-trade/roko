//! Post-processing visual effects that operate directly on a ratatui Buffer.
//!
//! Every function takes `(area: Rect, buf: &mut Buffer, ...params)` and modifies
//! cell foreground/background colors in place.

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::widgets::rosedust::MoriTheme;

// ---------------------------------------------------------------------------
// Public NervViz state
// ---------------------------------------------------------------------------

/// State snapshot used to drive NervViz-style overlays.
#[derive(Debug, Clone, Copy, Default)]
pub struct VizContext {
    /// Completed task progress in the `0.0..=1.0` range.
    pub task_progress: f64,
    /// Completed plan progress in the `0.0..=1.0` range.
    pub plan_progress: f64,
    /// Context pressure in the `0.0..=1.0` range.
    pub context_pressure: f64,
    /// Normalized token throughput in the `0.0..=1.0` range.
    pub token_rate: f64,
    /// Whether at least one agent is currently active.
    pub agent_active: bool,
    /// Current iteration counter used as a deterministic seed.
    pub iteration: u32,
    /// Whether the current execution state is in error.
    pub error_state: bool,
}

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
fn cell_fg_rgb(cell: &Cell) -> Option<(u8, u8, u8)> {
    opt_to_rgb(cell.style().fg)
}

/// Read the background color of a cell as RGB.
fn cell_bg_rgb(cell: &Cell) -> Option<(u8, u8, u8)> {
    opt_to_rgb(cell.style().bg)
}

/// Check whether a buffer cell is still blank.
fn is_blank(cell: &Cell) -> bool {
    let symbol = cell.symbol();
    symbol == " " || symbol.is_empty()
}

/// Clamp a floating-point value to the 0.0..=1.0 range.
fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

/// Linear interpolation between two `u8` values.
fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    let t = clamp01(t);
    ((a as f64) + (b as f64 - a as f64) * t).round().clamp(0.0, 255.0) as u8
}

/// Linear interpolation between two RGB colors.
fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> Color {
    Color::Rgb(
        lerp_u8(a.0, b.0, t),
        lerp_u8(a.1, b.1, t),
        lerp_u8(a.2, b.2, t),
    )
}

/// SplitMix64-style hash for deterministic pseudo-random values.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Convert a deterministic hash to `0.0..1.0`.
fn unit_from_hash(x: u64) -> f64 {
    const SCALE: f64 = 1.0 / ((1u64 << 53) as f64);
    ((splitmix64(x) >> 11) as f64) * SCALE
}

/// A compact rose/violet gradient used by the new overlays.
fn rose_violet(t: f64) -> Color {
    let t = clamp01(t);
    if t < 0.5 {
        lerp_rgb(
            match MoriTheme::ROSE_DIM {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (140, 96, 112),
            },
            match MoriTheme::ROSE {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (185, 120, 148),
            },
            t * 2.0,
        )
    } else {
        lerp_rgb(
            match MoriTheme::ROSE {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (185, 120, 148),
            },
            match MoriTheme::DREAM {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (120, 115, 165),
            },
            (t - 0.5) * 2.0,
        )
    }
}

/// Tint between rose and violet using a narrow pulse range.
fn pulse_tint(t: f64, pulse: f64) -> Color {
    let t = clamp01(t);
    let pulse = clamp01(pulse);
    let base = rose_violet(t);
    match base {
        Color::Rgb(r, g, b) => Color::Rgb(
            lerp_u8(r, 255, 0.25 * pulse),
            lerp_u8(g, 210, 0.10 * pulse),
            lerp_u8(b, 255, 0.30 * pulse),
        ),
        other => other,
    }
}

/// Convert a braille bit-mask into a Unicode braille character.
fn braille(bits: u8) -> char {
    char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
}

/// Braille dots used for small spark particles.
const PARTICLE_FRAMES: [u8; 8] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];

/// Return a braille mask with a few sub-cell dots lit.
fn braille_mask(seed: u64, x: u16, y: u16, elapsed: f64, phase: f64) -> u8 {
    let h = splitmix64(
        seed ^ ((x as u64) << 16) ^ ((y as u64) << 32) ^ elapsed.to_bits() ^ phase.to_bits(),
    );
    let frame = (h as usize) % PARTICLE_FRAMES.len();
    let mut bits = PARTICLE_FRAMES[frame];
    if (h & 1) != 0 {
        bits |= 0x20;
    }
    if (h & 2) != 0 {
        bits |= 0x04;
    }
    bits
}

/// Write a braille glyph only if the target cell is still blank.
fn write_blank_braille(cell: &mut Cell, bits: u8, fg: Color, bg: Color) {
    if !is_blank(cell) {
        return;
    }
    cell.set_char(braille(bits));
    cell.set_fg(fg);
    cell.set_bg(bg);
}

/// Draw pulsing connector traces across empty regions.
fn guide_lines(area: Rect, buf: &mut Buffer, elapsed: f64, intensity: f64, seed: u64) {
    if area.width < 8 || area.height < 4 {
        return;
    }

    let intensity = clamp01(intensity);
    if intensity <= 0.0 {
        return;
    }

    let line_count = 2 + (intensity * 2.0).round() as usize;
    let pulse = (elapsed * (1.1 + intensity)).sin() * 0.5 + 0.5;

    for idx in 0..line_count {
        let line_seed = seed ^ (idx as u64 * 0x9E37_79B9_7F4A_7C15);
        let from_y = area.top()
            + (unit_from_hash(line_seed ^ 0x11) * area.height.saturating_sub(1) as f64).round()
                as u16;
        let to_y = area.top()
            + (unit_from_hash(line_seed ^ 0x29) * area.height.saturating_sub(1) as f64).round()
                as u16;
        let slope = (to_y as f64 - from_y as f64) / area.width.max(1) as f64;
        let phase = unit_from_hash(line_seed ^ 0x51) * std::f64::consts::TAU;

        for step in 0..area.width {
            let x = area.left() + step;
            let y = (from_y as f64
                + slope * step as f64
                + ((elapsed * 1.8 + step as f64 * 0.15 + phase).sin() * intensity * 0.8))
                .round()
                .clamp(area.top() as f64, area.bottom().saturating_sub(1) as f64)
                as u16;

            if let Some(cell) = buf.cell_mut((x, y)) {
                if !is_blank(cell) {
                    continue;
                }

                let strength = 0.35 + 0.65 * pulse;
                let fg = pulse_tint(0.45 + 0.35 * intensity, strength);
                let bg = pulse_tint(0.05 + 0.10 * intensity, strength * 0.25);
                let bits = match (step + idx as u16) % 4 {
                    0 => 0x09,
                    1 => 0x12,
                    2 => 0x24,
                    _ => 0x48,
                };
                write_blank_braille(cell, bits, fg, bg);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// NervViz overlays
// ---------------------------------------------------------------------------

/// Pulsing progress field used for task/plan completion overlays.
///
/// The effect only writes into blank cells and uses a rose/violet fill band
/// with a braille-lit leading edge.
pub fn progress_field(area: Rect, buf: &mut Buffer, elapsed: f64, progress: f64) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let progress = clamp01(progress);
    if progress <= 0.0 {
        return;
    }

    let fill_rows = ((area.height as f64) * progress).ceil() as u16;
    if fill_rows == 0 {
        return;
    }

    let start_y = area.bottom().saturating_sub(fill_rows);
    let pulse = (elapsed * 2.2).sin() * 0.5 + 0.5;
    let edge_line = start_y;

    for y in start_y..area.bottom() {
        let depth = (area.bottom() - 1 - y) as f64 / fill_rows.max(1) as f64;
        let fill_mix = (1.0 - depth).powf(0.75) * (0.35 + 0.65 * pulse);
        let bg = pulse_tint(fill_mix, pulse);
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                if !is_blank(cell) {
                    continue;
                }

                cell.set_bg(bg);

                // The leading edge gets a denser braille contour.
                if y == edge_line || ((x as f64 * 0.37 + elapsed * 1.5 + depth * 5.0).sin() > 0.68) {
                    let bits = braille_mask(0xA17C_0F11, x, y, elapsed, progress);
                    let fg = pulse_tint(0.55 + 0.45 * pulse, pulse);
                    cell.set_char(braille(bits));
                    cell.set_fg(fg);
                }
            }
        }
    }
}

/// Concentric ripple waves that expand from a small set of deterministic centers.
///
/// The `activity` parameter controls both the number of ripple centers and the
/// ring thickness.
pub fn activity_ripples(area: Rect, buf: &mut Buffer, elapsed: f64, activity: f64) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let activity = clamp01(activity);
    if activity <= 0.0 {
        return;
    }

    let w = area.width.max(1) as f64;
    let h = area.height.max(1) as f64;
    let max_radius = (w * w + h * h).sqrt() * 0.52;
    let center_count = 1 + (activity * 2.0).round() as usize;
    let thickness = 0.45 + activity * 0.95;
    let pulse = (elapsed * (0.8 + activity * 1.8)).sin() * 0.5 + 0.5;

    for i in 0..center_count {
        let seed = 0x51_49_5A_45_u64 ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let cx = area.left()
            + (unit_from_hash(seed) * (area.width.saturating_sub(1) as f64)).round() as u16;
        let cy = area.top()
            + (unit_from_hash(seed ^ 0xB4) * (area.height.saturating_sub(1) as f64)).round()
                as u16;

        let radius = ((elapsed * (0.9 + activity * 1.3) + unit_from_hash(seed ^ 0xD1) * 2.0)
            % max_radius.max(1.0))
            .max(0.0);

        let reach = (radius + thickness + 1.0).ceil() as u16;
        let x0 = cx.saturating_sub(reach).max(area.left());
        let y0 = cy.saturating_sub(reach).max(area.top());
        let x1 = cx
            .saturating_add(reach)
            .min(area.right().saturating_sub(1));
        let y1 = cy
            .saturating_add(reach)
            .min(area.bottom().saturating_sub(1));

        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f64 - cx as f64;
                let dy = y as f64 - cy as f64;
                let dist = (dx * dx + dy * dy).sqrt();
                let delta = (dist - radius).abs();
                if delta > thickness {
                    continue;
                }

                if let Some(cell) = buf.cell_mut((x, y)) {
                    if !is_blank(cell) {
                        continue;
                    }

                    let strength = 1.0 - (delta / thickness).clamp(0.0, 1.0);
                    let fg = pulse_tint(0.45 + 0.55 * strength, pulse * strength);
                    let bg = pulse_tint(0.10 + 0.25 * strength, pulse * 0.5);
                    let bits = braille_mask(seed, x, y, elapsed, strength);
                    write_blank_braille(cell, bits, fg, bg);
                }
            }
        }
    }
}

/// Matrix-style data rain scaled by normalized token throughput.
///
/// The effect keeps to blank cells, emits braille dots along falling streams,
/// and fades columns with a rose/violet palette.
pub fn data_rain(area: Rect, buf: &mut Buffer, elapsed: f64, throughput: f64) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let throughput = clamp01(throughput);
    if throughput <= 0.0 {
        return;
    }

    let cols = area.width as usize;
    let stream_prob = (0.12 + throughput * 0.72).clamp(0.0, 0.95);
    let speed = 4.0 + throughput * 8.0;
    let trail = 2 + (throughput * 8.0).round() as u16;
    let pulse = (elapsed * (1.4 + throughput * 2.2)).sin() * 0.5 + 0.5;

    for col in 0..cols {
        let x = area.x + col as u16;
        let seed = 0xD4_61_52_45_u64 ^ x as u64;
        if unit_from_hash(seed ^ elapsed.to_bits()) > stream_prob {
            continue;
        }

        let phase = unit_from_hash(seed ^ 0x19) * area.height.max(1) as f64;
        let head = ((elapsed * speed + phase) % area.height.max(1) as f64) as u16;
        let head_y = area.top().saturating_add(head.min(area.height.saturating_sub(1)));

        for offset in 0..trail {
            let y = head_y.saturating_sub(offset);
            if y < area.top() || y >= area.bottom() {
                continue;
            }

            let fade = 1.0 - offset as f64 / trail.max(1) as f64;
            let fg = pulse_tint(0.35 + 0.55 * fade, pulse * fade);
            let bg = pulse_tint(0.06 + 0.12 * fade, pulse * 0.35);
            let bits = match offset {
                0 => 0x7F,
                1 => 0x3B,
                2 => 0x1B,
                _ => braille_mask(seed, x, y, elapsed, fade),
            };

            if let Some(cell) = buf.cell_mut((x, y)) {
                write_blank_braille(cell, bits, fg, bg);
            }
        }
    }
}

/// Composite NervViz layer that combines progress, guide lines, ripples, and rain.
///
/// The composite is intentionally layered from broadest signal to smallest
/// detail, and every sub-effect still obeys the blank-cell-only rule.
pub fn state_viz(area: Rect, buf: &mut Buffer, elapsed: f64, ctx: &VizContext) {
    let progress = ctx.task_progress.max(ctx.plan_progress).clamp(0.0, 1.0);
    let activity = if ctx.agent_active {
        (0.25 + ctx.context_pressure * 0.55).clamp(0.0, 1.0)
    } else {
        ctx.context_pressure.clamp(0.0, 1.0) * 0.25
    };
    let token_rate = ctx.token_rate.clamp(0.0, 1.0);
    let error_boost = if ctx.error_state { 0.18 } else { 0.0 };

    if progress <= 0.0
        && activity <= 0.0
        && token_rate <= 0.0
        && error_boost <= 0.0
    {
        return;
    }

    progress_field(area, buf, elapsed, (progress + error_boost).clamp(0.0, 1.0));
    if ctx.agent_active {
        guide_lines(
            area,
            buf,
            elapsed + ctx.iteration as f64 * 0.07,
            (0.25 + ctx.context_pressure * 0.65 + error_boost).clamp(0.0, 1.0),
            ctx.iteration as u64 ^ 0xA6_56_4E_65,
        );
    }
    data_rain(area, buf, elapsed, token_rate.max(error_boost));
    activity_ripples(area, buf, elapsed, (activity + error_boost).clamp(0.0, 1.0));
}

/// Lightweight floating particle overlay for active-agent scenes.
///
/// `density` is normalized to `0.0..=1.0`. The overlay is deterministic for a
/// given `elapsed`/`seed` pair and only paints blank cells.
pub fn particle_overlay(
    area: Rect,
    buf: &mut Buffer,
    elapsed: f64,
    density: f64,
    brightness: u8,
    seed: u64,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let density = clamp01(density);
    if density <= 0.0 {
        return;
    }

    let area_cells = area.width as usize * area.height as usize;
    let slot_count = ((area_cells as f64).sqrt() * (0.85 + density * 1.8)).round() as usize;
    let slot_count = slot_count.clamp(4, 72);
    let lifetime = 2.0;
    let rise_speed = 0.55 + density * 0.9;
    let drift_speed = 0.18 + density * 0.35;
    let brightness = brightness.max(16);

    for slot in 0..slot_count {
        let slot_seed = seed ^ (slot as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let phase = unit_from_hash(slot_seed ^ 0x21);
        let age = (elapsed + phase * lifetime) % lifetime;
        let opacity = 1.0 - age / lifetime;
        if opacity <= 0.08 {
            continue;
        }

        let start_x = area.left().saturating_add(
            (unit_from_hash(slot_seed ^ 0x31) * area.width.saturating_sub(1) as f64).round()
                as u16,
        );
        let start_y = area.top().saturating_add(
            (unit_from_hash(slot_seed ^ 0x41) * area.height.saturating_sub(1) as f64).round()
                as u16,
        );

        let sway = ((elapsed * (0.7 + density) + phase * std::f64::consts::TAU).sin()) * 0.5;
        let x = (start_x as f64 + sway * drift_speed * area.width as f64 * age * 0.35)
            .round()
            .clamp(area.left() as f64, area.right().saturating_sub(1) as f64) as u16;
        let y = (start_y as f64 - age * rise_speed * area.height.max(1) as f64 * 0.33)
            .round()
            .clamp(area.top() as f64, area.bottom().saturating_sub(1) as f64) as u16;

        if let Some(cell) = buf.cell_mut((x, y)) {
            if !is_blank(cell) {
                continue;
            }

            let tint = pulse_tint(0.25 + 0.75 * opacity, opacity);
            let fg = match tint {
                Color::Rgb(r, g, b) => Color::Rgb(
                    scale(r, brightness as f64 / 160.0 * (0.55 + 0.45 * opacity)),
                    scale(g, brightness as f64 / 170.0 * (0.55 + 0.45 * opacity)),
                    scale(b, brightness as f64 / 155.0 * (0.55 + 0.45 * opacity)),
                ),
                other => other,
            };
            let bg = pulse_tint(0.05 + 0.15 * opacity, opacity * 0.25);
            let bits = braille_mask(slot_seed, x, y, elapsed, opacity);
            write_blank_braille(cell, bits, fg, bg);
        }
    }
}

// ---------------------------------------------------------------------------
// Existing screen effects
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

                        let kernel_area = ((r_end - r_start) * (c_end - c_start)).max(1) as f64;
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
            let dist = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
            let factor = 1.0 - intensity * (dist / max_dist).powi(2);
            let factor = factor.clamp(0.0, 1.0);

            if let Some(cell) = buf.cell_mut((x, y)) {
                if let Some((r, g, b)) = cell_fg_rgb(cell) {
                    cell.set_fg(Color::Rgb(
                        scale(r, factor),
                        scale(g, factor),
                        scale(b, factor),
                    ));
                }
                if let Some((r, g, b)) = cell_bg_rgb(cell) {
                    cell.set_bg(Color::Rgb(
                        scale(r, factor),
                        scale(g, factor),
                        scale(b, factor),
                    ));
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
                    cell.set_fg(Color::Rgb(
                        scale(r, factor),
                        scale(g, factor),
                        scale(b, factor),
                    ));
                }
                if let Some((r, g, b)) = cell_bg_rgb(cell) {
                    cell.set_bg(Color::Rgb(
                        scale(r, factor),
                        scale(g, factor),
                        scale(b, factor),
                    ));
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
        let seed = i as f64 * 1.618;
        let drift_x = ((elapsed * 0.3 + seed * 7.0).sin() * 0.5 + 0.5) * (area.width as f64 - 2.0);
        let drift_y =
            ((elapsed * 0.2 + seed * 13.0).cos() * 0.5 + 0.5) * (area.height as f64 - 2.0);
        let ox = area.x + 1 + drift_x as u16;
        let oy = area.y + 1 + drift_y as u16;

        let breath = (elapsed * 1.5 + seed * 3.0).sin() * 0.5 + 0.5;
        let b = (brightness as f64 * (0.3 + 0.7 * breath)).min(255.0) as u8;

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

                let dist_factor = if dx == 0 && dy == 0 { 1.0 } else { 0.4 };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_field_does_not_overwrite_text() {
        let area = Rect::new(0, 0, 8, 4);
        let mut buf = Buffer::empty(area);
        buf[(2, 2)].set_char('X');

        progress_field(area, &mut buf, 0.4, 1.0);

        assert_eq!(buf[(2, 2)].symbol(), "X");
    }

    #[test]
    fn particle_overlay_leaves_non_blank_cells_alone() {
        let area = Rect::new(0, 0, 8, 4);
        let mut buf = Buffer::empty(area);
        buf[(1, 1)].set_char('A');

        particle_overlay(area, &mut buf, 1.0, 1.0, 64, 7);

        assert_eq!(buf[(1, 1)].symbol(), "A");
    }
}
