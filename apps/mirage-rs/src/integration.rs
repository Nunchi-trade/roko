//! Client and test-harness integration for local mirage instances.

#![allow(clippy::single_match_else)]

use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use alloy_primitives::{Address, B256, Bytes, hex};
use futures_util::StreamExt;
use futures_util::stream::{self, BoxStream};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{process::Child, time::sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{
    MirageError, Result, TransactionRequest,
    fork::MirageStatus,
    resources::ResourceUsage,
    scenario::{RunMode, Scenario, ScenarioJob},
};

/// Connection config for a mirage sidecar.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirageConfig {
    /// Base URL for the local JSON-RPC server.
    pub url: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// Retry attempts on transport errors.
    pub retry_attempts: u32,
    /// Initial retry backoff.
    pub retry_backoff: Duration,
}

/// Shared JSON-RPC method name for `mirage_watchContract`.
pub(crate) const MIRAGE_WATCH_CONTRACT_METHOD: &str = "mirage_watchContract";
/// Shared JSON-RPC method name for `mirage_getPosition`.
pub(crate) const MIRAGE_GET_POSITION_METHOD: &str = "mirage_getPosition";
/// Shared JSON-RPC method name for `mirage_status`.
pub(crate) const MIRAGE_STATUS_METHOD: &str = "mirage_status";
/// Shared JSON-RPC method name for `mirage_getResourceUsage`.
pub(crate) const MIRAGE_GET_RESOURCE_USAGE_METHOD: &str = "mirage_getResourceUsage";
/// Shared JSON-RPC method name for `mirage_beginScenarioSet`.
pub(crate) const MIRAGE_BEGIN_SCENARIO_SET_METHOD: &str = "mirage_beginScenarioSet";
/// Shared JSON-RPC method name for `mirage_defineScenario`.
pub(crate) const MIRAGE_DEFINE_SCENARIO_METHOD: &str = "mirage_defineScenario";
/// Shared JSON-RPC method name for `mirage_runScenarioSet`.
pub(crate) const MIRAGE_RUN_SCENARIO_SET_METHOD: &str = "mirage_runScenarioSet";
/// Shared JSON-RPC method name for `mirage_getScenarioResults`.
pub(crate) const MIRAGE_GET_SCENARIO_RESULTS_METHOD: &str = "mirage_getScenarioResults";
/// Shared JSON-RPC method name for `mirage_subscribeEvents`.
pub(crate) const MIRAGE_SUBSCRIBE_EVENTS_METHOD: &str = "mirage_subscribeEvents";
/// Shared JSON-RPC method name for `mirage_shutdown`.
pub(crate) const MIRAGE_SHUTDOWN_METHOD: &str = "mirage_shutdown";

/// Poll interval for [`MirageClient::wait_ready`] between `mirage_status` calls (plan decomposition Step 17).
const WAIT_READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Startup timeout for test-sidecar artifact publication.
const STARTUP_ARTIFACT_TIMEOUT: Duration = Duration::from_secs(10);

impl MirageConfig {
    /// Returns the default local development config.
    #[must_use]
    pub fn default_local() -> Self {
        Self {
            url: "http://127.0.0.1:8545".to_owned(),
            timeout: Duration::from_secs(30),
            retry_attempts: 3,
            retry_backoff: Duration::from_millis(500),
        }
    }
}

/// Position helper request payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionRequest {
    /// Position owner address.
    pub owner: Address,
    /// Protocol type string.
    pub protocol_type: String,
    /// Optional contract address.
    pub contract: Option<Address>,
    /// Addresses to include in the raw balance snapshot.
    pub token_addresses: Vec<Address>,
}

/// Position helper response payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionSnapshot {
    /// Requested owner address.
    pub owner: Address,
    /// Echoed protocol type.
    pub protocol_type: String,
    /// Raw payload for protocol-specific readers.
    pub data: serde_json::Value,
}

/// Event-source provenance.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventSource {
    /// Emitted by a locally submitted transaction.
    LocalTx,
    /// Emitted while replaying upstream state.
    FollowerReplay,
}

/// Event-stream filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFilter {
    /// Optional address filter.
    pub addresses: Option<Vec<Address>>,
    /// Optional topic filter.
    pub topics: Option<Vec<B256>>,
}

/// Event payload delivered to downstream consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirageEvent {
    /// Block number containing the event.
    pub block_number: u64,
    /// Transaction hash.
    pub tx_hash: B256,
    /// Log index within the receipt.
    pub log_index: u32,
    /// Contract that emitted the log.
    pub contract: Address,
    /// Event topics.
    pub topics: Vec<B256>,
    /// Raw log data.
    pub data: Bytes,
    /// Event provenance.
    pub source: EventSource,
    /// Optional decoded payload.
    pub decoded: Option<serde_json::Value>,
}

/// Async JSON-RPC client for mirage.
///
/// Wire method names use JSON-RPC / Ethereum `camelCase` (for example `mirage_watchContract`), matching
/// the strings registered in `crate::rpc` (plan V2 cross-crate contract).
#[derive(Debug, Clone)]
pub struct MirageClient {
    config: MirageConfig,
    inner: reqwest::Client,
}

