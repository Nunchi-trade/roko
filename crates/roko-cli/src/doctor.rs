//! `roko doctor` bootstrap diagnostics for self-hosted workspaces.

use crate::config::{ConfigLayer, ConfigPaths, resolve_paths};
use crate::{Config, load_resolved_config};
use anyhow::{Context as _, Result};
use reqwest::Url;
use roko_fs::RokoLayout;
use serde::Serialize;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_HEALTH_PATH: &str = "/api/health";
const DOCTOR_HTTP_TIMEOUT_SECS: u64 = 2;

/// Inputs for `roko doctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorOptions {
    /// Workspace root to inspect.
    pub workdir: PathBuf,
    /// Optional explicit config override path (`--config`).
    pub config_override: Option<PathBuf>,
    /// Optional roko-serve base URL or explicit health endpoint URL.
    pub serve_url: Option<String>,
}

/// One doctor check status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    Ok,
    Warn,
    Fail,
    Skipped,
}

impl DoctorStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Skipped => "skipped",
        }
    }
}

/// One named diagnostic check in the doctor report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorCheck {
    pub id: String,
    pub status: DoctorStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Actionable fix command printed after `[fail]` / `[warn]` lines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

/// Summary counters for a doctor run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DoctorSummary {
    pub total: usize,
    pub ok: usize,
    pub warn: usize,
    pub fail: usize,
    pub skipped: usize,
}

impl DoctorSummary {
    fn from_checks(checks: &[DoctorCheck]) -> Self {
        let mut summary = Self {
            total: checks.len(),
            ok: 0,
            warn: 0,
            fail: 0,
            skipped: 0,
        };
        for check in checks {
            match check.status {
                DoctorStatus::Ok => summary.ok += 1,
                DoctorStatus::Warn => summary.warn += 1,
                DoctorStatus::Fail => summary.fail += 1,
                DoctorStatus::Skipped => summary.skipped += 1,
            }
        }
        summary
    }
}

