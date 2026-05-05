//! `roko show` state inspection command.

use crate::*;
use roko_cli::DashboardData;
use roko_fs::RokoLayout;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShowSubject {
    Overview,
    Costs,
    Agents,
    Knowledge,
    Plans,
    Learning,
    History,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ShowTarget {
    Subject(ShowSubject),
    WorkId(String),
}

impl ShowTarget {
    fn parse(subject: Option<String>) -> Self {
        let Some(subject) = subject else {
            return Self::Subject(ShowSubject::Overview);
        };
        let normalized = subject.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "" | "overview" | "summary" => Self::Subject(ShowSubject::Overview),
            "cost" | "costs" => Self::Subject(ShowSubject::Costs),
            "agent" | "agents" => Self::Subject(ShowSubject::Agents),
            "knowledge" | "know" | "neuro" => Self::Subject(ShowSubject::Knowledge),
            "plan" | "plans" => Self::Subject(ShowSubject::Plans),
            "learning" | "learn" | "router" | "routing" => Self::Subject(ShowSubject::Learning),
            "history" | "events" | "log" => Self::Subject(ShowSubject::History),
            _ => Self::WorkId(subject),
        }
    }
}

#[derive(Debug)]
struct ShowState {
    workdir: PathBuf,
    layout: RokoLayout,
    data: DashboardData,
    work_items: Vec<WorkItemSummary>,
}

#[derive(Debug, Clone)]
struct WorkItemSummary {
    id: String,
    kind: String,
    status: String,
    prompt: String,
    tasks_done: Option<usize>,
    tasks_total: Option<usize>,
    cost_usd: Option<f64>,
    created: Option<String>,
    source: PathBuf,
    modified_ms: u64,
}

#[derive(Debug, Default)]
struct CostAggregate {
    turns: usize,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
}

pub(crate) async fn cmd_show(
    cli: &Cli,
    workdir: Option<PathBuf>,
    live: bool,
    subject: Option<String>,
) -> Result<i32> {
    if live {
        return super::dashboard::cmd_dashboard(cli, workdir, None, false, false, None).await;
    }

    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let state = load_show_state(&workdir);
    let output = match ShowTarget::parse(subject) {
        ShowTarget::Subject(ShowSubject::Overview) => render_overview(&state),
        ShowTarget::Subject(ShowSubject::Costs) => render_costs(&state),
        ShowTarget::Subject(ShowSubject::Agents) => render_agents(&state),
        ShowTarget::Subject(ShowSubject::Knowledge) => render_knowledge(&state),
        ShowTarget::Subject(ShowSubject::Plans) => render_plans(&state),
        ShowTarget::Subject(ShowSubject::Learning) => render_learning(&state),
        ShowTarget::Subject(ShowSubject::History) => render_history(&state),
        ShowTarget::WorkId(work_id) => render_work_detail(&state, &work_id)?,
    };

    print!("{output}");
    Ok(EXIT_SUCCESS)
}

fn load_show_state(workdir: &Path) -> ShowState {
    let layout = RokoLayout::for_project(workdir);
    let data = DashboardData::load_best_effort(workdir);
    let work_items = collect_work_items(workdir, &layout, &data);
    ShowState {
        workdir: workdir.to_path_buf(),
        layout,
        data,
        work_items,
    }
}

