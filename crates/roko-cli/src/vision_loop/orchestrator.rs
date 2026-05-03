//! Loop orchestrator: iteration cycle, stopping logic, regression detection.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use tokio::sync::watch;
use tracing::{info, warn};

use super::checkpoint::{CheckpointManager, RunMetadata};
use super::evaluator::VisionEvaluator;
use super::screenshot::ScreenshotService;
use super::{IterationRecord, StopReason, VisionLoopConfig, VisionLoopResult};

/// Orchestrates the vision-loop iteration cycle.
pub struct LoopOrchestrator {
    config: VisionLoopConfig,
    run_id: String,
    cancel_rx: watch::Receiver<bool>,
    cancel_tx: watch::Sender<bool>,
}

impl LoopOrchestrator {
    pub fn new(config: VisionLoopConfig) -> Result<Self> {
        if !config.target_file.exists() {
            bail!(
                "target file does not exist: {}",
                config.target_file.display()
            );
        }
        if config.goal.trim().is_empty() {
            bail!("goal must not be empty");
        }
        if config.url.trim().is_empty() {
            bail!("url must not be empty");
        }

        let run_id = uuid::Uuid::new_v4().to_string();
        let (cancel_tx, cancel_rx) = watch::channel(false);

        Ok(Self {
            config,
            run_id,
            cancel_rx,
            cancel_tx,
        })
    }

    /// Signal cancellation from outside (e.g. HTTP cancel endpoint).
    pub fn cancel(&self) {
        let _ = self.cancel_tx.send(true);
    }

    /// The run ID for this loop.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Execute the full vision loop.
    pub async fn run(self) -> Result<VisionLoopResult> {
        // 1. Validate prerequisites.
        ScreenshotService::check_availability().await?;

        let file_ext = self
            .config
            .target_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("txt")
            .to_string();

        // 2. Resolve working directory for .roko/.
        let roko_dir = find_roko_dir(&self.config.target_file)?;

        // 3. Load config for the evaluator.
        let roko_config = load_roko_config(&roko_dir)?;

        // 4. Initialize subsystems.
        let screenshot = ScreenshotService::new(
            self.config.viewport_width,
            self.config.viewport_height,
            self.config.wait_ms,
        );
        let checkpoint = CheckpointManager::new(&roko_dir, &self.run_id, &file_ext).await?;
        let project_root = roko_dir
            .parent()
            .unwrap_or(roko_dir.as_path())
            .to_path_buf();
        let evaluator = VisionEvaluator::new(
            roko_config,
            self.config.model_key.clone(),
            self.config.goal.clone(),
            file_ext.clone(),
            project_root,
        )?;

        // 5. Save original file.
        let original_code = tokio::fs::read_to_string(&self.config.target_file)
            .await
            .context("failed to read target file")?;
        checkpoint.save_original(&original_code).await?;

        // Save initial metadata.
        checkpoint
            .save_metadata(&RunMetadata {
                config: self.config.clone(),
                result: None,
            })
            .await?;

        info!(
            run_id = %self.run_id,
            target = %self.config.target_file.display(),
            model = %evaluator.model_key(),
            max_iterations = self.config.max_iterations,
            "starting vision loop"
        );

        // 6. Iteration loop.
        let mut history: Vec<IterationRecord> = Vec::new();
        let mut best_score: f64 = 0.0;
        let mut best_iteration: u32 = 0;
        let mut consecutive_target_hits: u32 = 0;
        let mut current_code = original_code;
        let mut stop_reason = StopReason::MaxIterations;
        let mut regression_hint_iter: Option<u32> = None;

        for iteration in 1..=self.config.max_iterations {
            // Check for cancellation.
            if *self.cancel_rx.borrow() {
                stop_reason = StopReason::UserCancel;
                break;
            }

            info!(iteration, "vision loop iteration starting");

            // 6a. Screenshot.
            let screenshot_path = checkpoint.screenshot_path(iteration);
            let png_bytes = screenshot
                .capture(&self.config.url, &screenshot_path)
                .await
                .context("screenshot capture failed")?;
            let data_uri = ScreenshotService::to_data_uri(&png_bytes);

            // 6b. Evaluate.
            let eval = evaluator
                .evaluate(&current_code, &data_uri, &history, regression_hint_iter)
                .await?;

            info!(
                iteration,
                score = eval.score,
                notes = %eval.notes,
                "vision evaluation complete"
            );

            // 6c. Save checkpoint.
            checkpoint
                .save_iteration(iteration, &eval.improved_code, &png_bytes, &eval)
                .await?;

            // 6d. Record history.
            let record = IterationRecord {
                iteration,
                score: eval.score,
                notes: eval.notes.clone(),
                timestamp: Utc::now(),
            };
            history.push(record);

            // 6e. Track best.
            if eval.score > best_score {
                best_score = eval.score;
                best_iteration = iteration;
            }

            // 6f. Regression detection.
            if best_score - eval.score >= self.config.regression_threshold {
                warn!(
                    iteration,
                    score = eval.score,
                    peak = best_score,
                    drop = best_score - eval.score,
                    "regression detected"
                );

                if regression_hint_iter.is_some() {
                    // Second regression after retry — give up.
                    info!("second regression after retry, rolling back to best");
                    let best_code = checkpoint.read_iteration_code(best_iteration).await?;
                    tokio::fs::write(&self.config.target_file, &best_code).await?;
                    stop_reason = StopReason::RegressionRollback;
                    break;
                }

                // First regression — retry once with hint.
                regression_hint_iter = Some(iteration);
                let best_code = checkpoint.read_iteration_code(best_iteration).await?;
                current_code = best_code.clone();
                tokio::fs::write(&self.config.target_file, &current_code).await?;
                info!("rolled back to iteration {best_iteration}, retrying with regression hint");
                continue;
            }

            // Clear regression hint on non-regressed iteration.
            regression_hint_iter = None;

            // 6g. Write improved code to target file (triggers HMR).
            current_code = eval.improved_code;
            tokio::fs::write(&self.config.target_file, &current_code).await?;

            // 6h. Check early stop.
            if eval.score >= self.config.target_score {
                consecutive_target_hits += 1;
                if consecutive_target_hits >= self.config.consecutive_target {
                    info!(
                        "target score {:.1} reached for {} consecutive iterations",
                        self.config.target_score, consecutive_target_hits
                    );
                    stop_reason = StopReason::TargetReached;
                    break;
                }
            } else {
                consecutive_target_hits = 0;
            }

            // 6i. Wait for HMR to settle before next screenshot.
            if iteration < self.config.max_iterations {
                tokio::time::sleep(tokio::time::Duration::from_millis(self.config.wait_ms)).await;
            }
        }

        // 7. Finalize: ensure we're on the best iteration.
        if !matches!(stop_reason, StopReason::RegressionRollback) && best_iteration > 0 {
            let best_code = checkpoint.read_iteration_code(best_iteration).await?;
            tokio::fs::write(&self.config.target_file, &best_code).await?;
        }

        let result = VisionLoopResult {
            run_id: self.run_id.clone(),
            stop_reason,
            iterations_completed: history.len() as u32,
            best_score,
            best_iteration,
            history,
        };

        // Save final metadata.
        checkpoint
            .save_metadata(&RunMetadata {
                config: self.config.clone(),
                result: Some(result.clone()),
            })
            .await?;

        info!(
            run_id = %self.run_id,
            stop = %result.stop_reason,
            best_score = result.best_score,
            best_iteration = result.best_iteration,
            iterations = result.iterations_completed,
            "vision loop complete"
        );

        Ok(result)
    }
}