impl MirageClient {
    /// Builds a new JSON-RPC HTTP client using [`MirageConfig::url`] as the POST endpoint and
    /// [`MirageConfig::timeout`] as the per-request timeout.
    ///
    /// # Errors
    ///
    /// Returns any `reqwest` client-construction error.
    pub async fn new(config: MirageConfig) -> Result<Self> {
        tokio::task::yield_now().await;
        let inner = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self { config, inner })
    }

    /// Executes `eth_call`.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn eth_call(&self, req: TransactionRequest) -> Result<Bytes> {
        let response: String = self
            .rpc_call("eth_call", serde_json::json!([req, "latest"]))
            .await?;
        parse_bytes_response(&response)
    }

    /// Executes `eth_sendTransaction` with params `[req]` (single-object array), matching the server
    /// JSON-RPC contract.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn eth_send_transaction(&self, req: TransactionRequest) -> Result<B256> {
        self.rpc_call("eth_sendTransaction", serde_json::json!([req]))
            .await
    }

    /// Captures an `evm_snapshot`.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn evm_snapshot(&self) -> Result<u64> {
        let raw: String = self.rpc_call("evm_snapshot", serde_json::json!([])).await?;
        parse_hex_u64(&raw)
    }

    /// Restores an `evm_revert` snapshot.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn evm_revert(&self, id: u64) -> Result<bool> {
        self.rpc_call("evm_revert", serde_json::json!([format!("0x{id:x}")]))
            .await
    }

    /// Adds a contract to the watch list.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn mirage_watch_contract(&self, addr: Address) -> Result<()> {
        let _: bool = self
            .rpc_call(MIRAGE_WATCH_CONTRACT_METHOD, serde_json::json!([addr]))
            .await?;
        Ok(())
    }

    /// Reads a position helper snapshot.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn mirage_get_position(&self, req: PositionRequest) -> Result<PositionSnapshot> {
        self.rpc_call(MIRAGE_GET_POSITION_METHOD, serde_json::json!([req]))
            .await
    }

    /// Reads the current status snapshot.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn mirage_status(&self) -> Result<MirageStatus> {
        self.rpc_call(MIRAGE_STATUS_METHOD, serde_json::json!([]))
            .await
    }

    /// Reads current resource usage.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn mirage_get_resource_usage(&self) -> Result<ResourceUsage> {
        self.rpc_call(MIRAGE_GET_RESOURCE_USAGE_METHOD, serde_json::json!([]))
            .await
    }

    /// Creates a new scenario set.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn mirage_begin_scenario_set(&self, baseline: &str) -> Result<String> {
        self.rpc_call(
            MIRAGE_BEGIN_SCENARIO_SET_METHOD,
            serde_json::json!([baseline]),
        )
        .await
    }

    /// Adds a scenario to an existing set.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn mirage_define_scenario(
        &self,
        set_id: &str,
        scenario: &Scenario,
    ) -> Result<String> {
        self.rpc_call(
            MIRAGE_DEFINE_SCENARIO_METHOD,
            serde_json::json!([set_id, scenario]),
        )
        .await
    }

    /// Starts scenario execution.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn mirage_run_scenario_set(&self, set_id: &str, mode: RunMode) -> Result<String> {
        self.rpc_call(
            MIRAGE_RUN_SCENARIO_SET_METHOD,
            serde_json::json!([set_id, mode]),
        )
        .await
    }

    /// Polls scenario results.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn mirage_get_scenario_results(&self, job_id: &str) -> Result<ScenarioJob> {
        self.rpc_call(
            MIRAGE_GET_SCENARIO_RESULTS_METHOD,
            serde_json::json!([job_id]),
        )
        .await
    }

    /// Waits until the sidecar reports `ready`, polling [`MIRAGE_STATUS_METHOD`] every
    /// [`WAIT_READY_POLL_INTERVAL`] until `timeout` elapses.
    ///
    /// # Errors
    ///
    /// Returns [`MirageError::Timeout`] if readiness is not reported before
    /// `timeout`, or any transport/JSON-RPC error when the sidecar request
    /// fails before the timeout is reached.
    pub async fn wait_ready(&self, timeout: Duration) -> Result<()> {
        let started = Instant::now();
        loop {
            match self.mirage_status().await {
                Ok(status) if status.status == "ready" => return Ok(()),
                Ok(_) | Err(_) if started.elapsed() < timeout => {
                    sleep(WAIT_READY_POLL_INTERVAL).await;
                }
                Ok(status) => {
                    return Err(MirageError::Timeout(format!(
                        "status remained {}",
                        status.status
                    )));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Returns a stream of currently known local events matching the filter.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors while creating
    /// the subscription or connecting to the event stream.
    pub async fn subscribe_events(
        &self,
        filter: EventFilter,
    ) -> Result<BoxStream<'static, MirageEvent>> {
        let stream_id: String = self
            .rpc_call(MIRAGE_SUBSCRIBE_EVENTS_METHOD, serde_json::json!([filter]))
            .await?;
        let mut url = reqwest::Url::parse(&self.config.url)
            .map_err(|error| MirageError::Unsupported(format!("invalid mirage url: {error}")))?;
        let scheme = match url.scheme() {
            "https" => "wss",
            _ => "ws",
        };
        url.set_scheme(scheme).map_err(|()| {
            MirageError::Unsupported("failed to convert mirage url to ws".to_owned())
        })?;
        url.set_path(&format!("/events/{stream_id}"));

        let (socket, _) = connect_async(url.as_str()).await.map_err(|error| {
            MirageError::Unsupported(format!("websocket connect failed: {error}"))
        })?;
        let (_, read) = socket.split();
        let events = stream::unfold(read, |mut read| async move {
            while let Some(message) = read.next().await {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        tracing::warn!("event stream closed: {error}");
                        return None;
                    }
                };
                let payload = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Binary(bytes) => match String::from_utf8(bytes.to_vec()) {
                        Ok(text) => text,
                        Err(error) => {
                            tracing::warn!("event stream delivered invalid utf8: {error}");
                            continue;
                        }
                    },
                    Message::Close(_) => return None,
                    _ => continue,
                };
                match serde_json::from_str::<MirageEvent>(&payload) {
                    Ok(event) => return Some((event, read)),
                    Err(error) => {
                        tracing::warn!("failed to decode mirage event: {error}");
                        continue;
                    }
                }
            }
            None
        });
        Ok(Box::pin(events))
    }

    /// Sends a shutdown request to the sidecar.
    ///
    /// # Errors
    ///
    /// Returns transport, timeout, or JSON-RPC decode errors from the sidecar
    /// request.
    pub async fn shutdown(&self) -> Result<bool> {
        self.rpc_call(MIRAGE_SHUTDOWN_METHOD, serde_json::json!([]))
            .await
    }

    async fn rpc_call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T> {
        let mut backoff = self.config.retry_backoff;
        let mut last_error = None;
        for _attempt in 0..=self.config.retry_attempts {
            match self.rpc_call_once(method, params.clone()).await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    last_error = Some(error);
                    sleep(backoff).await;
                    backoff = backoff.saturating_mul(2);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| MirageError::Timeout(method.to_owned())))
    }

    async fn rpc_call_once<T: DeserializeOwned>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T> {
        let response = self
            .inner
            .post(&self.config.url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params,
            }))
            .send()
            .await?;
        let value = response.json::<serde_json::Value>().await?;
        if let Some(error) = value.get("error") {
            return Err(MirageError::Unsupported(error.to_string()));
        }
        let result = value
            .get("result")
            .cloned()
            .ok_or_else(|| MirageError::Unsupported(format!("missing result for {method}")))?;
        serde_json::from_value(result).map_err(Into::into)
    }
}

