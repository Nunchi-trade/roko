//! Shared workflow service construction for CLI, server, and ACP.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use roko_agent::ModelCallService;
use roko_compose::prompt_assembly_service::PromptAssemblyService;
use roko_core::agent::resolve_model;
use roko_core::config::schema::RokoConfig;
use roko_core::foundation::{
    AffectPolicy, FeedbackEvent, FeedbackSink, GateRunner, ModelCaller, PromptAssembler,
};
use roko_core::{Result, RokoError};
use roko_daimon::policy::DaimonPolicy;
use roko_gate::gate_service::GateService;
use roko_learn::feedback_service::FeedbackService;
use roko_neuro::knowledge_store::KnowledgeStore;
use roko_runtime::effect_driver::EffectServices;

/// Input settings for constructing shared workflow services.
#[derive(Clone)]
pub struct ServiceConfig {
    /// Workspace root used by service implementations.
    pub workdir: PathBuf,
    /// `.roko` directory used for persistent service state.
    pub roko_dir: PathBuf,
    /// Runtime workspace configuration for model/provider dispatch.
    pub workspace_config: RokoConfig,
    /// Optional model key or slug overriding `workspace_config.agent.default_model`.
    pub model_key: Option<String>,
    /// Optional MCP config passed into model provider construction.
    pub mcp_config: Option<PathBuf>,
    /// Whether feedback should persist through `FeedbackService`.
    pub feedback_enabled: bool,
    /// Whether affect modulation should be backed by Daimon state.
    pub affect_enabled: bool,
    /// Stable run id used by service-level event and feedback records.
    pub run_id: Option<String>,
}

impl ServiceConfig {
    /// Build a production service config from a workspace root and Roko config.
    #[must_use]
    pub fn production(workdir: impl Into<PathBuf>, workspace_config: RokoConfig) -> Self {
        let workdir = workdir.into();
        Self {
            roko_dir: workdir.join(".roko"),
            workdir,
            workspace_config,
            model_key: None,
            mcp_config: None,
            feedback_enabled: true,
            affect_enabled: true,
            run_id: None,
        }
    }
}

/// Concrete service bundle shared by all runtime entry points.
pub struct ServiceBundle {
    /// Resolved default model slug.
    pub model: String,
    /// Concrete model-call gateway used by HTTP inference and workflow effects.
    pub model_call_service: Arc<ModelCallService>,
    /// Prompt assembly service exposed as the foundation trait.
    pub prompt_assembler: Arc<dyn PromptAssembler>,
    /// Feedback service exposed as the foundation trait.
    pub feedback_sink: Arc<dyn FeedbackSink>,
    /// Gate execution service exposed as the foundation trait.
    pub gate_runner: Arc<dyn GateRunner>,
    /// Optional affect policy shared with the effect driver.
    pub affect_policy: Option<Arc<tokio::sync::Mutex<dyn AffectPolicy>>>,
}

impl ServiceBundle {
    /// Build the `EffectServices` value consumed by `WorkflowEngine`.
    #[must_use]
    pub fn effect_services(&self) -> EffectServices {
        let model_caller: Arc<dyn ModelCaller> = self.model_call_service.clone();
        EffectServices {
            model: self.model.clone(),
            model_caller,
            prompt_assembler: Arc::clone(&self.prompt_assembler),
            feedback_sink: Arc::clone(&self.feedback_sink),
            gate_runner: Arc::clone(&self.gate_runner),
            affect_policy: self.affect_policy.clone(),
        }
    }
}

/// Factory for constructing the shared service bundle.
pub struct ServiceFactory;

impl ServiceFactory {
    /// Construct all workflow services through the canonical path.
    pub fn build(config: ServiceConfig) -> Result<ServiceBundle> {
        let mut workspace_config = config.workspace_config;
        let model_key = config
            .model_key
            .clone()
            .unwrap_or_else(|| workspace_config.agent.default_model.clone());
        if model_key.trim().is_empty() {
            return Err(RokoError::invalid("model is not configured for service factory"));
        }
        let model = resolve_model(&workspace_config, &model_key).slug;
        if model.trim().is_empty() {
            return Err(RokoError::invalid(format!(
                "model key {model_key:?} resolved to an empty model slug"
            )));
        }
        workspace_config.agent.default_model = model.clone();

        let feedback_sink: Arc<dyn FeedbackSink> = if config.feedback_enabled {
            Arc::new(FeedbackService::from_roko_dir_with_episodes(&config.roko_dir))
        } else {
            Arc::new(MemoryFeedbackSink::default())
        };

        let mut model_call_service = ModelCallService::new(model.clone())
            .with_config(workspace_config)
            .with_feedback_sink(Arc::clone(&feedback_sink))
            .with_run_id(config.run_id.unwrap_or_else(default_run_id));
        if let Some(mcp_config) = config.mcp_config {
            model_call_service = model_call_service.with_mcp_config(mcp_config);
        }
        let model_call_service = Arc::new(model_call_service);

        let knowledge_store = Arc::new(KnowledgeStore::for_roko_dir(&config.roko_dir));
        let prompt_assembler: Arc<dyn PromptAssembler> =
            Arc::new(PromptAssemblyService::new().with_knowledge_store(knowledge_store));
        let gate_runner: Arc<dyn GateRunner> = Arc::new(GateService::new());
        let affect_policy = config.affect_enabled.then(|| {
            let state_path = config.roko_dir.join("state").join("daimon.json");
            Arc::new(tokio::sync::Mutex::new(DaimonPolicy::new(state_path)))
                as Arc<tokio::sync::Mutex<dyn AffectPolicy>>
        });

        Ok(ServiceBundle {
            model,
            model_call_service,
            prompt_assembler,
            feedback_sink,
            gate_runner,
            affect_policy,
        })
    }
}

#[derive(Default)]
struct MemoryFeedbackSink {
    events: tokio::sync::Mutex<Vec<FeedbackEvent>>,
}

#[async_trait]
impl FeedbackSink for MemoryFeedbackSink {
    async fn record(&self, event: FeedbackEvent) -> Result<()> {
        self.events.lock().await.push(event);
        Ok(())
    }

    async fn flush(&self) -> Result<()> {
        self.events.lock().await.clear();
        Ok(())
    }
}

fn default_run_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("service_factory_{millis}")
}
