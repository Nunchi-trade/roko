use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use roko_acp::{
    bridge_events::handle_session_prompt,
    session::{AcpSession, SessionNewParams},
    transport::StdioTransport,
    types::{ContentBlock, JsonRpcNotification, SessionPromptParams},
};
use roko_core::{ContentHash, ProviderKind};
use roko_core::config::schema::{ModelProfile, ProviderConfig, RokoConfig};
use roko_learn::{
    cascade_router::CascadeRouter,
    cost_table::CostTable,
    episode_logger::{Episode, EpisodeLogger, Usage as EpisodeUsage},
    model_router::{RoutingContext, compute_routing_reward_v2, normalized_cost},
};
use roko_agent::Usage as AgentUsage;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, duplex},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::timeout,
};

#[derive(Debug, Clone)]
pub struct MockResponse {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone)]
pub enum MockPhaseResponse {
    Implement(String, u64, u64),
    GatePass,
    ReviewApprove(String, u64, u64),
}

#[derive(Debug, Clone)]
pub struct DispatchResult {
    pub total_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub phases_completed: usize,
    pub notifications: Vec<JsonRpcNotification>,
}

#[derive(Debug, Clone)]
pub struct TestSession {
    workdir: PathBuf,
    model: String,
    mode: String,
    workflow: String,
}

pub fn create_test_session(workdir: &Path) -> TestSession {
    create_test_session_with_workflow(workdir, "none")
}

pub fn create_test_session_with_workflow(workdir: &Path, workflow: &str) -> TestSession {
    std::fs::create_dir_all(workdir.join(".roko").join("learn"))
        .expect("create test .roko/learn tree");

    TestSession {
        workdir: workdir.to_path_buf(),
        model: "gpt-5.4".to_string(),
        mode: "code".to_string(),
        workflow: workflow.to_string(),
    }
}

impl TestSession {
    async fn mock_dispatch_inner(
        &self,
        prompt: &str,
        response: Option<MockResponse>,
        response_delay: Duration,
        provider_timeout_ms: u64,
    ) -> Result<DispatchResult> {
        let server = spawn_mock_provider_server(
            response,
            response_delay,
            provider_timeout_ms,
            self.model.as_str(),
        )
        .await?;

        let roko_config = build_mock_config(&server.base_url, provider_timeout_ms);
        let mut session = build_session(&roko_config, &self.mode, &self.model, &self.workflow);

        let (client_to_server, server_reader) = duplex(16 * 1024);
        let (server_writer, client_from_server) = duplex(16 * 1024);
        let _client_input_guard = client_to_server;
        let mut transport = StdioTransport::from_io(server_reader, server_writer);
        let session_id = session.session_id.clone();
        let outcome = handle_session_prompt(
            &mut transport,
            &mut session,
            SessionPromptParams {
                session_id,
                prompt: vec![ContentBlock::Text {
                    text: prompt.to_string(),
                }],
                include_context: false,
            },
            &self.workdir,
            &roko_config,
        )
        .await;

        let notifications = collect_notifications(client_from_server).await?;
        server
            .task
            .await
            .context("mock provider task join failed")??;

        outcome?;

        let (total_tokens, cost_usd) = usage_from_notifications(&notifications);
        Ok(DispatchResult {
            total_tokens,
            cost_usd,
            phases_completed: 1,
            notifications,
        })
    }

    pub async fn mock_dispatch(&self, prompt: &str, response: MockResponse) -> Result<DispatchResult> {
        self.mock_dispatch_inner(
            prompt,
            Some(response),
            Duration::from_millis(5),
            500,
        )
        .await
    }

    pub async fn mock_dispatch_failure(
        &self,
        prompt: &str,
        _failure_label: &str,
    ) -> Result<DispatchResult> {
        self.mock_dispatch_inner(
            prompt,
            None,
            Duration::from_millis(250),
            100,
        )
        .await
    }