/// Spawned mirage process managed by tests.
#[derive(Debug)]
pub struct MirageTestInstance {
    process: Child,
    port: u16,
    pid_file: PathBuf,
    status_file: PathBuf,
}

impl MirageTestInstance {
    /// Returns the connection config for this process.
    ///
    /// Uses [`MirageConfig::default_local`] for timeout and retry settings (INV-018 / INV-019) and
    /// overrides `url` to this instance's bound port.
    #[must_use]
    pub fn config(&self) -> MirageConfig {
        let mut config = MirageConfig::default_local();
        config.url = format!("http://127.0.0.1:{}", self.port);
        config
    }

    /// Asks the sidecar to shut down via [`MIRAGE_SHUTDOWN_METHOD`], waits for exit, then on Unix
    /// sends `SIGTERM` via `/bin/kill`, and finally calls [`Child::kill`](tokio::process::Child::kill)
    /// if the child is still alive.
    ///
    /// # Errors
    ///
    /// Returns process-management or wait errors from killing and reaping the
    /// child process.
    pub async fn shutdown(&mut self) -> Result<()> {
        let client = MirageClient::new(self.config()).await?;
        let _ = client.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.process.wait()).await;
        if self.process.try_wait()?.is_none() {
            #[cfg(unix)]
            {
                if let Some(pid) = self.process.id() {
                    let _ = std::process::Command::new("/bin/kill")
                        .args(["-TERM", &pid.to_string()])
                        .status();
                    let _ = tokio::time::timeout(Duration::from_secs(2), self.process.wait()).await;
                }
            }
            if self.process.try_wait()?.is_none() {
                self.process.kill().await?;
            }
        }
        let _ = tokio::fs::remove_file(&self.status_file).await;
        let _ = tokio::fs::remove_file(&self.pid_file).await;
        Ok(())
    }
}

/// Spawns a new test instance and waits for readiness.
///
/// # Errors
///
/// Returns process-spawn, filesystem, startup-artifact, or readiness-polling
/// errors encountered while bringing the instance up.
pub async fn spawn_mirage_test_instance(
    rpc_url: Option<&str>,
    port: Option<u16>,
) -> Result<MirageTestInstance> {
    let port = port.unwrap_or(18_545);
    let pid_file = PathBuf::from(format!("/tmp/mirage-{port}.pid"));
    let status_file = PathBuf::from(format!("/tmp/mirage-{port}-status.json"));
    let _ = tokio::fs::remove_file(&pid_file).await;
    let _ = tokio::fs::remove_file(&status_file).await;
    let executable = match std::env::var("CARGO_BIN_EXE_mirage-rs") {
        Ok(path) => path,
        Err(_) => {
            let current = std::env::current_exe()?;
            let target_dir = current
                .parent()
                .and_then(|path| path.parent())
                .ok_or_else(|| {
                    MirageError::Unsupported(
                        "workspace target/debug directory not found".to_owned(),
                    )
                })?;
            target_dir.join("mirage-rs").to_string_lossy().into_owned()
        }
    };

    let mut command = tokio::process::Command::new(executable);
    command
        .arg("--port")
        .arg(port.to_string())
        .arg("--no-persist")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.env("BARDO_AVAILABLE_MEMORY_BYTES", "8589934592");
    if let Some(url) = rpc_url {
        command.arg("--rpc-url").arg(url);
    }
    let process = command.spawn()?;
    let instance = MirageTestInstance {
        process,
        port,
        pid_file,
        status_file,
    };
    let mut instance = instance;
    wait_for_startup_artifacts(&mut instance, STARTUP_ARTIFACT_TIMEOUT).await?;
    let client = MirageClient::new(instance.config()).await?;
    if let Err(error) = client.wait_ready(Duration::from_secs(10)).await {
        let _ = instance.shutdown().await;
        return Err(error);
    }
    Ok(instance)
}