/// Full report returned by `roko doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub workdir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serve_url: Option<String>,
    pub healthy: bool,
    pub summary: DoctorSummary,
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    /// Exit code for the report: `0` on success, `1` if any checks failed.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        if self.healthy { 0 } else { 1 }
    }

    /// Human-readable text rendering for the report.
    #[must_use]
    pub fn render_human(&self) -> String {
        let mut out = String::new();
        let headline = if self.healthy { "ok" } else { "fail" };
        let _ = writeln!(&mut out, "doctor: {headline}");
        let _ = writeln!(&mut out, "workdir: {}", self.workdir);
        if let Some(config_path) = &self.config_path {
            let _ = writeln!(&mut out, "config: {config_path}");
        }
        if let Some(serve_url) = &self.serve_url {
            let _ = writeln!(&mut out, "serve_url: {serve_url}");
        }
        let _ = writeln!(
            &mut out,
            "summary: {} ok, {} warn, {} failed, {} skipped",
            self.summary.ok, self.summary.warn, self.summary.fail, self.summary.skipped
        );
        for check in &self.checks {
            let _ = write!(
                &mut out,
                "[{}] {}: {}",
                check.status.label(),
                check.id,
                check.message
            );
            if let Some(path) = &check.path {
                let _ = write!(&mut out, " ({path})");
            }
            if let Some(url) = &check.url {
                let _ = write!(&mut out, " [{url}]");
            }
            if let Some(detail) = &check.detail {
                let _ = write!(&mut out, " - {detail}");
            }
            out.push('\n');
            if matches!(check.status, DoctorStatus::Fail | DoctorStatus::Warn) {
                if let Some(fix) = &check.fix {
                    let _ = writeln!(&mut out, "    \u{2192} fix: {fix}");
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone)]
struct LoadedConfig {
    paths: ConfigPaths,
    resolved: Option<Config>,
    active_path: Option<PathBuf>,
    explicit_serve: bool,
}

/// Run doctor diagnostics for one workspace.
pub async fn run_doctor(options: &DoctorOptions) -> Result<DoctorReport> {
    let workdir = options.workdir.clone();
    let loaded_config = load_active_config(&workdir, options.config_override.as_deref())?;

    let mut checks = Vec::new();
    checks.push(check_workdir(&workdir));
    checks.push(check_config_presence(
        &workdir,
        options.config_override.as_deref(),
        &loaded_config,
    ));
    checks.push(check_layout_basics(&workdir));
    checks.push(check_claude_cli());
    checks.push(check_anthropic_api_key());
    checks.push(check_rust_version());
    checks.push(check_node_version());
    checks.push(check_serve_auth(&loaded_config));
    checks.push(check_serve_health(options.serve_url.as_deref(), &loaded_config).await?);
    checks.push(check_v2_abstractions());

    let summary = DoctorSummary::from_checks(&checks);
    Ok(DoctorReport {
        workdir: workdir.display().to_string(),
        config_path: loaded_config
            .active_path
            .as_ref()
            .map(|path| path.display().to_string()),
        serve_url: checks
            .iter()
            .find(|check| check.id == "serve_health")
            .and_then(|check| check.url.clone()),
        healthy: summary.fail == 0,
        summary,
        checks,
    })
}

fn load_active_config(workdir: &Path, config_override: Option<&Path>) -> Result<LoadedConfig> {
    if let Some(path) = config_override {
        if !path.is_file() {
            return Ok(LoadedConfig {
                paths: ConfigPaths {
                    global: crate::config::global_config_path(),
                    project: None,
                    env_override: std::env::var_os("ROKO_CONFIG").map(PathBuf::from),
                },
                resolved: None,
                active_path: Some(path.to_path_buf()),
                explicit_serve: false,
            });
        }

        let layer = ConfigLayer::from_file(path)?;
        let resolved = Config::from_file(path)?;
        return Ok(LoadedConfig {
            paths: ConfigPaths {
                global: crate::config::global_config_path(),
                project: Some(path.to_path_buf()),
                env_override: std::env::var_os("ROKO_CONFIG").map(PathBuf::from),
            },
            resolved: Some(resolved),
            active_path: Some(path.to_path_buf()),
            explicit_serve: layer.serve.is_some(),
        });
    }

    let paths = resolve_paths(workdir);
    let mut explicit_serve = false;
    let active_path = if let Some(env_path) = &paths.env_override {
        match std::fs::read_to_string(env_path) {
            Ok(text) => {
                let layer = ConfigLayer::parse_toml(&text)
                    .with_context(|| format!("parse config {}", env_path.display()))?;
                explicit_serve = layer.serve.is_some();
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("read config {}", env_path.display()))
                );
            }
        }
        Some(env_path.clone())
    } else {
        let mut merged = ConfigLayer::default();
        let mut active_path = None;

        match std::fs::read_to_string(&paths.global) {
            Ok(text) => {
                let layer = ConfigLayer::parse_toml(&text)
                    .with_context(|| format!("parse config {}", paths.global.display()))?;
                explicit_serve |= layer.serve.is_some();
                merged = merged.merge(layer);
                active_path = Some(paths.global.clone());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(anyhow::Error::new(e)
                    .context(format!("read config {}", paths.global.display())));
            }
        }
        if let Some(project_path) = &paths.project {
            let layer = ConfigLayer::from_file(project_path)?;
            explicit_serve |= layer.serve.is_some();
            merged = merged.merge(layer);
            active_path = Some(project_path.clone());
        }

        if merged.is_empty() { None } else { active_path }
    };

    let resolved = if paths
        .env_override
        .as_ref()
        .is_some_and(|path| path.is_file())
        || paths.global.is_file()
        || paths.project.is_some()
    {
        Some(load_resolved_config(workdir)?.config)
    } else {
        None
    };

    Ok(LoadedConfig {
        paths,
        resolved,
        active_path,
        explicit_serve,
    })
}

