//! job command handlers.
#![allow(unused_imports)]

use crate::*;

pub(crate) async fn cmd_job(cli: &Cli, cmd: JobCmd) -> Result<i32> {
    let jobs_dir = |wd: &Path| wd.join(".roko").join("jobs");

    match cmd {
        JobCmd::List { workdir, status } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let dir = jobs_dir(&wd);
            if !dir.is_dir() {
                println!(
                    "No jobs found (directory does not exist: {})",
                    dir.display()
                );
                return Ok(EXIT_SUCCESS);
            }
            let mut entries: Vec<_> = std::fs::read_dir(&dir)?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext == "json")
                })
                .collect();
            entries.sort_by_key(|e| e.file_name());

            if cli.json {
                let mut jobs: Vec<roko_core::MarketplaceJob> = Vec::new();
                for entry in &entries {
                    let data = std::fs::read_to_string(entry.path())?;
                    if let Ok(job) = serde_json::from_str::<roko_core::MarketplaceJob>(&data) {
                        jobs.push(job);
                    }
                }
                println!("{}", serde_json::to_string_pretty(&jobs)?);
                return Ok(EXIT_SUCCESS);
            }

            let mut count = 0usize;
            for entry in &entries {
                let data = std::fs::read_to_string(entry.path())?;
                let job: roko_core::MarketplaceJob =
                    serde_json::from_str(&data).unwrap_or_default();
                let effective_status = job.effective_status();
                if let Some(ref filter) = status {
                    if !effective_status.eq_ignore_ascii_case(filter) {
                        continue;
                    }
                }
                let icon = match effective_status {
                    "open" | "pending" => "\u{25cb}",
                    "assigned" => "\u{25d4}",
                    "in_progress" | "active" | "running" => "\u{25b6}",
                    "submitted" => "\u{25d1}",
                    "completed" | "done" => "\u{2713}",
                    "failed" | "cancelled" => "\u{2717}",
                    _ => "\u{00b7}",
                };
                println!(
                    "{icon} [{:>12}] {:>10}  {}  {}",
                    job.job_type,
                    effective_status,
                    &job.id[..job.id.len().min(8)],
                    job.title
                );
                count += 1;
            }
            if count == 0 {
                println!("No jobs found.");
            } else {
                println!("\n{count} job(s)");
            }
            Ok(EXIT_SUCCESS)
        }
        JobCmd::Create {
            title,
            r#type,
            description,
            priority,
            auto_execute,
            plan_id,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let dir = jobs_dir(&wd);
            std::fs::create_dir_all(&dir)?;
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let job = roko_core::MarketplaceJob {
                id: id.clone(),
                title: title.trim().to_string(),
                description: description.trim().to_string(),
                job_type: r#type.trim().to_string(),
                status: "open".to_string(),
                priority: priority.trim().to_string(),
                auto_execute,
                plan_id: plan_id.unwrap_or_default(),
                created_at: now.clone(),
                updated_at: now,
                ..Default::default()
            };
            let path = dir.join(format!("{id}.json"));
            let rendered = serde_json::to_string_pretty(&job)?;
            std::fs::write(&path, &rendered)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&job)?);
            } else {
                println!("Created job: {id}");
                println!("  title:    {}", job.title);
                println!("  type:     {}", job.job_type);
                println!("  priority: {}", job.priority);
                println!("  auto_execute: {}", job.auto_execute);
                println!("  path:     {}", path.display());
            }
            Ok(EXIT_SUCCESS)
        }
        JobCmd::Match {
            title,
            serve_url,
            description,
            language,
            min_tier,
            reward,
            skills,
            workdir,
        } => {
            let default_wd = resolve_workdir(cli);
            let wd = workdir.as_deref().unwrap_or(&default_wd);
            let auth_cfg = load_layered(wd)
                .map(|r| r.config.serve.auth)
                .unwrap_or_default();
            let headers = match auth::resolve_api_key(&auth_cfg, None) {
                Some(resolved) => auth::auth_headers(&resolved.key),
                None => reqwest::header::HeaderMap::new(),
            };
            let body = serde_json::json!({
                "title": title,
                "description": description,
                "language": language,
                "minTier": min_tier,
                "reward": reward,
                "skills": skills,
            });
            let client = reqwest::Client::new();
            let resp = client
                .post(format!(
                    "{}/api/jobs/match",
                    serve_url.trim_end_matches('/')
                ))
                .headers(headers)
                .json(&body)
                .send()
                .await?;
            let status = resp.status();
            let payload: serde_json::Value = resp.json().await.unwrap_or_default();
            if !status.is_success() {
                anyhow::bail!(
                    "failed to match job: {} {}",
                    status,
                    serde_json::to_string_pretty(&payload).unwrap_or_default()
                );
            }

            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).unwrap_or_default()
                );
                return Ok(EXIT_SUCCESS);
            }

            let candidates = payload
                .get("candidates")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            if candidates.is_empty() {
                println!("No matching agents found.");
                return Ok(EXIT_SUCCESS);
            }

            println!(
                "Matched {} candidate(s), total_fee={}, eta_hours={}",
                candidates.len(),
                payload
                    .get("totalFee")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                payload
                    .get("etaHours")
                    .and_then(serde_json::Value::as_f64)
                    .map(|v| format!("{v:.1}"))
                    .unwrap_or_else(|| "?".to_string())
            );
            println!(
                "{:<24} {:<11} {:>5} {:>9} {:>9} {:>14}",
                "agent", "tier", "rep", "inflight", "jobs", "bid"
            );
            for candidate in candidates {
                let agent = candidate
                    .get("agentId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let tier = candidate
                    .get("tier")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let reputation = candidate
                    .get("reputation")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let inflight = candidate
                    .get("inflightJobs")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let max_concurrent = candidate
                    .get("maxConcurrentJobs")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let past_jobs = candidate
                    .get("pastJobs")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let bid = candidate
                    .get("bidShare")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                println!(
                    "{:<24} {:<11} {:>5} {:>4}/{:<4} {:>9} {:>14}",
                    agent, tier, reputation, inflight, max_concurrent, past_jobs, bid
                );
            }
            Ok(EXIT_SUCCESS)
        }
        JobCmd::Show { id, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let path = resolve_job_path(&jobs_dir(&wd), &id)?;
            let data = std::fs::read_to_string(&path)?;
            let job: roko_core::MarketplaceJob = serde_json::from_str(&data)?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&job)?);
                return Ok(EXIT_SUCCESS);
            }

            let effective_status = job.effective_status();
            println!("id:           {}", job.id);
            println!("title:        {}", job.title);
            println!("type:         {}", job.job_type);
            println!("status:       {effective_status}");
            println!("priority:     {}", job.priority);
            println!("posted_by:    {}", job.posted_by);
            println!("assigned_to:  {}", job.assigned_to);
            println!("auto_execute: {}", job.auto_execute);
            println!("plan_id:      {}", job.plan_id);
            println!("created_at:   {}", job.created_at);
            println!("updated_at:   {}", job.updated_at);
            if !job.tags.is_empty() {
                println!("tags:         {}", job.tags.join(", "));
            }
            if !job.description.is_empty() {
                println!("\n--- description ---\n{}", job.description);
            }
            if let Some(ref sub) = job.submission {
                println!(
                    "\n--- submission ---\n{}",
                    serde_json::to_string_pretty(sub).unwrap_or_default()
                );
            }
            if let Some(ref eval) = job.evaluation {
                println!(
                    "\n--- evaluation ---\n{}",
                    serde_json::to_string_pretty(eval).unwrap_or_default()
                );
            }
            Ok(EXIT_SUCCESS)
        }
        JobCmd::Execute {
            id,
            serve_url,
            workdir,
        } => {
            if let Some(url) = serve_url {
                // Delegate to roko-serve
                let default_wd = resolve_workdir(cli);
                let wd = workdir.as_deref().unwrap_or(&default_wd);
                let auth_cfg = load_layered(wd)
                    .map(|r| r.config.serve.auth)
                    .unwrap_or_default();
                let headers = match auth::resolve_api_key(&auth_cfg, None) {
                    Some(resolved) => auth::auth_headers(&resolved.key),
                    None => reqwest::header::HeaderMap::new(),
                };
                let client = reqwest::Client::new();
                let resp = client
                    .post(format!("{url}/api/jobs/{id}/execute"))
                    .headers(headers)
                    .send()
                    .await?;
                let status = resp.status();
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                if status.is_success() {
                    println!("Job '{id}' execution started via serve.");
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&body).unwrap_or_default()
                    );
                } else {
                    anyhow::bail!(
                        "failed to execute job '{id}': {} {}",
                        status,
                        serde_json::to_string_pretty(&body).unwrap_or_default()
                    );
                }
            } else {
                // Local inline execution — load config and use run_once
                let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
                let path = resolve_job_path(&jobs_dir(&wd), &id)?;
                let data = std::fs::read_to_string(&path)?;
                let mut job: roko_core::MarketplaceJob = serde_json::from_str(&data)?;
                println!("Executing job '{id}' locally...");

                // Transition to in_progress
                job.status = "in_progress".to_string();
                job.updated_at = chrono::Utc::now().to_rfc3339();
                std::fs::write(&path, serde_json::to_string_pretty(&job)?)?;

                // Build prompt based on job type
                let prompt = match job.job_type.as_str() {
                    "research" => format!(
                        "Research the following topic and produce a detailed report with citations:\n\n{}",
                        job.description
                    ),
                    "coding_task" | "coding" => {
                        if !job.plan_id.is_empty() {
                            format!("Execute plan '{}' in the current workspace", job.plan_id)
                        } else {
                            job.description.clone()
                        }
                    }
                    _ => job.description.clone(),
                };

                let config = resolve_config_for_workdir(cli, &wd)?;
                let result = run_once(&wd, &config, &prompt, None, None).await;
                match result {
                    Ok(report) => {
                        job.status = "completed".to_string();
                        job.submission = Some(serde_json::json!({
                            "result_summary": if report.overall_success() { "success" } else { "completed with failures" },
                            "completed_at": chrono::Utc::now().to_rfc3339(),
                        }));
                        job.updated_at = chrono::Utc::now().to_rfc3339();
                        std::fs::write(&path, serde_json::to_string_pretty(&job)?)?;
                        println!("Job '{id}' completed successfully.");
                    }
                    Err(e) => {
                        job.status = "failed".to_string();
                        job.updated_at = chrono::Utc::now().to_rfc3339();
                        std::fs::write(&path, serde_json::to_string_pretty(&job)?)?;
                        return Err(e.context(format!("job '{id}' failed")));
                    }
                }
            }
            Ok(EXIT_SUCCESS)
        }
        JobCmd::Cancel { id, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let path = resolve_job_path(&jobs_dir(&wd), &id)?;
            let data = std::fs::read_to_string(&path)?;
            let mut job: roko_core::MarketplaceJob = serde_json::from_str(&data)?;
            let effective_status = job.effective_status();
            if matches!(effective_status, "completed" | "failed" | "cancelled") {
                bail!("cannot cancel job '{id}': status '{effective_status}' is terminal");
            }
            job.status = "cancelled".to_string();
            job.updated_at = chrono::Utc::now().to_rfc3339();
            std::fs::write(&path, serde_json::to_string_pretty(&job)?)?;
            println!("Job '{id}' cancelled.");
            Ok(EXIT_SUCCESS)
        }
    }
}

