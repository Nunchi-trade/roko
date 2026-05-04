//! Vision evaluator: multimodal LLM call + response parsing.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use roko_agent::{
    AgentResult,
    provider::{AgentOptions, create_agent_for_model},
};
use roko_core::agent::resolve_model;
use roko_core::chat_types::{ChatMessage, ContentBlock, ImageUrl, MessageContent};
use roko_core::config::schema::RokoConfig;
use roko_core::defaults::DEFAULT_REQUEST_TIMEOUT_MS;
use roko_core::{Body, Engram, Kind};
use roko_learn::model_call_feedback::{ModelCallFeedback, ModelCallFeedbackRecorder};

use crate::learning_helpers::{capture_runtime_model_slugs, provider_id_for_model};

use super::prompt;
use super::{Evaluation, IterationRecord};

/// Evaluates screenshots against a goal using a vision-capable LLM.
pub struct VisionEvaluator {
    config: RokoConfig,
    model_key: String,
    goal: String,
    file_ext: String,
    workdir: PathBuf,
}

impl VisionEvaluator {
    pub fn new(
        config: RokoConfig,
        model_key: Option<String>,
        goal: String,
        file_ext: String,
        workdir: PathBuf,
    ) -> Result<Self> {
        let model_key = match model_key {
            Some(key) => key,
            None => find_vision_model(&config)
                .context("no vision-capable model found in roko.toml config")?,
        };

        // Verify the model supports vision.
        if let Some(profile) = config.effective_models().get(&model_key) {
            if !profile.supports_vision {
                bail!(
                    "model '{}' does not support vision (supports_vision = false)",
                    model_key
                );
            }
        }

        Ok(Self {
            config,
            model_key,
            goal,
            file_ext,
            workdir,
        })
    }

    /// Evaluate the current code + screenshot and return improved code with a score.
    pub async fn evaluate(
        &self,
        current_code: &str,
        screenshot_data_uri: &str,
        history: &[IterationRecord],
        regression_hint_iter: Option<u32>,
    ) -> Result<Evaluation> {
        let mut sys_prompt = prompt::system_prompt(&self.goal, &self.file_ext, history);
        if let Some(iter) = regression_hint_iter {
            sys_prompt.push_str(&prompt::regression_hint(iter));
        }

        let user_text = prompt::user_code_block(current_code);

        // Build the multimodal prompt as a text signal for the Agent trait.
        // The system prompt and user content (text + image) are combined into
        // the prompt engram. The agent's provider adapter will parse them into
        // the correct wire format (OpenAI messages, Anthropic blocks, etc.).
        let full_prompt = build_multimodal_prompt(&sys_prompt, &user_text, screenshot_data_uri);

        let options = AgentOptions {
            system_prompt: Some(sys_prompt),
            timeout_ms: Some(DEFAULT_REQUEST_TIMEOUT_MS),
            name: "vision-evaluator".to_string(),
            ..Default::default()
        };

        let agent = create_agent_for_model(&self.config, &self.model_key, options)
            .map_err(|e| anyhow::anyhow!("failed to create vision agent: {e}"))?;

        let input = Engram::builder(Kind::Prompt)
            .body(Body::text(&full_prompt))
            .build();

        let started = Instant::now();
        let result = agent.run(&input, &roko_core::Context::now()).await;
        let latency_ms = started.elapsed().as_millis() as u64;

        let raw_output = result.output.body.as_text().unwrap_or("").to_string();
        let evaluation = if result.success {
            parse_evaluation(&raw_output)
        } else {
            let msg = if raw_output.trim().is_empty() {
                "unknown error".to_string()
            } else {
                raw_output
            };
            Err(anyhow::anyhow!("vision model call failed: {msg}"))
        };

        self.record_model_call_feedback(&result, latency_ms, evaluation.is_ok())
            .await;

        evaluation
    }

    /// The model key being used.
    pub fn model_key(&self) -> &str {
        &self.model_key
    }