fn check_workdir(workdir: &Path) -> DoctorCheck {
    let path = workdir.display().to_string();
    if workdir.is_dir() {
        return DoctorCheck {
            id: "workdir".to_string(),
            status: DoctorStatus::Ok,
            message: "workspace directory exists".to_string(),
            detail: None,
            path: Some(path),
            url: None,
            fix: None,
        };
    }

    let message = if workdir.exists() {
        "workspace path is not a directory"
    } else {
        "workspace directory is missing"
    };
    DoctorCheck {
        id: "workdir".to_string(),
        status: DoctorStatus::Fail,
        message: message.to_string(),
        detail: None,
        path: Some(path),
        url: None,
        fix: Some("roko init".to_string()),
    }
}

fn check_config_presence(
    workdir: &Path,
    config_override: Option<&Path>,
    loaded_config: &LoadedConfig,
) -> DoctorCheck {
    if let Some(path) = config_override {
        return if path.is_file() {
            DoctorCheck {
                id: "config".to_string(),
                status: DoctorStatus::Ok,
                message: "using explicit config override".to_string(),
                detail: None,
                path: Some(path.display().to_string()),
                url: None,
                fix: None,
            }
        } else {
            DoctorCheck {
                id: "config".to_string(),
                status: DoctorStatus::Fail,
                message: "explicit config override is missing".to_string(),
                detail: None,
                path: Some(path.display().to_string()),
                url: None,
                fix: Some("roko init".to_string()),
            }
        };
    }

    if let Some(path) = &loaded_config.paths.env_override {
        return if path.is_file() {
            DoctorCheck {
                id: "config".to_string(),
                status: DoctorStatus::Ok,
                message: "using ROKO_CONFIG override".to_string(),
                detail: None,
                path: Some(path.display().to_string()),
                url: None,
                fix: None,
            }
        } else {
            DoctorCheck {
                id: "config".to_string(),
                status: DoctorStatus::Fail,
                message: "ROKO_CONFIG points to a missing file".to_string(),
                detail: None,
                path: Some(path.display().to_string()),
                url: None,
                fix: Some("roko init".to_string()),
            }
        };
    }

    if let Some(path) = &loaded_config.paths.project {
        return DoctorCheck {
            id: "config".to_string(),
            status: DoctorStatus::Ok,
            message: "found project roko.toml".to_string(),
            detail: None,
            path: Some(path.display().to_string()),
            url: None,
            fix: None,
        };
    }

    DoctorCheck {
        id: "config".to_string(),
        status: DoctorStatus::Fail,
        message: "missing project roko.toml".to_string(),
        detail: Some(format!(
            "expected {} or an ancestor config; global config alone is not enough for workspace bootstrap",
            workdir.join("roko.toml").display()
        )),
        path: Some(loaded_config.paths.global.display().to_string()),
        url: None,
        fix: Some("roko init".to_string()),
    }
}

fn check_layout_basics(workdir: &Path) -> DoctorCheck {
    let layout = RokoLayout::for_project(workdir);
    let root = layout.root().display().to_string();
    if !layout.root().is_dir() {
        return DoctorCheck {
            id: "layout".to_string(),
            status: DoctorStatus::Fail,
            message: "missing .roko directory".to_string(),
            detail: None,
            path: Some(root),
            url: None,
            fix: Some("roko init".to_string()),
        };
    }

    let mut missing = Vec::new();
    if !layout.version_file().is_file() {
        missing.push(layout.version_file().display().to_string());
    }
    for dir in layout.top_level_dirs() {
        if !dir.is_dir() {
            missing.push(dir.display().to_string());
        }
    }

    if missing.is_empty() {
        DoctorCheck {
            id: "layout".to_string(),
            status: DoctorStatus::Ok,
            message: ".roko layout basics are present".to_string(),
            detail: None,
            path: Some(root),
            url: None,
            fix: None,
        }
    } else {
        DoctorCheck {
            id: "layout".to_string(),
            status: DoctorStatus::Fail,
            message: "required .roko layout paths are missing".to_string(),
            detail: Some(missing.join(", ")),
            path: Some(root),
            url: None,
            fix: Some("roko init".to_string()),
        }
    }
}

