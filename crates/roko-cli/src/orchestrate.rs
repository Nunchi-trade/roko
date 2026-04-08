//! Plan-driven orchestration loop: reads plans → builds executor → dispatches
//! agents → runs gates → persists results → advances phases.
//!
//! This is the runtime harness that connects the CLI to the orchestrator's
//! pure state machine. The orchestrator's [`ParallelExecutor`] never does I/O
//! — it returns [`ExecutorAction`]s. This module dispatches those actions to
//! real agents, gates, and git, then feeds results back as events.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use roko_agent::translate::{ClaudeTranslator, RenderedTools, Translator};
use roko_agent::{Agent, AgentResult, ClaudeCliAgent, ExecAgent};
use roko_compose::{
    Placement, PromptComposer, PromptSection, RoleSystemPromptSpec, SectionPriority, TaskContext,
};
use roko_core::tool::ToolRegistry;
use roko_core::{
    AgentRole, Body, Budget, Composer, Context, Gate, Kind, PhaseKind, Provenance, Signal,
    Substrate, Verdict,
};
use roko_fs::FileSubstrate;
use roko_gate::{
    clippy_gate::ClippyGate, compile::CompileGate, payload::GatePayload, test_gate::TestGate,
};
use roko_orchestrator::worktree::{WorktreeConfig, WorktreeManager};
use roko_orchestrator::{
    EventKind, EventLog, EventLogSnapshot, ExecutorAction, ExecutorConfig, ExecutorEvent,
    ExecutorSnapshot, GateResult, ParallelExecutor, PlanState, PostMergeRunner, discover_plans,
};
use roko_std::NoOpScorer;
use roko_std::StaticToolRegistry;

use crate::config::Config;

/// Default number of actions between auto-saves.
const AUTOSAVE_INTERVAL: usize = 5;
const DEFAULT_WORKTREE_IDLE_TTL_SECS: u64 = 30 * 60;

// ─── Report types ─────────────────────────────────────────────────────────

/// Report returned after a single plan's execution completes.
#[derive(Debug, Clone)]
pub struct PlanRunReport {
    /// Plan ID.
    pub plan_id: String,
    /// Whether the plan reached a success terminal phase.
    pub succeeded: bool,
    /// Number of agent invocations for this plan.
    pub agent_calls: usize,
    /// Gate results collected during execution.
    pub gate_results: Vec<(String, bool)>,
}

/// Summary of the entire orchestration run across all plans.
#[derive(Debug, Clone)]
pub struct OrchestrationReport {
    /// Per-plan results.
    pub plans: Vec<PlanRunReport>,
    /// Total agent invocations across all plans.
    pub total_agent_calls: usize,
    /// Total gate runs across all plans.
    pub total_gate_runs: usize,
}

impl OrchestrationReport {
    /// True if every plan reached a success terminal state.
    #[must_use]
    pub fn all_succeeded(&self) -> bool {
        self.plans.iter().all(|p| p.succeeded)
    }
}

// ─── PlanRunner ───────────────────────────────────────────────────────────

/// The runtime harness that drives plan execution end-to-end.
///
/// Connects the CLI to the orchestrator, agents, and gates. Maintains
/// an event log for crash recovery and periodically auto-saves state.
pub struct PlanRunner {
    /// Working directory (repo root).
    workdir: PathBuf,
    /// CLI config for agent/gate settings.
    config: Config,
    /// The executor state machine.
    executor: ParallelExecutor,
    /// Append-only event log for crash recovery.
    event_log: EventLog,
    /// Counters for reporting.
    agent_calls: usize,
    gate_runs: usize,
    /// Per-plan worktree manager.
    worktrees: WorktreeManager,
    /// Post-merge regression history and follow-up decisions.
    post_merge: PostMergeRunner,
    /// Optional Claude session resume id from upper layers.
    claude_resume_session: Option<String>,
    /// Actions dispatched since last auto-save.
    actions_since_save: usize,
    /// Per-plan tracking.
    per_plan_agents: HashMap<String, usize>,
    per_plan_gates: HashMap<String, Vec<(String, bool)>>,
}

