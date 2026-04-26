//! Checkpoint management for vision-loop runs.
//!
//! Layout:
//! ```text
//! .roko/vision-loops/{run_id}/
//!   original.{ext}        # Backup before any mutations
//!   metadata.json          # Config + result
//!   001/
//!     code.{ext}
//!     screenshot.png
//!     eval.json            # { score, notes }
//!   002/
//!     ...
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{Evaluation, VisionLoopConfig, VisionLoopResult};

/// Manages checkpoint persistence for a single vision-loop run.
pub struct CheckpointManager {
    run_dir: PathBuf,
    file_ext: String,
}

/// Persisted metadata for the run.
#[derive(Debug, Serialize, Deserialize)]
pub struct RunMetadata {
    pub config: VisionLoopConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<VisionLoopResult>,
}

impl CheckpointManager {
    /// Create a new checkpoint manager. Creates the run directory.
    pub async fn new(roko_dir: &Path, run_id: &str, ext: &str) -> Result<Self> {
        let run_dir = roko_dir.join("vision-loops").join(run_id);
        tokio::fs::create_dir_all(&run_dir)
            .await
            .context("failed to create vision-loop checkpoint directory")?;
        Ok(Self {
            run_dir,
            file_ext: ext.to_string(),
        })
    }

    /// Save the original file contents before any mutations.
    pub async fn save_original(&self, contents: &str) -> Result<()> {
        let path = self.run_dir.join(format!("original.{}", self.file_ext));
        tokio::fs::write(&path, contents)
            .await
            .context("failed to save original file backup")?;
        Ok(())
    }

    /// Read back the original file contents.
    pub async fn read_original(&self) -> Result<String> {
        let path = self.run_dir.join(format!("original.{}", self.file_ext));
        tokio::fs::read_to_string(&path)
            .await
            .context("failed to read original file backup")
    }

    /// Save an iteration's code, screenshot, and evaluation.
    pub async fn save_iteration(
        &self,
        iteration: u32,
        code: &str,
        screenshot_bytes: &[u8],
        eval: &Evaluation,
    ) -> Result<()> {
        let iter_dir = self.run_dir.join(format!("{iteration:03}"));
        tokio::fs::create_dir_all(&iter_dir).await?;

        let code_path = iter_dir.join(format!("code.{}", self.file_ext));
        let screenshot_path = iter_dir.join("screenshot.png");
        let eval_path = iter_dir.join("eval.json");

        tokio::fs::write(&code_path, code).await?;
        tokio::fs::write(&screenshot_path, screenshot_bytes).await?;

        let eval_json = serde_json::json!({
            "score": eval.score,
            "notes": eval.notes,
        });
        tokio::fs::write(&eval_path, serde_json::to_string_pretty(&eval_json)?).await?;

        Ok(())
    }

    /// Read code from a specific iteration.
    pub async fn read_iteration_code(&self, iteration: u32) -> Result<String> {
        let path = self
            .run_dir
            .join(format!("{iteration:03}"))
            .join(format!("code.{}", self.file_ext));
        tokio::fs::read_to_string(&path)
            .await
            .context("failed to read iteration code")
    }

    /// Save run metadata (config + result).
    pub async fn save_metadata(&self, metadata: &RunMetadata) -> Result<()> {
        let path = self.run_dir.join("metadata.json");
        let json = serde_json::to_string_pretty(metadata)?;
        tokio::fs::write(&path, json).await?;
        Ok(())
    }

    /// The run directory path.
    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    /// Path to a specific iteration's screenshot.
    pub fn screenshot_path(&self, iteration: u32) -> PathBuf {
        self.run_dir
            .join(format!("{iteration:03}"))
            .join("screenshot.png")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn save_and_read_original_roundtrip() {
        let tmp = tempdir().unwrap();
        let mgr = CheckpointManager::new(tmp.path(), "test-run", "tsx")
            .await
            .unwrap();

        let original = "export default function App() { return <div>Hello</div>; }";
        mgr.save_original(original).await.unwrap();

        let read_back = mgr.read_original().await.unwrap();
        assert_eq!(read_back, original);
    }

    #[tokio::test]
    async fn save_iteration_creates_expected_files() {
        let tmp = tempdir().unwrap();
        let mgr = CheckpointManager::new(tmp.path(), "test-run", "tsx")
            .await
            .unwrap();

        let eval = Evaluation {
            score: 7.0,
            notes: "good layout".into(),
            improved_code: "<div>improved</div>".into(),
        };

        mgr.save_iteration(1, "<div>improved</div>", b"fake-png", &eval)
            .await
            .unwrap();

        let iter_dir = mgr.run_dir().join("001");
        assert!(iter_dir.join("code.tsx").exists());
        assert!(iter_dir.join("screenshot.png").exists());
        assert!(iter_dir.join("eval.json").exists());

        let code = mgr.read_iteration_code(1).await.unwrap();
        assert_eq!(code, "<div>improved</div>");

        let eval_json: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(iter_dir.join("eval.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(eval_json["score"], 7.0);
    }

    #[tokio::test]
    async fn save_metadata_roundtrip() {
        let tmp = tempdir().unwrap();
        let mgr = CheckpointManager::new(tmp.path(), "test-run", "vue")
            .await
            .unwrap();

        let metadata = RunMetadata {
            config: VisionLoopConfig {
                target_file: "src/App.vue".into(),
                goal: "make it pretty".into(),
                url: "http://localhost:5173".into(),
                ..Default::default()
            },
            result: None,
        };

        mgr.save_metadata(&metadata).await.unwrap();

        let path = mgr.run_dir().join("metadata.json");
        let read: RunMetadata =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(read.config.goal, "make it pretty");
        assert!(read.result.is_none());
    }
}