fn render_overview(state: &ShowState) -> String {
    let mut out = header(state, "overview");
    push_section(&mut out, "work items");
    if state.work_items.is_empty() {
        push_empty(&mut out, "No work items found in .roko/work, .roko/jobs, or plans.");
    } else {
        for item in state.work_items.iter().take(8) {
            push_work_item_row(&mut out, item);
        }
    }

    push_section(&mut out, "agents");
    let agent_rows = agent_rows(state);
    if agent_rows.is_empty() {
        push_empty(
            &mut out,
            "No agents found in .roko/state/executor.json or efficiency events.",
        );
    } else {
        for row in agent_rows.iter().take(6) {
            push_kv(&mut out, &row.id, &row.summary);
        }
    }

    push_section(&mut out, "costs");
    let efficiency = &state.data.efficiency;
    push_kv(
        &mut out,
        "total",
        &format!(
            "{} across {} turn(s)",
            format_cost(efficiency.total_cost_usd),
            efficiency.event_count
        ),
    );
    push_kv(
        &mut out,
        "tokens",
        &format!(
            "{} input, {} output",
            format_count(efficiency.total_input_tokens),
            format_count(efficiency.total_output_tokens)
        ),
    );

    push_section(&mut out, "learning");
    push_learning_summary(state, &mut out);
    push_section(&mut out, "try");
    push_empty(
        &mut out,
        "roko show costs | roko show agents | roko show learning | roko show <work-id>",
    );
    out
}

fn render_costs(state: &ShowState) -> String {
    let mut out = header(state, "costs");
    let efficiency = &state.data.efficiency;
    push_section(&mut out, "summary");
    push_kv(&mut out, "turns", &efficiency.event_count.to_string());
    push_kv(&mut out, "total cost", &format_cost(efficiency.total_cost_usd));
    push_kv(
        &mut out,
        "avg turn cost",
        &format_cost(if efficiency.event_count == 0 {
            0.0
        } else {
            efficiency.total_cost_usd / efficiency.event_count as f64
        }),
    );
    push_kv(
        &mut out,
        "pass rate",
        &format_percent(ratio(efficiency.passed_count, efficiency.event_count)),
    );

    let by_model = cost_by_model(state);
    push_section(&mut out, "by model");
    if by_model.is_empty() {
        push_empty(&mut out, "No model cost events found in .roko/learn/efficiency.jsonl.");
    } else {
        for (model, aggregate) in by_model {
            push_kv(
                &mut out,
                &model,
                &format!(
                    "{} | {} turn(s) | {} in / {} out",
                    format_cost(aggregate.cost_usd),
                    aggregate.turns,
                    format_count(aggregate.input_tokens),
                    format_count(aggregate.output_tokens)
                ),
            );
        }
    }

    let by_task = cost_by_task(state);
    push_section(&mut out, "by task");
    if by_task.is_empty() {
        push_empty(&mut out, "No task cost events found in .roko/learn/efficiency.jsonl.");
    } else {
        for (task, aggregate) in by_task.into_iter().take(12) {
            push_kv(
                &mut out,
                &task,
                &format!("{} | {} turn(s)", format_cost(aggregate.cost_usd), aggregate.turns),
            );
        }
    }

    let by_day = cost_by_day(state);
    push_section(&mut out, "by day");
    if by_day.is_empty() {
        push_empty(&mut out, "No dated cost events found in .roko/learn/efficiency.jsonl.");
    } else {
        for (day, aggregate) in by_day {
            push_kv(
                &mut out,
                &day,
                &format!("{} | {} turn(s)", format_cost(aggregate.cost_usd), aggregate.turns),
            );
        }
    }
    out
}

fn render_agents(state: &ShowState) -> String {
    let mut out = header(state, "agents");
    let rows = agent_rows(state);
    push_section(&mut out, "agents");
    if rows.is_empty() {
        push_empty(
            &mut out,
            "No agents found in .roko/state/executor.json or .roko/learn/efficiency.jsonl.",
        );
    } else {
        for row in rows {
            push_kv(&mut out, &row.id, &row.summary);
        }
    }
    out
}

fn render_knowledge(state: &ShowState) -> String {
    let mut out = header(state, "knowledge");
    let entries = &state.data.knowledge_entries;
    push_section(&mut out, "store");
    push_kv(
        &mut out,
        "path",
        &display_rel(
            &state.workdir,
            &state.layout.root().join("neuro").join("knowledge.jsonl"),
        ),
    );
    push_kv(&mut out, "entries", &entries.len().to_string());

    push_section(&mut out, "recent entries");
    if entries.is_empty() {
        push_empty(&mut out, "No knowledge entries found in .roko/neuro/knowledge.jsonl.");
    } else {
        let mut sorted = entries.clone();
        sorted.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        for entry in sorted.iter().take(12) {
            let tags = if entry.tags.is_empty() {
                String::from("no tags")
            } else {
                entry.tags.join(", ")
            };
            push_kv(
                &mut out,
                &entry.id,
                &format!(
                    "{} | {} | conf {:.2} | {} | {}",
                    entry.kind,
                    entry.tier,
                    entry.confidence,
                    tags,
                    entry.content_preview
                ),
            );
        }
    }
    out
}