async fn wait_for_startup_artifacts(
    instance: &mut MirageTestInstance,
    timeout: Duration,
) -> Result<()> {
    let expected_pid = instance.process.id().ok_or_else(|| {
        MirageError::Unsupported("spawned mirage process did not expose an OS pid".to_owned())
    })?;
    let expected_pid = expected_pid.to_string();
    let started = Instant::now();

    loop {
        if let Some(status) = instance.process.try_wait()? {
            let error = MirageError::Unsupported(format!(
                "mirage process exited before publishing startup artifacts on port {}: {status}",
                instance.port
            ));
            let _ = instance.shutdown().await;
            return Err(error);
        }

        let pid_matches = match tokio::fs::read_to_string(&instance.pid_file).await {
            Ok(pid) => pid.trim() == expected_pid,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                let _ = instance.shutdown().await;
                return Err(error.into());
            }
        };
        let status_exists = match tokio::fs::try_exists(&instance.status_file).await {
            Ok(exists) => exists,
            Err(error) => {
                let _ = instance.shutdown().await;
                return Err(error.into());
            }
        };

        if pid_matches && status_exists {
            return Ok(());
        }

        if started.elapsed() >= timeout {
            let error = MirageError::Timeout(format!(
                "mirage startup artifacts not published for port {}",
                instance.port
            ));
            let _ = instance.shutdown().await;
            return Err(error);
        }

        sleep(WAIT_READY_POLL_INTERVAL).await;
    }
}

fn parse_hex_u64(value: &str) -> Result<u64> {
    u64::from_str_radix(value.trim_start_matches("0x"), 16)
        .map_err(|error| MirageError::InvalidParams(format!("invalid hex quantity: {error}")))
}