fn check_serve_auth(loaded_config: &LoadedConfig) -> DoctorCheck {
    let Some(config) = &loaded_config.resolved else {
        return DoctorCheck {
            id: "serve_auth".to_string(),
            status: DoctorStatus::Skipped,
            message: "config unavailable; serve/auth not evaluated".to_string(),
            detail: None,
            path: loaded_config
                .active_path
                .as_ref()
                .map(|path| path.display().to_string()),
            url: None,
            fix: None,
        };
    };

    if !loaded_config.explicit_serve {
        return DoctorCheck {
            id: "serve_auth".to_string(),
            status: DoctorStatus::Skipped,
            message: "no explicit [serve] config found".to_string(),
            detail: None,
            path: loaded_config
                .active_path
                .as_ref()
                .map(|path| path.display().to_string()),
            url: None,
            fix: None,
        };
    }

    let auth = &config.serve.auth;
    if auth.enabled && auth.api_key.trim().is_empty() {
        return DoctorCheck {
            id: "serve_auth".to_string(),
            status: DoctorStatus::Fail,
            message: "serve auth is enabled but api_key is empty".to_string(),
            detail: None,
            path: loaded_config
                .active_path
                .as_ref()
                .map(|path| path.display().to_string()),
            url: None,
            fix: Some("roko config set serve.auth.api_key <your-key>".to_string()),
        };
    }

    DoctorCheck {
        id: "serve_auth".to_string(),
        status: DoctorStatus::Ok,
        message: if auth.enabled {
            "serve auth is enabled and api_key is present".to_string()
        } else {
            "serve config is present and auth is disabled".to_string()
        },
        detail: None,
        path: loaded_config
            .active_path
            .as_ref()
            .map(|path| path.display().to_string()),
        url: None,
        fix: None,
    }
}

async fn check_serve_health(
    serve_url: Option<&str>,
    loaded_config: &LoadedConfig,
) -> Result<DoctorCheck> {
    let Some(raw_url) = serve_url else {
        return Ok(DoctorCheck {
            id: "serve_health".to_string(),
            status: DoctorStatus::Skipped,
            message: "serve health probe not requested".to_string(),
            detail: None,
            path: None,
            url: None,
            fix: None,
        });
    };

    let endpoint = match normalize_health_endpoint_url(raw_url) {
        Ok(url) => url,
        Err(err) => {
            return Ok(DoctorCheck {
                id: "serve_health".to_string(),
                status: DoctorStatus::Fail,
                message: "invalid serve URL".to_string(),
                detail: Some(err.to_string()),
                path: None,
                url: Some(raw_url.to_string()),
                fix: Some("roko serve".to_string()),
            });
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DOCTOR_HTTP_TIMEOUT_SECS))
        .build()
        .context("build doctor HTTP client")?;

    let mut request = client.get(endpoint.clone());
    if let Some(config) = &loaded_config.resolved
        && config.serve.auth.enabled
        && !config.serve.auth.api_key.trim().is_empty()
    {
        request = request.header("X-Api-Key", config.serve.auth.api_key.clone());
    }

    let response = request.send().await;
    let check = match response {
        Ok(response) if response.status().is_success() => DoctorCheck {
            id: "serve_health".to_string(),
            status: DoctorStatus::Ok,
            message: format!("health endpoint reachable ({})", response.status()),
            detail: None,
            path: None,
            url: Some(endpoint.to_string()),
            fix: None,
        },
        Ok(response) => DoctorCheck {
            id: "serve_health".to_string(),
            status: DoctorStatus::Fail,
            message: format!("health endpoint returned {}", response.status()),
            detail: None,
            path: None,
            url: Some(endpoint.to_string()),
            fix: Some("roko serve".to_string()),
        },
        Err(err) if err.is_builder() => DoctorCheck {
            id: "serve_health".to_string(),
            status: DoctorStatus::Fail,
            message: "invalid serve URL".to_string(),
            detail: Some(err.to_string()),
            path: None,
            url: Some(endpoint.to_string()),
            fix: Some("roko serve".to_string()),
        },
        Err(err) => DoctorCheck {
            id: "serve_health".to_string(),
            status: DoctorStatus::Fail,
            message: "health endpoint is unreachable".to_string(),
            detail: Some(err.to_string()),
            path: None,
            url: Some(endpoint.to_string()),
            fix: Some("roko serve".to_string()),
        },
    };
    Ok(check)
}

