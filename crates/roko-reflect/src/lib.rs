//! REFLECT — nightly performance review + APEX config adapter.
//!
//! Ports the public surface of `modules/reflect_adapter.py`,
//! `reflect_engine.py` (metrics), and `reflect_reporter.py`. The
//! Obsidian vault writer and statistical deep-dives land in a follow-up
//! commit (plans/P08-trading-surface.md T9 close-out).

#![forbid(unsafe_code)]
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

/// Core performance summary over a window of round-trip trades.
///
/// Mirrors `modules.reflect_engine.ReflectMetrics`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReflectMetrics {
    pub total_round_trips: u32,
    pub win_rate: f64,
    pub gross_pnl: f64,
    pub total_fees: f64,
    pub net_pnl: f64,
    /// False-drop rate — % of trades that exited for stagnation or conviction collapse.
    pub fdr: f64,
    pub avg_hold_minutes: f64,
    pub best_trade_pnl: f64,
    pub worst_trade_pnl: f64,
}

impl ReflectMetrics {
    #[must_use]
    pub fn profitable(&self) -> bool {
        self.net_pnl > 0.0
    }
}

/// A single proposed config adjustment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Adjustment {
    pub param: String,
    pub old_value: serde_json::Value,
    pub new_value: serde_json::Value,
    pub reason: String,
}

/// Result of running the adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptOutcome {
    pub adjustments: Vec<Adjustment>,
    pub summary: String,
}

/// Adapter that targets APEX config fields. Pure — never mutates config in
/// place. Callers apply returned [`Adjustment`]s themselves.
pub fn adapt(metrics: &ReflectMetrics, snapshot: &ConfigSnapshot) -> AdaptOutcome {
    let mut out = AdaptOutcome {
        adjustments: vec![],
        summary: String::new(),
    };

    if metrics.total_round_trips < 3 {
        out.summary = "REFLECT: insufficient data (<3 round trips)".into();
        return out;
    }

    if metrics.total_fees > metrics.gross_pnl.abs() && metrics.total_round_trips >= 3 {
        out.adjustments.extend(emergency_tighten(snapshot));
        out.summary = "REFLECT: EMERGENCY — fees exceed gross PnL, tightening all entries".into();
        return out;
    }

    if metrics.fdr > 30.0 {
        if let Some(adj) = clamp_adjust(
            "radar_score_threshold",
            snapshot.radar_score_threshold as f64 + 10.0,
            "FDR critical (>30%): raise radar threshold",
            snapshot.radar_score_threshold as f64,
            BOUNDS_RADAR,
        ) {
            out.adjustments.push(adj);
        }
        if snapshot.pulse_immediate_auto_entry {
            out.adjustments.push(Adjustment {
                param: "pulse_immediate_auto_entry".into(),
                old_value: serde_json::Value::Bool(true),
                new_value: serde_json::Value::Bool(false),
                reason: "FDR critical: disable immediate mover entries".into(),
            });
        }
    } else if metrics.fdr > 20.0 {
        if let Some(adj) = clamp_adjust(
            "pulse_confidence_threshold",
            snapshot.pulse_confidence_threshold + 5.0,
            "FDR warning (>20%): raise pulse confidence bar",
            snapshot.pulse_confidence_threshold,
            BOUNDS_PULSE,
        ) {
            out.adjustments.push(adj);
        }
    }

    if metrics.win_rate < 40.0 && metrics.total_round_trips >= 5 {
        if let Some(adj) = clamp_adjust(
            "radar_score_threshold",
            snapshot.radar_score_threshold as f64 + 10.0,
            &format!("Win rate low ({:.0}%): raise radar threshold", metrics.win_rate),
            snapshot.radar_score_threshold as f64,
            BOUNDS_RADAR,
        ) {
            out.adjustments.push(adj);
        }
        if let Some(adj) = clamp_adjust(
            "pulse_confidence_threshold",
            snapshot.pulse_confidence_threshold + 10.0,
            &format!("Win rate low ({:.0}%): raise pulse confidence", metrics.win_rate),
            snapshot.pulse_confidence_threshold,
            BOUNDS_PULSE,
        ) {
            out.adjustments.push(adj);
        }
    }

    if metrics.gross_pnl > 0.0 && metrics.net_pnl < 0.0 {
        if let Some(adj) = clamp_adjust(
            "daily_loss_limit",
            (snapshot.daily_loss_limit * 0.8).max(50.0),
            "Fee bleed: tighten daily loss limit",
            snapshot.daily_loss_limit,
            BOUNDS_DAILY,
        ) {
            out.adjustments.push(adj);
        }
    }

    out.summary = format!(
        "REFLECT: {} adjustments (round_trips={}, fdr={:.1}%, win_rate={:.1}%)",
        out.adjustments.len(),
        metrics.total_round_trips,
        metrics.fdr,
        metrics.win_rate
    );
    out
}

