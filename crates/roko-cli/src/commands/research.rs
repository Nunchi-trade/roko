//! research command handlers.
#![allow(unused_imports)]

use crate::*;
use roko_core::config::DEFAULT_TTFT_TIMEOUT_MS;

pub(crate) async fn cmd_research(cli: &Cli, cmd: ResearchCmd) -> Result<i32> {
    use roko_cli::agent_config::{command_from_config, load_gateway_env, model_from_config};
    use roko_cli::agent_exec::{AgentExecOpts, run_agent_capture_silent};
    use roko_cli::research::{
        ResearchMode, build_research_prompt, build_research_prompt_gemini,
        build_research_prompt_perplexity, grounding_to_citations, save_research_with_grounding,
    };

    let workdir = resolve_workdir(cli);
    roko_cli::research::ensure_dirs(&workdir)?;
    let gw = load_gateway_env(&workdir);
    let model = cli.model.clone().or_else(|| model_from_config(&workdir));
    let model_ref = model.as_deref();
    let effort = cli.effort.map(|effort| effort.to_string());
    let effort_ref = effort.as_deref();
    let resume_session = cli.resume.as_deref();
    let agent_command = command_from_config(&workdir).unwrap_or_else(|| "claude".to_string());
    let config = roko_core::config::loader::load_config_unified(&workdir).unwrap_or_default();

    match cmd {
        ResearchCmd::Topic { topic, deep } => {
            let topic = topic.join(" ");
            println!("🔬 Researching: {topic}");

            // --deep: use PerplexityDeepResearchAgent (sonar-deep-research, async polling)
            if deep {
                use roko_agent::perplexity::types::PerplexityMetadata;
                use roko_core::Body;

                let model_slug = config
                    .perplexity
                    .default_research_model
                    .clone()
                    .unwrap_or_else(|| "sonar-deep-research".to_string());

                let (combined_prompt, _) = build_research_prompt_perplexity(
                    &workdir,
                    &topic,
                    "",
                    ResearchMode::Topic,
                    &config.perplexity,
                );

                let (routing_config, timeout_ms) =
                    with_perplexity_research_model(&config, &model_slug, true);
                let agent = spawn_agent_scoped(
                    &routing_config,
                    SpawnAgentSpec {
                        model: model_slug.clone(),
                        command: None,
                        timeout_ms: Some(timeout_ms),
                        system_prompt: None,
                        cached_content: None,
                        tools: None,
                        mcp_config: None,
                        working_dir: Some(workdir.clone()),
                        env: Vec::new(),
                        extra_args: Vec::new(),
                        effort: None,
                        bare_mode: false,
                        dangerously_skip_permissions: false,
                        name: String::new(),
                        role: Some("researcher".to_string()),
                    },
                    format!("create Perplexity deep research agent for model {model_slug}"),
                )?;
                println!("⏳ Deep research submitted ({model_slug}). This takes 1-10 min...");

                let input = roko_core::Engram::builder(Kind::Prompt)
                    .body(Body::text(&combined_prompt))
                    .build();

                let started = Instant::now();
                let mut handle =
                    tokio::spawn(async move { agent.run(&input, &Context::now()).await });
                let poll_started = std::time::Instant::now();
                let result = loop {
                    tokio::select! {
                        r = &mut handle => break r.context("agent task panicked")?,
                        _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {
                            let elapsed = poll_started.elapsed().as_secs();
                            println!("  ⏳ Still researching... ({elapsed}s elapsed)");
                        }
                    }
                };

                if !result.success {
                    let err_text = result.output.body.as_text().unwrap_or("unknown error");
                    let output = result.output.body.as_text().unwrap_or_default().to_string();
                    let _ = crate::commands::util::persist_capture_episode(
                        &workdir,
                        "perplexity",
                        Some(&model_slug),
                        "research-topic-deep",
                        &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                        &combined_prompt,
                        &output,
                        false,
                        started.elapsed().as_millis() as u64,
                        resume_session,
                    )
                    .await;
                    anyhow::bail!("Deep research failed: {err_text}");
                }

                let content = result
                    .output
                    .body
                    .as_text()
                    .map_err(|e| anyhow::anyhow!("response body not text: {e}"))?
                    .to_string();

                let citations: Vec<String> = result
                    .output
                    .tag("pplx_meta")
                    .and_then(|meta_json| {
                        serde_json::from_str::<PerplexityMetadata>(meta_json)
                            .ok()
                            .map(|m| m.citations)
                    })
                    .unwrap_or_default();

                let mut output = content;
                if !citations.is_empty() {
                    output.push_str("\n\n## Sources\n\n");
                    for (i, url) in citations.iter().enumerate() {
                        let _ = writeln!(output, "{}. {url}", i + 1);
                    }
                }

                let slug = topic.to_lowercase().replace(' ', "-");
                let out_path = workdir
                    .join(".roko/research")
                    .join(format!("{slug}-deep.md"));
                std::fs::write(&out_path, &output)
                    .with_context(|| format!("write {}", out_path.display()))?;
                println!("📄 Saved: {}", out_path.display());
                if !citations.is_empty() {
                    println!("📚 {} citations", citations.len());
                }
                let _ = crate::commands::util::persist_capture_episode(
                    &workdir,
                    "perplexity",
                    Some(&model_slug),
                    "research-topic-deep",
                    &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                    &combined_prompt,
                    &output,
                    true,
                    started.elapsed().as_millis() as u64,
                    resume_session,
                )
                .await;
                return Ok(0);
            }

            // If Perplexity is configured, use PerplexityChatAgent for search-grounded research.
            if let Some(model_slug) = config.gemini.grounding_model.clone() {
                use roko_agent::gemini::GeminiMetadata;
                use roko_core::Body;

                let (combined_prompt, enable_grounding) = build_research_prompt_gemini(
                    &workdir,
                    &topic,
                    ResearchMode::Topic,
                    &config.gemini,
                );
                if enable_grounding {
                    let configured_profile = config.models.get(&model_slug).cloned();
                    let provider_key = configured_profile
                        .as_ref()
                        .map(|profile| profile.provider.clone())
                        .unwrap_or_else(|| "gemini".to_string());
                    let configured_provider = config
                        .providers
                        .get(&provider_key)
                        .cloned()
                        .or_else(|| config.providers.get("gemini").cloned());
                    let base_url = configured_provider
                        .as_ref()
                        .and_then(|provider| provider.base_url.clone())
                        .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string());
                    let timeout_ms = configured_provider
                        .as_ref()
                        .and_then(|provider| provider.timeout_ms)
                        .unwrap_or(300_000);

                    let mut model_profile = configured_profile.unwrap_or_else(|| ModelProfile {
                        provider: provider_key.clone(),
                        slug: model_slug.clone(),
                        context_window: 1_048_576,
                        max_output: Some(65_536),
                        supports_tools: true,
                        supports_thinking: true,
                        supports_vision: false,
                        supports_web_search: false,
                        supports_mcp_tools: false,
                        supports_partial: false,
                        supports_grounding: true,
                        supports_code_execution: false,
                        supports_caching: false,
                        provider_routing: None,
                        tool_format: "gemini_native".to_string(),
                        cost_input_per_m: None,
                        cost_output_per_m: None,
                        cost_input_per_m_high: None,
                        cost_output_per_m_high: None,
                        cost_cache_read_per_m: None,
                        cost_cache_write_per_m: None,
                        thinking_level: Some(config.gemini.thinking_level.clone()),
                        max_tools: None,
                        tokenizer_ratio: None,
                        supports_search: false,
                        supports_citations: false,
                        supports_async: false,
                        is_embedding_model: false,
                        search_context_size: None,
                        cost_per_request: None,
                        ..Default::default()
                    });
                    model_profile.supports_grounding = true;
                    model_profile.tool_format = "gemini_native".to_string();
                    if model_profile.thinking_level.is_none() {
                        model_profile.thinking_level = Some(config.gemini.thinking_level.clone());
                    }

                    let routing_config = with_research_provider_model(
                        &config,
                        &provider_key,
                        configured_provider.unwrap_or(ProviderConfig {
                            kind: ProviderKind::GeminiApi,
                            base_url: Some(base_url),
                            api_key_env: Some("GEMINI_API_KEY".to_string()),
                            command: None,
                            args: None,
                            timeout_ms: Some(timeout_ms),
                            ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
                            connect_timeout_ms: Some(5_000),
                            extra_headers: None,
                            max_concurrent: None,
                        }),
                        model_profile,
                    );
                    let agent = spawn_agent_scoped(
                        &routing_config,
                        SpawnAgentSpec {
                            model: model_slug.clone(),
                            command: None,
                            timeout_ms: Some(timeout_ms),
                            system_prompt: None,
                            cached_content: None,
                            tools: None,
                            mcp_config: None,
                            effort: Some(config.gemini.thinking_level.clone()),
                            name: format!("gemini:{model_slug}"),
                            working_dir: Some(workdir.clone()),
                            env: Vec::new(),
                            extra_args: Vec::new(),
                            bare_mode: false,
                            dangerously_skip_permissions: false,
                            role: Some("researcher".to_string()),
                        },
                        format!("create Gemini research agent for model {model_slug}"),
                    )?;

                    let input = roko_core::Engram::builder(Kind::Prompt)
                        .body(Body::text(&combined_prompt))
                        .build();
                    let started = Instant::now();
                    let result = agent.run(&input, &Context::now()).await;

                    if !result.success {
                        let err_text = result.output.body.as_text().unwrap_or("unknown error");
                        let output = result.output.body.as_text().unwrap_or_default().to_string();
                        let _ = crate::commands::util::persist_capture_episode(
                            &workdir,
                            "gemini",
                            Some(&model_slug),
                            "research-topic-gemini",
                            &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                            &combined_prompt,
                            &output,
                            false,
                            started.elapsed().as_millis() as u64,
                            resume_session,
                        )
                        .await;
                        anyhow::bail!("Gemini research failed: {err_text}");
                    }

                    let content = result
                        .output
                        .body
                        .as_text()
                        .map_err(|e| anyhow::anyhow!("response body not text: {e}"))?
                        .to_string();

                    let grounding = result
                        .output
                        .tag("gemini_meta")
                        .and_then(|meta_json| {
                            serde_json::from_str::<GeminiMetadata>(meta_json).ok()
                        })
                        .and_then(|metadata| metadata.grounding_metadata);

                    let out_path = if let Some(grounding) = &grounding {
                        save_research_with_grounding(&workdir, &topic, &content, grounding)?
                    } else {
                        let slug = topic.to_lowercase().replace(' ', "-");
                        let out_path = workdir.join(".roko/research").join(format!("{slug}.md"));
                        std::fs::write(&out_path, &content)
                            .with_context(|| format!("write {}", out_path.display()))?;
                        out_path
                    };

                    println!("📄 Saved: {}", out_path.display());
                    if let Some(grounding) = &grounding {
                        let citations = grounding_to_citations(grounding);
                        if !citations.is_empty() {
                            println!("📚 {} citations", citations.len());
                        }
                    }
                    let _ = crate::commands::util::persist_capture_episode(
                        &workdir,
                        "gemini",
                        Some(&model_slug),
                        "research-topic-gemini",
                        &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                        &combined_prompt,
                        &content,
                        true,
                        started.elapsed().as_millis() as u64,
                        resume_session,
                    )
                    .await;
                    return Ok(0);
                }
            }

            if let Some(model_slug) = config.perplexity.default_search_model.clone() {
                use roko_agent::perplexity::types::PerplexityMetadata;
                use roko_core::Body;

                let (combined_prompt, search_opts) = build_research_prompt_perplexity(
                    &workdir,
                    &topic,
                    "",
                    ResearchMode::Topic,
                    &config.perplexity,
                );
                let (routing_config, timeout_ms) =
                    with_perplexity_research_model(&config, &model_slug, false);
                let agent = spawn_agent_scoped(
                    &routing_config,
                    SpawnAgentSpec {
                        model: model_slug.clone(),
                        command: None,
                        timeout_ms: Some(timeout_ms),
                        system_prompt: None,
                        cached_content: None,
                        tools: None,
                        mcp_config: None,
                        working_dir: Some(workdir.clone()),
                        env: Vec::new(),
                        extra_args: vec![format!(
                            "{}{}",
                            roko_agent::provider::PERPLEXITY_SEARCH_OPTIONS_ARG_PREFIX,
                            serde_json::to_string(&search_opts)
                                .expect("Perplexity search options must serialize"),
                        )],
                        effort: None,
                        bare_mode: false,
                        dangerously_skip_permissions: false,
                        name: String::new(),
                        role: Some("researcher".to_string()),
                    },
                    format!("create Perplexity research agent for model {model_slug}"),
                )?;

                let input = roko_core::Engram::builder(Kind::Prompt)
                    .body(Body::text(&combined_prompt))
                    .build();
                let started = Instant::now();
                let result = agent.run(&input, &Context::now()).await;

                if !result.success {
                    let err_text = result.output.body.as_text().unwrap_or("unknown error");
                    let output = result.output.body.as_text().unwrap_or_default().to_string();
                    let _ = crate::commands::util::persist_capture_episode(
                        &workdir,
                        "perplexity",
                        Some(&model_slug),
                        "research-topic-perplexity",
                        &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                        &combined_prompt,
                        &output,
                        false,
                        started.elapsed().as_millis() as u64,
                        resume_session,
                    )
                    .await;
                    anyhow::bail!("Perplexity research failed: {err_text}");
                }

                let content = result
                    .output
                    .body
                    .as_text()
                    .map_err(|e| anyhow::anyhow!("response body not text: {e}"))?
                    .to_string();

                let citations: Vec<String> = result
                    .output
                    .tag("pplx_meta")
                    .and_then(|meta_json| {
                        serde_json::from_str::<PerplexityMetadata>(meta_json)
                            .ok()
                            .map(|m| m.citations)
                    })
                    .unwrap_or_default();

                let mut output = content;
                if !citations.is_empty() {
                    output.push_str("\n\n## Sources\n\n");
                    for (i, url) in citations.iter().enumerate() {
                        let _ = writeln!(output, "{}. {url}", i + 1);
                    }
                }

                let slug = topic.to_lowercase().replace(' ', "-");
                let out_path = workdir.join(".roko/research").join(format!("{slug}.md"));
                std::fs::write(&out_path, &output)
                    .with_context(|| format!("write {}", out_path.display()))?;
                println!("📄 Saved: {}", out_path.display());
                if !citations.is_empty() {
                    println!("📚 {} citations", citations.len());
                }
                let _ = crate::commands::util::persist_capture_episode(
                    &workdir,
                    "perplexity",
                    Some(&model_slug),
                    "research-topic-perplexity",
                    &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                    &combined_prompt,
                    &output,
                    true,
                    started.elapsed().as_millis() as u64,
                    resume_session,
                )
                .await;
                return Ok(0);
            }

            // Claude CLI fallback
            let task_prompt = format!(
                "Research the topic: \"{topic}\". \
                 Save your findings to .roko/research/{slug}.md with full citations. \
                 Read existing docs in .roko/prd/ and .roko/research/ for context on the project.",
                slug = topic.to_lowercase().replace(' ', "-")
            );
            let system = build_research_prompt(&workdir, &topic, "", ResearchMode::Topic);
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("researcher"),
                allowed_tools: Some("Read,Write,Edit"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = crate::commands::util::persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "research-topic-claude",
                &format!("research:topic:{}", topic.to_lowercase().replace(' ', "-")),
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
        }
        ResearchCmd::EnhancePrd { slug } => {
            let prd_path = crate::commands::prd::find_prd(&workdir, &slug)?;
            let content = std::fs::read_to_string(&prd_path)
                .with_context(|| format!("read {}", prd_path.display()))?;
            println!("🔬 Enhancing PRD: {slug}");
            let task_prompt = format!(
                "Read the PRD at {path} and enhance it: \
                 (1) Add academic citations [AUTHOR-YEAR] for every design decision. \
                 (2) Add mermaid diagrams with color styling where architecture would be clearer. \
                 (3) Identify improvements from recent research. \
                 (4) Flag claims that contradict recent findings. \
                 Update the file in place. Also save a research summary to .roko/research/enhance-{slug}.md",
                path = prd_path.display()
            );
            let system = build_research_prompt(&workdir, &slug, &content, ResearchMode::EnhancePrd);
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("researcher"),
                allowed_tools: Some("Read,Write,Edit"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = crate::commands::util::persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "research-enhance-prd",
                &format!("research:enhance-prd:{slug}"),
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
        }
        ResearchCmd::EnhancePlan { plan } => {
            let plan_dir = roko_cli::plan::plans_dir(&workdir).join(&plan);
            if !plan_dir.is_dir() {
                anyhow::bail!("Plan directory not found: {}", plan_dir.display());
            }
            println!("🔬 Enhancing plan: {plan}");
            let task_prompt = format!(
                "Read the plan at .roko/plans/{plan}/plan.md and .roko/plans/{plan}/tasks.toml. \
                 Optimize them using research-backed techniques: \
                 (1) Better task decomposition (cite SWE-bench, Agentless). \
                 (2) More precise context injection per task (exact file:line ranges). \
                 (3) Stronger verification (executable commands, not descriptions). \
                 (4) Cost optimization (assign cheapest model per task tier). \
                 Update the files in place."
            );
            let mut context = String::new();
            for name in ["plan.md", "tasks.toml"] {
                let p = plan_dir.join(name);
                if p.exists() {
                    let c = std::fs::read_to_string(&p).unwrap_or_default();
                    let _ = write!(context, "### {name}\n```\n{c}\n```\n\n");
                }
            }
            let system =
                build_research_prompt(&workdir, &plan, &context, ResearchMode::EnhancePlan);
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("researcher"),
                allowed_tools: Some("Read,Write,Edit"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = crate::commands::util::persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "research-enhance-plan",
                &format!("research:enhance-plan:{plan}"),
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
        }
        ResearchCmd::EnhanceTasks { plan } => {
            let tasks_path = roko_cli::plan::plans_dir(&workdir)
                .join(&plan)
                .join("tasks.toml");
            if !tasks_path.exists() {
                anyhow::bail!("tasks.toml not found: {}", tasks_path.display());
            }
            println!("🔬 Optimizing tasks: {plan}");
            let content = std::fs::read_to_string(&tasks_path)?;
            let task_prompt = format!(
                "Read .roko/plans/{plan}/tasks.toml and optimize every task: \
                 (1) Split any task >50 LOC into smaller subtasks. \
                 (2) Add context.read_files with exact line ranges for each task. \
                 (3) Ensure every acceptance criterion is a runnable shell command. \
                 (4) Remove unnecessary dependency edges to increase parallelism. \
                 (5) Assign tier (mechanical/focused/integrative/architectural) and model_hint. \
                 Search the codebase to verify file paths exist. Update tasks.toml in place."
            );
            let system =
                build_research_prompt(&workdir, &plan, &content, ResearchMode::EnhanceTasks);
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("researcher"),
                allowed_tools: Some("Read,Write,Edit"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = crate::commands::util::persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "research-enhance-tasks",
                &format!("research:enhance-tasks:{plan}"),
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
        }
        ResearchCmd::Analyze => {
            let episodes_path = workdir.join(".roko/memory/episodes.jsonl");
            let context = if episodes_path.exists() {
                std::fs::read_to_string(&episodes_path).unwrap_or_default()
            } else {
                String::from("(no episodes yet — run some tasks first)")
            };
            println!("🔬 Analyzing execution data");
            let task_prompt = "Read .roko/memory/episodes.jsonl and analyze: \
                 (1) First-attempt pass rate by task tier and model. \
                 (2) Cost per task — are expensive models used for easy tasks? \
                 (3) Retry patterns — what kinds of tasks fail most? \
                 (4) Recommendations: which bandit weights to adjust. \
                 Save analysis to .roko/research/execution-analysis.md"
                .to_string();
            let system = build_research_prompt(
                &workdir,
                "execution-analysis",
                &context,
                ResearchMode::AnalyzeExecution,
            );
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("researcher"),
                allowed_tools: Some("Read,Write,Edit"),
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = crate::commands::util::persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "research-analyze",
                "research:analyze:execution",
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
        }
        ResearchCmd::List => {
            let files = roko_cli::research::list_research(&workdir)?;
            if files.is_empty() {
                println!("No research artifacts. Run: roko research topic \"your topic\"");
            } else {
                println!("═══ Research Artifacts ═══");
                for f in &files {
                    let name = f.file_stem().unwrap_or_default().to_string_lossy();
                    let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
                    println!("  {name:<45} {size:>6} bytes");
                }
            }
            Ok(0)
        }
        ResearchCmd::Search {
            query,
            domains,
            recency,
        } => {
            use roko_agent::perplexity::search::{PerplexitySearchClient, SearchQuery};

            let query_str = query.join(" ");
            if query_str.trim().is_empty() {
                anyhow::bail!("provide a search query");
            }

            let api_key =
                std::env::var("PERPLEXITY_API_KEY").context("PERPLEXITY_API_KEY not set")?;

            let date_range = recency.as_deref().map(|r| {
                let now = chrono::Local::now();
                let after = match r {
                    "day" => now - chrono::Duration::days(1),
                    "week" => now - chrono::Duration::weeks(1),
                    "month" => now - chrono::Duration::days(30),
                    "year" => now - chrono::Duration::days(365),
                    _ => now - chrono::Duration::days(30),
                };
                (
                    after.format("%Y-%m-%d").to_string(),
                    now.format("%Y-%m-%d").to_string(),
                )
            });

            let search_query = SearchQuery {
                query: query_str.clone(),
                domain_filter: if domains.is_empty() {
                    None
                } else {
                    Some(domains)
                },
                date_range,
                ..Default::default()
            };

            println!("🔍 Searching: {query_str}");

            let client = PerplexitySearchClient::new(api_key);
            let responses = client
                .search_batch(&[search_query])
                .await
                .map_err(|e| anyhow::anyhow!("search error: {e}"))?;

            let results: Vec<_> = responses.into_iter().flat_map(|r| r.results).collect();

            if results.is_empty() {
                println!("No results found.");
            } else {
                println!("\n═══ Results ═══\n");
                for (i, r) in results.iter().enumerate() {
                    println!("{}. {}", i + 1, r.title);
                    println!("   {}", r.url);
                    if let Some(date) = &r.date {
                        println!("   Published: {date}");
                    }
                    let snippet = if r.content.len() > 300 {
                        format!("{}…", &r.content[..300])
                    } else {
                        r.content.clone()
                    };
                    println!("   {snippet}");
                    println!();
                }
            }

            Ok(0)
        }
    }
}

pub(crate) fn with_research_provider_model(
    config: &RokoConfig,
    provider_key: &str,
    provider_config: ProviderConfig,
    model_profile: ModelProfile,
) -> RokoConfig {
    let mut routing_config = config.clone();
    routing_config
        .providers
        .entry(provider_key.to_string())
        .or_insert(provider_config);
    routing_config
        .models
        .entry(model_profile.slug.clone())
        .or_insert(model_profile);
    routing_config
}

pub(crate) fn with_perplexity_research_model(
    config: &RokoConfig,
    model_slug: &str,
    supports_async: bool,
) -> (RokoConfig, u64) {
    let configured_profile = config.models.get(model_slug).cloned();
    let provider_key = configured_profile
        .as_ref()
        .map(|profile| profile.provider.clone())
        .unwrap_or_else(|| "perplexity".to_string());
    let configured_provider = config
        .providers
        .get(&provider_key)
        .cloned()
        .or_else(|| config.providers.get("perplexity").cloned());
    let timeout_ms = configured_provider
        .as_ref()
        .and_then(|provider| provider.timeout_ms)
        .unwrap_or(300_000);

    let mut model_profile = configured_profile.unwrap_or_else(|| ModelProfile {
        provider: provider_key.clone(),
        slug: model_slug.to_string(),
        context_window: 127_072,
        max_output: Some(8_192),
        supports_tools: false,
        supports_thinking: false,
        supports_vision: false,
        supports_web_search: true,
        supports_mcp_tools: false,
        supports_partial: false,
        supports_grounding: false,
        supports_code_execution: false,
        supports_caching: false,
        provider_routing: None,
        tool_format: "openai_json".to_string(),
        cost_input_per_m: None,
        cost_output_per_m: None,
        cost_input_per_m_high: None,
        cost_output_per_m_high: None,
        cost_cache_read_per_m: None,
        cost_cache_write_per_m: None,
        thinking_level: None,
        max_tools: None,
        tokenizer_ratio: None,
        supports_search: true,
        supports_citations: true,
        supports_async,
        is_embedding_model: false,
        search_context_size: None,
        cost_per_request: None,
        ..Default::default()
    });
    model_profile.supports_search = true;
    model_profile.supports_citations = true;
    model_profile.supports_async |= supports_async;

    let routing_config = with_research_provider_model(
        config,
        &provider_key,
        configured_provider.unwrap_or(ProviderConfig {
            kind: ProviderKind::PerplexityApi,
            base_url: Some("https://api.perplexity.ai".to_string()),
            api_key_env: Some("PERPLEXITY_API_KEY".to_string()),
            command: None,
            args: None,
            timeout_ms: Some(timeout_ms),
            ttft_timeout_ms: Some(DEFAULT_TTFT_TIMEOUT_MS),
            connect_timeout_ms: Some(5_000),
            extra_headers: None,
            max_concurrent: None,
        }),
        model_profile,
    );

    (routing_config, timeout_ms)
}
