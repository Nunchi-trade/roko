//! Main ACP dispatch loop.

use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

use crate::{
    bridge_events::handle_session_prompt,
    config::AcpConfig,
    config_watch::ConfigWatcher,
    session::SessionManager,
    transport::{StdioTransport, TransportError},
    types::{
        ACP_PROTOCOL_VERSION, ACP_SPEC_VERSION, AgentCapabilities, AgentInfo, ConfigUpdateParams,
        ConfigUpdateResult, INVALID_PARAMS, InitializeParams, InitializeResult, JsonRpcId,
        JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, METHOD_NOT_FOUND, McpCapabilities,
        PARSE_ERROR, PromptCapabilities, SESSION_NOT_FOUND, SessionCancelParams,
        SessionCloseParams, SessionLoadParams, SessionNewParams, SessionPromptParams,
        SessionSetModeParams,
    },
};

/// Runs the ACP stdio server until stdin reaches EOF or a fatal transport error occurs.
pub async fn run_acp_server(config: AcpConfig) -> Result<()> {
    match run_acp_server_inner(config).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // Send JSON-RPC error on stdout so the editor (e.g. Zed) can display it
            // instead of showing a silent "server shut down unexpectedly" message.
            let error_response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32603,
                    "message": format!("ACP server failed to start: {e:#}")
                }
            });
            let _ = writeln!(std::io::stdout(), "{error_response}");
            Err(e)
        }
    }
}

async fn run_acp_server_inner(config: AcpConfig) -> Result<()> {
    // Ensure .roko/ workspace directory exists before any file operations.
    let workdir = config
        .workdir
        .canonicalize()
        .unwrap_or_else(|_| config.workdir.clone());
    let roko_dir = workdir.join(".roko");
    if let Err(e) = std::fs::create_dir_all(&roko_dir) {
        // Non-fatal — we'll fall back to /tmp for logging.
        eprintln!("warning: cannot create .roko/: {e}");
    }

    let _guard = setup_file_logging(config.log_file())
        .or_else(|e| {
            // Fallback: log to /tmp if .roko/ is unavailable.
            let fallback =
                std::env::temp_dir().join(format!("roko-acp-{}.log", std::process::id()));
            eprintln!("warning: {e}, falling back to {}", fallback.display());
            setup_file_logging(&fallback)
        })
        .with_context(|| "failed to initialize ACP logging")?;

    let mut transport = StdioTransport::new();
    run_acp_server_with_transport(config, &mut transport).await
}