fn render_plans(state: &ShowState) -> String {
    let mut out = header(state, "plans");
    if let Some(current) = &state.data.current_plan_execution {
        push_section(&mut out, "current");
        push_kv(&mut out, "plan", &format!("{} | {}", current.plan_id, current.plan_title));
        push_kv(
            &mut out,
            "tasks",
            &format!("{}/{}", current.tasks_done, current.tasks_total),
        );
        if let Some(task) = &current.current_task {
            push_kv(&mut out, "current task", &format!("{} | {}", task.task_id, task.description));
        }
    }

    push_section(&mut out, "all plans");
    if state.data.plans.is_empty() {
        push_empty(&mut out, "No plans found in plans/ or .roko/plans.");
    } else {
        for plan in &state.data.plans {
            let status = plan_status(plan.completed, plan.tasks_done, plan.tasks_failed);
            push_kv(
                &mut out,
                &plan.id,
                &format!(
                    "{} | {} | tasks {}/{} done, {} failed",
                    status, plan.title, plan.tasks_done, plan.task_count, plan.tasks_failed
                ),
            );
            if let Some(error) = plan.last_error.as_deref().filter(|error| !error.is_empty()) {
                push_kv(&mut out, "last error", error);
            }
        }
    }
    out
}

fn render_learning(state: &ShowState) -> String {
    let mut out = header(state, "learning");
    push_section(&mut out, "routing");
    let router = &state.data.cascade_router;
    if router.model_slugs.is_empty() && router.confidence_stats.is_empty() {
        push_empty(&mut out, "No cascade router state found in .roko/learn/cascade-router.json.");
    } else {
        push_kv(&mut out, "models", &router.model_slugs.join(", "));
        for (model, stats) in router.confidence_stats.iter() {
            push_kv(
                &mut out,
                model,
                &format!(
                    "{} success over {} trial(s)",
                    format_percent(ratio(stats.successes as usize, stats.trials as usize)),
                    stats.trials
                ),
            );
        }
    }

    push_section(&mut out, "experiments");
    if state.data.experiments.is_empty() {
        push_empty(&mut out, "No prompt experiments found in .roko/learn/experiments.json.");
    } else {
        for experiment in state.data.experiments.iter().take(10) {
            let winner = experiment.winner_id.as_deref().unwrap_or("none");
            push_kv(
                &mut out,
                &experiment.experiment_id,
                &format!(
                    "{} | {} variant(s) | {} trial(s) | winner {}",
                    experiment.status, experiment.active_variants, experiment.total_trials, winner
                ),
            );
        }
    }

    push_section(&mut out, "gates");
    if state.data.gate_results_page.gate_rows.is_empty() {
        push_empty(&mut out, "No gate signal rows found in .roko/engrams.jsonl.");
    } else {
        for gate in state.data.gate_results_page.gate_rows.iter().take(10) {
            push_kv(
                &mut out,
                &gate.gate_name,
                &format!(
                    "{} pass | {} run(s) | avg {}",
                    format_percent(gate.pass_rate),
                    gate.total_runs,
                    format_duration_ms(gate.avg_duration_ms)
                ),
            );
        }
    }

    push_section(&mut out, "c-factor");
    if let Some(cfactor) = &state.data.cfactor {
        push_kv(&mut out, "overall", &format!("{:.2}", cfactor.overall));
        push_kv(
            &mut out,
            "cost efficiency",
            &format_percent(cfactor.components.cost_efficiency),
        );
        push_kv(
            &mut out,
            "knowledge growth",
            &format_percent(cfactor.components.knowledge_growth),
        );
    } else {
        push_empty(&mut out, "No C-Factor snapshots found in .roko/learn/c-factor.jsonl.");
    }
    out
}

