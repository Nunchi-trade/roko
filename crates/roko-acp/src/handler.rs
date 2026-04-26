//! Main ACP dispatch loop.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use tracing::{debug, error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use crate::{
    config::AcpConfig,
    session::SessionManager,
    transport::{StdioTransport, TransportError},
    types::{
        ACP_PROTOCOL_VERSION, ACP_SPEC_VERSION, AgentCapabilities, AgentInfo, ConfigUpdateParams,
        ConfigUpdateResult, InitializeParams, InitializeResult, JsonRpcMessage,
        JsonRpcNotification, JsonRpcRequest, METHOD_NOT_FOUND, McpCapabilities,
        PromptCapabilities, SESSION_NOT_FOUND, SESSION_BUSY, SessionCancelParams,
        SessionLoadParams, SessionNewParams, SessionPromptParams,
        SessionPromptResult, SessionSetModeParams, StopReason,
    },
};

/// Runs the ACP stdio server until stdin reaches EOF or a fatal transport error occurs.
pub async fn run_acp_server(config: AcpConfig) -> Result<()> {
    let _guard = setup_file_logging(config.log_file())
        .with_context(|| format!("failed to initialize ACP logging at {}", config.log_file().display()))?;
    let mut transport = StdioTransport::new();
    run_acp_server_with_transport(config, &mut transport).await
}

async fn run_acp_server_with_transport(
    _config: AcpConfig,
    transport: &mut StdioTransport,
) -> Result<()> {
    let mut sessions = SessionManager::new();

    loop {
        let message = match transport.read_message().await {
            Ok(Some(message)) => message,
            Ok(None) => {
                info!("stdin reached EOF; shutting down ACP server");
                return Ok(());
            }
            Err(TransportError::Json(error)) => {
                error!(error = %error, "failed to decode inbound JSON-RPC message");
                continue;
            }
            Err(error) => return Err(error).context("failed to read ACP message"),
        };

        match message {
            JsonRpcMessage::Request(request) => {
                handle_request(transport, &mut sessions, request).await?;
            }
            JsonRpcMessage::Response(response) => {
                transport.handle_incoming_response(response);
            }
            JsonRpcMessage::Notification(notification) => {
                handle_notification(&mut sessions, notification);
            }
        }
    }
}

