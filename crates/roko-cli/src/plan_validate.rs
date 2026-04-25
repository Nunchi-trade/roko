use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use roko_core::AgentRole;
use roko_core::config::schema::ModelProfile;
use roko_gate::AcceptanceContract;
use roko_orchestrator::detect_cycle_nodes;
use serde::Serialize;
use toml::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warn",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub rule_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanDiagnostics {
    pub plan_id: String,
    pub path: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct Totals {
    pub errors: usize,
    pub warnings: usize,
    pub plans_checked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationReport {
    pub plans: Vec<PlanDiagnostics>,
    pub totals: Totals,
}

impl ValidationReport {
    #[must_use]
    pub fn exit_code(&self, strict: bool) -> i32 {
        if self.totals.errors > 0 || (strict && self.totals.warnings > 0) {
            1
        } else {
            0
        }
    }
}

#[derive(Debug, Clone)]
struct TaskSnapshot {
    ordinal: usize,
    task_id: Option<String>,
    title: Option<String>,
    role: Option<String>,
    model: Option<String>,
    depends_on: Vec<String>,
    has_depends_on_field: bool,
    gate_rung: Option<u32>,
    gate_rung_invalid: bool,
    has_files: bool,
    has_context_read_files: bool,
    has_verify_steps: bool,
    acceptance_contract: Option<Value>,
}

impl TaskSnapshot {
    fn label(&self) -> String {
        self.task_id
            .clone()
            .unwrap_or_else(|| format!("task #{}", self.ordinal))
    }
}

pub fn validate_plans_dir(
    dir: &Path,
    models: Option<&HashMap<String, ModelProfile>>,
) -> Result<ValidationReport> {
    let tasks_files = collect_tasks_files(dir)?;
    let mut plans = Vec::with_capacity(tasks_files.len());
    let mut totals = Totals {
        plans_checked: tasks_files.len(),
        ..Totals::default()
    };

    for tasks_path in tasks_files {
        let plan = validate_tasks_file(&tasks_path, models)
            .with_context(|| format!("validate {}", tasks_path.display()))?;
        for diagnostic in &plan.diagnostics {
            match diagnostic.severity {
                Severity::Error => totals.errors += 1,
                Severity::Warning => totals.warnings += 1,
            }
        }
        if !plan.diagnostics.is_empty() {
            plans.push(plan);
        }
    }

    Ok(ValidationReport { plans, totals })
}

pub fn render_text(report: &ValidationReport) -> String {
    let mut out = String::new();
    let mut printed_plan = false;

    for plan in report
        .plans
        .iter()
        .filter(|plan| !plan.diagnostics.is_empty())
    {
        if printed_plan {
            out.push('\n');
        }
        printed_plan = true;

        let _ = writeln!(out, "{}", plan.path);
        for diagnostic in &plan.diagnostics {
            let _ = writeln!(
                out,
                "  {:<5} {:<8} {}",
                diagnostic.severity.label(),
                diagnostic.rule_id,
                diagnostic.message
            );
        }
    }

    let diagnostic_count = report.totals.errors + report.totals.warnings;
    let plan_word = if report.totals.plans_checked == 1 {
        "plan"
    } else {
        "plans"
    };
    let _ = write!(
        out,
        "{diagnostic_count} diagnostics in {} {plan_word}",
        report.totals.plans_checked
    );
    out
}

pub fn render_json(report: &ValidationReport) -> Result<String> {
    serde_json::to_string_pretty(report).context("serialize plan validation report")
}

fn collect_tasks_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if dir.is_file() {
        if dir.file_name().is_some_and(|name| name == "tasks.toml") {
            return Ok(vec![dir.to_path_buf()]);
        }
        bail!(
            "{} is not a plans directory or tasks.toml file",
            dir.display()
        );
    }

    if !dir.exists() {
        bail!("{} does not exist", dir.display());
    }
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    let mut out = Vec::new();
    collect_tasks_files_recursive(dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_tasks_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("read directory {}", dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("read directory entries for {}", dir.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_tasks_files_recursive(&path, out)?;
        } else if path.is_file() && path.file_name().is_some_and(|name| name == "tasks.toml") {
            out.push(path);
        }
    }

    Ok(())
}

fn validate_tasks_file(
    tasks_path: &Path,
    models: Option<&HashMap<String, ModelProfile>>,
) -> Result<PlanDiagnostics> {
    let content = std::fs::read_to_string(tasks_path)
        .with_context(|| format!("read {}", tasks_path.display()))?;
    let path = tasks_path.display().to_string();
    let fallback_plan_id = tasks_path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown-plan".to_string());

    let parsed: Value = match toml::from_str(&content) {
        Ok(value) => value,
        Err(error) => {
            return Ok(PlanDiagnostics {
                plan_id: fallback_plan_id.clone(),
                path,
                diagnostics: vec![Diagnostic {
                    severity: Severity::Error,
                    rule_id: "PLAN_001".to_string(),
                    plan_id: Some(fallback_plan_id),
                    task_id: None,
                    message: format!("failed to parse TOML: {error}"),
                }],
            });
        }
    };

    let plan_id = parsed
        .get("meta")
        .and_then(Value::as_table)
        .and_then(|meta| meta.get("plan"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or(fallback_plan_id);
    let is_architecture_queue = parsed
        .get("meta")
        .and_then(Value::as_table)
        .is_some_and(is_architecture_queue_meta);

    let mut diagnostics = Vec::new();
    let tasks = parsed
        .get("task")
        .and_then(Value::as_array)
        .map(|array| {
            array
                .iter()
                .enumerate()
                .map(|(index, task)| snapshot_task(index + 1, task))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if tasks.is_empty() {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            rule_id: "PLAN_002".to_string(),
            plan_id: Some(plan_id.clone()),
            task_id: None,
            message: "task array is missing or empty".to_string(),
        });
    }

    for task in &tasks {
        emit_required_field(
            &mut diagnostics,
            &plan_id,
            task,
            "id",
            task.task_id.as_deref(),
        );
        emit_required_field(
            &mut diagnostics,
            &plan_id,
            task,
            "title",
            task.title.as_deref(),
        );
        emit_required_field(
            &mut diagnostics,
            &plan_id,
            task,
            "role",
            task.role.as_deref(),
        );

        if task.gate_rung_invalid || task.gate_rung.is_some_and(|gate_rung| gate_rung > 6) {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                rule_id: "PLAN_007".to_string(),
                plan_id: Some(plan_id.clone()),
                task_id: task.task_id.clone(),
                message: format!(
                    "task '{}' uses invalid gate_rung; expected an integer in 0..=6",
                    task.label()
                ),
            });
        }

        if task.gate_rung == Some(0) && !task.has_verify_steps {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                rule_id: "PLAN_011".to_string(),
                plan_id: Some(plan_id.clone()),
                task_id: task.task_id.clone(),
                message: format!(
                    "task '{}' sets gate_rung = 0 but has no verify steps",
                    task.label()
                ),
            });
        }

        if let Some(contract_value) = &task.acceptance_contract {
            match contract_value.clone().try_into::<AcceptanceContract>() {
                Ok(contract) => {
                    let decision = contract.validate_contract();
                    for issue in decision.issues {
                        diagnostics.push(Diagnostic {
                            severity: if issue.blocking {
                                Severity::Error
                            } else {
                                Severity::Warning
                            },
                            rule_id: issue.code,
                            plan_id: Some(plan_id.clone()),
                            task_id: task.task_id.clone(),
                            message: format!(
                                "task '{}' has invalid acceptance_contract: {}",
                                task.label(),
                                issue.message
                            ),
                        });
                    }
                }
                Err(error) => diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    rule_id: "PLAN_012".to_string(),
                    plan_id: Some(plan_id.clone()),
                    task_id: task.task_id.clone(),
                    message: format!(
                        "task '{}' has malformed acceptance_contract: {error}",
                        task.label()
                    ),
                }),
            }
        }

        if is_architecture_queue {
            validate_architecture_queue_task(&mut diagnostics, &plan_id, task);
        }
    }

    let mut seen_ids = HashSet::new();
    let mut duplicate_ids = BTreeSet::new();
    let known_ids = tasks
        .iter()
        .filter_map(|task| normalized_field(task.task_id.as_deref()))
        .map(ToOwned::to_owned)
        .collect::<HashSet<_>>();

    for task in &tasks {
        if let Some(task_id) = normalized_field(task.task_id.as_deref()) {
            if !seen_ids.insert(task_id.to_string()) {
                duplicate_ids.insert(task_id.to_string());
            }
        }
    }

    for task_id in duplicate_ids {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            rule_id: "PLAN_004".to_string(),
            plan_id: Some(plan_id.clone()),
            task_id: Some(task_id.clone()),
            message: format!("task id '{task_id}' is duplicated within this plan"),
        });
    }

    for task in &tasks {
        for dependency in &task.depends_on {
            if !known_ids.contains(dependency.as_str()) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    rule_id: "PLAN_005".to_string(),
                    plan_id: Some(plan_id.clone()),
                    task_id: task.task_id.clone(),
                    message: format!(
                        "task '{}' depends on unknown task '{}'",
                        task.label(),
                        dependency
                    ),
                });
            }
        }
    }

    let mut valid_deps = BTreeMap::<String, BTreeSet<String>>::new();
    for task in &tasks {
        if let Some(task_id) = normalized_field(task.task_id.as_deref()) {
            valid_deps.insert(
                task_id.to_string(),
                task.depends_on
                    .iter()
                    .filter(|dependency| known_ids.contains(dependency.as_str()))
                    .cloned()
                    .collect(),
            );
        }
    }

    let cycle_nodes = detect_cycle_nodes(&valid_deps);
    for cycle_node in &cycle_nodes {
        let peers = cycle_nodes
            .iter()
            .filter(|candidate| *candidate != cycle_node)
            .cloned()
            .collect::<Vec<_>>();
        let message = if peers.is_empty() {
            format!("task '{cycle_node}' participates in a cycle")
        } else {
            format!(
                "task '{cycle_node}' forms a cycle with {}",
                peers
                    .iter()
                    .map(|peer| format!("'{peer}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            rule_id: "PLAN_006".to_string(),
            plan_id: Some(plan_id.clone()),
            task_id: Some(cycle_node.clone()),
            message,
        });
    }

    let reachable = reachable_from_roots(&tasks, &known_ids);
    for task in &tasks {
        if let Some(task_id) = normalized_field(task.task_id.as_deref())
            && !reachable.contains(task_id)
        {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                rule_id: "PLAN_010".to_string(),
                plan_id: Some(plan_id.clone()),
                task_id: Some(task_id.to_string()),
                message: format!("task '{task_id}' is reachable from no root"),
            });
        }
    }

    for task in &tasks {
        if let Some(role) = normalized_field(task.role.as_deref()) {
            let has_template = parse_task_role(role).is_some_and(role_has_compose_template);
            if !has_template {
                diagnostics.push(Diagnostic {
                    severity: Severity::Warning,
                    rule_id: "PLAN_008".to_string(),
                    plan_id: Some(plan_id.clone()),
                    task_id: task.task_id.clone(),
                    message: format!(
                        "task '{}' uses role '{}' which has no template",
                        task.label(),
                        role
                    ),
                });
            }
        }

        if let (Some(model), Some(known_models)) = (normalized_field(task.model.as_deref()), models)
            && !model_is_known(model, known_models)
        {
            diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                rule_id: "PLAN_009".to_string(),
                plan_id: Some(plan_id.clone()),
                task_id: task.task_id.clone(),
                message: format!(
                    "task '{}' uses model '{}' which is not configured in roko.toml",
                    task.label(),
                    model
                ),
            });
        }
    }

    diagnostics.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.rule_id.cmp(&right.rule_id))
            .then_with(|| left.task_id.cmp(&right.task_id))
            .then_with(|| left.message.cmp(&right.message))
    });

    Ok(PlanDiagnostics {
        plan_id,
        path,
        diagnostics,
    })
}

