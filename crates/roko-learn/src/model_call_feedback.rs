//! Durable model-call feedback recording helpers.
//!
//! This module centralizes the common persistence sequence used by direct
//! model-call surfaces: write efficiency feedback, update provider health, and
//! save cascade-router observations under `.roko/learn`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use roko_core::Result;
use roko_core::foundation::{FeedbackEvent, FeedbackSink};

use crate::cascade_router::CascadeRouter;
use crate::feedback_service::FeedbackService;
use crate::model_router::CONTEXT_DIM;
use crate::provider_health::{ErrorClass, ProviderHealthRegistry};

/// Metrics and identity for one model call.
#[derive(Debug, Clone)]
pub struct ModelCallFeedback {
    /// Optional workflow run id.
    pub run_id: Option<String>,
    /// Optional request id for correlating this call with caller logs.
    pub request_id: Option<String>,
    /// Prompt section ids included in the request.
    pub prompt_section_ids: Vec<String>,
    /// Knowledge ids included in the request.
    pub knowledge_ids: Vec<String>,
    /// Model slug used for learning and cascade-router observations.
    pub model: String,
    /// Provider id used for feedback and persisted provider health.
    pub provider: String,
    /// Caller role or surface, for example `dispatch_v2`.
    pub role: String,
    /// Input tokens reported by the provider.
    pub input_tokens: u64,
    /// Output tokens reported by the provider.
    pub output_tokens: u64,
    /// Cost reported by the provider.
    pub cost_usd: f64,
    /// End-to-end model-call latency in milliseconds.
    pub latency_ms: u64,
    /// Learning outcome for this call or workflow stage.
    pub success: bool,
    /// Provider transport outcome. When omitted, uses [`Self::success`].
    pub provider_success: Option<bool>,
}

impl ModelCallFeedback {
    /// Total tokens reported by the provider.
    #[must_use]
    pub const fn token_usage(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    fn provider_health_success(&self) -> bool {
        self.provider_success.unwrap_or(self.success)
    }
}

/// Recorder for direct model-call learning persistence.
pub struct ModelCallFeedbackRecorder {
    learn_dir: PathBuf,
    cascade_path: PathBuf,
    cascade_router: Option<Arc<CascadeRouter>>,
    save_cascade_router: bool,
}

impl ModelCallFeedbackRecorder {
    /// Create a recorder rooted at a workspace/project directory.
    #[must_use]
    pub fn from_workdir(workdir: &Path, model_slugs: Vec<String>) -> Self {
        Self::from_learn_dir(workdir.join(".roko").join("learn"), model_slugs)
    }

    /// Create a recorder rooted directly at a `.roko/learn` directory.
    #[must_use]
    pub fn from_learn_dir(learn_dir: PathBuf, model_slugs: Vec<String>) -> Self {
        let cascade_path = learn_dir.join("cascade-router.json");
        let cascade_router = (!model_slugs.is_empty())
            .then(|| Arc::new(CascadeRouter::load_or_new(&cascade_path, model_slugs)));
        Self {
            learn_dir,
            cascade_path,
            cascade_router,
            save_cascade_router: true,
        }
    }

    /// Create a recorder using an existing in-memory cascade router.
    ///
    /// This is useful for long-lived services that keep one router in memory
    /// and need direct model-call observations to update that same instance.
    #[must_use]
    pub fn with_cascade_router(learn_dir: PathBuf, cascade_router: Arc<CascadeRouter>) -> Self {
        Self {
            cascade_path: learn_dir.join("cascade-router.json"),
            learn_dir,
            cascade_router: Some(cascade_router),
            save_cascade_router: true,
        }
    }

    /// Create a recorder without cascade-router observation.
    #[must_use]
    pub fn without_cascade_router(learn_dir: PathBuf) -> Self {
        Self {
            cascade_path: learn_dir.join("cascade-router.json"),
            learn_dir,
            cascade_router: None,
            save_cascade_router: false,
        }
    }

    /// Record model-call feedback, provider health, and cascade observation.
    ///
    /// # Errors
    ///
    /// Returns an error if any durable write fails.
    pub async fn record(&self, feedback: ModelCallFeedback) -> Result<()> {
        self.record_provider_health(&feedback)?;

        let mut feedback_service = FeedbackService::new(self.learn_dir.clone());
        if let Some(router) = &self.cascade_router {
            feedback_service = feedback_service.with_cascade_router(Arc::clone(router));
        }

        let token_usage = feedback.token_usage();
        feedback_service
            .record(FeedbackEvent::ModelCall {
                run_id: feedback.run_id,
                request_id: feedback.request_id,
                prompt_section_ids: feedback.prompt_section_ids,
                knowledge_ids: feedback.knowledge_ids,
                model: Some(feedback.model),
                provider: Some(feedback.provider),
                token_usage: Some(token_usage),
                cost: Some(feedback.cost_usd),
                role: feedback.role,
                input_tokens: feedback.input_tokens,
                output_tokens: feedback.output_tokens,
                cost_usd: feedback.cost_usd,
                latency_ms: feedback.latency_ms,
                success: feedback.success,
            })
            .await?;
        feedback_service.flush_async().await?;

        if self.save_cascade_router
            && let Some(router) = &self.cascade_router
        {
            router.save(&self.cascade_path).map_err(|e| {
                roko_core::error::RokoError::Io(std::io::Error::other(e.to_string()))
            })?;
        }

        Ok(())
    }