    pub async fn mock_pipeline_dispatch(
        &self,
        prompt: &str,
        phases: Vec<MockPhaseResponse>,
    ) -> Result<DispatchResult> {
        if phases.is_empty() {
            bail!("pipeline dispatch needs at least one phase response");
        }

        let episodes_path = self.workdir.join(".roko").join("episodes.jsonl");
        let router_path = self
            .workdir
            .join(".roko")
            .join("learn")
            .join("cascade-router.json");
        let logger = EpisodeLogger::new(&episodes_path);
        let router = CascadeRouter::load_or_new(&router_path, vec![self.model.clone()]);
        let model_idx = router
            .model_index_for_slug(&self.model)
            .context("mock pipeline model missing from cascade router")?;
        let context = RoutingContext::default().to_features();
        let cost_table = CostTable::with_defaults();

        let mut assistant_text = String::new();
        let mut total_input_tokens = 0u64;
        let mut total_output_tokens = 0u64;
        let mut phase_count = 0usize;

        for (idx, phase) in phases.iter().enumerate() {
            match phase {
                MockPhaseResponse::Implement(text, input_tokens, output_tokens)
                | MockPhaseResponse::ReviewApprove(text, input_tokens, output_tokens) => {
                    assistant_text.push_str(text);
                    assistant_text.push('\n');
                    total_input_tokens += input_tokens;
                    total_output_tokens += output_tokens;

                    let phase_latency_ms = 120 + (idx as u64 * 25);
                    let normalized = normalized_cost(&self.model, &cost_table);
                    let reward = compute_routing_reward_v2(
                        1.0,
                        normalized,
                        phase_latency_ms as f64,
                        1_000.0,
                    );
                    router.observe(context.clone(), model_idx, reward);
                    phase_count += 1;
                }
                MockPhaseResponse::GatePass => {
                    phase_count += 1;
                }
            }
        }

        let agent_usage = AgentUsage {
            input_tokens: total_input_tokens,
            output_tokens: total_output_tokens,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
            ..AgentUsage::default()
        };
        let total_cost_usd = cost_table.calculate(&self.model, &agent_usage);
        router.save(&router_path).context("save mock cascade router")?;

        let mut episode = Episode::new(self.mode.clone(), format!("pipeline-{}", self.workflow));
        episode.kind = format!("acp-pipeline-{}", self.workflow);
        episode.agent_template = self.mode.clone();
        episode.model = self.model.clone();
        episode.backend = ProviderKind::OpenAiCompat.label().to_string();
        episode.trigger_kind = "acp_pipeline".to_string();
        episode.trigger_signal_hash = ContentHash::of(prompt.as_bytes()).to_hex();
        episode.input_signal_hash = episode.trigger_signal_hash.clone();
        episode.output_signal_hash = ContentHash::of(assistant_text.as_bytes()).to_hex();
        episode.episode_id = episode.id.clone();
        episode.duration_secs = 0.001;
        episode.usage = EpisodeUsage {
            input_tokens: total_input_tokens,
            output_tokens: total_output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: total_cost_usd,
            cost_usd_without_cache: total_cost_usd,
            wall_ms: 1,
        };
        episode.tokens_used = total_input_tokens + total_output_tokens;
        episode.success = true;
        episode.extra.insert("entry_point".to_string(), json!("acp"));
        episode.extra.insert("model".to_string(), json!(self.model));
        episode.extra.insert("mode".to_string(), json!(self.mode));
        episode
            .extra
            .insert("workflow".to_string(), json!(self.workflow));
        episode
            .extra
            .insert("phases_completed".to_string(), json!(phase_count as u64));
        logger.append(&episode).await.context("append mock pipeline episode")?;

        let notifications = vec![JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "session/update".to_string(),
            params: Some(json!({
                "sessionId": "pipeline",
                "update": {
                    "sessionUpdate": "usage_update",
                    "used": episode.tokens_used,
                    "size": 128_000,
                    "cost": {
                        "amount": total_cost_usd,
                        "currency": "USD"
                    }
                }
            })),
        }];

        Ok(DispatchResult {
            total_tokens: Some(episode.tokens_used),
            cost_usd: Some(total_cost_usd),
            phases_completed: phase_count,
            notifications,
        })
    }
}

fn build_mock_config(base_url: &str, timeout_ms: u64) -> RokoConfig {
    let mut config = RokoConfig::default();
    config.providers.insert(
        "mock".to_string(),
        ProviderConfig {
            kind: ProviderKind::OpenAiCompat,
            base_url: Some(base_url.to_string()),
            api_key_env: None,
            command: None,
            args: None,
            timeout_ms: Some(timeout_ms),
            ttft_timeout_ms: Some(timeout_ms),
            connect_timeout_ms: Some(timeout_ms),
            extra_headers: None,
            max_concurrent: None,
        },
    );
    config.models.insert(
        "gpt-5.4".to_string(),
        ModelProfile {
            provider: "mock".to_string(),
            slug: "gpt-5.4".to_string(),
            context_window: 128_000,
            max_output: Some(8_192),
            supports_tools: true,
            supports_thinking: false,
            supports_vision: false,
            supports_web_search: false,
            supports_mcp_tools: false,
            supports_partial: false,
            supports_grounding: false,
            supports_code_execution: false,
            supports_caching: false,
            provider_routing: None,
            tool_format: "openai_json".to_string(),
            cost_input_per_m: Some(3.0),
            cost_output_per_m: Some(12.0),
            cost_input_per_m_high: None,
            cost_output_per_m_high: None,
            cost_cache_read_per_m: Some(0.75),
            cost_cache_write_per_m: Some(3.75),
            thinking_level: None,
            max_tools: Some(64),
            tokenizer_ratio: Some(1.0),
            supports_search: false,
            supports_citations: false,
            supports_async: false,
            is_embedding_model: false,
            search_context_size: None,
            cost_per_request: None,
        },
    );
    config
}

