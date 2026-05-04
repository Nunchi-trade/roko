//! `roko doctor` bootstrap diagnostics for self-hosted workspaces.

use crate::config::{ConfigLayer, ConfigPaths, resolve_paths};
use crate::{Config, load_layered};
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
    checks.push(check_serve_auth(&loaded_config));
    checks.push(check_serve_health(options.serve_url.as_deref(), &loaded_config).await?);

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
        Some(load_layered(workdir)?.config)
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
            }
        } else {
            DoctorCheck {
                id: "config".to_string(),
                status: DoctorStatus::Fail,
                message: "explicit config override is missing".to_string(),
                detail: None,
                path: Some(path.display().to_string()),
                url: None,
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
            }
        } else {
            DoctorCheck {
                id: "config".to_string(),
                status: DoctorStatus::Fail,
                message: "ROKO_CONFIG points to a missing file".to_string(),
                detail: None,
                path: Some(path.display().to_string()),
                url: None,
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
        }
    } else {
        DoctorCheck {
            id: "layout".to_string(),
            status: DoctorStatus::Fail,
            message: "required .roko layout paths are missing".to_string(),
            detail: Some(missing.join(", ")),
            path: Some(root),
            url: None,
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
        },
        Ok(response) => DoctorCheck {
            id: "serve_health".to_string(),
            status: DoctorStatus::Fail,
            message: format!("health endpoint returned {}", response.status()),
            detail: None,
            path: None,
            url: Some(endpoint.to_string()),
        },
        Err(err) if err.is_builder() => DoctorCheck {
            id: "serve_health".to_string(),
            status: DoctorStatus::Fail,
            message: "invalid serve URL".to_string(),
            detail: Some(err.to_string()),
            path: None,
            url: Some(endpoint.to_string()),
        },
        Err(err) => DoctorCheck {
            id: "serve_health".to_string(),
            status: DoctorStatus::Fail,
            message: "health endpoint is unreachable".to_string(),
            detail: Some(err.to_string()),
            path: None,
            url: Some(endpoint.to_string()),
        },
    };
    Ok(check)
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
}