fn render_history(state: &ShowState) -> String {
    let mut out = header(state, "history");
    push_section(&mut out, "state events");
    if state.data.event_log.is_empty() {
        push_empty(&mut out, "No event log entries found in .roko/state/events.json.");
    } else {
        let mut events = state.data.event_log.clone();
        events.sort_by_key(|event| event.timestamp_ms);
        for event in events.iter().rev().take(20).rev() {
            let scope = event_scope(&event.plan_id, &event.task_id);
            push_kv(
                &mut out,
                &event.event_type,
                &format!("{} | {}", scope, event.message),
            );
        }
    }

    push_section(&mut out, "recent turns");
    if state.data.efficiency_events.is_empty() {
        push_empty(&mut out, "No efficiency events found in .roko/learn/efficiency.jsonl.");
    } else {
        for event in state.data.efficiency_events.iter().rev().take(12).rev() {
            push_kv(
                &mut out,
                &event.timestamp,
                &format!(
                    "{}:{} | {} | {} | {}",
                    event.plan_id,
                    event.task_id,
                    event.agent_id,
                    event.model,
                    format_cost(event.cost_usd)
                ),
            );
        }
    }
    out
}

fn render_work_detail(state: &ShowState, work_id: &str) -> Result<String> {
    let Some(item) = state.work_items.iter().find(|item| item.id == work_id) else {
        anyhow::bail!(
            "no work item `{work_id}` found in .roko/work, .roko/jobs, plans, or active execution state"
        );
    };

    let mut out = header(state, work_id);
    push_section(&mut out, "work item");
    push_kv(&mut out, "id", &item.id);
    push_kv(&mut out, "kind", &item.kind);
    push_kv(&mut out, "status", &item.status);
    push_kv(&mut out, "source", &display_rel(&state.workdir, &item.source));
    if !item.prompt.is_empty() {
        push_kv(&mut out, "prompt", &item.prompt);
    }
    if let Some(created) = &item.created {
        push_kv(&mut out, "created", created);
    }
    if item.tasks_done.is_some() || item.tasks_total.is_some() {
        push_kv(
            &mut out,
            "tasks",
            &format!(
                "{}/{}",
                item.tasks_done.unwrap_or_default(),
                item.tasks_total.unwrap_or_default()
            ),
        );
    }
    if let Some(cost) = item.cost_usd {
        push_kv(&mut out, "cost", &format_cost(cost));
    }

    if let Some(plan) = state.data.plans.iter().find(|plan| plan.id == item.id) {
        push_section(&mut out, "plan");
        push_kv(&mut out, "title", &plan.title);
        push_kv(
            &mut out,
            "tasks",
            &format!("{}/{} done, {} failed", plan.tasks_done, plan.task_count, plan.tasks_failed),
        );
        if let Some(error) = plan.last_error.as_deref().filter(|error| !error.is_empty()) {
            push_kv(&mut out, "last error", error);
        }
    }

    let related_costs = state
        .data
        .efficiency_events
        .iter()
        .filter(|event| event.plan_id == item.id || event.task_id == item.id)
        .fold(CostAggregate::default(), |mut aggregate, event| {
            aggregate.turns += 1;
            aggregate.input_tokens += event.input_tokens;
            aggregate.output_tokens += event.output_tokens;
            aggregate.cost_usd += event.cost_usd;
            aggregate
        });
    push_section(&mut out, "costs");
    if related_costs.turns == 0 {
        push_empty(&mut out, "No related efficiency events found.");
    } else {
        push_kv(&mut out, "turns", &related_costs.turns.to_string());
        push_kv(&mut out, "cost", &format_cost(related_costs.cost_usd));
        push_kv(
            &mut out,
            "tokens",
            &format!(
                "{} input, {} output",
                format_count(related_costs.input_tokens),
                format_count(related_costs.output_tokens)
            ),
        );
    }

    push_section(&mut out, "history");
    let mut wrote_event = false;
    for event in state
        .data
        .event_log
        .iter()
        .filter(|event| event.plan_id == item.id || event.task_id == item.id)
        .take(12)
    {
        wrote_event = true;
        push_kv(
            &mut out,
            &event.event_type,
            &format!("{} | {}", event_scope(&event.plan_id, &event.task_id), event.message),
        );
    }
    if !wrote_event {
        push_empty(&mut out, "No related events found in .roko/state/events.json.");
    }

    Ok(out)
}

