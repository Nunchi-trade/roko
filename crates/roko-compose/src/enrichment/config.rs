//! Enrichment pipeline configuration.
//!
//! `EnrichmentConfig` carries all parameters for an enrichment run. It has no
//! `Default` impl — callers must be explicit about every field.
//!
//! Anti-pattern #3: all environment comes from this struct. No env-var reads
//! inside the pipeline.

use std::path::PathBuf;

use super::step::LlmBackend;

/// Configuration for an enrichment run.
///
/// No `Default` impl — callers must be explicit about every field.
/// Roko-owned enrichment run context.
#[allow(clippy::struct_excessive_bools)]
pub struct EnrichmentConfig {
    /// Project root directory. Plan artifacts live under
    /// `{repo_root}/.roko/plans/{plan_base}/`.
    pub repo_root: PathBuf,

    /// LLM backend selector (Claude, Codex, Cursor).
    pub backend: LlmBackend,

    /// Gateway URL (if routing through a model gateway).
    pub gateway_url: Option<String>,

    /// Gateway API key.
    pub gateway_key: Option<String>,

    /// Use batch API instead of real-time.
    pub batch_mode: bool,

    /// Override the default model for all steps.
    pub model_override: Option<String>,

    /// Regenerate even if output file already exists and is fresh.
    pub force: bool,

    /// Print what would be done without executing.
    pub dry_run: bool,

    /// Suppress direct stdout/stderr logging (when run inside a TUI).
    pub quiet: bool,
}

impl EnrichmentConfig {
    /// Resolve the plan directory for a given plan base name.
    ///
    /// Checks `{repo_root}/plans/{plan_base}` first (top-level layout used by
    /// `roko plan run`), falling back to `{repo_root}/.roko/plans/{plan_base}`.
    pub fn plan_dir(&self, plan_base: &str) -> PathBuf {
        let top = self.repo_root.join("plans").join(plan_base);
        if top.is_dir() {
            return top;
        }
        self.repo_root.join(".roko").join("plans").join(plan_base)
    }
}