/// Minimal subset of APEX config the adapter reads from / writes to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    pub radar_score_threshold: u32,
    pub pulse_confidence_threshold: f64,
    pub daily_loss_limit: f64,
    pub pulse_immediate_auto_entry: bool,
}

const BOUNDS_RADAR: (f64, f64) = (120.0, 280.0);
const BOUNDS_PULSE: (f64, f64) = (40.0, 95.0);
const BOUNDS_DAILY: (f64, f64) = (50.0, 5_000.0);

fn emergency_tighten(snapshot: &ConfigSnapshot) -> Vec<Adjustment> {
    let mut out = vec![];
    if let Some(adj) = clamp_adjust(
        "radar_score_threshold",
        snapshot.radar_score_threshold as f64 + 30.0,
        "EMERGENCY: fees > gross PnL",
        snapshot.radar_score_threshold as f64,
        BOUNDS_RADAR,
    ) {
        out.push(adj);
    }
    if let Some(adj) = clamp_adjust(
        "pulse_confidence_threshold",
        snapshot.pulse_confidence_threshold + 15.0,
        "EMERGENCY: fees > gross PnL",
        snapshot.pulse_confidence_threshold,
        BOUNDS_PULSE,
    ) {
        out.push(adj);
    }
    if snapshot.pulse_immediate_auto_entry {
        out.push(Adjustment {
            param: "pulse_immediate_auto_entry".into(),
            old_value: serde_json::Value::Bool(true),
            new_value: serde_json::Value::Bool(false),
            reason: "EMERGENCY: fees > gross PnL".into(),
        });
    }
    out
}

fn clamp_adjust(param: &str, candidate: f64, reason: &str, current: f64, bounds: (f64, f64)) -> Option<Adjustment> {
    let new_value = candidate.clamp(bounds.0, bounds.1);
    if (new_value - current).abs() < f64::EPSILON {
        return None;
    }
    Some(Adjustment {
        param: param.into(),
        old_value: serde_json::Value::from(current),
        new_value: serde_json::Value::from(new_value),
        reason: reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_snapshot() -> ConfigSnapshot {
        ConfigSnapshot {
            radar_score_threshold: 170,
            pulse_confidence_threshold: 70.0,
            daily_loss_limit: 500.0,
            pulse_immediate_auto_entry: true,
        }
    }

    #[test]
    fn insufficient_data_emits_no_adjustments() {
        let metrics = ReflectMetrics { total_round_trips: 2, ..ReflectMetrics::default() };
        let out = adapt(&metrics, &base_snapshot());
        assert!(out.adjustments.is_empty());
    }

    #[test]
    fn emergency_tightens_when_fees_exceed_gross_pnl() {
        let metrics = ReflectMetrics {
            total_round_trips: 5,
            gross_pnl: 100.0,
            total_fees: 150.0,
            net_pnl: -50.0,
            ..ReflectMetrics::default()
        };
        let out = adapt(&metrics, &base_snapshot());
        assert!(out.summary.contains("EMERGENCY"));
        assert!(!out.adjustments.is_empty());
    }

    #[test]
    fn fdr_above_30_raises_radar_threshold() {
        let metrics = ReflectMetrics {
            total_round_trips: 10,
            fdr: 35.0,
            gross_pnl: 200.0,
            total_fees: 50.0,
            net_pnl: 150.0,
            win_rate: 60.0,
            ..ReflectMetrics::default()
        };
        let out = adapt(&metrics, &base_snapshot());
        assert!(out.adjustments.iter().any(|a| a.param == "radar_score_threshold"));
    }
}