    async fn record_model_call_feedback(
        &self,
        result: &AgentResult,
        latency_ms: u64,
        learning_success: bool,
    ) {
        let resolved = resolve_model(&self.config, &self.model_key);
        let model_slug = resolved.slug;
        let provider_id = provider_id_for_model(&self.config, &self.model_key)
            .or_else(|| {
                resolved
                    .profile
                    .as_ref()
                    .map(|profile| profile.provider.clone())
            })
            .unwrap_or_else(|| resolved.provider_kind.label().to_string());

        let cascade_model_slugs = capture_runtime_model_slugs(&self.config, &model_slug);
        let recorder = ModelCallFeedbackRecorder::from_workdir(&self.workdir, cascade_model_slugs);
        if let Err(error) = recorder
            .record(ModelCallFeedback {
                run_id: None,
                request_id: Some(format!(
                    "vision-evaluator-{}",
                    chrono::Utc::now().timestamp_millis()
                )),
                prompt_section_ids: Vec::new(),
                knowledge_ids: Vec::new(),
                model: model_slug.clone(),
                provider: provider_id.clone(),
                role: "vision_evaluator".to_string(),
                input_tokens: u64::from(result.usage.input_tokens),
                output_tokens: u64::from(result.usage.output_tokens),
                cost_usd: f64::from(result.usage.cost_usd),
                latency_ms,
                success: learning_success,
                provider_success: Some(result.success),
            })
            .await
        {
            tracing::warn!(
                provider = %provider_id,
                model = %model_slug,
                error = %error,
                "failed to record vision evaluator feedback"
            );
        }
    }
}

/// Build a prompt that includes both text and image reference.
/// The agent system prompt is set via `AgentOptions::system_prompt`, so the
/// prompt engram carries the user turn: code text + image instruction.
fn build_multimodal_prompt(
    _system_prompt: &str,
    user_text: &str,
    screenshot_data_uri: &str,
) -> String {
    // For providers that don't natively support multimodal content blocks via
    // the Agent trait, we embed the image reference as a structured hint.
    // Providers that do support vision (Anthropic API, OpenAI compat) will
    // extract the image_url from the prompt when it matches this pattern.
    format!(
        "{user_text}\n\n\
         [IMAGE: {screenshot_data_uri}]\n\n\
         Evaluate the screenshot above against the goal and respond with ONLY valid JSON."
    )
}

/// Parse the model response into an `Evaluation`, stripping markdown fences if present.
fn parse_evaluation(raw: &str) -> Result<Evaluation> {
    let cleaned = strip_json_fences(raw);

    // Try direct JSON parse first.
    if let Ok(eval) = serde_json::from_str::<Evaluation>(&cleaned) {
        return validate_evaluation(eval);
    }

    // Fallback: try to find a JSON object in the response.
    if let Some(start) = cleaned.find('{') {
        if let Some(end) = cleaned.rfind('}') {
            let json_str = &cleaned[start..=end];
            if let Ok(eval) = serde_json::from_str::<Evaluation>(json_str) {
                return validate_evaluation(eval);
            }
        }
    }

    bail!(
        "failed to parse vision model response as JSON. Raw output:\n{}",
        &raw[..raw.len().min(500)]
    )
}

fn validate_evaluation(eval: Evaluation) -> Result<Evaluation> {
    if eval.score < 1.0 || eval.score > 10.0 {
        bail!("score {} out of range 1-10", eval.score);
    }
    if eval.improved_code.trim().is_empty() {
        bail!("improved_code is empty");
    }
    Ok(eval)
}

/// Strip markdown code fences (```json ... ```) from the response.
fn strip_json_fences(s: &str) -> String {
    let trimmed = s.trim();
    // Handle ```json\n...\n``` or ```\n...\n```
    if let Some(after) = trimmed.strip_prefix("```json") {
        if let Some(content) = after.strip_suffix("```") {
            return content.trim().to_string();
        }
    }
    if let Some(after) = trimmed.strip_prefix("```") {
        if let Some(content) = after.strip_suffix("```") {
            return content.trim().to_string();
        }
    }
    trimmed.to_string()
}

/// Find the first vision-capable model in the config.
fn find_vision_model(config: &RokoConfig) -> Option<String> {
    config
        .effective_models()
        .iter()
        .find(|(_, profile)| profile.supports_vision)
        .map(|(key, _)| key.clone())
}