fn snapshot_task(ordinal: usize, task: &Value) -> TaskSnapshot {
    let table = task.as_table();
    let gate_rung_value = table.and_then(|table| table.get("gate_rung"));

    TaskSnapshot {
        ordinal,
        task_id: table.and_then(|table| string_field(table.get("id"))),
        title: table.and_then(|table| string_field(table.get("title"))),
        role: table.and_then(|table| string_field(table.get("role"))),
        model: table.and_then(|table| {
            string_field(table.get("model")).or_else(|| string_field(table.get("model_hint")))
        }),
        has_depends_on_field: table.is_some_and(|table| table.contains_key("depends_on")),
        depends_on: table
            .and_then(|table| table.get("depends_on"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| string_field(Some(item)))
                    .collect()
            })
            .unwrap_or_default(),
        gate_rung: gate_rung_value
            .and_then(Value::as_integer)
            .and_then(|value| u32::try_from(value).ok()),
        gate_rung_invalid: gate_rung_value.is_some()
            && gate_rung_value
                .and_then(Value::as_integer)
                .and_then(|value| u32::try_from(value).ok())
                .is_none(),
        has_files: table
            .and_then(|table| table.get("files"))
            .or_else(|| table.and_then(|table| table.get("write_files")))
            .and_then(Value::as_array)
            .is_some_and(|files| files.iter().any(|file| string_field(Some(file)).is_some())),
        has_context_read_files: table
            .and_then(|table| table.get("context"))
            .and_then(Value::as_table)
            .and_then(|context| context.get("read_files"))
            .and_then(Value::as_array)
            .is_some_and(|files| {
                files.iter().any(|file| {
                    file.as_table()
                        .and_then(|table| table.get("path"))
                        .and_then(Value::as_str)
                        .is_some_and(|path| !path.trim().is_empty())
                })
            }),
        has_verify_steps: table
            .and_then(|table| table.get("verify"))
            .and_then(Value::as_array)
            .is_some_and(|steps| !steps.is_empty()),
        acceptance_contract: table
            .and_then(|table| table.get("acceptance_contract"))
            .cloned(),
    }
}