impl PlanRunner {
    /// Discover plans from a directory and build the executor.
    ///
    /// # Errors
    ///
    /// Returns an error if the plans directory doesn't exist, contains no
    /// plans, or plan discovery fails.
    pub fn from_plans_dir(plans_dir: &Path, workdir: &Path, config: Config) -> Result<Self> {
        if !plans_dir.exists() {
            return Err(anyhow!(
                "plans directory does not exist: {}",
                plans_dir.display()
            ));
        }

        let plans = discover_plans(plans_dir).map_err(|e| anyhow!("plan discovery failed: {e}"))?;

        if plans.is_empty() {
            return Err(anyhow!("no plans found in {}", plans_dir.display()));
        }

        let mut executor = ParallelExecutor::new(ExecutorConfig::default());

        for plan_info in &plans {
            let plan_id = plan_info
                .frontmatter
                .as_ref()
                .and_then(|fm| fm.plan.clone())
                .unwrap_or_else(|| plan_info.base.clone());
            let state = PlanState::new(&plan_id);
            executor.add_plan(state);
        }

        Ok(Self {
            workdir: workdir.to_path_buf(),
            config,
            executor,
            event_log: EventLog::default(),
            agent_calls: 0,
            gate_runs: 0,
            worktrees: default_worktree_manager(workdir),
            post_merge: PostMergeRunner::new(),
            claude_resume_session: None,
            actions_since_save: 0,
            per_plan_agents: HashMap::new(),
            per_plan_gates: HashMap::new(),
        })
    }

    /// Restore a runner from a snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if snapshot parsing fails.
    pub fn from_snapshot(snapshot_json: &str, workdir: &Path, config: Config) -> Result<Self> {
        let snapshot =
            ExecutorSnapshot::from_json(snapshot_json).map_err(|e| anyhow!("bad snapshot: {e}"))?;
        let executor = ParallelExecutor::from_snapshot(ExecutorConfig::default(), snapshot);
        Ok(Self {
            workdir: workdir.to_path_buf(),
            config,
            executor,
            event_log: EventLog::default(),
            agent_calls: 0,
            gate_runs: 0,
            worktrees: default_worktree_manager(workdir),
            post_merge: PostMergeRunner::new(),
            claude_resume_session: None,
            actions_since_save: 0,
            per_plan_agents: HashMap::new(),
            per_plan_gates: HashMap::new(),
        })
    }

    /// Restore a runner from both an executor snapshot and an event log snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if parsing fails.
    pub fn from_snapshots(
        executor_json: &str,
        event_log_json: &str,
        workdir: &Path,
        config: Config,
    ) -> Result<Self> {
        let exec_snap = ExecutorSnapshot::from_json(executor_json)
            .map_err(|e| anyhow!("bad executor snapshot: {e}"))?;
        let log_snap: EventLogSnapshot = serde_json::from_str(event_log_json)
            .map_err(|e| anyhow!("bad event log snapshot: {e}"))?;
        let executor = ParallelExecutor::from_snapshot(ExecutorConfig::default(), exec_snap);
        let event_log = EventLog::restore(log_snap);
        Ok(Self {
            workdir: workdir.to_path_buf(),
            config,
            executor,
            event_log,
            agent_calls: 0,
            gate_runs: 0,
            worktrees: default_worktree_manager(workdir),
            post_merge: PostMergeRunner::new(),
            claude_resume_session: None,
            actions_since_save: 0,
            per_plan_agents: HashMap::new(),
            per_plan_gates: HashMap::new(),
        })
    }

    /// Thread an optional Claude resume id from upper-layer orchestration
    /// context into per-agent launches.
    pub fn set_claude_resume_session(&mut self, session_id: Option<String>) {
        self.claude_resume_session = normalize_resume_session(session_id);
    }