fn collect_work_items(
    workdir: &Path,
    layout: &RokoLayout,
    data: &DashboardData,
) -> Vec<WorkItemSummary> {
    let mut items = BTreeMap::<String, WorkItemSummary>::new();
    let plan_costs = cost_by_plan(data);

    for plan in &data.plans {
        let plan_path = roko_cli::plan::plans_dir(workdir).join(&plan.id);
        items.insert(
            plan.id.clone(),
            WorkItemSummary {
                id: plan.id.clone(),
                kind: String::from("plan"),
                status: plan_status(plan.completed, plan.tasks_done, plan.tasks_failed),
                prompt: plan.title.clone(),
                tasks_done: Some(plan.tasks_done),
                tasks_total: Some(plan.task_count),
                cost_usd: plan_costs.get(&plan.id).copied(),
                created: None,
                source: plan_path.clone(),
                modified_ms: path_modified_ms(&plan_path),
            },
        );
    }

    for path in read_json_paths(&layout.root().join("jobs")) {
        if let Some(item) = work_item_from_json_path(&path, "job") {
            items.insert(item.id.clone(), item);
        }
    }

    for path in read_json_paths(&layout.root().join("work")) {
        if let Some(item) = work_item_from_json_path(&path, "work") {
            items.insert(item.id.clone(), item);
        }
    }

    if let Some(current) = &data.current_plan_execution {
        if !current.plan_id.is_empty() {
            items.insert(
                current.plan_id.clone(),
                WorkItemSummary {
                    id: current.plan_id.clone(),
                    kind: String::from("current-plan"),
                    status: String::from("running"),
                    prompt: current.plan_title.clone(),
                    tasks_done: Some(current.tasks_done),
                    tasks_total: Some(current.tasks_total),
                    cost_usd: plan_costs.get(&current.plan_id).copied(),
                    created: None,
                    source: layout.executor_snapshot(),
                    modified_ms: path_modified_ms(&layout.executor_snapshot()),
                },
            );
        }
    }

    let mut values = items.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .modified_ms
            .cmp(&left.modified_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    values
}

fn work_item_from_json_path(path: &Path, kind: &str) -> Option<WorkItemSummary> {
    let text = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&text).ok()?;
    let id = value_string(&value, &["id", "work_id", "job_id"])
        .or_else(|| file_stem(path))
        .unwrap_or_else(|| String::from("unknown"));
    let status = value_string(&value, &["status", "state"])
        .unwrap_or_else(|| String::from("recorded"));
    let prompt = value_string(&value, &["prompt", "intent", "title", "description"])
        .unwrap_or_default();
    let tasks_done = value_usize_path(
        &value,
        &[
            &["tasks_completed"][..],
            &["tasks_done"][..],
            &["progress", "done"][..],
            &["cost", "tasks_completed"][..],
        ],
    );
    let tasks_total = value_usize_path(
        &value,
        &[
            &["tasks_total"][..],
            &["task_count"][..],
            &["progress", "total"][..],
            &["cost", "tasks_total"][..],
        ],
    );
    let cost_usd = value_f64_path(
        &value,
        &[
            &["cost_usd"][..],
            &["total_cost_usd"][..],
            &["cost", "total_usd"][..],
            &["cost", "usd"][..],
            &["cost", "total"][..],
            &["cost_summary", "total_usd"][..],
        ],
    );
    let created = value_string(&value, &["created", "created_at", "started_at", "updated_at"]);
    Some(WorkItemSummary {
        id,
        kind: String::from(kind),
        status,
        prompt,
        tasks_done,
        tasks_total,
        cost_usd,
        created,
        source: path.to_path_buf(),
        modified_ms: path_modified_ms(path),
    })
}