fn is_architecture_queue_meta(meta: &toml::map::Map<String, Value>) -> bool {
    ["queue_kind", "queue_schema", "kind"].iter().any(|field| {
        meta.get(*field)
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim() == "architecture_implementation")
    })
}

fn validate_architecture_queue_task(
    diagnostics: &mut Vec<Diagnostic>,
    plan_id: &str,
    task: &TaskSnapshot,
) {
    let requirements = [
        (
            !task.has_depends_on_field,
            "PLAN_020",
            "declares no depends_on array for dependency metadata",
        ),
        (
            !task.has_context_read_files,
            "PLAN_021",
            "declares no context.read_files source docs",
        ),
        (
            !task.has_files,
            "PLAN_022",
            "declares no files list for likely crates/artifacts",
        ),
        (
            !task.has_verify_steps,
            "PLAN_023",
            "declares no executable verify steps",
        ),
        (
            task.acceptance_contract.is_none(),
            "PLAN_024",
            "declares no typed acceptance_contract",
        ),
    ];

    for (missing, rule_id, message) in requirements {
        if missing {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                rule_id: rule_id.to_string(),
                plan_id: Some(plan_id.to_string()),
                task_id: task.task_id.clone(),
                message: format!("architecture queue task '{}' {message}", task.label()),
            });
        }
    }
}