async fn handle_request(
    transport: &mut StdioTransport,
    sessions: &mut SessionManager,
    request: JsonRpcRequest,
) -> Result<()> {
    let JsonRpcRequest {
        id,
        method,
        params,
        ..
    } = request;
    debug!(method = %method, request_id = ?id, "handling ACP request");

    match method.as_str() {
        "initialize" => {
            let _params: InitializeParams = match parse_params(params, &method) {
                Ok(params) => params,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            let result = InitializeResult {
                protocol_version: ACP_PROTOCOL_VERSION,
                agent_capabilities: AgentCapabilities {
                    load_session: true,
                    prompt_capabilities: Some(PromptCapabilities {
                        image: false,
                        audio: false,
                        embedded_context: true,
                    }),
                    mcp_capabilities: Some(McpCapabilities {
                        http: true,
                        sse: true,
                    }),
                },
                agent_info: AgentInfo {
                    name: "roko".to_owned(),
                    title: "Roko ACP".to_owned(),
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                },
                auth_methods: Vec::new(),
            };
            send_success(transport, id, result).await
        }
        "session/new" => {
            let params: SessionNewParams = match parse_params(params, &method) {
                Ok(params) => params,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            let result = sessions.create_session(params);
            send_success(transport, id, result).await
        }
        "session/list" => {
            let result = sessions.list_sessions();
            send_success(transport, id, result).await
        }
        "session/load" => {
            let params: SessionLoadParams = match parse_params(params, &method) {
                Ok(params) => params,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            let result = match sessions.load_session(&params.session_id) {
                Ok(result) => result,
                Err(_) => {
                    return send_error_response(
                        transport,
                        id,
                        session_not_found_error(&params.session_id),
                    )
                    .await;
                }
            };
            send_success(transport, id, result).await
        }
        "session/prompt" => {
            let params: SessionPromptParams = match parse_params(params, &method) {
                Ok(params) => params,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            let session = match get_session_mut(sessions, &params.session_id) {
                Ok(session) => session,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            if session.is_busy() {
                return send_error_response(transport, id, session_busy_error(&params.session_id))
                    .await;
            }

            session.begin_prompt();
            let result = SessionPromptResult {
                session_id: session.session_id.clone(),
                stop_reason: StopReason::EndTurn,
                usage: None,
            };
            session.finish_prompt();
            send_success(transport, id, result).await
        }
        "session/config/update" => {
            let params: ConfigUpdateParams = match parse_params(params, &method) {
                Ok(params) => params,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            let session = match get_session_mut(sessions, &params.session_id) {
                Ok(session) => session,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            debug!(
                session_id = %session.session_id,
                option_id = %params.option_id,
                "received config update request before ACP15 wiring"
            );
            let result = ConfigUpdateResult {
                config_options: session.config_options(),
            };
            send_success(transport, id, result).await
        }
        "session/set_mode" => {
            let params: SessionSetModeParams = match parse_params(params, &method) {
                Ok(params) => params,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            let session = match get_session_mut(sessions, &params.session_id) {
                Ok(session) => session,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            session.set_mode(params.mode_id);
            let result = ConfigUpdateResult {
                config_options: session.config_options(),
            };
            send_success(transport, id, result).await
        }
        _ => {
            let error = json_rpc_error(
                METHOD_NOT_FOUND,
                format!("method '{method}' is not supported"),
            );
            send_error_response(transport, id, error).await
        }
    }
}

fn handle_notification(
    sessions: &mut SessionManager,
    notification: JsonRpcNotification,
) {
    debug!(method = %notification.method, "handling ACP notification");

    match notification.method.as_str() {
        "session/cancel" => {
            let params: SessionCancelParams =
                match parse_params(notification.params, &notification.method) {
                    Ok(params) => params,
                    Err(error) => {
                        warn!(
                            method = %notification.method,
                            code = error.0,
                            message = %error.1,
                            "dropping malformed ACP notification"
                        );
                        return;
                    }
                };

            match sessions.get_session_mut(&params.session_id) {
                Some(session) => session.cancel(),
                None => warn!(session_id = %params.session_id, "received cancel for unknown ACP session"),
            }
        }
        _ => warn!(method = %notification.method, "ignoring unsupported ACP notification"),
    }
}

#[cfg(test)]
fn get_session<'a>(
    sessions: &'a SessionManager,
    session_id: &str,
) -> std::result::Result<&'a crate::session::AcpSession, (i32, String)> {
    sessions
        .get_session(session_id)
        .ok_or_else(|| session_not_found_error(session_id))
}

fn get_session_mut<'a>(
    sessions: &'a mut SessionManager,
    session_id: &str,
) -> std::result::Result<&'a mut crate::session::AcpSession, (i32, String)> {
    sessions
        .get_session_mut(session_id)
        .ok_or_else(|| session_not_found_error(session_id))
}

fn parse_params<T>(
    params: Option<serde_json::Value>,
    method: &str,
) -> std::result::Result<T, (i32, String)>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(params.unwrap_or(serde_json::Value::Null)).map_err(|error| {
        json_rpc_error(
            crate::types::INVALID_PARAMS,
            format!("invalid params for '{method}': {error}"),
        )
    })
}

fn json_rpc_error(code: i32, message: String) -> (i32, String) {
    (code, message)
}

fn session_not_found_error(session_id: &str) -> (i32, String) {
    json_rpc_error(
        SESSION_NOT_FOUND,
        format!("session '{session_id}' was not found"),
    )
}

fn session_busy_error(session_id: &str) -> (i32, String) {
    json_rpc_error(
        SESSION_BUSY,
        format!("session '{session_id}' already has an active prompt"),
    )
}

async fn send_success<T>(
    transport: &mut StdioTransport,
    id: crate::types::JsonRpcId,
    result: T,
) -> Result<()>
where
    T: serde::Serialize,
{
    let value = serde_json::to_value(result).context("failed to serialize JSON-RPC result")?;
    transport
        .send_response(id, value)
        .await
        .context("failed to send JSON-RPC response")
}

async fn send_error_response(
    transport: &mut StdioTransport,
    id: crate::types::JsonRpcId,
    error: (i32, String),
) -> Result<()> {
    transport
        .send_error(id, error.0, error.1)
        .await
        .context("failed to send JSON-RPC error response")
}

fn setup_file_logging(log_file: &Path) -> Result<WorkerGuard> {
    if let Some(parent) = log_file.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create ACP log directory {}", parent.display())
        })?;
    }

    let file_name = log_file
        .file_name()
        .ok_or_else(|| anyhow!("ACP log file path must include a file name"))?;
    let directory = log_file.parent().unwrap_or_else(|| Path::new("."));

    let file_appender = tracing_appender::rolling::never(directory, file_name);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(non_blocking)
        .with_env_filter(EnvFilter::new("roko_acp=debug"))
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
    info!(
        protocol_version = ACP_PROTOCOL_VERSION,
        spec_version = ACP_SPEC_VERSION,
        log_file = %log_file.display(),
        "ACP logging initialized"
    );

    Ok(guard)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_params_reports_invalid_payloads() {
        let error = parse_params::<InitializeParams>(
            Some(serde_json::json!({ "protocolVersion": "wrong" })),
            "initialize",
        )
        .expect_err("payload should be rejected");

        assert_eq!(error.0, crate::types::INVALID_PARAMS);
        assert!(error.1.contains("initialize"));
    }

    #[test]
    fn session_lookup_reports_missing_session() {
        let sessions = SessionManager::new();

        let error = get_session(&sessions, "sess_missing").expect_err("session should be absent");

        assert_eq!(error.0, SESSION_NOT_FOUND);
        assert!(error.1.contains("sess_missing"));
    }

    #[test]
    fn initialize_result_advertises_expected_protocol_version() {
        let result = InitializeResult {
            protocol_version: ACP_PROTOCOL_VERSION,
            agent_capabilities: AgentCapabilities {
                load_session: true,
                prompt_capabilities: Some(PromptCapabilities {
                    image: false,
                    audio: false,
                    embedded_context: true,
                }),
                mcp_capabilities: Some(McpCapabilities {
                    http: true,
                    sse: true,
                }),
            },
            agent_info: AgentInfo {
                name: "roko".to_owned(),
                title: "Roko ACP".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            auth_methods: Vec::new(),
        };

        assert_eq!(result.protocol_version, ACP_PROTOCOL_VERSION);
        assert!(result.agent_capabilities.load_session);
    }
}