fn build_session(
    roko_config: &RokoConfig,
    mode: &str,
    model: &str,
    workflow: &str,
) -> AcpSession {
    let mut session = AcpSession::new_with_config(
        SessionNewParams {
            session_name: Some("telemetry-test".to_string()),
            client_capabilities: None,
            mcp_servers: Vec::new(),
        },
        roko_config,
    );
    session.config_state.agent_mode = mode.to_string();
    session.config_state.model = model.to_string();
    session.config_state.workflow = workflow.to_string();
    session
}

struct MockServer {
    base_url: String,
    task: JoinHandle<Result<()>>,
}

async fn spawn_mock_provider_server(
    response: Option<MockResponse>,
    response_delay: Duration,
    timeout_ms: u64,
    expected_model: &str,
) -> Result<MockServer> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind mock provider listener")?;
    let addr = listener.local_addr().context("read mock provider addr")?;
    let base_url = format!("http://{addr}");
    let expected_model = expected_model.to_string();

    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.context("accept mock provider request")?;
        let (request_line, body) = read_http_request(&mut stream).await?;
        assert!(
            request_line.starts_with("POST /chat/completions "),
            "expected chat/completions request, got {request_line}"
        );
        assert_eq!(
            body.get("model").and_then(Value::as_str),
            Some(expected_model.as_str()),
            "request body must carry the resolved model"
        );
        assert!(
            body.get("messages").and_then(Value::as_array).is_some(),
            "request body must include messages"
        );
        assert_eq!(
            body.get("stream").and_then(Value::as_bool),
            Some(true),
            "request must stream"
        );

        tokio::time::sleep(response_delay).await;

        if let Some(response) = response {
            let body = mock_sse_body(&response);
            let response_bytes = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response_bytes.as_bytes())
                .await
                .context("write mock provider response")?;
            stream.flush().await.context("flush mock provider response")?;
            stream.shutdown().await.context("shutdown mock provider response")?;
        } else {
            let _ = timeout_ms;
        }

        Ok(())
    });

    Ok(MockServer { base_url, task })
}

async fn read_http_request(stream: &mut TcpStream) -> Result<(String, Value)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.context("read mock request bytes")?;
        assert!(read > 0, "request closed before headers completed");
        buf.extend_from_slice(&chunk[..read]);

        if let Some(pos) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let headers = String::from_utf8_lossy(&buf[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);

    while buf.len() < header_end + content_length {
        let read = stream.read(&mut chunk).await.context("read mock request body")?;
        assert!(read > 0, "request closed before body completed");
        buf.extend_from_slice(&chunk[..read]);
    }

    let request = String::from_utf8(buf).context("mock request must be valid utf-8")?;
    let request_line = request
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let body = &request[header_end..header_end + content_length];
    let json = serde_json::from_str(body).context("parse mock request body")?;
    Ok((request_line, json))
}

fn mock_sse_body(response: &MockResponse) -> String {
    let content = json!({
        "choices": [{
            "delta": {
                "content": response.text
            }
        }]
    });
    let finish = json!({
        "choices": [{
            "delta": {},
            "finish_reason": "stop"
        }]
    });
    let usage = json!({
        "usage": {
            "prompt_tokens": response.input_tokens,
            "completion_tokens": response.output_tokens,
            "prompt_tokens_details": {
                "cached_tokens": 0
            }
        }
    });

    format!("data: {content}\n\ndata: {finish}\n\ndata: {usage}\n\ndata: [DONE]\n\n")
}

async fn collect_notifications(
    client_output: DuplexStream,
) -> Result<Vec<JsonRpcNotification>> {
    let mut reader = BufReader::new(client_output);
    let mut notifications = Vec::new();

    loop {
        let mut line = String::new();
        match timeout(Duration::from_millis(50), reader.read_line(&mut line)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(_)) => {
                if line.trim().is_empty() {
                    continue;
                }
                let notification: JsonRpcNotification =
                    serde_json::from_str(&line).context("parse ACP notification")?;
                notifications.push(notification);
            }
            Ok(Err(err)) => return Err(err).context("read ACP notification"),
        }
    }

    Ok(notifications)
}

fn usage_from_notifications(notifications: &[JsonRpcNotification]) -> (Option<u64>, Option<f64>) {
    for notification in notifications {
        if notification.method != "session/update" {
            continue;
        }
        let Some(params) = notification.params.as_ref() else {
            continue;
        };
        let Some(update) = params.get("update") else {
            continue;
        };
        if update.get("sessionUpdate").and_then(Value::as_str) != Some("usage_update") {
            continue;
        }

        let total_tokens = update.get("used").and_then(Value::as_u64);
        let cost_usd = update
            .get("cost")
            .and_then(|cost| cost.get("amount"))
            .and_then(Value::as_f64);
        return (total_tokens, cost_usd);
    }

    (None, None)
}