fn string_field(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalized_field(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn emit_required_field(
    diagnostics: &mut Vec<Diagnostic>,
    plan_id: &str,
    task: &TaskSnapshot,
    field: &'static str,
    value: Option<&str>,
) {
    if value.is_some_and(|value| !value.trim().is_empty()) {
        return;
    }

    diagnostics.push(Diagnostic {
        severity: Severity::Error,
        rule_id: "PLAN_003".to_string(),
        plan_id: Some(plan_id.to_string()),
        task_id: task.task_id.clone(),
        message: format!(
            "task '{}' is missing required field '{}'",
            task.label(),
            field
        ),
    });
}

fn reachable_from_roots(tasks: &[TaskSnapshot], known_ids: &HashSet<String>) -> HashSet<String> {
    let mut dependents = HashMap::<String, Vec<String>>::new();
    let mut roots = Vec::new();

    for task in tasks {
        let Some(task_id) = normalized_field(task.task_id.as_deref()) else {
            continue;
        };

        let valid_deps = task
            .depends_on
            .iter()
            .filter(|dependency| known_ids.contains(dependency.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        if valid_deps.is_empty() && task.depends_on.is_empty() {
            roots.push(task_id.to_string());
        }

        for dependency in valid_deps {
            dependents
                .entry(dependency)
                .or_default()
                .push(task_id.to_string());
        }
    }

    roots.sort();
    for children in dependents.values_mut() {
        children.sort();
    }

    let mut reachable = HashSet::new();
    let mut stack = roots;
    while let Some(task_id) = stack.pop() {
        if !reachable.insert(task_id.clone()) {
            continue;
        }
        if let Some(children) = dependents.get(&task_id) {
            for child in children.iter().rev() {
                stack.push(child.clone());
            }
        }
    }

    reachable
}

fn role_has_compose_template(role: AgentRole) -> bool {
    matches!(
        role,
        AgentRole::Strategist
            | AgentRole::Implementer
            | AgentRole::Architect
            | AgentRole::Auditor
            | AgentRole::QuickReviewer
            | AgentRole::Scribe
            | AgentRole::Critic
            | AgentRole::AutoFixer
            | AgentRole::IntegrationTester
            | AgentRole::Refactorer
    )
}

fn parse_task_role(role: &str) -> Option<AgentRole> {
    let normalized = role.trim().to_ascii_lowercase();
    let normalized = normalized
        .strip_prefix("agentrole::")
        .unwrap_or(&normalized)
        .replace(['_', ' '], "-");
    Some(match normalized.as_str() {
        "conductor" => AgentRole::Conductor,
        "strategist" => AgentRole::Strategist,
        "implementer" | "engineer" | "coder" => AgentRole::Implementer,
        "architect" => AgentRole::Architect,
        "researcher" => AgentRole::Researcher,
        "auditor" => AgentRole::Auditor,
        "quick-reviewer" | "quickreviewer" => AgentRole::QuickReviewer,
        "scribe" => AgentRole::Scribe,
        "critic" => AgentRole::Critic,
        "auto-fixer" | "autofixer" => AgentRole::AutoFixer,
        "refactorer" => AgentRole::Refactorer,
        "pre-planner" | "preplanner" => AgentRole::PrePlanner,
        "doc-verifier" | "docverifier" => AgentRole::DocVerifier,
        "integration-tester" | "integrationtester" => AgentRole::IntegrationTester,
        "merge-resolver" | "mergeresolver" => AgentRole::MergeResolver,
        "terminal-validator" | "terminalvalidator" => AgentRole::TerminalValidator,
        "golem-lifecycle-tester" | "golemlifecycletester" => AgentRole::GolemLifecycleTester,
        "spec-drift-detector" | "specdriftdetector" => AgentRole::SpecDriftDetector,
        "regression-detector" | "regressiondetector" => AgentRole::RegressionDetector,
        "performance-sentinel" | "performancesentinel" => AgentRole::PerformanceSentinel,
        "coverage-tracker" | "coveragetracker" => AgentRole::CoverageTracker,
        "plan-lifecycle-mgr" | "plan-lifecycle-manager" | "planlifecyclemanager" => {
            AgentRole::PlanLifecycleManager
        }
        "cross-system-tester" | "crosssystemtester" => AgentRole::CrossSystemTester,
        "error-diagnoser" | "errordiagnoser" => AgentRole::ErrorDiagnoser,
        "dep-validator" | "dependency-validator" | "dependencyvalidator" => {
            AgentRole::DependencyValidator
        }
        "pattern-extractor" | "patternextractor" => AgentRole::PatternExtractor,
        "snapshot-comparator" | "snapshotcomparator" => AgentRole::SnapshotComparator,
        "full-loop-validator" | "fullloopvalidator" => AgentRole::FullLoopValidator,
        _ => return None,
    })
}

fn model_is_known(model: &str, known_models: &HashMap<String, ModelProfile>) -> bool {
    known_models.contains_key(model) || known_models.values().any(|profile| profile.slug == model)
}