fn read_json_paths(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn cost_by_model(state: &ShowState) -> BTreeMap<String, CostAggregate> {
    let mut rows = BTreeMap::<String, CostAggregate>::new();
    for event in &state.data.efficiency_events {
        let aggregate = rows.entry(non_empty(&event.model, "unknown")).or_default();
        aggregate.turns += 1;
        aggregate.input_tokens += event.input_tokens;
        aggregate.output_tokens += event.output_tokens;
        aggregate.cost_usd += event.cost_usd;
    }
    rows
}

fn cost_by_task(state: &ShowState) -> Vec<(String, CostAggregate)> {
    let mut rows = BTreeMap::<String, CostAggregate>::new();
    for event in &state.data.efficiency_events {
        let task = if event.plan_id.is_empty() {
            non_empty(&event.task_id, "unknown-task")
        } else {
            format!("{}:{}", event.plan_id, non_empty(&event.task_id, "task"))
        };
        let aggregate = rows.entry(task).or_default();
        aggregate.turns += 1;
        aggregate.input_tokens += event.input_tokens;
        aggregate.output_tokens += event.output_tokens;
        aggregate.cost_usd += event.cost_usd;
    }
    let mut rows = rows.into_iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .1
            .cost_usd
            .partial_cmp(&left.1.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    rows
}

fn cost_by_day(state: &ShowState) -> BTreeMap<String, CostAggregate> {
    let mut rows = BTreeMap::<String, CostAggregate>::new();
    for event in &state.data.efficiency_events {
        let day = if event.timestamp.len() >= 10 {
            event.timestamp[..10].to_string()
        } else {
            String::from("undated")
        };
        let aggregate = rows.entry(day).or_default();
        aggregate.turns += 1;
        aggregate.input_tokens += event.input_tokens;
        aggregate.output_tokens += event.output_tokens;
        aggregate.cost_usd += event.cost_usd;
    }
    rows
}

fn cost_by_plan(data: &DashboardData) -> HashMap<String, f64> {
    let mut rows = HashMap::<String, f64>::new();
    for event in &data.efficiency_events {
        if !event.plan_id.is_empty() {
            *rows.entry(event.plan_id.clone()).or_default() += event.cost_usd;
        }
    }
    rows
}

#[derive(Debug)]
struct AgentRow {
    id: String,
    summary: String,
}

fn agent_rows(state: &ShowState) -> Vec<AgentRow> {
    let mut by_agent = BTreeMap::<String, AgentRow>::new();
    for agent in &state.data.agents {
        let scope = agent.plan_id.as_deref().unwrap_or("workspace");
        by_agent.insert(
            agent.id.clone(),
            AgentRow {
                id: agent.id.clone(),
                summary: format!("{} | {} | {}", agent.label, agent.status, scope),
            },
        );
    }

    for event in &state.data.efficiency_events {
        by_agent.entry(event.agent_id.clone()).or_insert_with(|| AgentRow {
            id: event.agent_id.clone(),
            summary: format!(
                "{} | {} | {} | last {}",
                non_empty(&event.role, "agent"),
                non_empty(&event.model, "unknown-model"),
                non_empty(&event.plan_id, "workspace"),
                non_empty(&event.timestamp, "unknown-time")
            ),
        });
    }

    by_agent.into_values().collect()
}

fn push_learning_summary(state: &ShowState, out: &mut String) {
    let router = &state.data.cascade_router;
    let mut trials = 0usize;
    let mut successes = 0usize;
    for stats in router.confidence_stats.values() {
        trials += stats.trials as usize;
        successes += stats.successes as usize;
    }
    push_kv(out, "routing confidence", &format_percent(ratio(successes, trials)));
    push_kv(out, "routing trials", &trials.to_string());
    push_kv(out, "experiments", &state.data.experiments.len().to_string());
    if let Some(cfactor) = &state.data.cfactor {
        push_kv(out, "c-factor", &format!("{:.2}", cfactor.overall));
    } else {
        push_kv(out, "c-factor", "no snapshots");
    }
}

fn push_work_item_row(out: &mut String, item: &WorkItemSummary) {
    let tasks = match (item.tasks_done, item.tasks_total) {
        (Some(done), Some(total)) => format!(" | tasks {done}/{total}"),
        _ => String::new(),
    };
    let cost = item
        .cost_usd
        .map(|cost| format!(" | {}", format_cost(cost)))
        .unwrap_or_default();
    let prompt = if item.prompt.is_empty() {
        String::new()
    } else {
        format!(" | {}", truncate(&item.prompt, 64))
    };
    push_kv(
        out,
        &item.id,
        &format!("{} | {}{}{}{}", item.status, item.kind, tasks, cost, prompt),
    );
}

fn header(state: &ShowState, title: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "roko show {title}");
    let _ = writeln!(out, "workspace: {}", state.workdir.display());
    let _ = writeln!(out, "state: {}", state.layout.root().display());
    out
}

fn push_section(out: &mut String, title: &str) {
    let _ = writeln!(out);
    let _ = writeln!(out, "{}", title.to_ascii_uppercase());
}

fn push_kv(out: &mut String, key: &str, value: &str) {
    let _ = writeln!(out, "  {:<22} {}", truncate(key, 22), value);
}

fn push_empty(out: &mut String, message: &str) {
    let _ = writeln!(out, "  {message}");
}

fn plan_status(completed: bool, tasks_done: usize, tasks_failed: usize) -> String {
    if completed {
        String::from("done")
    } else if tasks_failed > 0 {
        String::from("failed")
    } else if tasks_done > 0 {
        String::from("running")
    } else {
        String::from("pending")
    }
}

fn event_scope(plan_id: &str, task_id: &str) -> String {
    match (plan_id.is_empty(), task_id.is_empty()) {
        (true, true) => String::from("workspace"),
        (false, true) => plan_id.to_string(),
        (true, false) => task_id.to_string(),
        (false, false) => format!("{plan_id}:{task_id}"),
    }
}

fn display_rel(workdir: &Path, path: &Path) -> String {
    path.strip_prefix(workdir)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn path_modified_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
}

fn value_string(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(found) = value.get(*key) else {
            continue;
        };
        if let Some(text) = found.as_str().filter(|text| !text.trim().is_empty()) {
            return Some(text.to_string());
        }
        if found.is_number() || found.is_boolean() {
            return Some(found.to_string());
        }
    }
    None
}

