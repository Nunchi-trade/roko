//! server command handlers.
#![allow(unused_imports)]

use crate::*;

pub(crate) async fn cmd_up(cli: &Cli, workdir: PathBuf) -> Result<i32> {
    use agent_serve::{run_agent_create, run_agent_start, run_agent_stop};

    prepare_runtime_hooks(&workdir, cli.quiet);

    // Ensure .roko/ exists (like `roko init` would).
    let _ = bootstrap_observability_dirs(&workdir);

    // Load RokoConfig to get [[agents]] definitions.
    let roko_toml_path = workdir.join("roko.toml");
    let roko_config: RokoConfig = if roko_toml_path.exists() {
        let text = std::fs::read_to_string(&roko_toml_path)
            .with_context(|| format!("read {}", roko_toml_path.display()))?;
        toml::from_str(&text).with_context(|| format!("parse {}", roko_toml_path.display()))?
    } else {
        RokoConfig::default()
    };

    let agents = roko_config.agents.clone();
    let port = roko_config.server.port;
    let bind = roko_config.server.bind.clone();

    // Start serve in background.
    let config = resolve_config_for_workdir(cli, &workdir)?;
    let repo_registry = RepoRegistry::load(&config, &workdir).unwrap_or_default();
    let runtime = RokoCliRuntime::new(config, repo_registry).into_arc();
    let serve_wd = workdir.clone();
    let serve_handle = tokio::spawn(async move {
        if let Err(e) = roko_serve::run_server(serve_wd, runtime, None, None).await {
            eprintln!("roko-serve error: {e}");
        }
    });

    // Brief wait for serve to start listening.
    tokio::time::sleep(Duration::from_millis(500)).await;
    println!("  roko-serve     http://{}:{}  \u{2713}", bind, port);

    // Create and start each configured agent.
    let mut started_agents: Vec<String> = Vec::new();
    for agent_def in &agents {
        if !agent_def.enabled {
            continue;
        }
        // Create manifest if it doesn't already exist.
        let manifest_path = workdir
            .join(".roko")
            .join("agents")
            .join(&agent_def.name)
            .join("manifest.toml");
        if !manifest_path.exists() {
            let prompt = if agent_def.prompt.is_empty() {
                None
            } else {
                Some(agent_def.prompt.as_str())
            };
            if let Err(e) = run_agent_create(
                &agent_def.name,
                &agent_def.domain,
                None,
                prompt,
                Some(&workdir),
            )
            .await
            {
                eprintln!(
                    "  {:<14}  {}     {:10}  \u{2717} ({})",
                    agent_def.name, agent_def.domain, ":auto", e
                );
                continue;
            }
        }

        // Start the agent.
        match run_agent_start(&agent_def.name, "127.0.0.1:0", Some(&workdir)) {
            Ok(()) => {
                println!(
                    "  {:<14}  {:<10} {:10}  \u{2713}",
                    agent_def.name, agent_def.domain, ":auto"
                );
                started_agents.push(agent_def.name.clone());
            }
            Err(e) => {
                eprintln!(
                    "  {:<14}  {:<10} {:10}  \u{2717} ({})",
                    agent_def.name, agent_def.domain, ":auto", e
                );
            }
        }
    }

    println!();
    if started_agents.is_empty() && agents.is_empty() {
        println!("  No [[agents]] configured in roko.toml.");
        println!("  Serve is running. Add [[agents]] blocks to start agents.");
    } else {
        println!(
            "  {} agent(s) registered. Dashboard: roko dashboard",
            started_agents.len()
        );
    }
    println!("  Press Ctrl+C to stop all.");
    println!();

    // Wait for Ctrl+C.
    tokio::signal::ctrl_c().await.context("listen for ctrl+c")?;

    println!("\nShutting down...");

    // Stop all started agents.
    for name in &started_agents {
        if let Err(e) = run_agent_stop(name, false, Some(&workdir)) {
            eprintln!("warning: failed to stop agent '{}': {}", name, e);
        }
    }

    // Abort the serve task.
    serve_handle.abort();
    let _ = serve_handle.await;

    println!("All services stopped.");
    Ok(EXIT_SUCCESS)
}