/// Walk up from the target file to find `.roko/` directory.
fn find_roko_dir(target_file: &std::path::Path) -> Result<PathBuf> {
    let mut dir = target_file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // Canonicalize to handle relative paths.
    if dir.is_relative() {
        dir = std::env::current_dir()?.join(dir);
    }

    loop {
        let candidate = dir.join(".roko");
        if candidate.is_dir() {
            return Ok(candidate);
        }
        if !dir.pop() {
            break;
        }
    }

    // Fallback: use cwd/.roko.
    let cwd = std::env::current_dir()?;
    let candidate = cwd.join(".roko");
    if candidate.is_dir() {
        return Ok(candidate);
    }

    bail!(
        "no .roko/ directory found. Run `roko init` first, or ensure .roko/ exists \
         in the project root."
    )
}

/// Load `roko.toml` configuration.
fn load_roko_config(roko_dir: &std::path::Path) -> Result<roko_core::config::schema::RokoConfig> {
    let project_root = roko_dir.parent().unwrap_or(roko_dir);
    roko_core::config::loader::load_config_unified(project_root).map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orchestrator_rejects_missing_target() {
        let config = VisionLoopConfig {
            target_file: "/nonexistent/file.tsx".into(),
            goal: "make it pretty".into(),
            url: "http://localhost:5173".into(),
            ..Default::default()
        };
        assert!(LoopOrchestrator::new(config).is_err());
    }

    #[test]
    fn orchestrator_rejects_empty_goal() {
        let config = VisionLoopConfig {
            target_file: std::env::current_exe().unwrap(), // use any existing file
            goal: "".into(),
            url: "http://localhost:5173".into(),
            ..Default::default()
        };
        assert!(LoopOrchestrator::new(config).is_err());
    }

    #[test]
    fn orchestrator_rejects_empty_url() {
        let config = VisionLoopConfig {
            target_file: std::env::current_exe().unwrap(),
            goal: "make it nice".into(),
            url: "".into(),
            ..Default::default()
        };
        assert!(LoopOrchestrator::new(config).is_err());
    }

    #[test]
    fn cancellation_works() {
        let config = VisionLoopConfig {
            target_file: std::env::current_exe().unwrap(),
            goal: "test".into(),
            url: "http://localhost:5173".into(),
            ..Default::default()
        };
        let orch = LoopOrchestrator::new(config).unwrap();
        assert!(!*orch.cancel_rx.borrow());
        orch.cancel();
        assert!(*orch.cancel_rx.borrow());
    }
}