fn parse_bytes_response(value: &str) -> Result<Bytes> {
    let bytes = hex::decode(value.trim_start_matches("0x"))
        .map_err(|error| MirageError::InvalidParams(format!("invalid bytes response: {error}")))?;
    Ok(Bytes::from(bytes))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use alloy_primitives::{Address, B256, Bytes, U256, address, hex, keccak256};
    use futures_util::StreamExt;
    use k256::{
        FieldBytes,
        ecdsa::{SigningKey, hazmat::SignPrimitive},
        sha2,
    };
    use serde_json::Value;

    use crate::{
        TransactionRequest,
        integration::{EventFilter, EventSource, MirageClient, MirageConfig},
        provider::UpstreamRpc,
        rpc::spawn_rpc_server_for_tests,
    };

    fn test_client_config(url: String) -> MirageConfig {
        MirageConfig {
            url,
            timeout: Duration::from_secs(5),
            retry_attempts: 1,
            retry_backoff: Duration::from_millis(50),
        }
    }

    #[derive(Debug)]
    struct MockJsonRpcServer {
        url: String,
        requests: Arc<AtomicUsize>,
        shutdown_tx: Option<mpsc::Sender<()>>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl MockJsonRpcServer {
        fn spawn(failures_before_success: usize, block_number: u64) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0")
                .unwrap_or_else(|error| panic!("bind mock upstream server: {error}"));
            let addr = listener
                .local_addr()
                .unwrap_or_else(|error| panic!("mock upstream local addr: {error}"));
            listener
                .set_nonblocking(true)
                .unwrap_or_else(|error| panic!("configure mock upstream server: {error}"));

            let requests = Arc::new(AtomicUsize::new(0));
            let requests_for_thread = Arc::clone(&requests);
            let (shutdown_tx, shutdown_rx) = mpsc::channel();
            let handle = thread::spawn(move || {
                loop {
                    if shutdown_rx.try_recv().is_ok() {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let request_index =
                                requests_for_thread.fetch_add(1, Ordering::SeqCst) + 1;
                            if request_index <= failures_before_success {
                                continue;
                            }

                            read_http_request(&mut stream);
                            let body = format!(
                                r#"{{"jsonrpc":"2.0","id":1,"result":"0x{block_number:x}"}}"#
                            );
                            let response = format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            stream
                                .write_all(response.as_bytes())
                                .unwrap_or_else(|error| {
                                    panic!("write mock upstream response {request_index}: {error}")
                                });
                            stream.flush().unwrap_or_else(|error| {
                                panic!("flush mock upstream response {request_index}: {error}")
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept mock upstream connection: {error}"),
                    }
                }
            });

            Self {
                url: format!("http://{addr}"),
                requests,
                shutdown_tx: Some(shutdown_tx),
                handle: Some(handle),
            }
        }
    }

    impl Drop for MockJsonRpcServer {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(handle) = self.handle.take() {
                handle
                    .join()
                    .unwrap_or_else(|_| panic!("join mock upstream server thread"));
            }
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) {
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap_or_else(|error| panic!("set mock upstream read timeout: {error}"));

        let mut buffer = [0_u8; 1024];
        let mut request = Vec::new();
        let mut header_end = None;
        let mut expected_len = None;

        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    request.extend_from_slice(&buffer[..read]);
                    if header_end.is_none() {
                        header_end = request
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                            .map(|index| index + 4);
                        if let Some(end) = header_end {
                            expected_len = Some(end + content_length(&request[..end]));
                        }
                    }
                    if expected_len.is_some_and(|len| request.len() >= len) {
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => panic!("read mock upstream request: {error}"),
            }
        }
    }

    fn content_length(headers: &[u8]) -> usize {
        String::from_utf8_lossy(headers)
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    #[derive(Clone)]
    enum TestRlpValue {
        Bytes(Vec<u8>),
        List(Vec<TestRlpValue>),
    }

    fn trim_leading_zeros(mut bytes: Vec<u8>) -> Vec<u8> {
        let first_non_zero = bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(bytes.len());
        bytes.drain(..first_non_zero);
        bytes
    }

    fn rlp_from_u64(value: u64) -> TestRlpValue {
        TestRlpValue::Bytes(trim_leading_zeros(value.to_be_bytes().to_vec()))
    }

    fn rlp_encode(value: &TestRlpValue) -> Vec<u8> {
        match value {
            TestRlpValue::Bytes(bytes) => rlp_encode_bytes(bytes),
            TestRlpValue::List(values) => {
                let payload = values.iter().flat_map(rlp_encode).collect::<Vec<_>>();
                rlp_encode_with_offset(&payload, 0xc0, 0xf7)
            }
        }
    }

    fn rlp_encode_bytes(bytes: &[u8]) -> Vec<u8> {
        if bytes.len() == 1 && bytes[0] < 0x80 {
            bytes.to_vec()
        } else {
            rlp_encode_with_offset(bytes, 0x80, 0xb7)
        }
    }

    fn rlp_encode_with_offset(payload: &[u8], short_offset: u8, long_offset: u8) -> Vec<u8> {
        if payload.len() <= 55 {
            let mut encoded = Vec::with_capacity(1 + payload.len());
            encoded.push(short_offset + payload.len() as u8);
            encoded.extend_from_slice(payload);
            encoded
        } else {
            let len_bytes = trim_leading_zeros((payload.len() as u64).to_be_bytes().to_vec());
            let mut encoded = Vec::with_capacity(1 + len_bytes.len() + payload.len());
            encoded.push(long_offset + len_bytes.len() as u8);
            encoded.extend_from_slice(&len_bytes);
            encoded.extend_from_slice(payload);
            encoded
        }
    }

    fn signing_key_address(signing_key: &SigningKey) -> Address {
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let hash = keccak256(&encoded.as_bytes()[1..]);
        Address::from_slice(&hash.as_slice()[12..])
    }

    fn sign_legacy(
        signing_key: &SigningKey,
        chain_id: u64,
        gas_price: u64,
        gas_limit: u64,
        to: Option<Address>,
        value: u64,
        data: &[u8],
    ) -> Bytes {
        let unsigned = TestRlpValue::List(vec![
            rlp_from_u64(0),
            rlp_from_u64(gas_price),
            rlp_from_u64(gas_limit),
            to.map_or_else(
                || TestRlpValue::Bytes(Vec::new()),
                |address| TestRlpValue::Bytes(address.as_slice().to_vec()),
            ),
            rlp_from_u64(value),
            TestRlpValue::Bytes(data.to_vec()),
            rlp_from_u64(chain_id),
            TestRlpValue::Bytes(Vec::new()),
            TestRlpValue::Bytes(Vec::new()),
        ]);
        let unsigned_rlp = rlp_encode(&unsigned);
        let hash = keccak256(&unsigned_rlp);
        let mut field_bytes = FieldBytes::default();
        field_bytes.copy_from_slice(hash.as_slice());
        let (signature, recovery_id) = signing_key
            .as_nonzero_scalar()
            .try_sign_prehashed_rfc6979::<sha2::Sha256>(&field_bytes, &[])
            .unwrap_or_else(|error| panic!("sign legacy prehash: {error}"));
        let recovery_id = recovery_id.unwrap_or_else(|| panic!("legacy recovery id present"));
        let v = chain_id * 2 + 35 + u64::from(recovery_id.to_byte());
        let signed = TestRlpValue::List(vec![
            rlp_from_u64(0),
            rlp_from_u64(gas_price),
            rlp_from_u64(gas_limit),
            to.map_or_else(
                || TestRlpValue::Bytes(Vec::new()),
                |address| TestRlpValue::Bytes(address.as_slice().to_vec()),
            ),
            rlp_from_u64(value),
            TestRlpValue::Bytes(data.to_vec()),
            rlp_from_u64(v),
            TestRlpValue::Bytes(signature.r().to_bytes().to_vec()),
            TestRlpValue::Bytes(signature.s().to_bytes().to_vec()),
        ]);
        Bytes::from(rlp_encode(&signed))
    }

    fn sign_typed(signing_key: &SigningKey, tx_type: u8, to: Option<Address>) -> Bytes {
        let unsigned = match tx_type {
            0x01 => TestRlpValue::List(vec![
                rlp_from_u64(1),
                rlp_from_u64(0),
                rlp_from_u64(3),
                rlp_from_u64(80_000),
                to.map_or_else(
                    || TestRlpValue::Bytes(Vec::new()),
                    |address| TestRlpValue::Bytes(address.as_slice().to_vec()),
                ),
                rlp_from_u64(11),
                TestRlpValue::Bytes(vec![0x01, 0x02]),
                TestRlpValue::List(Vec::new()),
            ]),
            0x02 => TestRlpValue::List(vec![
                rlp_from_u64(1),
                rlp_from_u64(0),
                rlp_from_u64(1),
                rlp_from_u64(4),
                rlp_from_u64(80_000),
                to.map_or_else(
                    || TestRlpValue::Bytes(Vec::new()),
                    |address| TestRlpValue::Bytes(address.as_slice().to_vec()),
                ),
                rlp_from_u64(11),
                TestRlpValue::Bytes(vec![0x01, 0x02]),
                TestRlpValue::List(Vec::new()),
            ]),
            0x03 => TestRlpValue::List(vec![
                rlp_from_u64(1),
                rlp_from_u64(0),
                rlp_from_u64(1),
                rlp_from_u64(4),
                rlp_from_u64(80_000),
                to.map_or_else(
                    || TestRlpValue::Bytes(Vec::new()),
                    |address| TestRlpValue::Bytes(address.as_slice().to_vec()),
                ),
                rlp_from_u64(11),
                TestRlpValue::Bytes(vec![0x01, 0x02]),
                TestRlpValue::List(Vec::new()),
                rlp_from_u64(5),
                TestRlpValue::List(Vec::new()),
            ]),
            _ => panic!("unsupported tx type {tx_type}"),
        };
        let unsigned_rlp = rlp_encode(&unsigned);
        let mut payload = vec![tx_type];
        payload.extend_from_slice(&unsigned_rlp);
        let hash = keccak256(&payload);
        let mut field_bytes = FieldBytes::default();
        field_bytes.copy_from_slice(hash.as_slice());
        let (signature, recovery_id) = signing_key
            .as_nonzero_scalar()
            .try_sign_prehashed_rfc6979::<sha2::Sha256>(&field_bytes, &[])
            .unwrap_or_else(|error| panic!("sign typed prehash: {error}"));
        let recovery_id = recovery_id.unwrap_or_else(|| panic!("typed recovery id present"));

        let mut fields = match unsigned {
            TestRlpValue::List(fields) => fields,
            TestRlpValue::Bytes(_) => unreachable!("typed tx unsigned payload is always a list"),
        };
        fields.push(rlp_from_u64(u64::from(recovery_id.to_byte())));
        fields.push(TestRlpValue::Bytes(signature.r().to_bytes().to_vec()));
        fields.push(TestRlpValue::Bytes(signature.s().to_bytes().to_vec()));

        let mut encoded = vec![tx_type];
        encoded.extend_from_slice(&rlp_encode(&TestRlpValue::List(fields)));
        Bytes::from(encoded)
    }

    #[tokio::test]
    async fn mirage_client_wait_ready() {
        let (url, handle) = match spawn_rpc_server_for_tests().await {
            Ok(value) => value,
            Err(error) => panic!("server starts: {error}"),
        };
        let client = MirageClient::new(test_client_config(url))
            .await
            .unwrap_or_else(|error| panic!("client initializes: {error}"));

        client
            .wait_ready(Duration::from_secs(2))
            .await
            .unwrap_or_else(|error| panic!("server becomes ready: {error}"));
        handle
            .stop()
            .unwrap_or_else(|error| panic!("server stops cleanly: {error}"));
    }

    #[tokio::test]
    async fn mirage_client_subscribe_events_streams_live_logs() {
        let (url, handle) = match spawn_rpc_server_for_tests().await {
            Ok(value) => value,
            Err(error) => panic!("server starts: {error}"),
        };
        let client = MirageClient::new(test_client_config(url))
            .await
            .unwrap_or_else(|error| panic!("client initializes: {error}"));

        let token = address!("0x3300000000000000000000000000000000000001");
        let owner = address!("0x3300000000000000000000000000000000000002");

        client
            .rpc_call::<bool>("mirage_setCode", serde_json::json!([token, "0x6001600055"]))
            .await
            .unwrap_or_else(|error| panic!("token code set: {error}"));
        client
            .rpc_call::<bool>(
                "mirage_mintERC20",
                serde_json::json!([token, owner, "0x10"]),
            )
            .await
            .unwrap_or_else(|error| panic!("token minted: {error}"));

        let mut events = client
            .subscribe_events(EventFilter {
                addresses: Some(vec![token]),
                topics: None,
            })
            .await
            .unwrap_or_else(|error| panic!("subscribe events: {error}"));

        let calldata = Bytes::from(hex::decode(
            "a9059cbb00000000000000000000000033000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000005",
        )
        .unwrap_or_else(|error| panic!("calldata bytes: {error}")));
        let request = TransactionRequest {
            from: Some(owner),
            to: Some(token),
            gas: Some(100_000),
            value: Some(U256::ZERO),
            data: Some(calldata),
            gas_price: None,
            nonce: None,
            chain_id: None,
        };
        client
            .eth_send_transaction(request)
            .await
            .unwrap_or_else(|error| panic!("send tx: {error}"));

        let event = tokio::time::timeout(Duration::from_secs(2), events.next())
            .await
            .unwrap_or_else(|error| panic!("event timeout: {error}"))
            .unwrap_or_else(|| panic!("event stream closed"));
        assert_eq!(event.contract, token);
        assert_eq!(event.tx_hash != alloy_primitives::B256::ZERO, true);
        assert_eq!(event.topics.len(), 3);

        handle
            .stop()
            .unwrap_or_else(|error| panic!("server stops cleanly: {error}"));
    }

    #[test]
    fn test_mirage_config_default_timeout() {
        let config = MirageConfig::default_local();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.url, "http://127.0.0.1:8545");
    }

    #[test]
    fn test_mirage_config_default_retries() {
        let config = MirageConfig::default_local();
        assert_eq!(config.retry_attempts, 3);
        assert_eq!(config.retry_backoff, Duration::from_millis(500));
    }

    #[test]
    fn wait_ready_poll_interval_matches_decomposition_step_17() {
        assert_eq!(super::WAIT_READY_POLL_INTERVAL, Duration::from_millis(100));
    }

    #[test]
    fn mirage_jsonrpc_method_constants_match_camel_case_registration() {
        assert_eq!(super::MIRAGE_WATCH_CONTRACT_METHOD, "mirage_watchContract");
        assert_eq!(super::MIRAGE_GET_POSITION_METHOD, "mirage_getPosition");
        assert_eq!(super::MIRAGE_STATUS_METHOD, "mirage_status");
        assert_eq!(
            super::MIRAGE_GET_RESOURCE_USAGE_METHOD,
            "mirage_getResourceUsage"
        );
        assert_eq!(
            super::MIRAGE_BEGIN_SCENARIO_SET_METHOD,
            "mirage_beginScenarioSet"
        );
        assert_eq!(
            super::MIRAGE_DEFINE_SCENARIO_METHOD,
            "mirage_defineScenario"
        );
        assert_eq!(
            super::MIRAGE_RUN_SCENARIO_SET_METHOD,
            "mirage_runScenarioSet"
        );
        assert_eq!(
            super::MIRAGE_GET_SCENARIO_RESULTS_METHOD,
            "mirage_getScenarioResults"
        );
        assert_eq!(
            super::MIRAGE_SUBSCRIBE_EVENTS_METHOD,
            "mirage_subscribeEvents"
        );
        assert_eq!(super::MIRAGE_SHUTDOWN_METHOD, "mirage_shutdown");
    }

    #[test]
    fn eth_send_transaction_params_json_is_single_request_object_array() {
        let req = TransactionRequest {
            from: Some(address!("0x1000000000000000000000000000000000000001")),
            to: Some(address!("0x2000000000000000000000000000000000000002")),
            gas: Some(21_000),
            value: Some(U256::from(1_u64)),
            data: Some(Bytes::from_static(&[0xab])),
            gas_price: None,
            nonce: None,
            chain_id: None,
        };
        let params = serde_json::json!([req]);
        let arr = params
            .as_array()
            .expect("eth_sendTransaction params must be a JSON array");
        assert_eq!(
            arr.len(),
            1,
            "server contract is [req], not flattened fields"
        );
        assert!(arr[0].is_object());
    }

    #[test]
    fn test_upstream_rps_limit() {
        let server = MockJsonRpcServer::spawn(0, 42);
        let upstream = UpstreamRpc::new_with_limits(Some(server.url.clone()), None, 1, 5, 5);

        let started = std::time::Instant::now();
        for _ in 0..6 {
            let block = upstream
                .get_block_number()
                .unwrap_or_else(|error| panic!("rate-limited block read succeeds: {error}"));
            assert_eq!(block, 42);
        }
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(150),
            "six requests with burst=5 and rps=5 should incur rate limiting, took {elapsed:?}"
        );
        assert_eq!(server.requests.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn test_upstream_exponential_backoff() {
        let upstream = UpstreamRpc::new(Some("http://127.0.0.1:1".to_owned()), None, 1);

        let started = std::time::Instant::now();
        let error = upstream
            .get_block_number()
            .expect_err("unreachable upstream should fail after exhausting retries");
        let elapsed = started.elapsed();

        assert!(
            error.to_string().contains("HTTP error"),
            "expected transport error after retries, got {error}"
        );
        assert!(
            elapsed >= Duration::from_millis(250),
            "two retries should apply exponential backoff before failing, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_eth_estimate_gas_buffer() {
        let (url, handle) = spawn_rpc_server_for_tests()
            .await
            .unwrap_or_else(|error| panic!("server starts: {error}"));
        let client = MirageClient::new(test_client_config(url))
            .await
            .unwrap_or_else(|error| panic!("client initializes: {error}"));
        let sender = address!("0x3400000000000000000000000000000000000001");
        let receiver = address!("0x3400000000000000000000000000000000000002");
        let request = TransactionRequest {
            from: Some(sender),
            to: Some(receiver),
            gas: Some(21_000),
            value: Some(U256::from(1_u64)),
            ..Default::default()
        };

        let estimate_hex: String = client
            .rpc_call("eth_estimateGas", serde_json::json!([request]))
            .await
            .unwrap_or_else(|error| panic!("estimate gas succeeds: {error}"));
        let estimate = super::parse_hex_u64(&estimate_hex)
            .unwrap_or_else(|error| panic!("parse estimate {estimate_hex}: {error}"));

        assert_eq!(estimate, 25_200);
        handle
            .stop()
            .unwrap_or_else(|error| panic!("server stops cleanly: {error}"));
    }

    #[tokio::test]
    async fn test_eip2718_type_parsing() {
        let (url, handle) = spawn_rpc_server_for_tests()
            .await
            .unwrap_or_else(|error| panic!("server starts: {error}"));
        let client = MirageClient::new(test_client_config(url))
            .await
            .unwrap_or_else(|error| panic!("client initializes: {error}"));
        let signing_key = SigningKey::from_bytes((&[9_u8; 32]).into())
            .unwrap_or_else(|error| panic!("signing key: {error}"));
        let expected_from = signing_key_address(&signing_key);
        let receiver = address!("0x3500000000000000000000000000000000000002");
        let cases = [
            (
                sign_legacy(&signing_key, 1, 5, 21_000, Some(receiver), 9, &[0xde, 0xad]),
                Some(receiver),
                21_000_u64,
                U256::from(9_u64),
                "0xdead".to_owned(),
            ),
            (
                sign_typed(&signing_key, 0x01, Some(receiver)),
                Some(receiver),
                80_000_u64,
                U256::from(11_u64),
                "0x0102".to_owned(),
            ),
            (
                sign_typed(&signing_key, 0x02, Some(receiver)),
                Some(receiver),
                80_000_u64,
                U256::from(11_u64),
                "0x0102".to_owned(),
            ),
            (
                sign_typed(&signing_key, 0x03, None),
                None,
                80_000_u64,
                U256::from(11_u64),
                "0x0102".to_owned(),
            ),
        ];

        for (raw, expected_to, expected_gas, expected_value, expected_input) in cases {
            let tx_hash: B256 = client
                .rpc_call(
                    "eth_sendRawTransaction",
                    serde_json::json!([format!("0x{}", hex::encode(&raw))]),
                )
                .await
                .unwrap_or_else(|error| panic!("send raw transaction succeeds: {error}"));
            assert_eq!(tx_hash, keccak256(raw.as_ref()));

            let tx: Value = client
                .rpc_call("eth_getTransactionByHash", serde_json::json!([tx_hash]))
                .await
                .unwrap_or_else(|error| panic!("load local transaction by hash: {error}"));
            assert_eq!(tx.get("from"), Some(&serde_json::json!(expected_from)));
            assert_eq!(tx.get("to"), Some(&serde_json::json!(expected_to)));
            assert_eq!(
                tx.get("value"),
                Some(&serde_json::json!(format!("0x{expected_value:x}")))
            );
            assert_eq!(tx.get("input"), Some(&serde_json::json!(expected_input)));
            assert_eq!(
                super::parse_hex_u64(
                    tx.get("gas")
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| panic!("gas field present on local tx")),
                )
                .unwrap_or_else(|error| panic!("parse tx gas: {error}")),
                expected_gas
            );

            let receipt: Value = client
                .rpc_call("eth_getTransactionReceipt", serde_json::json!([tx_hash]))
                .await
                .unwrap_or_else(|error| panic!("load local receipt by hash: {error}"));
            assert_eq!(
                receipt.get("transactionHash"),
                Some(&serde_json::json!(tx_hash))
            );
            assert_eq!(receipt.get("to"), Some(&serde_json::json!(expected_to)));
        }

        handle
            .stop()
            .unwrap_or_else(|error| panic!("server stops cleanly: {error}"));
    }

    #[tokio::test]
    async fn test_local_tx_event_sequence() {
        let (url, handle) = spawn_rpc_server_for_tests()
            .await
            .unwrap_or_else(|error| panic!("server starts: {error}"));
        let client = MirageClient::new(test_client_config(url))
            .await
            .unwrap_or_else(|error| panic!("client initializes: {error}"));
        let sender = address!("0x3600000000000000000000000000000000000001");
        let contract = address!("0x3600000000000000000000000000000000000002");
        let mut events = client
            .subscribe_events(EventFilter {
                addresses: Some(vec![contract]),
                topics: None,
            })
            .await
            .unwrap_or_else(|error| panic!("subscribe events: {error}"));

        let tx_hash = client
            .eth_send_transaction(TransactionRequest {
                from: Some(sender),
                to: Some(contract),
                gas: Some(150_000),
                value: Some(U256::ZERO),
                data: Some(Bytes::from_static(&[0x41, 0x4b, 0xf3, 0x89, 0x00, 0x01])),
                ..Default::default()
            })
            .await
            .unwrap_or_else(|error| panic!("send protocol touch tx: {error}"));

        let event = tokio::time::timeout(Duration::from_secs(2), events.next())
            .await
            .unwrap_or_else(|error| panic!("event timeout: {error}"))
            .unwrap_or_else(|| panic!("event stream closed"));

        let dirty_slots: Value = client
            .rpc_call("mirage_getDirtySlots", serde_json::json!([contract]))
            .await
            .unwrap_or_else(|error| panic!("load dirty slots after tx: {error}"));
        let watch_list: Vec<Value> = client
            .rpc_call("mirage_getWatchList", serde_json::json!([]))
            .await
            .unwrap_or_else(|error| panic!("load watch list after tx: {error}"));
        let receipt: Value = client
            .rpc_call("eth_getTransactionReceipt", serde_json::json!([tx_hash]))
            .await
            .unwrap_or_else(|error| panic!("load receipt after tx: {error}"));

        assert_eq!(event.tx_hash, tx_hash);
        assert_eq!(event.contract, contract);
        assert_eq!(event.log_index, 0);
        assert!(matches!(event.source, EventSource::LocalTx));
        assert!(
            dirty_slots
                .as_object()
                .is_some_and(|slots| slots.len() >= 3),
            "protocol touch should dirty the selector, caller, and calldata slots"
        );
        assert!(
            watch_list
                .iter()
                .any(|entry| entry.get("address") == Some(&serde_json::json!(contract))),
            "classifier should watch the touched protocol contract before the receipt is observed"
        );
        assert_eq!(
            receipt.get("transactionHash"),
            Some(&serde_json::json!(tx_hash))
        );
        assert_eq!(receipt.get("to"), Some(&serde_json::json!(Some(contract))));
        assert_eq!(
            receipt
                .get("logs")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default(),
            1
        );

        handle
            .stop()
            .unwrap_or_else(|error| panic!("server stops cleanly: {error}"));
    }
}