/// Runs the ACP server against an injected transport.
pub async fn run_acp_server_with_transport<R, W>(
    config: AcpConfig,
    transport: &mut StdioTransport<R, W>,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let roko_config = config.load_roko_config();
    if !config.workdir.join("roko.toml").is_file() {
        warn!(
            workdir = %config.workdir.display(),
            "no roko.toml found in ACP workdir; using defaults and inherited config"
        );
    }
    info!(
        providers = roko_config.providers.len(),
        models = roko_config.models.len(),
        "loaded roko.toml configuration"
    );
    let mut sessions = SessionManager::new(config.workdir.clone(), roko_config);
    sessions.config_sources = config.config_sources();
    let mut config_watcher = ConfigWatcher::start(&config);

    // GC old persisted sessions at startup (7 days).
    sessions.gc_old_sessions(chrono::Duration::days(7));

    loop {
        let message = match transport.read_message().await {
            Ok(Some(message)) => message,
            Ok(None) => {
                info!("stdin reached EOF; shutting down ACP server");
                return Ok(());
            }
            Err(TransportError::Json(error)) => {
                error!(error = %error, "failed to decode inbound JSON-RPC message");
                transport
                    .send_error(
                        JsonRpcId::Null,
                        PARSE_ERROR,
                        format!("failed to parse JSON-RPC message: {error}"),
                    )
                    .await
                    .context("failed to send JSON-RPC parse error response")?;
                continue;
            }
            Err(error) => return Err(error).context("failed to read ACP message"),
        };

        match message {
            JsonRpcMessage::Request(request) => {
                if config_watcher.changed() {
                    let refreshed = config.load_roko_config();
                    info!(
                        providers = refreshed.providers.len(),
                        models = refreshed.models.len(),
                        "ACP config changed; reloaded roko.toml"
                    );
                    sessions.replace_roko_config(refreshed);
                }
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
    transport: &mut StdioTransport<impl AsyncRead + Unpin, impl AsyncWrite + Unpin>,
    sessions: &mut SessionManager,
    request: JsonRpcRequest,
) -> Result<()> {
    let JsonRpcRequest {
        id, method, params, ..
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
                    prompt_capabilities: PromptCapabilities {
                        image: false,
                        audio: false,
                        embedded_context: true,
                    },
                    mcp_capabilities: McpCapabilities {
                        http: true,
                        sse: true,
                    },
                },
                auth_methods: Vec::new(),
                agent_info: Some(AgentInfo {
                    name: "roko".to_owned(),
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                    title: Some("Roko".to_owned()),
                }),
                config_sources: sessions.config_sources.clone(),
            };
            send_success(transport, id, result).await
        }
        "session/new" => {
            let params: SessionNewParams = match parse_params(params, &method) {
                Ok(params) => params,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            let result = sessions.create_session(params);
            let session_id = result.session_id.clone();
            let bare_mode = sessions.roko_config.agent.bare_mode;
            send_success(transport, id, result).await?;
            send_slash_commands_notification(transport, &session_id, bare_mode).await
        }
        "session/list" => {
            let result = sessions.list_sessions_with_persisted();
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
            let workdir = sessions.workdir.clone();
            let roko_config = sessions.roko_config.clone();
            let session = match get_session_mut(sessions, &params.session_id) {
                Ok(session) => session,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            let session_id_for_persist = params.session_id.clone();
            let result =
                match handle_session_prompt(transport, session, params, &workdir, &roko_config)
                    .await
                {
                    Ok(result) => result,
                    Err(error) => {
                        if let Some(rpc_error) = error.rpc_error() {
                            return send_error_response(transport, id, rpc_error).await;
                        }
                        return Err(error).context("failed to handle ACP session prompt");
                    }
                };
            // Persist session after prompt completes.
            sessions.persist_session(&session_id_for_persist);
            send_success(transport, id, result).await
        }
        "session/config/update" | "session/set_config_option" => {
            let params: ConfigUpdateParams = match parse_params(params, &method) {
                Ok(params) => params,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            let roko_config = sessions.roko_config.clone();
            let session = match get_session_mut(sessions, &params.session_id) {
                Ok(session) => session,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            debug!(
                session_id = %session.session_id,
                option_id = %params.option_id,
                new_value = %params.new_value,
                "received config update request"
            );
            if let Err(message) =
                session.update_config(&params.option_id, &params.new_value, &roko_config)
            {
                return send_error_response(transport, id, json_rpc_error(INVALID_PARAMS, message))
                    .await;
            }
            let result = ConfigUpdateResult {
                config_options: session.config_options(),
            };
            send_success(transport, id, result).await
        }
        "session/close" => {
            let params: SessionCloseParams = match parse_params(params, &method) {
                Ok(params) => params,
                Err(error) => return send_error_response(transport, id, error).await,
            };
            sessions.close_session(&params.session_id);
            send_success(transport, id, serde_json::json!({})).await
        }
        "session/resume" => {
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
            let session_id = params.session_id.clone();
            let bare_mode = sessions.roko_config.agent.bare_mode;
            send_success(transport, id, result).await?;
            send_slash_commands_notification(transport, &session_id, bare_mode).await?;
            if let Some(session) = sessions.get_session(&session_id) {
                let options = serde_json::to_value(session.config_options())
                    .unwrap_or_else(|_| serde_json::json!([]));
                send_config_options_notification(transport, &session_id, options).await?;
            }
            Ok(())
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

fn handle_notification(sessions: &mut SessionManager, notification: JsonRpcNotification) {
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
                None => {
                    warn!(session_id = %params.session_id, "received cancel for unknown ACP session")
                }
            }
        }
        _ => warn!(method = %notification.method, "ignoring unsupported ACP notification"),
    }
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

async fn send_slash_commands_notification(
    transport: &mut StdioTransport<impl AsyncRead + Unpin, impl AsyncWrite + Unpin>,
    session_id: &str,
    bare_mode: bool,
) -> Result<()> {
    let commands = crate::session::build_slash_commands(bare_mode);
    let update = serde_json::json!({
        "sessionId": session_id,
        "update": {
            "sessionUpdate": "available_commands_update",
            "availableCommands": commands,
        }
    });
    transport
        .send_notification("session/update", update)
        .await
        .context("failed to send slash commands notification")
}

async fn send_config_options_notification(
    transport: &mut StdioTransport<impl AsyncRead + Unpin, impl AsyncWrite + Unpin>,
    session_id: &str,
    config_options: serde_json::Value,
) -> Result<()> {
    let update = serde_json::json!({
        "sessionId": session_id,
        "update": {
            "sessionUpdate": "config_option_update",
            "configOptions": config_options,
        }
    });
    transport
        .send_notification("session/update", update)
        .await
        .context("failed to send config options notification")
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

async fn send_success<T>(
    transport: &mut StdioTransport<impl AsyncRead + Unpin, impl AsyncWrite + Unpin>,
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
    transport: &mut StdioTransport<impl AsyncRead + Unpin, impl AsyncWrite + Unpin>,
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
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create ACP log directory {}", parent.display()))?;
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
}