/// Resolve a (possibly prefix-truncated) job ID to the full UUID by scanning
/// `.roko/jobs/*.json`.  Exact matches are preferred; if no exact match is
/// found the prefix is tried.  Ambiguous prefixes produce an error listing
/// the candidates.
pub(crate) fn resolve_job_path(jobs_dir: &Path, id: &str) -> Result<std::path::PathBuf> {
    // 1. Try exact match first.
    let exact = jobs_dir.join(format!("{id}.json"));
    if exact.exists() {
        return Ok(exact);
    }
    // 2. Prefix scan.
    let lower = id.to_ascii_lowercase();
    let mut matches: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(jobs_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(stem) = std::path::Path::new(&name)
                .file_stem()
                .and_then(|s| s.to_str())
            else {
                continue;
            };
            if stem.to_ascii_lowercase().starts_with(&lower) {
                matches.push(entry.path());
            }
        }
    }
    match matches.len() {
        0 => anyhow::bail!(
            "job '{id}' not found — no files in {} match that prefix",
            jobs_dir.display()
        ),
        1 => Ok(matches.into_iter().next().unwrap()),
        n => {
            let ids: Vec<String> = matches
                .iter()
                .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
                .collect();
            anyhow::bail!(
                "ambiguous job prefix '{id}' — {n} matches: {}",
                ids.join(", ")
            )
        }
    }
}