fn value_usize_path(value: &Value, paths: &[&[&str]]) -> Option<usize> {
    value_number_path(value, paths).and_then(|number| {
        if number >= 0.0 {
            Some(number as usize)
        } else {
            None
        }
    })
}

fn value_f64_path(value: &Value, paths: &[&[&str]]) -> Option<f64> {
    value_number_path(value, paths)
}

fn value_number_path(value: &Value, paths: &[&[&str]]) -> Option<f64> {
    for path in paths {
        let mut current = value;
        let mut missing = false;
        for key in *path {
            if let Some(next) = current.get(*key) {
                current = next;
            } else {
                missing = true;
                break;
            }
        }
        if missing {
            continue;
        }
        if let Some(number) = current.as_f64() {
            return Some(number);
        }
        if let Some(text) = current.as_str() {
            if let Ok(number) = text.parse::<f64>() {
                return Some(number);
            }
        }
    }
    None
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn format_percent(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

fn format_cost(value: f64) -> String {
    format!("${value:.4}")
}

fn format_count(value: u64) -> String {
    let text = value.to_string();
    let mut out = String::new();
    for (idx, ch) in text.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn format_duration_ms(value: f64) -> String {
    if value >= 1000.0 {
        format!("{:.2}s", value / 1000.0)
    } else {
        format!("{value:.0}ms")
    }
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    out.push('.');
    out
}