    fn record_provider_health(&self, feedback: &ModelCallFeedback) -> Result<()> {
        record_provider_health_at(
            &self.learn_dir,
            &feedback.provider,
            feedback.provider_health_success(),
        )
    }
}

/// Persist one provider-health outcome under a workspace `.roko/learn` tree.
///
/// # Errors
///
/// Returns an error when the health registry directory or JSON file cannot be
/// written.
pub fn record_provider_health_for_workdir(
    workdir: &Path,
    provider: &str,
    success: bool,
) -> Result<()> {
    record_provider_health_at(&workdir.join(".roko").join("learn"), provider, success)
}

/// Persist one provider-health outcome under a `.roko/learn` directory.
///
/// # Errors
///
/// Returns an error when the health registry directory or JSON file cannot be
/// written.
pub fn record_provider_health_at(learn_dir: &Path, provider: &str, success: bool) -> Result<()> {
    let provider = provider.trim();
    if provider.is_empty() {
        return Ok(());
    }

    std::fs::create_dir_all(learn_dir)?;
    let path = learn_dir.join("provider-health.json");
    let registry = ProviderHealthRegistry::load_or_new(&path);
    if success {
        registry.record_success(provider);
    } else {
        registry.record_failure(provider, ErrorClass::Unknown);
    }
    registry.save(&path)?;
    Ok(())
}

/// Record a model-call reward observation on an existing cascade router.
pub fn observe_model_call_on_router(
    router: &CascadeRouter,
    model: &str,
    role: &str,
    success: bool,
    latency_ms: u64,
) {
    let Some(model_idx) = router.model_index_for_slug(model) else {
        tracing::debug!("model {model} not in cascade router slug list, skipping observe");
        return;
    };

    let reward = if success { 1.0 } else { 0.0 };
    router.observe(model_call_context_vec(role, latency_ms), model_idx, reward);
}

fn model_call_context_vec(role: &str, latency_ms: u64) -> Vec<f64> {
    let role_feature = simple_role_hash(role);
    let latency_feature = (latency_ms as f64 / 60_000.0).min(1.0);
    let mut context_vec = vec![0.0; CONTEXT_DIM];

    context_vec[0] = role_feature;
    context_vec[1] = latency_feature;
    context_vec[16] = 1.0;

    context_vec
}

fn simple_role_hash(role: &str) -> f64 {
    let hash: u32 = role.bytes().fold(0u32, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(u32::from(b))
    });
    f64::from(hash % 1000) / 1000.0
}

#[cfg(test)]
mod tests {
    use super::{ModelCallFeedback, ModelCallFeedbackRecorder};
    use tempfile::tempdir;

    #[tokio::test]
    async fn recorder_writes_feedback_health_and_cascade_router() {
        let tmp = tempdir().expect("tempdir");
        let recorder =
            ModelCallFeedbackRecorder::from_workdir(tmp.path(), vec!["model-slug".to_string()]);

        recorder
            .record(ModelCallFeedback {
                run_id: None,
                request_id: Some("request-1".to_string()),
                prompt_section_ids: Vec::new(),
                knowledge_ids: Vec::new(),
                model: "model-slug".to_string(),
                provider: "provider-id".to_string(),
                role: "test_role".to_string(),
                input_tokens: 12,
                output_tokens: 34,
                cost_usd: 0.056,
                latency_ms: 789,
                success: true,
                provider_success: None,
            })
            .await
            .expect("record feedback");

        let learn_dir = tmp.path().join(".roko/learn");
        let efficiency =
            std::fs::read_to_string(learn_dir.join("efficiency.jsonl")).expect("efficiency");
        assert!(efficiency.contains("\"kind\":\"model_call\""));
        assert!(efficiency.contains("\"model\":\"model-slug\""));
        assert!(efficiency.contains("\"provider\":\"provider-id\""));

        let provider_health =
            std::fs::read_to_string(learn_dir.join("provider-health.json")).expect("health");
        assert!(provider_health.contains("provider-id"));

        let cascade =
            std::fs::read_to_string(learn_dir.join("cascade-router.json")).expect("cascade");
        assert!(cascade.contains("model-slug"));
    }
}