    /// Take a snapshot of the current executor state.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn snapshot(&self) -> Result<String> {
        #[allow(clippy::cast_possible_truncation)]
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let snap = self.executor.snapshot(ts);
        snap.to_json().map_err(|e| anyhow!("snapshot: {e}"))
    }

    /// Take a snapshot of the event log.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn event_log_snapshot(&self) -> Result<String> {
        let snap = self.event_log.snapshot();
        serde_json::to_string_pretty(&snap).map_err(|e| anyhow!("event log: {e}"))
    }

    /// Persist both executor and event log snapshots to `.roko/state/`.
    ///
    /// Uses atomic write (write to temp + rename) for safety.
    ///
    /// # Errors
    ///
    /// Returns an error if the state directory cannot be created or the
    /// files cannot be written.
    pub fn save_state(&self) -> Result<()> {
        let state_dir = self.workdir.join(".roko").join("state");
        std::fs::create_dir_all(&state_dir).map_err(|e| anyhow!("create state dir: {e}"))?;

        // Executor snapshot — atomic write.
        let exec_json = self.snapshot()?;
        let exec_path = state_dir.join("executor.json");
        let exec_tmp = state_dir.join("executor.json.tmp");
        std::fs::write(&exec_tmp, &exec_json).map_err(|e| anyhow!("write executor tmp: {e}"))?;
        std::fs::rename(&exec_tmp, &exec_path)
            .map_err(|e| anyhow!("rename executor snapshot: {e}"))?;

        // Event log snapshot — atomic write.
        let log_json = self.event_log_snapshot()?;
        let log_path = state_dir.join("events.json");
        let log_tmp = state_dir.join("events.json.tmp");
        std::fs::write(&log_tmp, &log_json).map_err(|e| anyhow!("write events tmp: {e}"))?;
        std::fs::rename(&log_tmp, &log_path).map_err(|e| anyhow!("rename events snapshot: {e}"))?;

        Ok(())
    }

    /// Returns a reference to the inner executor (for status queries).
    #[must_use]
    pub const fn executor(&self) -> &ParallelExecutor {
        &self.executor
    }

    /// Run all plans to completion (or failure).
    ///
    /// This is the main orchestration loop. It calls `tick()` on the
    /// executor, dispatches the returned actions, feeds results back as
    /// events, and repeats until all plans are terminal.
    ///
    /// # Errors
    ///
    /// Returns an error if agent dispatch, gate execution, or substrate
    /// I/O fails fatally (per-plan failures are recorded in the report).
    pub async fn run_all(&mut self) -> Result<OrchestrationReport> {
        // Start all queued plans.
        let plan_ids: Vec<String> = self
            .executor
            .snapshot(0)
            .plan_states
            .keys()
            .cloned()
            .collect();
        for plan_id in &plan_ids {
            let _ = self.executor.apply_event(plan_id, &ExecutorEvent::Start);
        }

        // Maximum iterations to prevent infinite loops.
        let max_iterations = 1000;
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > max_iterations {
                eprintln!("[orchestrate] hit max iterations ({max_iterations}), stopping");
                break;
            }

            let actions = self.executor.tick();

            if actions.is_empty() {
                if self.all_terminal(&plan_ids) {
                    break;
                }
                // No actions but not all terminal — wait and retry.
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                continue;
            }

            for action in actions {
                self.dispatch_action(action).await;
            }

            // Auto-save periodically.
            if self.actions_since_save >= AUTOSAVE_INTERVAL {
                if let Err(e) = self.save_state() {
                    eprintln!("[orchestrate] auto-save failed: {e}");
                }
                self.actions_since_save = 0;
            }
        }

        // Build the report.
        let plans: Vec<PlanRunReport> = plan_ids
            .iter()
            .map(|id| {
                let state = self.executor.plan_state(id);
                let succeeded =
                    state.is_some_and(|s| s.current_phase.kind() == PhaseKind::Complete);
                PlanRunReport {
                    plan_id: id.clone(),
                    succeeded,
                    agent_calls: self.per_plan_agents.get(id).copied().unwrap_or(0),
                    gate_results: self.per_plan_gates.get(id).cloned().unwrap_or_default(),
                }
            })
            .collect();

        // Final save before returning.
        if let Err(e) = self.save_state() {
            eprintln!("[orchestrate] final save failed: {e}");
        }

        Ok(OrchestrationReport {
            total_agent_calls: self.agent_calls,
            total_gate_runs: self.gate_runs,
            plans,
        })
    }

    // ── Internal dispatch ─────────────────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    async fn dispatch_action(&mut self, action: ExecutorAction) {
        self.actions_since_save += 1;

        match action {
            ExecutorAction::SpawnAgent {
                plan_id,
                role,
                task,
            } => {
                eprintln!("[orchestrate] SpawnAgent plan={plan_id} role={role:?} task={task}");
                self.event_log.append(
                    EventKind::AgentSpawned,
                    serde_json::json!({"plan_id": plan_id, "role": format!("{role:?}"), "task": task}),
                );
                match self.dispatch_agent(&plan_id, role, &task).await {
                    Ok(_) => {
                        *self.per_plan_agents.entry(plan_id.clone()).or_default() += 1;
                        self.agent_calls += 1;
                        let event = self.agent_completion_event(&plan_id);
                        self.log_transition(&plan_id, &event);
                        let _ = self.executor.apply_event(&plan_id, &event);
                    }
                    Err(e) => {
                        eprintln!("[orchestrate] agent failed for {plan_id}: {e}");
                        self.event_log.append(
                            EventKind::ErrorOccurred,
                            serde_json::json!({"plan_id": plan_id, "error": e.to_string()}),
                        );
                        let _ = self.executor.apply_event(
                            &plan_id,
                            &ExecutorEvent::Fatal(format!("agent error: {e}")),
                        );
                    }
                }
            }
            ExecutorAction::RunGate { plan_id, rung } => {
                eprintln!("[orchestrate] RunGate plan={plan_id} rung={rung}");
                match self.run_gate_pipeline(&plan_id, rung).await {
                    Ok(passed) => {
                        self.gate_runs += 1;
                        self.per_plan_gates
                            .entry(plan_id.clone())
                            .or_default()
                            .push((format!("rung-{rung}"), passed));
                        self.event_log.append(
                            EventKind::GateResult,
                            serde_json::json!({"plan_id": plan_id, "rung": rung, "passed": passed}),
                        );
                        let event = if passed {
                            ExecutorEvent::GatePassed
                        } else {
                            ExecutorEvent::GateFailed
                        };
                        let _ = self.executor.apply_event(&plan_id, &event);
                    }
                    Err(e) => {
                        eprintln!("[orchestrate] gate failed for {plan_id}: {e}");
                        self.event_log.append(
                            EventKind::ErrorOccurred,
                            serde_json::json!({"plan_id": plan_id, "error": e.to_string()}),
                        );
                        let _ = self
                            .executor
                            .apply_event(&plan_id, &ExecutorEvent::GateFailed);
                    }
                }
            }
            ExecutorAction::MergeBranch { plan_id } => {
                eprintln!("[orchestrate] MergeBranch plan={plan_id}");
                self.event_log.append(
                    EventKind::MergeAttempted,
                    serde_json::json!({"plan_id": plan_id}),
                );
                match self.merge_branch(&plan_id).await {
                    Ok(()) => {
                        match self.run_post_merge_follow_up(&plan_id).await {
                            Ok(true) => {
                                let _ = self
                                    .executor
                                    .apply_event(&plan_id, &ExecutorEvent::MergeSucceeded);
                            }
                            Ok(false) => {
                                let _ = self
                                    .executor
                                    .apply_event(&plan_id, &ExecutorEvent::MergeFailed);
                            }
                            Err(e) => {
                                eprintln!(
                                    "[orchestrate] post-merge checks failed for {plan_id}: {e}"
                                );
                                self.event_log.append(
                                    EventKind::ErrorOccurred,
                                    serde_json::json!({"plan_id": plan_id, "error": format!("post-merge follow-up failed: {e}")}),
                                );
                                // Keep historical behavior on infrastructure errors:
                                // merge itself succeeded.
                                let _ = self
                                    .executor
                                    .apply_event(&plan_id, &ExecutorEvent::MergeSucceeded);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[orchestrate] merge failed for {plan_id}: {e}");
                        let _ = self
                            .executor
                            .apply_event(&plan_id, &ExecutorEvent::MergeFailed);
                    }
                }
            }
            ExecutorAction::DispatchPlan { plan_id } => {
                eprintln!("[orchestrate] DispatchPlan {plan_id}");
                self.event_log.append(
                    EventKind::PlanStarted,
                    serde_json::json!({"plan_id": plan_id}),
                );
                let _ = self.executor.apply_event(&plan_id, &ExecutorEvent::Start);
            }
            ExecutorAction::PausePlan { plan_id } => {
                eprintln!("[orchestrate] PausePlan {plan_id}");
                self.executor.pause_plan(&plan_id);
            }
            ExecutorAction::ResumePlan { plan_id } => {
                eprintln!("[orchestrate] ResumePlan {plan_id}");
                self.executor.resume_plan(&plan_id);
            }
            _ => {
                // Future ExecutorAction variants — log and skip.
                eprintln!("[orchestrate] unhandled action: {action:?}");
            }
        }
    }

    /// Log a phase transition event.
    fn log_transition(&self, plan_id: &str, event: &ExecutorEvent) {
        self.event_log.append(
            EventKind::PhaseTransition,
            serde_json::json!({"plan_id": plan_id, "event": format!("{event:?}")}),
        );
    }

    fn all_terminal(&self, plan_ids: &[String]) -> bool {
        plan_ids.iter().all(|id| {
            self.executor
                .plan_state(id)
                .is_none_or(PlanState::is_terminal)
        })
    }

    /// Determine which completion event to fire after an agent completes.
    #[allow(clippy::match_same_arms)] // Each phase listed for clarity
    fn agent_completion_event(&self, plan_id: &str) -> ExecutorEvent {
        let Some(state) = self.executor.plan_state(plan_id) else {
            return ExecutorEvent::Fatal("unknown plan".into());
        };
        match state.current_phase.kind() {
            PhaseKind::Enriching => ExecutorEvent::EnrichmentDone,
            PhaseKind::Implementing => ExecutorEvent::ImplementationDone,
            PhaseKind::AutoFixing => ExecutorEvent::AutoFixDone,
            PhaseKind::Verifying => ExecutorEvent::VerifyPassed,
            PhaseKind::Reviewing => ExecutorEvent::ReviewApproved,
            PhaseKind::DocRevision => ExecutorEvent::DocRevisionDone,
            PhaseKind::RegeneratingVerify => ExecutorEvent::VerifyRegenDone,
            _ => ExecutorEvent::ImplementationDone,
        }
    }

    /// Compose a prompt for the given task/role and run the agent.
    async fn dispatch_agent(
        &self,
        plan_id: &str,
        role: AgentRole,
        task: &str,
    ) -> Result<AgentResult> {
        let ctx = Context::now();
        let exec_dir = self.plan_exec_dir(plan_id).await;

        // Build the prompt from role + task description.
        let task_text =
            format!("Plan: {plan_id}\nTask: {task}\n\nImplement the task described above.");
        let claude_tools_csv = claude_tool_allowlist(role);
        let role_instruction = build_system_prompt(role, plan_id, task, &claude_tools_csv);
        let role_section = PromptSection::new("role", &role_instruction)
            .with_priority(SectionPriority::Critical)
            .with_placement(Placement::Start)
            .into_signal()
            .map_err(|e| anyhow!("role section: {e}"))?;
        let task_section = PromptSection::new("task", &task_text)
            .with_priority(SectionPriority::Critical)
            .with_placement(Placement::End)
            .into_signal()
            .map_err(|e| anyhow!("task section: {e}"))?;

        let sections = vec![role_section, task_section];

        let composer = PromptComposer::new();
        let prompt = composer
            .compose(
                &sections,
                &Budget::tokens(self.config.prompt.token_budget),
                &NoOpScorer,
                &ctx,
            )
            .map_err(|e| anyhow!("compose: {e}"))?;

        // Persist the prompt.
        let substrate_dir = self.workdir.join(".roko");
        let substrate = FileSubstrate::open(&substrate_dir)
            .await
            .map_err(|e| anyhow!("open substrate: {e}"))?;
        substrate
            .put(prompt.clone())
            .await
            .map_err(|e| anyhow!("persist prompt: {e}"))?;

        // Run the agent.
        // ClaudeCliAgent handles the subprocess wiring internally: the
        // system prompt is forwarded via `--append-system-prompt`, the role
        // allowlist becomes `--tools`, and the safety hooks are passed with
        // `--settings`.
        let result: AgentResult = if self.config.agent.command == "claude" {
            let model = self
                .config
                .agent
                .model
                .clone()
                .unwrap_or_else(|| "claude-opus-4-6".to_string());
            let mut agent = ClaudeCliAgent::new(&self.config.agent.command, &exec_dir, model)
                .with_timeout_ms(self.config.agent.timeout_ms)
                .with_bare_mode(self.config.agent.bare_mode)
                .with_effort(self.config.agent.effort.clone())
                .with_system_prompt(role_instruction.clone())
                .with_tools(claude_tools_csv)
                .with_settings_json(roko_agent::claude_cli_agent::build_settings_json())
                .with_dangerously_skip_permissions(claude_skip_permissions_for_role(role))
                .with_optional_resume(self.claude_resume_session.clone())
                .with_extra_args(self.config.agent.args.clone());
            if let Some(fallback_model) = &self.config.agent.fallback_model {
                agent = agent.with_fallback_model(fallback_model.clone());
            }
            for (k, v) in &self.config.agent.env {
                agent = agent.with_env_var(k, v);
            }
            agent.run(&prompt, &ctx).await
        } else {
            let mut agent =
                ExecAgent::new(&self.config.agent.command, self.config.agent.args.clone())
                    .with_timeout_ms(self.config.agent.timeout_ms);
            for (k, v) in &self.config.agent.env {
                agent = agent.with_env_var(k, v);
            }
            agent.run(&prompt, &ctx).await
        };

        // Persist the output.
        substrate
            .put(result.output.clone())
            .await
            .map_err(|e| anyhow!("persist agent output: {e}"))?;

        if !result.success {
            return Err(anyhow!(
                "agent returned failure for plan={plan_id} task={task}"
            ));
        }

        Ok(result)
    }

    /// Run gates at the specified rung level and return whether all passed.
    async fn run_gate_pipeline(&mut self, plan_id: &str, rung: u32) -> Result<bool> {
        let exec_dir = self.plan_exec_dir(plan_id).await;
        let payload = GatePayload::in_dir(&exec_dir).with_label(format!("{plan_id}:rung-{rung}"));
        let payload_sig = Signal::builder(Kind::Task)
            .body(Body::from_json(&payload)?)
            .provenance(Provenance::trusted("orchestrate"))
            .tag("plan_id", plan_id)
            .tag("rung", rung.to_string())
            .build();

        let verdicts = Self::run_gate_rung(&payload_sig, rung).await;

        // Persist verdicts.
        let substrate_dir = self.workdir.join(".roko");
        if let Ok(substrate) = FileSubstrate::open(&substrate_dir).await {
            for verdict in &verdicts {
                let sig = payload_sig
                    .derive(
                        Kind::GateVerdict,
                        Body::from_json(verdict)
                            .unwrap_or_else(|_| Body::text(format!("{verdict:?}"))),
                    )
                    .provenance(Provenance::trusted("orchestrate"))
                    .tag("gate", &verdict.gate)
                    .tag("passed", verdict.passed.to_string())
                    .build();
                let _ = substrate.put(sig).await;
            }
        }

        // Record gate results on the plan state.
        if let Some(state) = self.executor.plan_state_mut(plan_id) {
            for verdict in &verdicts {
                state
                    .gate_results
                    .push(GateResult::from_verdict(verdict, rung));
            }
        }

        let all_passed = verdicts.iter().all(|v| v.passed);
        Ok(all_passed)
    }

    /// Attempt a git merge for a plan's branch.
    async fn merge_branch(&self, plan_id: &str) -> Result<()> {
        let branch_name = self
            .worktrees
            .get(plan_id)
            .map_or_else(|| format!("roko/{plan_id}"), |h| h.branch);
        let output = tokio::process::Command::new("git")
            .args(["merge", "--no-ff", &branch_name])
            .current_dir(&self.workdir)
            .output()
            .await
            .context("git merge")?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow!("merge failed: {stderr}"))
        }
    }

    async fn run_post_merge_follow_up(&self, plan_id: &str) -> Result<bool> {
        let payload =
            GatePayload::in_dir(&self.workdir).with_label(format!("{plan_id}:post-merge"));
        let payload_sig = Signal::builder(Kind::Task)
            .body(Body::from_json(&payload)?)
            .provenance(Provenance::trusted("orchestrate"))
            .tag("plan_id", plan_id)
            .tag("rung", "post-merge")
            .build();

        let verdicts = Self::run_gate_rung(&payload_sig, 3).await;
        let merged_at_ms = now_unix_ms_i64();
        let (_check, follow_up) =
            self.post_merge
                .run_record_and_follow_up(plan_id, merged_at_ms, &verdicts);

        if follow_up.needs_revert() {
            self.event_log.append(
                EventKind::ErrorOccurred,
                serde_json::json!({
                    "plan_id": plan_id,
                    "error": "post-merge regression detected",
                    "failing_tests": follow_up.failing_tests,
                }),
            );
            return Ok(false);
        }

        Ok(true)
    }

    async fn plan_exec_dir(&self, plan_id: &str) -> PathBuf {
        match self.worktrees.ensure_for_plan(plan_id).await {
            Ok(handle) => handle.path,
            Err(err) => {
                eprintln!(
                    "[orchestrate] worktree unavailable for plan={plan_id}, using repo root: {err}"
                );
                self.workdir.clone()
            }
        }
    }

    async fn run_gate_rung(payload_sig: &Signal, rung: u32) -> Vec<Verdict> {
        let ctx = Context::now();
        // Rung 0 = compile, rung 1 = test, rung 2 = clippy, rung 3+ = all.
        match rung {
            0 => {
                let gate = CompileGate::cargo();
                vec![gate.verify(payload_sig, &ctx).await]
            }
            1 => {
                let gate = TestGate::cargo();
                vec![gate.verify(payload_sig, &ctx).await]
            }
            2 => {
                let gate = ClippyGate::cargo();
                vec![gate.verify(payload_sig, &ctx).await]
            }
            _ => {
                let c = CompileGate::cargo();
                let t = TestGate::cargo();
                let cl = ClippyGate::cargo();
                vec![
                    c.verify(payload_sig, &ctx).await,
                    t.verify(payload_sig, &ctx).await,
                    cl.verify(payload_sig, &ctx).await,
                ]
            }
        }
    }
}