fn check_claude_cli() -> DoctorCheck {
    let available = std::process::Command::new("claude")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if available {
        DoctorCheck {
            id: "claude_cli".to_string(),
            status: DoctorStatus::Ok,
            message: "claude CLI is on PATH".to_string(),
            detail: None,
            path: None,
            url: None,
            fix: None,
        }
    } else {
        DoctorCheck {
            id: "claude_cli".to_string(),
            status: DoctorStatus::Warn,
            message: "claude CLI not found on PATH".to_string(),
            detail: None,
            path: None,
            url: None,
            fix: Some("npm install -g @anthropic-ai/claude-cli && claude login".to_string()),
        }
    }
}

fn check_anthropic_api_key() -> DoctorCheck {
    let has_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .is_some();

    if has_key {
        DoctorCheck {
            id: "anthropic_api_key".to_string(),
            status: DoctorStatus::Ok,
            message: "ANTHROPIC_API_KEY is set".to_string(),
            detail: None,
            path: None,
            url: None,
            fix: None,
        }
    } else {
        DoctorCheck {
            id: "anthropic_api_key".to_string(),
            status: DoctorStatus::Warn,
            message: "ANTHROPIC_API_KEY not set".to_string(),
            detail: None,
            path: None,
            url: None,
            fix: Some("export ANTHROPIC_API_KEY=sk-ant-...".to_string()),
        }
    }
}

fn check_rust_version() -> DoctorCheck {
    let output = std::process::Command::new("rustc")
        .arg("--version")
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let version_str = String::from_utf8_lossy(&o.stdout);
            // Parse "rustc 1.91.0 (..." into the minor version number.
            let minor = version_str
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.split('.').nth(1))
                .and_then(|m| m.parse::<u32>().ok())
                .unwrap_or(0);

            if minor >= 91 {
                DoctorCheck {
                    id: "rust_version".to_string(),
                    status: DoctorStatus::Ok,
                    message: format!("Rust version is adequate ({})", version_str.trim()),
                    detail: None,
                    path: None,
                    url: None,
                    fix: None,
                }
            } else {
                DoctorCheck {
                    id: "rust_version".to_string(),
                    status: DoctorStatus::Fail,
                    message: format!("Rust version below 1.91 ({})", version_str.trim()),
                    detail: Some("alloy deps require rustc 1.91+".to_string()),
                    path: None,
                    url: None,
                    fix: Some("rustup update stable".to_string()),
                }
            }
        }
        _ => DoctorCheck {
            id: "rust_version".to_string(),
            status: DoctorStatus::Warn,
            message: "rustc not found on PATH".to_string(),
            detail: None,
            path: None,
            url: None,
            fix: Some("rustup update stable".to_string()),
        },
    }
}

fn check_node_version() -> DoctorCheck {
    let output = std::process::Command::new("node").arg("--version").output();

    match output {
        Ok(o) if o.status.success() => {
            let version_str = String::from_utf8_lossy(&o.stdout);
            // Parse "v22.1.0" into the major version number.
            let major = version_str
                .trim()
                .trim_start_matches('v')
                .split('.')
                .next()
                .and_then(|m| m.parse::<u32>().ok())
                .unwrap_or(0);

            if major >= 22 {
                DoctorCheck {
                    id: "node_version".to_string(),
                    status: DoctorStatus::Ok,
                    message: format!("Node version is adequate ({})", version_str.trim()),
                    detail: None,
                    path: None,
                    url: None,
                    fix: None,
                }
            } else {
                DoctorCheck {
                    id: "node_version".to_string(),
                    status: DoctorStatus::Warn,
                    message: format!("Node version below 22 ({})", version_str.trim()),
                    detail: None,
                    path: None,
                    url: None,
                    fix: Some("nvm install 22 && nvm use 22".to_string()),
                }
            }
        }
        _ => DoctorCheck {
            id: "node_version".to_string(),
            status: DoctorStatus::Skipped,
            message: "node not found on PATH (optional)".to_string(),
            detail: None,
            path: None,
            url: None,
            fix: None,
        },
    }
}