/// Build a proper multimodal `ChatMessage` for providers that support it.
/// This is not used by the Agent trait path but is available for direct
/// provider/backend integration.
#[allow(dead_code)]
pub fn build_multimodal_messages(
    system_prompt: &str,
    user_text: &str,
    screenshot_data_uri: &str,
) -> Vec<ChatMessage> {
    vec![
        ChatMessage::System {
            content: system_prompt.to_string(),
        },
        ChatMessage::User {
            content: MessageContent::Blocks(vec![
                ContentBlock::Text {
                    text: user_text.to_string(),
                },
                ContentBlock::ImageUrl {
                    image_url: ImageUrl {
                        url: screenshot_data_uri.to_string(),
                    },
                },
            ]),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::agent::ProviderKind;
    use roko_core::config::schema::{ModelProfile, ProviderConfig};
    use roko_core::defaults::{
        DEFAULT_CONNECT_TIMEOUT_MS, DEFAULT_REQUEST_TIMEOUT_MS, DEFAULT_TTFT_TIMEOUT_MS,
    };
    use tempfile::tempdir;

    fn write_fake_claude_script(tmp: &tempfile::TempDir, body: &str) -> PathBuf {
        let script = tmp.path().join("claude-fake.sh");
        std::fs::write(&script, body).expect("write fake claude script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut perms = std::fs::metadata(&script).expect("metadata").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).expect("chmod");
        }
        script
    }

    #[test]
    fn strip_json_fences_strips_json_block() {
        let input = "```json\n{\"score\": 7}\n```";
        assert_eq!(strip_json_fences(input), "{\"score\": 7}");
    }

    #[test]
    fn strip_json_fences_strips_plain_block() {
        let input = "```\n{\"score\": 7}\n```";
        assert_eq!(strip_json_fences(input), "{\"score\": 7}");
    }

    #[test]
    fn strip_json_fences_passthrough_no_fences() {
        let input = "{\"score\": 7}";
        assert_eq!(strip_json_fences(input), "{\"score\": 7}");
    }

    #[test]
    fn parse_evaluation_valid_json() {
        let input = r#"{"score": 7.5, "notes": "good", "improved_code": "<div>ok</div>"}"#;
        let eval = parse_evaluation(input).unwrap();
        assert!((eval.score - 7.5).abs() < f64::EPSILON);
        assert_eq!(eval.notes, "good");
        assert_eq!(eval.improved_code, "<div>ok</div>");
    }

    #[test]
    fn parse_evaluation_fenced_json() {
        let input =
            "```json\n{\"score\": 8, \"notes\": \"nice\", \"improved_code\": \"code\"}\n```";
        let eval = parse_evaluation(input).unwrap();
        assert!((eval.score - 8.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_evaluation_embedded_json() {
        let input = "Here is the result:\n{\"score\": 6, \"notes\": \"ok\", \"improved_code\": \"x\"}\nDone.";
        let eval = parse_evaluation(input).unwrap();
        assert!((eval.score - 6.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_evaluation_rejects_out_of_range_score() {
        let input = r#"{"score": 15, "notes": "ok", "improved_code": "x"}"#;
        assert!(parse_evaluation(input).is_err());
    }

    #[test]
    fn parse_evaluation_rejects_empty_code() {
        let input = r#"{"score": 5, "notes": "ok", "improved_code": "  "}"#;
        assert!(parse_evaluation(input).is_err());
    }

    #[test]
    fn parse_evaluation_rejects_garbage() {
        assert!(parse_evaluation("not json at all").is_err());
    }

    #[test]
    fn find_vision_model_returns_first_match() {
        let mut config = RokoConfig::default();
        config.models.insert(
            "text-only".to_string(),
            ModelProfile {
                provider: "openai".to_string(),
                slug: "gpt-4".to_string(),
                supports_vision: false,
                ..Default::default()
            },
        );
        config.models.insert(
            "vision-model".to_string(),
            ModelProfile {
                provider: "anthropic".to_string(),
                slug: "claude-opus-4-6".to_string(),
                supports_vision: true,
                ..Default::default()
            },
        );

        let found = find_vision_model(&config);
        assert!(found.is_some());
        // Should find one of the vision-capable models
        let key = found.unwrap();
        let profile = config.models.get(&key).unwrap();
        assert!(profile.supports_vision);
    }

    #[test]
    fn find_vision_model_returns_none_when_no_vision() {
        let mut config = RokoConfig::default();
        config.models.insert(
            "text-only".to_string(),
            ModelProfile {
                supports_vision: false,
                ..Default::default()
            },
        );
        assert!(find_vision_model(&config).is_none());
    }

    #[test]
    fn multimodal_messages_have_correct_shape() {
        let msgs = build_multimodal_messages("sys", "code here", "data:image/png;base64,abc");
        assert_eq!(msgs.len(), 2);
        match &msgs[0] {
            ChatMessage::System { content } => assert_eq!(content, "sys"),
            _ => panic!("expected system message"),
        }
        match &msgs[1] {
            ChatMessage::User {
                content: MessageContent::Blocks(blocks),
            } => {
                assert_eq!(blocks.len(), 2);
                match &blocks[0] {
                    ContentBlock::Text { text } => assert_eq!(text, "code here"),
                    _ => panic!("expected text block"),
                }
                match &blocks[1] {
                    ContentBlock::ImageUrl { image_url } => {
                        assert!(image_url.url.starts_with("data:image/png;base64,"));
                    }
                    _ => panic!("expected image block"),
                }
            }
            _ => panic!("expected user message with blocks"),
        }
    }

    #[tokio::test]
    async fn evaluate_records_feedback_and_provider_health() {
        let tmp = tempdir().expect("tempdir");
        let script = write_fake_claude_script(
            &tmp,
            r#"#!/bin/sh
set -eu
cat >/dev/null
printf '%s\n' '{"type":"content_block_delta","delta":{"text":"{\"score\":8.5,\"notes\":\"ok\",\"improved_code\":\"<div>better</div>\"}"}}'
"#,
        );

        let mut config = RokoConfig::default();
        config.providers.clear();
        config.models.clear();
        config.agent.default_model = "vision-model".to_string();
        config.providers.insert(
            "vision-cli".to_string(),
            ProviderConfig {
                kind: ProviderKind::ClaudeCli,
                base_url: None,
                api_key_env: None,
                command: Some(script.display().to_string()),
                args: None,
                timeout_ms: Some(DEFAULT_REQUEST_TIMEOUT_MS),
                ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
                connect_timeout_ms: Some(DEFAULT_CONNECT_TIMEOUT_MS),
                extra_headers: None,
                max_concurrent: None,
            },
        );
        config.models.insert(
            "vision-model".to_string(),
            ModelProfile {
                provider: "vision-cli".to_string(),
                slug: "claude-sonnet-4-6".to_string(),
                supports_vision: true,
                ..Default::default()
            },
        );

        let evaluator = VisionEvaluator::new(
            config,
            Some("vision-model".to_string()),
            "make it clearer".to_string(),
            "tsx".to_string(),
            tmp.path().to_path_buf(),
        )
        .expect("evaluator");

        let eval = evaluator
            .evaluate("<div>before</div>", "data:image/png;base64,abc", &[], None)
            .await
            .expect("evaluate");

        assert!((eval.score - 8.5).abs() < f64::EPSILON);
        assert_eq!(eval.improved_code, "<div>better</div>");

        let efficiency_path = tmp.path().join(".roko/learn/efficiency.jsonl");
        let efficiency = std::fs::read_to_string(&efficiency_path).expect("read efficiency");
        assert!(efficiency.contains(r#""kind":"model_call""#));
        assert!(efficiency.contains(r#""role":"vision_evaluator""#));
        assert!(efficiency.contains(r#""model":"claude-sonnet-4-6""#));
        assert!(efficiency.contains(r#""provider":"vision-cli""#));
        assert!(efficiency.contains(r#""success":true"#));

        let provider_health =
            std::fs::read_to_string(tmp.path().join(".roko/learn/provider-health.json"))
                .expect("read provider health");
        assert!(provider_health.contains("vision-cli"));

        let cascade_router =
            std::fs::read_to_string(tmp.path().join(".roko/learn/cascade-router.json"))
                .expect("read cascade router");
        assert!(cascade_router.contains("claude-sonnet-4-6"));
    }
}
