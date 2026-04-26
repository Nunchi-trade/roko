//! Screenshot capture via Playwright CLI.

use std::path::Path;

use anyhow::{Context, Result, bail};
use base64::Engine;
use tokio::process::Command;

/// Captures screenshots of a URL using `npx playwright screenshot`.
pub struct ScreenshotService {
    viewport_width: u32,
    viewport_height: u32,
    wait_ms: u64,
}

impl ScreenshotService {
    pub fn new(viewport_width: u32, viewport_height: u32, wait_ms: u64) -> Self {
        Self {
            viewport_width,
            viewport_height,
            wait_ms,
        }
    }

    /// Check that Playwright is available.
    pub async fn check_availability() -> Result<()> {
        let output = Command::new("npx")
            .args(["playwright", "--version"])
            .output()
            .await
            .context("failed to run `npx playwright --version` — is Node.js installed?")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "Playwright not available. Install with `npx playwright install chromium`.\n{stderr}"
            );
        }
        Ok(())
    }

    /// Capture a screenshot and save it to `output_path`. Returns the raw PNG bytes.
    pub async fn capture(&self, url: &str, output_path: &Path) -> Result<Vec<u8>> {
        // Ensure parent directory exists.
        if let Some(parent) = output_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let viewport = format!("{}x{}", self.viewport_width, self.viewport_height);

        let output = Command::new("npx")
            .args([
                "playwright",
                "screenshot",
                "--viewport-size",
                &viewport,
                "--wait-for-timeout",
                &self.wait_ms.to_string(),
                url,
                &output_path.display().to_string(),
            ])
            .output()
            .await
            .context("failed to run playwright screenshot")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("playwright screenshot failed: {stderr}");
        }

        let bytes = tokio::fs::read(output_path)
            .await
            .context("failed to read screenshot file")?;

        Ok(bytes)
    }

    /// Encode PNG bytes as a base64 data URI for multimodal model input.
    pub fn to_data_uri(png_bytes: &[u8]) -> String {
        let encoded = base64::engine::general_purpose::STANDARD.encode(png_bytes);
        format!("data:image/png;base64,{encoded}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_data_uri_produces_valid_prefix() {
        let bytes = vec![0x89, 0x50, 0x4e, 0x47]; // PNG magic bytes
        let uri = ScreenshotService::to_data_uri(&bytes);
        assert!(uri.starts_with("data:image/png;base64,"));
        // Verify base64 portion decodes back
        let b64 = uri.strip_prefix("data:image/png;base64,").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        assert_eq!(decoded, bytes);
    }
}