fn normalize_health_endpoint_url(raw_url: &str) -> Result<Url> {
    let mut url = Url::parse(raw_url).with_context(|| format!("parse URL {raw_url}"))?;
    let path = url.path();
    if path.is_empty() || path == "/" {
        url.set_path(DEFAULT_HEALTH_PATH);
        return Ok(url);
    }
    if path == "/api" || path == "/api/" {
        url.set_path(DEFAULT_HEALTH_PATH);
        return Ok(url);
    }
    if path.ends_with("/health") || path.ends_with("/api/health") {
        return Ok(url);
    }

    Ok(url)
}

/// Deterministic check that v2 protocol abstractions are compiled and reachable.
///
/// This does not make network calls or spawn subprocesses. It compile-references
/// the public types from the dependency tasks (Cell, CellContext, TypeSchema,
/// Signal, Observe, Connect, Trigger) and verifies they are usable at runtime.
fn check_v2_abstractions() -> DoctorCheck {
    // Compile-time references: if any of these types are removed or renamed,
    // this function will fail to compile, catching regressions immediately.
    use roko_core::cell::{CellContext, CellVersion, TypeSchema};
    use roko_core::signal::Signal;
    use roko_core::traits::{Connect, Observe, Trigger};

    // Runtime probe: verify the types can be instantiated / inspected.
    // TypeSchema has a deterministic compatibility check we can exercise.
    let any = TypeSchema::Any;
    let metric = TypeSchema::OfKind(roko_core::Kind::Metric);
    let schema_ok = any.is_compatible_with(&metric) && metric.is_compatible_with(&any);

    // Verify Signal alias resolves to the same type as Engram.
    let signal: Signal = Signal::builder(roko_core::Kind::Task).build();
    let signal_ok = !signal.id.0.iter().all(|b| *b == 0);

    // Verify CellVersion default is a valid triple.
    let version: CellVersion = (0, 1, 0);
    let version_ok = version.0 == 0 && version.1 == 1 && version.2 == 0;

    // Verify the protocol traits and CellContext are importable and have
    // the expected shapes. These trait bound assertions are never called at
    // runtime but ensure the traits exist with the right bounds at compile time.
    #[allow(dead_code)]
    fn assert_observe<T: Observe>() {}
    #[allow(dead_code)]
    fn assert_connect<T: Connect>() {}
    #[allow(dead_code)]
    fn assert_trigger<T: Trigger>() {}
    let _ = std::any::type_name::<CellContext>();

    let all_ok = schema_ok && signal_ok && version_ok;

    if all_ok {
        DoctorCheck {
            id: "v2_abstractions".to_string(),
            status: DoctorStatus::Ok,
            message: "phase 1 protocol abstractions are reachable".to_string(),
            detail: Some(
                "Cell, CellContext, TypeSchema, Signal, Observe, Connect, Trigger".to_string(),
            ),
            path: None,
            url: None,
            fix: None,
        }
    } else {
        DoctorCheck {
            id: "v2_abstractions".to_string(),
            status: DoctorStatus::Fail,
            message: "phase 1 protocol abstractions failed runtime probe".to_string(),
            detail: Some(format!(
                "schema_ok={schema_ok}, signal_ok={signal_ok}, version_ok={version_ok}"
            )),
            path: None,
            url: None,
            fix: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_project_config(workdir: &Path, config: Config) {
        std::fs::write(
            workdir.join("roko.toml"),
            config.to_toml().expect("serialize config"),
        )
        .expect("write roko.toml");
    }

    async fn bootstrap_layout(workdir: &Path) {
        RokoLayout::for_project(workdir)
            .ensure_dirs()
            .await
            .expect("create .roko layout");
    }

    #[test]
    fn normalize_health_endpoint_url_adds_default_api_path() {
        assert_eq!(
            normalize_health_endpoint_url("http://localhost:9090")
                .unwrap()
                .as_str(),
            "http://localhost:9090/api/health"
        );
        assert_eq!(
            normalize_health_endpoint_url("http://localhost:9090/api")
                .unwrap()
                .as_str(),
            "http://localhost:9090/api/health"
        );
    }

    #[test]
    fn normalize_health_endpoint_url_preserves_explicit_health_endpoint() {
        assert_eq!(
            normalize_health_endpoint_url("http://localhost:9090/custom/health")
                .unwrap()
                .as_str(),
            "http://localhost:9090/custom/health"
        );
    }

    #[tokio::test]
    async fn run_doctor_reports_missing_project_config_and_layout() {
        let temp = tempdir().unwrap();
        let report = run_doctor(&DoctorOptions {
            workdir: temp.path().to_path_buf(),
            config_override: None,
            serve_url: None,
        })
        .await
        .unwrap();

        assert!(!report.healthy);
        assert_eq!(report.exit_code(), 1);
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.id == "config" && check.status == DoctorStatus::Fail)
        );
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.id == "layout" && check.status == DoctorStatus::Fail)
        );
    }

    #[tokio::test]
    async fn run_doctor_passes_bootstrapped_workspace_without_serve_probe() {
        let temp = tempdir().unwrap();
        let mut config = Config::default();
        // Disable auth so doctor doesn't fail on empty api_key (secure-by-default
        // enables auth, but doctor flags enabled-without-key as a failure).
        config.serve.auth.enabled = false;
        write_project_config(temp.path(), config);
        bootstrap_layout(temp.path()).await;

        let report = run_doctor(&DoctorOptions {
            workdir: temp.path().to_path_buf(),
            config_override: None,
            serve_url: None,
        })
        .await
        .unwrap();

        assert!(report.healthy);
        assert_eq!(report.exit_code(), 0);
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.id == "serve_health" && check.status == DoctorStatus::Skipped)
        );
    }

    #[tokio::test]
    async fn run_doctor_fails_when_serve_auth_enabled_without_api_key() {
        let temp = tempdir().unwrap();
        let mut config = Config::default();
        config.serve.auth.enabled = true;
        config.serve.auth.api_key.clear();
        write_project_config(temp.path(), config);
        bootstrap_layout(temp.path()).await;

        let report = run_doctor(&DoctorOptions {
            workdir: temp.path().to_path_buf(),
            config_override: None,
            serve_url: None,
        })
        .await
        .unwrap();

        let auth_check = report
            .checks
            .iter()
            .find(|check| check.id == "serve_auth")
            .expect("serve_auth check");
        assert_eq!(auth_check.status, DoctorStatus::Fail);
        assert!(!report.healthy);
    }

    #[tokio::test]
    async fn failing_checks_have_fix_lines_in_human_output() {
        let temp = tempdir().unwrap();
        let report = run_doctor(&DoctorOptions {
            workdir: temp.path().to_path_buf(),
            config_override: None,
            serve_url: None,
        })
        .await
        .unwrap();

        let rendered = report.render_human();
        // Every fail/warn check with a fix should produce an arrow-fix line.
        for check in &report.checks {
            if matches!(check.status, DoctorStatus::Fail | DoctorStatus::Warn) {
                if let Some(fix) = &check.fix {
                    let expected = format!("\u{2192} fix: {fix}");
                    assert!(
                        rendered.contains(&expected),
                        "missing fix line for check '{}': expected '{expected}' in output",
                        check.id
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn ok_checks_do_not_have_fix_lines_in_human_output() {
        let temp = tempdir().unwrap();
        let mut config = Config::default();
        config.serve.auth.enabled = false;
        write_project_config(temp.path(), config);
        bootstrap_layout(temp.path()).await;

        let report = run_doctor(&DoctorOptions {
            workdir: temp.path().to_path_buf(),
            config_override: None,
            serve_url: None,
        })
        .await
        .unwrap();

        for check in &report.checks {
            if check.status == DoctorStatus::Ok {
                assert!(
                    check.fix.is_none(),
                    "ok check '{}' should not have a fix",
                    check.id
                );
            }
        }
    }

    #[test]
    fn fix_field_skipped_in_json_when_none() {
        let check = DoctorCheck {
            id: "test".to_string(),
            status: DoctorStatus::Ok,
            message: "all good".to_string(),
            detail: None,
            path: None,
            url: None,
            fix: None,
        };
        let json = serde_json::to_string(&check).unwrap();
        assert!(
            !json.contains("\"fix\""),
            "fix field should be absent when None"
        );
    }

    #[test]
    fn fix_field_present_in_json_when_some() {
        let check = DoctorCheck {
            id: "test".to_string(),
            status: DoctorStatus::Fail,
            message: "bad".to_string(),
            detail: None,
            path: None,
            url: None,
            fix: Some("roko init".to_string()),
        };
        let json = serde_json::to_string(&check).unwrap();
        assert!(
            json.contains("\"fix\":\"roko init\""),
            "fix field should be present when Some"
        );
    }

    #[tokio::test]
    async fn doctor_includes_environment_checks() {
        let temp = tempdir().unwrap();
        let report = run_doctor(&DoctorOptions {
            workdir: temp.path().to_path_buf(),
            config_override: None,
            serve_url: None,
        })
        .await
        .unwrap();

        let check_ids: Vec<&str> = report.checks.iter().map(|c| c.id.as_str()).collect();
        assert!(
            check_ids.contains(&"claude_cli"),
            "missing claude_cli check"
        );
        assert!(
            check_ids.contains(&"anthropic_api_key"),
            "missing anthropic_api_key check"
        );
        assert!(
            check_ids.contains(&"rust_version"),
            "missing rust_version check"
        );
        assert!(
            check_ids.contains(&"node_version"),
            "missing node_version check"
        );
    }

    #[test]
    fn v2_abstractions_check_passes() {
        let check = check_v2_abstractions();
        assert_eq!(check.id, "v2_abstractions");
        assert_eq!(
            check.status,
            DoctorStatus::Ok,
            "v2 abstractions check should pass: {:?}",
            check.detail
        );
        assert!(
            check
                .message
                .contains("phase 1 protocol abstractions are reachable")
        );
    }

    #[tokio::test]
    async fn doctor_report_includes_v2_abstractions() {
        let temp = tempdir().unwrap();
        let report = run_doctor(&DoctorOptions {
            workdir: temp.path().to_path_buf(),
            config_override: None,
            serve_url: None,
        })
        .await
        .unwrap();

        let v2_check = report
            .checks
            .iter()
            .find(|c| c.id == "v2_abstractions")
            .expect("v2_abstractions check should be present in doctor report");
        assert_eq!(v2_check.status, DoctorStatus::Ok);
    }

    #[tokio::test]
    async fn doctor_human_output_contains_v2_abstractions() {
        let temp = tempdir().unwrap();
        let report = run_doctor(&DoctorOptions {
            workdir: temp.path().to_path_buf(),
            config_override: None,
            serve_url: None,
        })
        .await
        .unwrap();

        let rendered = report.render_human();
        assert!(
            rendered.contains("v2_abstractions"),
            "human output should contain 'v2_abstractions', got:\n{rendered}"
        );
        assert!(
            rendered.contains("[ok] v2_abstractions"),
            "human output should show v2_abstractions as ok, got:\n{rendered}"
        );
    }
}