pub(crate) async fn cmd_daemon(cli: &Cli, cmd: DaemonCmd) -> Result<i32> {
    let workdir = resolve_workdir(cli);
    match cmd {
        DaemonCmd::Start { foreground, port } => {
            prepare_runtime_hooks(&workdir, cli.quiet);
            roko_cli::daemon::daemon_start(&workdir, foreground, port).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Stop => {
            roko_cli::daemon::daemon_stop(&workdir).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Status => {
            roko_cli::daemon::daemon_status(&workdir).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Logs { follow, lines } => {
            roko_cli::daemon::daemon_logs(&workdir, follow, lines).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Reload => {
            roko_cli::daemon::daemon_reload(&workdir).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Restart { port } => {
            prepare_runtime_hooks(&workdir, cli.quiet);
            roko_cli::daemon::daemon_restart(&workdir, port).await?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Install => {
            roko_cli::daemon::daemon_install()?;
            Ok(EXIT_SUCCESS)
        }
        DaemonCmd::Uninstall => {
            roko_cli::daemon::daemon_uninstall()?;
            Ok(EXIT_SUCCESS)
        }
    }
}

/// Print a security checklist and fail unless auth is enabled or `--unsafe-public` is set.
///
/// Does not block for `localhost`/`127.0.0.1` targets.
fn check_security_posture(
    config: &roko_core::config::schema::RokoConfig,
    unsafe_public: bool,
) -> Result<()> {
    let auth = &config.serve.auth;
    let auth_ok = auth.enabled && (!auth.api_key.is_empty() || !auth.api_keys.is_empty());
    let cors_configured = !config.server.cors_origins.is_empty();
    // Terminal is disabled when not explicitly set; treat absence as disabled.
    let terminal_disabled = true; // terminal feature is not on by default

    println!("  Security checklist:");
    println!(
        "  [{}] serve.auth.enabled = {}",
        if auth_ok { "x" } else { " " },
        auth.enabled
    );
    println!(
        "  [{}] serve.auth.api_key set",
        if !auth.api_key.is_empty() || !auth.api_keys.is_empty() {
            "x"
        } else {
            " "
        }
    );
    println!(
        "  [{}] server.cors_origins configured",
        if cors_configured { "x" } else { " " }
    );
    println!(
        "  [{}] terminal disabled (not exposed)",
        if terminal_disabled { "x" } else { " " }
    );
    println!();

    if !auth_ok {
        if unsafe_public {
            eprintln!(
                "WARNING: proceeding without auth (--unsafe-public). \
                 Your server will be accessible to the internet without authentication."
            );
        } else {
            anyhow::bail!(
                "deployment blocked: serve.auth is not configured.\n\
                 Add to roko.toml:\n\
                 \n  [serve.auth]\n  enabled = true\n  api_key = \"<secret>\"\n\n\
                 Or bypass with: roko deploy railway --unsafe-public"
            );
        }
    }

    Ok(())
}

pub(crate) async fn cmd_deploy(cli: &Cli, cmd: DeployCmd) -> Result<i32> {
    match cmd {
        DeployCmd::Railway {
            workdir,
            with_mirage,
            workers,
            unsafe_public,
        } => cmd_deploy_railway(cli, workdir, with_mirage, workers, unsafe_public).await,
        DeployCmd::Fly {
            workdir,
            unsafe_public,
        } => cmd_deploy_fly(cli, workdir, unsafe_public).await,
        DeployCmd::Docker {
            workdir,
            registry,
            unsafe_public,
        } => cmd_deploy_docker(cli, workdir, registry, unsafe_public).await,
    }
}

pub(crate) async fn cmd_deploy_railway(
    cli: &Cli,
    workdir: Option<PathBuf>,
    with_mirage: bool,
    workers: Vec<String>,
    unsafe_public: bool,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    run_release_build(&workdir).await?;

    let config = load_roko_config(&workdir)?;
    println!("Checking security posture...");
    check_security_posture(&config, unsafe_public)?;
    let deploy_webhooks = match load_layered(&workdir) {
        Ok(resolved) => resolved.config.serve.deploy.webhooks,
        Err(err) => {
            warn!(
                error = %err,
                "failed to load configured deploy webhooks; skipping GitHub webhook registration"
            );
            Vec::new()
        }
    };
    let deploy_config = &config.deploy;
    let token = deploy_config
        .railway_api_token
        .as_deref()
        .ok_or_else(|| anyhow!("deploy.railway_api_token is required for Railway deployment"))?;
    let repo_slug = git_remote_slug(&workdir)?;
    let branch =
        git_current_branch(&workdir).unwrap_or_else(|_| config.project.fresh_base_branch.clone());
    let env_vars = collect_railway_env_vars();

    // Load persisted project context as fallback for config-level IDs.
    let saved_ctx = load_railway_context(&workdir);
    let project_id = deploy_config
        .project_id
        .clone()
        .or_else(|| saved_ctx.as_ref().map(|c| c.project_id.clone()));
    let environment_id = deploy_config
        .environment_id
        .clone()
        .or_else(|| saved_ctx.as_ref().map(|c| c.environment_id.clone()));

    let backend = roko_serve::deploy::railway_api::RailwayApiBackend::new(
        token.to_string(),
        project_id.clone(),
        environment_id.clone(),
    );

    // 1. Deploy roko-serve (control plane)
    let (deployment, railway_ctx) = backend
        .deploy_roko_app(&roko_serve::deploy::railway_api::RailwayDeploySpec {
            project_name: config.project.name.clone(),
            project_id: project_id.clone(),
            environment_id: environment_id.clone(),
            service_name: "roko".to_string(),
            repo_slug: repo_slug.clone(),
            branch: branch.clone(),
            dockerfile_path: "docker/roko.Dockerfile".to_string(),
            root_directory: ".".to_string(),
            healthcheck_path: "/api/health".to_string(),
            volume_mount_path: "/workspace/.roko".to_string(),
            region: deploy_config.default_region.clone(),
            env_vars: env_vars.clone(),
        })
        .await?;

    // Persist project context for subsequent deploys.
    save_railway_context(&workdir, &railway_ctx)?;

    let control_url = deployment
        .url
        .as_deref()
        .ok_or_else(|| anyhow!("Railway deployment finished without a public URL"))?;

    println!("roko-serve: {control_url}");

    // 2. Deploy mirage if requested
    if with_mirage {
        let mut mirage_env = env_vars.clone();
        mirage_env.insert(
            "MIRAGE_STATE_DIR".to_string(),
            "/workspace/.roko/state".to_string(),
        );

        let (_mirage_dep, _) = backend
            .deploy_roko_app(&roko_serve::deploy::railway_api::RailwayDeploySpec {
                project_name: config.project.name.clone(),
                project_id: Some(railway_ctx.project_id.clone()),
                environment_id: Some(railway_ctx.environment_id.clone()),
                service_name: "mirage".to_string(),
                repo_slug: repo_slug.clone(),
                branch: branch.clone(),
                dockerfile_path: "docker/mirage.Dockerfile".to_string(),
                root_directory: ".".to_string(),
                healthcheck_path: "/relay/health".to_string(),
                volume_mount_path: "/workspace/.roko".to_string(),
                region: deploy_config.default_region.clone(),
                env_vars: mirage_env,
            })
            .await?;

        if let Some(url) = _mirage_dep.url.as_deref() {
            println!("mirage: {url}");
        }
    }

    // 3. Deploy worker services for each requested template
    for template_name in &workers {
        // Load the template to base64-encode as ROKO_TEMPLATE_JSON
        let template_json = load_template_for_deploy(&workdir, template_name)?;
        let template_b64 =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &template_json);

        let mut worker_env = env_vars.clone();
        worker_env.insert("ROKO_TEMPLATE_JSON".to_string(), template_b64);
        worker_env.insert(
            "ROKO_CONTROL_PLANE_URL".to_string(),
            control_url.to_string(),
        );

        let service_name = format!("roko-worker-{template_name}");
        let (_worker_dep, _) = backend
            .deploy_roko_app(&roko_serve::deploy::railway_api::RailwayDeploySpec {
                project_name: config.project.name.clone(),
                project_id: Some(railway_ctx.project_id.clone()),
                environment_id: Some(railway_ctx.environment_id.clone()),
                service_name: service_name.clone(),
                repo_slug: repo_slug.clone(),
                branch: branch.clone(),
                dockerfile_path: "docker/worker.Dockerfile".to_string(),
                root_directory: ".".to_string(),
                healthcheck_path: "/health".to_string(),
                volume_mount_path: "/home/roko/.roko".to_string(),
                region: deploy_config.default_region.clone(),
                env_vars: worker_env,
            })
            .await?;

        if let Some(url) = _worker_dep.url.as_deref() {
            println!("{service_name}: {url}");
        }
    }

    register_deployment_github_webhooks(
        &deploy_webhooks,
        control_url,
        &config.webhooks.github.secret,
    )
    .await?;

    Ok(EXIT_SUCCESS)
}

pub(crate) async fn cmd_deploy_fly(
    cli: &Cli,
    workdir: Option<PathBuf>,
    unsafe_public: bool,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let config = load_roko_config(&workdir)?;
    println!("Checking security posture...");
    check_security_posture(&config, unsafe_public)?;

    write_fly_toml(&workdir)?;
    run_command_status(&workdir, "flyctl", &["deploy", "--remote-only"])?;

    Ok(EXIT_SUCCESS)
}

pub(crate) async fn cmd_deploy_docker(
    cli: &Cli,
    workdir: Option<PathBuf>,
    registry: Option<String>,
    unsafe_public: bool,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let config = load_roko_config(&workdir)?;
    println!("Checking security posture...");
    check_security_posture(&config, unsafe_public)?;
    let registry = resolve_docker_registry(&config, registry)?;
    let tagged_image = format!("{registry}/roko:latest");

    run_command_status(&workdir, "docker", &["build", "-t", "roko", "."])?;
    run_command_status(&workdir, "docker", &["tag", "roko:latest", &tagged_image])?;

    Ok(EXIT_SUCCESS)
}

/// Load persisted Railway project context from `.roko/state/railway.json`.
pub(crate) fn load_railway_context(
    workdir: &Path,
) -> Option<roko_serve::deploy::railway_api::RailwayProjectContext> {
    let path = workdir.join(".roko/state/railway.json");
    let text = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Persist Railway project context to `.roko/state/railway.json` so that
/// subsequent deploys (including worker deploys) reuse the same project.
pub(crate) fn save_railway_context(
    workdir: &Path,
    ctx: &roko_serve::deploy::railway_api::RailwayProjectContext,
) -> Result<()> {
    let dir = workdir.join(".roko/state");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join("railway.json");
    let json = serde_json::to_string_pretty(ctx).context("serialize railway context")?;
    std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    info!(path = %path.display(), "persisted Railway project context");
    Ok(())
}

/// Load an agent template by name and serialize it to JSON bytes for worker env.
pub(crate) fn load_template_for_deploy(workdir: &Path, name: &str) -> Result<Vec<u8>> {
    use roko_serve::templates::TemplateRegistry;

    // Try loading from the registry (disk + builtins)
    let mut registry = TemplateRegistry::new(workdir.to_path_buf());
    registry.scan_with_builtins();

    let template = registry
        .get(name)
        .ok_or_else(|| anyhow!("template '{name}' not found in registry"))?
        .clone();

    serde_json::to_vec(&template).context("serialize template for worker env")
}

pub(crate) fn write_fly_toml(workdir: &Path) -> Result<PathBuf> {
    let path = workdir.join("fly.toml");
    std::fs::write(&path, FLY_TOML_TEMPLATE)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

pub(crate) fn resolve_docker_registry(
    config: &roko_core::config::schema::RokoConfig,
    registry: Option<String>,
) -> Result<String> {
    if let Some(registry) = registry {
        let registry = registry.trim().trim_end_matches('/');
        if registry.is_empty() {
            bail!("deploy.docker.registry cannot be empty");
        }
        return Ok(registry.to_string());
    }

    let worker_image =
        config.deploy.worker_image.as_deref().ok_or_else(|| {
            anyhow!("deploy.docker.registry is required or set deploy.worker_image")
        })?;

    let registry = worker_image
        .rsplit_once('/')
        .map(|(registry, _)| registry)
        .filter(|registry| !registry.trim().is_empty())
        .ok_or_else(|| {
            anyhow!("unable to derive Docker registry from deploy.worker_image: {worker_image}")
        })?;

    Ok(registry.trim().trim_end_matches('/').to_string())
}

pub(crate) async fn run_release_build(workdir: &Path) -> Result<()> {
    let workdir = workdir.to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("cargo")
            .args(["build", "--release", "-p", "roko-cli"])
            .current_dir(&workdir)
            .output()
    })
    .await
    .context("join cargo build task")?
    .context("run cargo build")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "cargo build --release -p roko-cli failed: {}",
            stderr.trim()
        );
    }

    Ok(())
}

pub(crate) async fn register_deployment_github_webhooks(
    webhooks: &[ServeDeployWebhookConfig],
    webhook_url: &str,
    secret: &str,
) -> Result<()> {
    if webhooks.is_empty() {
        return Ok(());
    }

    if secret.trim().is_empty() {
        warn!("github webhook secret is not configured; skipping webhook registration");
        return Ok(());
    }

    let token = match env::var("GITHUB_TOKEN").or_else(|_| env::var("GH_TOKEN")) {
        Ok(token) if !token.trim().is_empty() => token,
        _ => {
            warn!("github token is not configured; skipping webhook registration");
            return Ok(());
        }
    };

    let github = Octocrab::builder()
        .personal_token(token)
        .build()
        .context("build GitHub client")?;

    let mut registered = 0usize;
    for webhook in webhooks {
        if webhook.provider != "github" {
            warn!(
                provider = %webhook.provider,
                owner = %webhook.owner,
                repo = %webhook.repo,
                "skipping non-GitHub deploy webhook registration"
            );
            continue;
        }

        if webhook.owner.trim().is_empty() || webhook.repo.trim().is_empty() {
            warn!(
                provider = %webhook.provider,
                owner = %webhook.owner,
                repo = %webhook.repo,
                "skipping deploy webhook with empty repository coordinates"
            );
            continue;
        }

        match register_github_webhook(
            &github,
            webhook.owner.trim(),
            webhook.repo.trim(),
            webhook_url,
            secret,
        )
        .await
        {
            Ok(()) => {
                registered += 1;
            }
            Err(err) => {
                warn!(
                    owner = %webhook.owner,
                    repo = %webhook.repo,
                    error = %err,
                    "failed to register GitHub webhook"
                );
            }
        }
    }

    if registered > 0 {
        info!(count = registered, "registered GitHub webhooks");
    }

    Ok(())
}

pub(crate) async fn register_github_webhook(
    github: &octocrab::Octocrab,
    owner: &str,
    repo: &str,
    webhook_url: &str,
    secret: &str,
) -> Result<()> {
    let webhook_endpoint = format!("{}/webhooks/github", webhook_url.trim_end_matches('/'));

    let existing_hooks: Vec<Hook> = github
        .get(format!("/repos/{owner}/{repo}/hooks"), None::<&()>)
        .await
        .with_context(|| format!("list GitHub webhooks for {owner}/{repo}"))?;

    if existing_hooks
        .iter()
        .any(|hook| hook.name == "web" && hook.config.url == webhook_endpoint)
    {
        info!(owner = %owner, repo = %repo, "GitHub webhook already registered");
        return Ok(());
    }

    let hook = Hook {
        name: "web".to_string(),
        active: true,
        events: vec![
            WebhookEventType::Push,
            WebhookEventType::PullRequest,
            WebhookEventType::Issues,
            WebhookEventType::IssueComment,
            WebhookEventType::PullRequestReview,
            WebhookEventType::CheckRun,
        ],
        config: HookConfig {
            url: webhook_endpoint,
            content_type: Some(ContentType::Json),
            insecure_ssl: None,
            secret: Some(secret.to_string()),
        },
        ..Hook::default()
    };

    github
        .repos(owner, repo)
        .create_hook(hook)
        .await
        .with_context(|| format!("create GitHub webhook for {owner}/{repo}"))?;

    Ok(())
}

pub(crate) fn git_remote_slug(workdir: &Path) -> Result<String> {
    let remote = run_command_output(workdir, "git", &["remote", "get-url", "origin"])?;
    let remote = remote.trim();
    let slug = remote
        .strip_prefix("git@github.com:")
        .or_else(|| remote.strip_prefix("https://github.com/"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))
        .ok_or_else(|| anyhow!("origin remote is not a GitHub URL: {remote}"))?
        .trim_end_matches(".git")
        .to_string();

    if slug.split('/').count() != 2 {
        return Err(anyhow!(
            "invalid GitHub repo slug derived from origin: {slug}"
        ));
    }

    Ok(slug)
}

pub(crate) fn git_current_branch(workdir: &Path) -> Result<String> {
    let branch = run_command_output(workdir, "git", &["branch", "--show-current"])?;
    let branch = branch.trim();
    if branch.is_empty() {
        bail!("unable to determine current git branch");
    }
    Ok(branch.to_string())
}

pub(crate) fn collect_railway_env_vars() -> std::collections::HashMap<String, String> {
    const NAMES: &[&str] = &[
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "SLACK_TOKEN",
        "SLACK_BOT_TOKEN",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "ROKO_SERVER_AUTH_TOKEN",
    ];

    let mut vars = std::collections::HashMap::new();
    for name in NAMES {
        if let Ok(value) = env::var(name) {
            if !value.trim().is_empty() {
                vars.insert((*name).to_string(), value);
            }
        }
    }
    vars
}

pub(crate) fn run_command_status(workdir: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(program)
        .args(args)
        .current_dir(workdir)
        .status()
        .with_context(|| format!("run {program} {}", args.join(" ")))?;

    if !status.success() {
        bail!("{program} {} failed with status {status}", args.join(" "));
    }

    Ok(())
}

pub(crate) fn run_command_output(workdir: &Path, program: &str, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(workdir)
        .output()
        .with_context(|| format!("run {program} {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{program} {} failed: {}", args.join(" "), stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) const FLY_TOML_TEMPLATE: &str = r#"app = "roko-agent"
primary_region = "iad"

[build]
dockerfile = "Dockerfile"

[http_service]
internal_port = 6677
force_https = true
auto_stop_machines = true
auto_start_machines = true
min_machines_running = 0

[[http_service.checks]]
interval = "30s"
timeout = "5s"
grace_period = "10s"
path = "/api/health"
method = "GET"

[mounts]
source = "roko_data"
destination = "/data/.roko"
"#;
