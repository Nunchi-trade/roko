//! Vision loop: iterative UI improvement via screenshot → vision model → code gen.
//!
//! `roko vision-loop` captures a running page, sends the screenshot to a vision
//! model alongside the current code and a goal description, receives improved
//! code with a quality score, writes the file (triggering HMR), and repeats for
//! N iterations.

pub mod checkpoint;
pub mod evaluator;
pub mod orchestrator;
pub mod prompt;
pub mod screenshot;

use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use roko_core::defaults::{
    DEFAULT_VISION_LOOP_CONSECUTIVE_TARGET, DEFAULT_VISION_LOOP_MAX_ITERATIONS,
    DEFAULT_VISION_LOOP_REGRESSION_THRESHOLD, DEFAULT_VISION_LOOP_TARGET_SCORE,
    DEFAULT_VISION_LOOP_VIEWPORT_HEIGHT, DEFAULT_VISION_LOOP_VIEWPORT_WIDTH,
    DEFAULT_VISION_LOOP_WAIT_MS,
};
use serde::{Deserialize, Serialize};

// ── Config ──────────────────────────────────────────────────────────────

/// Configuration for a single vision-loop run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionLoopConfig {
    /// Source file to iterate on (e.g. `src/pages/Home.tsx`).
    pub target_file: PathBuf,
    /// What the UI should look/feel like.
    pub goal: String,
    /// URL to screenshot (e.g. `http://localhost:5173/`).
    pub url: String,
    /// Maximum number of iterations before stopping.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Score threshold for early stopping (1–10).
    #[serde(default = "default_target_score")]
    pub target_score: f64,
    /// How many consecutive iterations must hit `target_score` to stop.
    #[serde(default = "default_consecutive_target")]
    pub consecutive_target: u32,
    /// Score drop from peak that triggers rollback.
    #[serde(default = "default_regression_threshold")]
    pub regression_threshold: f64,
    /// Model key from `roko.toml` (must support vision).
    pub model_key: Option<String>,
    /// Viewport width in pixels.
    #[serde(default = "default_viewport_width")]
    pub viewport_width: u32,
    /// Viewport height in pixels.
    #[serde(default = "default_viewport_height")]
    pub viewport_height: u32,
    /// Milliseconds to wait after writing the file (HMR settle time).
    #[serde(default = "default_wait_ms")]
    pub wait_ms: u64,
}

fn default_max_iterations() -> u32 {
    DEFAULT_VISION_LOOP_MAX_ITERATIONS
}
fn default_target_score() -> f64 {
    DEFAULT_VISION_LOOP_TARGET_SCORE
}
fn default_consecutive_target() -> u32 {
    DEFAULT_VISION_LOOP_CONSECUTIVE_TARGET
}
fn default_regression_threshold() -> f64 {
    DEFAULT_VISION_LOOP_REGRESSION_THRESHOLD
}
fn default_viewport_width() -> u32 {
    DEFAULT_VISION_LOOP_VIEWPORT_WIDTH
}
fn default_viewport_height() -> u32 {
    DEFAULT_VISION_LOOP_VIEWPORT_HEIGHT
}
fn default_wait_ms() -> u64 {
    DEFAULT_VISION_LOOP_WAIT_MS
}

impl Default for VisionLoopConfig {
    fn default() -> Self {
        Self {
            target_file: PathBuf::new(),
            goal: String::new(),
            url: String::new(),
            max_iterations: default_max_iterations(),
            target_score: default_target_score(),
            consecutive_target: default_consecutive_target(),
            regression_threshold: default_regression_threshold(),
            model_key: None,
            viewport_width: default_viewport_width(),
            viewport_height: default_viewport_height(),
            wait_ms: default_wait_ms(),
        }
    }
}

// ── Evaluation ──────────────────────────────────────────────────────────

/// Result of a single vision model evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evaluation {
    /// Quality score from 1 to 10.
    pub score: f64,
    /// What's good, what needs improvement.
    pub notes: String,
    /// Full replacement file contents.
    pub improved_code: String,
}

// ── Iteration record ────────────────────────────────────────────────────

/// Recorded state for a single iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationRecord {
    pub iteration: u32,
    pub score: f64,
    pub notes: String,
    pub timestamp: DateTime<Utc>,
}

// ── Stop reason ─────────────────────────────────────────────────────────

/// Why the vision loop ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum StopReason {
    TargetReached,
    MaxIterations,
    RegressionRollback,
    UserCancel,
    Error { message: String },
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TargetReached => write!(f, "target score reached"),
            Self::MaxIterations => write!(f, "max iterations reached"),
            Self::RegressionRollback => write!(f, "regression detected, rolled back to best"),
            Self::UserCancel => write!(f, "cancelled by user"),
            Self::Error { message } => write!(f, "error: {message}"),
        }
    }
}

// ── Result ──────────────────────────────────────────────────────────────

/// Summary of a completed vision-loop run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionLoopResult {
    pub run_id: String,
    pub stop_reason: StopReason,
    pub iterations_completed: u32,
    pub best_score: f64,
    pub best_iteration: u32,
    pub history: Vec<IterationRecord>,
}

// ── CLI entry point ─────────────────────────────────────────────────────

/// Run the vision loop from the CLI.
pub async fn cmd_vision_loop(config: VisionLoopConfig) -> Result<VisionLoopResult> {
    let orch = orchestrator::LoopOrchestrator::new(config)?;
    orch.run().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sensible_values() {
        let cfg = VisionLoopConfig::default();
        assert_eq!(cfg.max_iterations, DEFAULT_VISION_LOOP_MAX_ITERATIONS);
        assert!((cfg.target_score - DEFAULT_VISION_LOOP_TARGET_SCORE).abs() < f64::EPSILON);
        assert_eq!(
            cfg.consecutive_target,
            DEFAULT_VISION_LOOP_CONSECUTIVE_TARGET
        );
        assert!(
            (cfg.regression_threshold - DEFAULT_VISION_LOOP_REGRESSION_THRESHOLD).abs()
                < f64::EPSILON
        );
        assert_eq!(cfg.viewport_width, DEFAULT_VISION_LOOP_VIEWPORT_WIDTH);
        assert_eq!(cfg.viewport_height, DEFAULT_VISION_LOOP_VIEWPORT_HEIGHT);
        assert_eq!(cfg.wait_ms, DEFAULT_VISION_LOOP_WAIT_MS);
    }

    #[test]
    fn stop_reason_display() {
        assert_eq!(
            StopReason::TargetReached.to_string(),
            "target score reached"
        );
        assert_eq!(
            StopReason::MaxIterations.to_string(),
            "max iterations reached"
        );
        assert_eq!(
            StopReason::Error {
                message: "oops".into()
            }
            .to_string(),
            "error: oops"
        );
    }

    #[test]
    fn evaluation_roundtrips_json() {
        let eval = Evaluation {
            score: 7.5,
            notes: "layout good, colors off".into(),
            improved_code: "<div>hello</div>".into(),
        };
        let json = serde_json::to_string(&eval).unwrap();
        let parsed: Evaluation = serde_json::from_str(&json).unwrap();
        assert!((parsed.score - 7.5).abs() < f64::EPSILON);
        assert_eq!(parsed.improved_code, "<div>hello</div>");
    }
}