// ─── Role-specific system prompts ────────────────────────────────────────

fn default_worktree_manager(workdir: &Path) -> WorktreeManager {
    let config = WorktreeConfig {
        repo_root: workdir.to_path_buf(),
        base_branch: "HEAD".to_string(),
        worktrees_root: workdir.join(".roko").join("worktrees"),
        max_live: None,
        idle_ttl: Duration::from_secs(DEFAULT_WORKTREE_IDLE_TTL_SECS),
    };
    WorktreeManager::new(config)
}

const fn claude_skip_permissions_for_role(role: AgentRole) -> bool {
    let perms = role.tool_permissions();
    perms.exec || perms.write || perms.git
}

fn normalize_resume_session(session_id: Option<String>) -> Option<String> {
    session_id.and_then(|id| {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn now_unix_ms_i64() -> i64 {
    #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0_i64, |d| d.as_millis() as i64)
    }
}

fn build_system_prompt(role: AgentRole, plan_id: &str, task: &str, tools_csv: &str) -> String {
    RoleSystemPromptSpec::new(
        role,
        TaskContext::new(task)
            .with_plan_id(plan_id)
            .with_workspace("roko-cli orchestration"),
        tools_csv,
    )
    .build()
}

fn claude_tool_allowlist(role: AgentRole) -> String {
    let registry = StaticToolRegistry::new();
    let tools: Vec<_> = registry.for_role(role).into_iter().cloned().collect();
    match ClaudeTranslator.render_tools(&tools) {
        RenderedTools::CliFlag(csv) => csv,
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn orchestration_report_all_succeeded() {
        let report = OrchestrationReport {
            plans: vec![
                PlanRunReport {
                    plan_id: "p1".into(),
                    succeeded: true,
                    agent_calls: 2,
                    gate_results: vec![("compile".into(), true)],
                },
                PlanRunReport {
                    plan_id: "p2".into(),
                    succeeded: true,
                    agent_calls: 1,
                    gate_results: vec![("test".into(), true)],
                },
            ],
            total_agent_calls: 3,
            total_gate_runs: 2,
        };
        assert!(report.all_succeeded());
    }

    #[test]
    fn orchestration_report_partial_failure() {
        let report = OrchestrationReport {
            plans: vec![
                PlanRunReport {
                    plan_id: "p1".into(),
                    succeeded: true,
                    agent_calls: 1,
                    gate_results: vec![],
                },
                PlanRunReport {
                    plan_id: "p2".into(),
                    succeeded: false,
                    agent_calls: 1,
                    gate_results: vec![],
                },
            ],
            total_agent_calls: 2,
            total_gate_runs: 1,
        };
        assert!(!report.all_succeeded());
    }

    #[test]
    fn role_prompt_coverage() {
        let roles = [
            AgentRole::Implementer,
            AgentRole::Auditor,
            AgentRole::Scribe,
            AgentRole::AutoFixer,
            AgentRole::Strategist,
            AgentRole::Researcher,
            AgentRole::Conductor,
        ];
        for role in roles {
            let prompt = roko_compose::role_identity_for(role);
            assert!(!prompt.is_empty(), "empty prompt for {role:?}");
        }
    }

    #[test]
    fn claude_skip_permissions_tracks_role_permissions() {
        assert!(claude_skip_permissions_for_role(AgentRole::Implementer));
        assert!(claude_skip_permissions_for_role(
            AgentRole::IntegrationTester
        ));
        assert!(!claude_skip_permissions_for_role(AgentRole::Auditor));
        assert!(!claude_skip_permissions_for_role(AgentRole::Strategist));
    }

    #[test]
    fn normalize_resume_session_trims_and_drops_blank_values() {
        assert_eq!(normalize_resume_session(None), None);
        assert_eq!(normalize_resume_session(Some(String::new())), None);
        assert_eq!(normalize_resume_session(Some("   ".to_string())), None);
        assert_eq!(
            normalize_resume_session(Some("  sess-42  ".to_string())),
            Some("sess-42".to_string())
        );
    }

    #[test]
    fn default_worktree_manager_paths_under_roko_directory() {
        let workdir = PathBuf::from("/tmp/roko-test");
        let manager = default_worktree_manager(&workdir);
        assert_eq!(
            manager.path_for("plan-1"),
            workdir.join(".roko").join("worktrees").join("plan-1")
        );
    }

    #[test]
    fn post_merge_follow_up_reports_unresolved_regression() {
        let runner = PostMergeRunner::new();
        let (_check, follow_up) =
            runner.run_record_and_follow_up("plan-a", 100, &[Verdict::fail("test", "boom")]);
        assert!(follow_up.needs_revert());
        assert_eq!(runner.unresolved_regressions(), vec!["plan-a".to_string()]);
    }
}
