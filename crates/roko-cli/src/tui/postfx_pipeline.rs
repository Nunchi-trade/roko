//! Per-tab post-processing pipeline.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::effects_config::EffectsConfig;
use super::postfx;
use super::state::TuiState;

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
    state: &TuiState,
) {
    let show_state_vfx = matches!(tab_idx, 0 | 1 | 2) && (fx.nerv_viz || fx.particles);
    if !fx.screen_postfx && !show_state_vfx {
        return;
    }

    if fx.screen_postfx {
        match tab_idx {
            0 | 1 | 2 => {
                self_glow(area, buf, 200, 0.12);
            }
            _ => {}
        }

        if fx.bloom_enabled {
            postfx::bloom(area, buf, 220, 1, fx.bloom_intensity);
        }

        if fx.shadows_enabled {
            postfx::drop_shadow(buf, area);
        }

        if fx.vfx_enabled {
            postfx::ambient_orbs(area, buf, elapsed, 3, 40);
            postfx::dream_atmosphere(area, buf, elapsed, frame);
        }
    }

    if show_state_vfx {
        let viz_ctx = build_viz_context(state);
        if fx.nerv_viz {
            postfx::state_viz(area, buf, elapsed, &viz_ctx);
        }
        if fx.particles {
            let active_agents = state.active_agent_count();
            if active_agents > 0 {
                let density = (active_agents as f64 / 4.0).clamp(0.25, 1.0);
                postfx::particle_overlay(area, buf, elapsed, density, 72, frame);
            }
        }
    }
}

fn build_viz_context(state: &TuiState) -> postfx::VizContext {
    let (done_tasks, total_tasks) = state.task_counts();
    let task_progress = ratio(done_tasks, total_tasks);
    let plan_progress = if state.plans.is_empty() {
        0.0
    } else {
        state
            .plans
            .iter()
            .map(|plan| {
                if plan.tasks_total > 0 {
                    ratio(plan.tasks_done, plan.tasks_total)
                } else if plan.status.is_done() {
                    1.0
                } else if plan.active {
                    0.35
                } else {
                    0.0
                }
            })
            .sum::<f64>()
            / state.plans.len() as f64
    };

    let active_context_pressure = state
        .agents
        .iter()
        .filter(|agent| agent.active)
        .filter_map(|agent| state.route_metrics.get(&agent.id))
        .map(utilization)
        .fold(0.0, f64::max);
    let context_pressure = if active_context_pressure > 0.0 {
        active_context_pressure
    } else {
        state
            .route_metrics
            .values()
            .map(utilization)
            .fold(0.0, f64::max)
    };

    postfx::VizContext {
        task_progress,
        plan_progress,
        context_pressure,
        token_rate: normalize_token_rate(state.token_rate),
        agent_active: state.active_agent_count() > 0,
        iteration: state.current_iteration.min(u32::MAX as usize) as u32,
        error_state: state.gate_results.iter().any(|gate| !gate.passed),
    }
}

fn ratio(done: usize, total: usize) -> f64 {
    if total > 0 {
        done as f64 / total as f64
    } else {
        0.0
    }
}

fn utilization(metric: &super::state::RouteMetrics) -> f64 {
    if metric.context_limit == 0 {
        0.0
    } else {
        (metric.context_used as f64 / metric.context_limit as f64).clamp(0.0, 1.0)
    }
}

fn normalize_token_rate(rate: f64) -> f64 {
    if rate <= 0.0 {
        0.0
    } else {
        (rate / (rate + 300.0)).clamp(0.0, 1.0)
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
