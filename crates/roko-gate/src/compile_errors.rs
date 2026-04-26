//! Structured compile error classification.
//!
//! Parses `cargo check --message-format=json` output and classifies errors
//! into categories that agents can act on programmatically.

use serde::{Deserialize, Serialize};

/// Category of compile error.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// Syntax error — malformed Rust code.
    Syntax,
    /// Unresolved import or symbol.
    UnresolvedImport,
    /// Type mismatch (expected X, found Y).
    TypeMismatch,
    /// Lifetime or borrow checker violation.
    Lifetime,
    /// Missing field, method, or trait implementation.
    MissingMember,
    /// Unused variable, import, or function.
    Unused,
    /// Visibility or access violation.
    Visibility,
    /// Macro expansion error.
    Macro,
    /// Trait bound not satisfied.
    TraitBound,
    /// Move/ownership error.
    Ownership,
    /// Other / unclassified error.
    Other,
}

/// Verify failure classes used by retries, replanning, and pre-agent remediation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// Rust syntax or parsing error.
    SyntaxError,
    /// Unresolved import, path, item, or symbol.
    ImportError,
    /// Type mismatch, trait bound, or missing member error.
    TypeError,
    /// Missing crate dependency or disabled Cargo feature.
    MissingDependencyOrFeature,
    /// Borrow checker, lifetime, move, or ownership failure.
    BorrowOrLifetime,
    /// A test assertion, snapshot, or expected-output check failed.
    TestExpectationFailure,
    /// Toolchain, network, permissions, timeout, or environment failure.
    ExternalEnvironment,
    /// Verify evidence suggests a stub, fake pass, or unsafe no-op production path.
    UnsafeStubOrPassBehavior,
    /// The agent likely lacked enough task/context information to continue safely.
    PromptContextInsufficiency,
    /// The failure is caused by missing role, tool, or filesystem permission.
    RoleToolPermission,
    /// The requested task conflicts with the current architecture or plan shape.
    ArchitecturalConflictRequiresReplan,
    /// The failure was not recognized.
    Unknown,
}

/// Structured next action recommended by a gate failure classification.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateFailureAction {
    /// Retry or deterministic remediation is still appropriate.
    #[default]
    Retry,
    /// The plan should be revised before rerunning the same task.
    NeedsReplan,
    /// Execution is blocked by an external/environmental condition.
    Blocked,
    /// Human input is required before continuing.
    NeedsHuman,
}

/// Coarse task-level failure kind used for retry policy decisions.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateFailureKind {
    /// A re-run may succeed without changing code.
    Transient,
    /// The agent needs to change code or inputs before retrying.
    Permanent,
    /// Resource exhaustion or unavailable local capacity; retrying immediately is unsafe.
    Resource,
    /// The verification contract/script/plan shape needs repair before retrying.
    Structural,
}

impl GateFailureKind {
    /// Whether this failure kind can be retried automatically.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient | Self::Permanent | Self::Structural)
    }

    /// Cooldown before retry, in seconds.
    #[must_use]
    pub const fn retry_cooldown_secs(&self) -> u64 {
        match self {
            Self::Transient => 2,
            Self::Permanent => 0,
            Self::Resource => 0,
            Self::Structural => 5,
        }
    }

    /// Whether retry input should include the full structured error digest.
    #[must_use]
    pub const fn needs_error_digest(&self) -> bool {
        matches!(self, Self::Permanent | Self::Structural)
    }

    /// Whether this failure should be persisted as learning/reflection evidence.
    #[must_use]
    pub const fn generates_reflection(&self) -> bool {
        matches!(self, Self::Permanent | Self::Structural)
    }

    /// Whether verify-script regeneration should be considered before retry.
    #[must_use]
    pub const fn needs_verify_regen(&self) -> bool {
        matches!(self, Self::Structural)
    }
}

/// Retry inputs derived from a structured gate failure.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GateRetryPolicy {
    /// Whether the orchestrator may retry this failure.
    pub retryable: bool,
    /// Cooldown before retry, in seconds.
    pub cooldown_secs: u64,
    /// Whether the retry prompt should include the structured error digest.
    pub include_error_digest: bool,
    /// Whether this failure should produce learning/reflection evidence.
    pub generate_reflection: bool,
    /// Whether verify-chain/script regeneration should be attempted.
    pub regenerate_verify: bool,
}

impl From<&GateFailureKind> for GateRetryPolicy {
    fn from(kind: &GateFailureKind) -> Self {
        Self {
            retryable: kind.is_retryable(),
            cooldown_secs: kind.retry_cooldown_secs(),
            include_error_digest: kind.needs_error_digest(),
            generate_reflection: kind.generates_reflection(),
            regenerate_verify: kind.needs_verify_regen(),
        }
    }
}

/// A single structured compile error.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompileError {
    /// Error category.
    pub category: ErrorCategory,
    /// Rustc error code (e.g. "E0433", "E0308").
    pub code: Option<String>,
    /// Human-readable message.
    pub message: String,
    /// File path (relative to workspace root).
    pub file: Option<String>,
    /// Line number.
    pub line: Option<u32>,
    /// Column number.
    pub column: Option<u32>,
    /// Rustc-suggested fix, if available.
    pub suggestion: Option<String>,
}

/// Structured failure classification for compile/test/lint gate output.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateFailureClassification {
    /// Verify that produced the failure, such as `compile:cargo`.
    pub gate: String,
    /// Primary class for retry and remediation decisions.
    pub primary: FailureClass,
    /// Coarse retry-policy failure kind.
    #[serde(default = "default_failure_kind")]
    pub failure_kind: GateFailureKind,
    /// Retry behavior derived from [`failure_kind`](Self::failure_kind).
    #[serde(default = "default_retry_policy")]
    pub retry_policy: GateRetryPolicy,
    /// Concise human-readable failure summary.
    #[serde(default)]
    pub summary: String,
    /// All observed classes in stable order.
    pub classes: Vec<FailureClass>,
    /// Structured compiler diagnostics when available.
    pub compile_errors: Vec<CompileError>,
    /// Total compiler errors observed.
    pub error_count: usize,
    /// Total compiler warnings observed.
    pub warning_count: usize,
    /// Whether deterministic `cargo fix` is a reasonable pre-agent attempt.
    pub cargo_fix_candidate: bool,
    /// Whether this should fail closed to agent retry or replan.
    pub agent_retry_needed: bool,
    /// Structured action the orchestrator should take next.
    #[serde(default)]
    pub recommended_action: GateFailureAction,
    /// Whether the failure is plan-shaped rather than retry-shaped.
    #[serde(default)]
    pub replan_candidate: bool,
    /// Blocking findings to preserve in retry/replan records.
    #[serde(default)]
    pub blocking_findings: Vec<String>,
    /// Gate execution duration in milliseconds, when known.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Short excerpt preserving enough original output for debugging.
    pub raw_excerpt: String,
}

impl GateFailureClassification {
    /// Attach a concise summary.
    #[must_use]
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    /// Attach observed gate duration.
    #[must_use]
    pub const fn with_duration_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }
}

/// Summary of all compile errors from a build.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompileErrorSummary {
    /// Total error count.
    pub error_count: usize,
    /// Total warning count.
    pub warning_count: usize,
    /// Classified errors.
    pub errors: Vec<CompileError>,
    /// Category distribution.
    pub categories: std::collections::HashMap<ErrorCategory, usize>,
}

/// Classify a rustc error code into a category.
pub fn classify_error_code(code: &str) -> ErrorCategory {
    match code {
        // Syntax / parsing
        "E0060" | "E0061" | "E0064" | "E0065" => ErrorCategory::Syntax,

        // Unresolved imports / paths
        "E0432" | "E0433" | "E0412" | "E0425" | "E0531" => ErrorCategory::UnresolvedImport,

        // Type mismatches
        "E0308" | "E0271" | "E0369" => ErrorCategory::TypeMismatch,

        // Trait bounds (E0277 = "trait bound not satisfied")
        "E0277" => ErrorCategory::TraitBound,

        // Lifetimes / borrowing
        "E0106" | "E0495" | "E0597" | "E0502" | "E0499" | "E0596" => ErrorCategory::Lifetime,

        // Missing fields / methods / impls
        "E0046" | "E0063" | "E0599" | "E0609" => ErrorCategory::MissingMember,

        // Unused
        "E0170" => ErrorCategory::Unused,

        // Visibility
        "E0603" | "E0624" | "E0616" => ErrorCategory::Visibility,

        // Macro
        "E0659" | "E0658" => ErrorCategory::Macro,

        // Move / ownership (E0505 = borrow while moved, E0507 = move out of borrow)
        "E0382" | "E0505" | "E0507" | "E0515" | "E0716" => ErrorCategory::Ownership,

        _ => ErrorCategory::Other,
    }
}

/// Classify a raw error message line (without a code) by pattern matching.
fn classify_message(msg: &str) -> ErrorCategory {
    let lower = msg.to_lowercase();
    if lower.contains("cannot find") || lower.contains("not found") || lower.contains("unresolved")
    {
        ErrorCategory::UnresolvedImport
    } else if lower.contains("expected") && lower.contains("found") {
        ErrorCategory::TypeMismatch
    } else if lower.contains("lifetime")
        || lower.contains("borrow")
        || lower.contains("does not live long enough")
    {
        ErrorCategory::Lifetime
    } else if lower.contains("missing field") || lower.contains("no method named") {
        ErrorCategory::MissingMember
    } else if lower.contains("unused") {
        ErrorCategory::Unused
    } else if lower.contains("private") || lower.contains("visibility") {
        ErrorCategory::Visibility
    } else if lower.contains("moved") || lower.contains("use of moved") {
        ErrorCategory::Ownership
    } else if lower.contains("trait bound") || lower.contains("is not satisfied") {
        ErrorCategory::TraitBound
    } else if lower.contains("macro") {
        ErrorCategory::Macro
    } else if lower.contains("syntax") || lower.contains("unexpected token") {
        ErrorCategory::Syntax
    } else {
        ErrorCategory::Other
    }
}

/// Parse cargo JSON diagnostic output into structured errors.
///
/// Input: the stderr from `cargo check --message-format=json`.
/// Each line is a JSON message; we extract `compiler-message` entries.
pub fn parse_cargo_json(stderr: &str) -> CompileErrorSummary {
    let mut summary = CompileErrorSummary::default();

    for line in stderr.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        // cargo emits {"reason": "compiler-message", "message": {...}}
        if msg.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }

        let Some(message) = msg.get("message") else {
            continue;
        };

        let level = message.get("level").and_then(|l| l.as_str()).unwrap_or("");
        let text = message
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let code_str = message
            .get("code")
            .and_then(|c| c.get("code"))
            .and_then(|c| c.as_str());

        if level == "warning" {
            summary.warning_count += 1;
            continue;
        }

        if level != "error" {
            continue;
        }

        summary.error_count += 1;

        // Extract location from primary span.
        let (file, line_num, col) = message
            .get("spans")
            .and_then(|s| s.as_array())
            .and_then(|spans| {
                spans
                    .iter()
                    .find(|s| s.get("is_primary") == Some(&serde_json::Value::Bool(true)))
            })
            .map(|span| {
                (
                    span.get("file_name")
                        .and_then(|f| f.as_str())
                        .map(String::from),
                    span.get("line_start")
                        .and_then(|l| l.as_u64())
                        .and_then(|l| u32::try_from(l).ok()),
                    span.get("column_start")
                        .and_then(|c| c.as_u64())
                        .and_then(|c| u32::try_from(c).ok()),
                )
            })
            .unwrap_or((None, None, None));

        // Extract suggestion from children.
        let suggestion = message
            .get("children")
            .and_then(|c| c.as_array())
            .and_then(|children| {
                children.iter().find_map(|child| {
                    let level = child.get("level").and_then(|l| l.as_str())?;
                    if level == "help" || level == "suggestion" {
                        child
                            .get("message")
                            .and_then(|m| m.as_str())
                            .map(String::from)
                    } else {
                        None
                    }
                })
            });

        let category = code_str.map_or_else(|| classify_message(text), classify_error_code);

        *summary.categories.entry(category.clone()).or_insert(0) += 1;

        summary.errors.push(CompileError {
            category,
            code: code_str.map(String::from),
            message: text.to_string(),
            file,
            line: line_num,
            column: col,
            suggestion,
        });
    }

    summary
}

/// Parse plain-text stderr (non-JSON) into structured errors.
///
/// Fallback for when `--message-format=json` isn't used.
pub fn parse_plain_stderr(stderr: &str) -> CompileErrorSummary {
    let mut summary = CompileErrorSummary::default();

    for line in stderr.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("error[") {
            summary.error_count += 1;

            // Parse "error[E0308]: mismatched types"
            let code = trimmed
                .get(6..)
                .and_then(|s| s.find(']').map(|i| &s[..i]))
                .map(String::from);

            let message = trimmed
                .find("]: ")
                .map(|i| trimmed[i + 3..].to_string())
                .unwrap_or_else(|| trimmed.to_string());

            let category = code
                .as_ref()
                .map_or_else(|| classify_message(&message), |c| classify_error_code(c));

            *summary.categories.entry(category.clone()).or_insert(0) += 1;

            summary.errors.push(CompileError {
                category,
                code,
                message,
                file: None,
                line: None,
                column: None,
                suggestion: None,
            });
        } else if let Some(rest) = trimmed.strip_prefix("error:") {
            summary.error_count += 1;
            let message = rest.trim().to_string();
            let category = classify_message(&message);

            *summary.categories.entry(category.clone()).or_insert(0) += 1;

            summary.errors.push(CompileError {
                category,
                code: None,
                message,
                file: None,
                line: None,
                column: None,
                suggestion: None,
            });
        } else if trimmed.starts_with("warning:") || trimmed.starts_with("warning[") {
            summary.warning_count += 1;
        }
    }

    summary
}

/// Classify raw gate output into a stable failure class.
#[must_use]
pub fn classify_gate_failure(gate: &str, output: &str) -> GateFailureClassification {
    let mut summary = parse_cargo_json(output);
    if summary.error_count == 0 && summary.warning_count == 0 {
        summary = parse_plain_stderr(output);
    }

    let mut classes = Vec::new();
    let lower = output.to_ascii_lowercase();

    if gate.starts_with("test")
        && (lower.contains("test result: failed")
            || lower.contains("assertion failed")
            || lower.contains("panicked at")
            || lower.contains("snapshot")
            || lower.contains("expected")
            || lower.contains("failed"))
    {
        push_unique(&mut classes, FailureClass::TestExpectationFailure);
    }

    if looks_external_environment_failure(&lower) {
        push_unique(&mut classes, FailureClass::ExternalEnvironment);
    }

    if looks_unsafe_stub_or_pass_behavior(&lower) {
        push_unique(&mut classes, FailureClass::UnsafeStubOrPassBehavior);
    }

    if looks_prompt_context_insufficient(&lower) {
        push_unique(&mut classes, FailureClass::PromptContextInsufficiency);
    }

    if looks_role_tool_permission_issue(&lower) {
        push_unique(&mut classes, FailureClass::RoleToolPermission);
    }

    if looks_architectural_conflict(&lower) {
        push_unique(
            &mut classes,
            FailureClass::ArchitecturalConflictRequiresReplan,
        );
    }

    if looks_missing_dependency_or_feature(&lower) {
        push_unique(&mut classes, FailureClass::MissingDependencyOrFeature);
    }

    for error in &summary.errors {
        push_unique(&mut classes, failure_class_for_compile_error(error));
    }

    if classes.is_empty() {
        push_unique(&mut classes, FailureClass::Unknown);
    }

    let primary = classes[0].clone();
    let cargo_fix_candidate = summary.errors.iter().any(|error| {
        error
            .suggestion
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            && !matches!(
                failure_class_for_compile_error(error),
                FailureClass::MissingDependencyOrFeature
                    | FailureClass::ExternalEnvironment
                    | FailureClass::Unknown
            )
    });
    let replan_candidate = classes.iter().any(|class| {
        matches!(
            class,
            FailureClass::UnsafeStubOrPassBehavior
                | FailureClass::PromptContextInsufficiency
                | FailureClass::ArchitecturalConflictRequiresReplan
        )
    });
    let recommended_action = if replan_candidate {
        GateFailureAction::NeedsReplan
    } else if classes.contains(&FailureClass::ExternalEnvironment) {
        GateFailureAction::Blocked
    } else if classes.contains(&FailureClass::RoleToolPermission) {
        GateFailureAction::NeedsHuman
    } else {
        GateFailureAction::Retry
    };
    let blocking_findings = classes
        .iter()
        .filter_map(blocking_finding_for_class)
        .collect();

    let raw_excerpt: String = output.chars().take(2000).collect();
    let failure_kind = classify_failure_kind(&primary, &classes, &recommended_action, &raw_excerpt);
    let retry_policy = GateRetryPolicy::from(&failure_kind);

    GateFailureClassification {
        gate: gate.to_string(),
        primary,
        failure_kind,
        retry_policy,
        summary: String::new(),
        classes,
        compile_errors: summary.errors,
        error_count: summary.error_count,
        warning_count: summary.warning_count,
        cargo_fix_candidate,
        agent_retry_needed: true,
        recommended_action,
        replan_candidate,
        blocking_findings,
        duration_ms: None,
        raw_excerpt,
    }
}

/// Classify and enrich a gate failure in one step.
#[must_use]
pub fn structured_gate_failure(
    gate: &str,
    output: &str,
    summary: impl Into<String>,
    duration_ms: u64,
) -> GateFailureClassification {
    classify_gate_failure(gate, output)
        .with_summary(summary)
        .with_duration_ms(duration_ms)
}

/// Render a structured classification as stable pretty JSON.
#[must_use]
pub fn render_failure_classification(classification: &GateFailureClassification) -> String {
    serde_json::to_string_pretty(classification).unwrap_or_else(|_| format!("{classification:?}"))
}

fn push_unique(classes: &mut Vec<FailureClass>, class: FailureClass) {
    if !classes.contains(&class) {
        classes.push(class);
    }
}

fn failure_class_for_compile_error(error: &CompileError) -> FailureClass {
    let lower = error.message.to_ascii_lowercase();
    if looks_missing_dependency_or_feature(&lower) {
        return FailureClass::MissingDependencyOrFeature;
    }

    match error.category {
        ErrorCategory::Syntax | ErrorCategory::Macro => FailureClass::SyntaxError,
        ErrorCategory::UnresolvedImport => FailureClass::ImportError,
        ErrorCategory::TypeMismatch
        | ErrorCategory::TraitBound
        | ErrorCategory::MissingMember
        | ErrorCategory::Visibility => FailureClass::TypeError,
        ErrorCategory::Lifetime | ErrorCategory::Ownership => FailureClass::BorrowOrLifetime,
        ErrorCategory::Unused | ErrorCategory::Other => FailureClass::Unknown,
    }
}

fn looks_missing_dependency_or_feature(lower: &str) -> bool {
    lower.contains("unlinked crate")
        || lower.contains("undeclared crate")
        || lower.contains("no matching package named")
        || lower.contains("failed to select a version")
        || lower.contains("does not have these features")
        || lower.contains("does not have feature")
        || lower.contains("package `") && lower.contains("depends on") && lower.contains("feature")
}

fn classify_failure_kind(
    primary: &FailureClass,
    classes: &[FailureClass],
    recommended_action: &GateFailureAction,
    raw_excerpt: &str,
) -> GateFailureKind {
    let lower = raw_excerpt.to_ascii_lowercase();
    if looks_resource_failure(&lower) {
        return GateFailureKind::Resource;
    }
    if matches!(recommended_action, GateFailureAction::NeedsReplan)
        || matches!(
            primary,
            FailureClass::UnsafeStubOrPassBehavior
                | FailureClass::ArchitecturalConflictRequiresReplan
        )
    {
        return GateFailureKind::Structural;
    }
    if matches!(recommended_action, GateFailureAction::Blocked) && !looks_likely_transient(&lower) {
        return GateFailureKind::Resource;
    }
    if classes.contains(&FailureClass::ExternalEnvironment) && looks_likely_transient(&lower) {
        return GateFailureKind::Transient;
    }
    if matches!(primary, FailureClass::Unknown) {
        return classify_failure_kind_from_raw(&lower);
    }
    GateFailureKind::Permanent
}

fn classify_failure_kind_from_raw(lower: &str) -> GateFailureKind {
    if looks_resource_failure(lower) {
        GateFailureKind::Resource
    } else if looks_likely_transient(lower) {
        GateFailureKind::Transient
    } else if lower.contains("verify script")
        || lower.contains("acceptance contract")
        || lower.contains("impossible")
    {
        GateFailureKind::Structural
    } else {
        GateFailureKind::Permanent
    }
}

fn looks_resource_failure(lower: &str) -> bool {
    lower.contains("out of memory")
        || lower.contains("oom")
        || lower.contains("no space left")
        || lower.contains("disk full")
        || lower.contains("too many open files")
        || lower.contains("cannot allocate memory")
}

fn looks_likely_transient(lower: &str) -> bool {
    lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("temporary failure")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("flaky")
        || lower.contains("intermittent")
        || lower.contains("race condition")
}

fn default_failure_kind() -> GateFailureKind {
    GateFailureKind::Permanent
}

fn default_retry_policy() -> GateRetryPolicy {
    GateRetryPolicy::from(&GateFailureKind::Permanent)
}

fn looks_external_environment_failure(lower: &str) -> bool {
    lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("spawn failed")
        || lower.contains("permission denied")
        || lower.contains("no such file or directory")
        || lower.contains("could not download")
        || lower.contains("failed to download")
        || lower.contains("failed to get")
        || lower.contains("temporary failure")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("dns")
        || lower.contains("network")
}

fn looks_unsafe_stub_or_pass_behavior(lower: &str) -> bool {
    ((lower.contains("stub") || lower.contains("noop") || lower.contains("no-op"))
        && (lower.contains("production") || lower.contains("gate") || lower.contains("pass")))
        || lower.contains("fake pass")
        || lower.contains("stub-pass")
}

fn looks_prompt_context_insufficient(lower: &str) -> bool {
    lower.contains("prompt/context insufficiency")
        || lower.contains("context insufficiency")
        || lower.contains("insufficient context")
        || lower.contains("missing context")
}

fn looks_role_tool_permission_issue(lower: &str) -> bool {
    lower.contains("role/tool permission")
        || lower.contains("tool permission")
        || lower.contains("permission issue")
        || lower.contains("not allowed to use tool")
}

fn looks_architectural_conflict(lower: &str) -> bool {
    lower.contains("architectural conflict")
        || lower.contains("requires replan")
        || lower.contains("needs replan")
        || lower.contains("cannot be solved by retry")
}

fn blocking_finding_for_class(class: &FailureClass) -> Option<String> {
    match class {
        FailureClass::UnsafeStubOrPassBehavior => {
            Some("gate evidence indicates unsafe stub/pass behavior".to_string())
        }
        FailureClass::PromptContextInsufficiency => {
            Some("retry lacks required prompt/context evidence".to_string())
        }
        FailureClass::RoleToolPermission => {
            Some("required role/tool permission is unavailable".to_string())
        }
        FailureClass::ArchitecturalConflictRequiresReplan => {
            Some("failure requires plan shape or dependency revision".to_string())
        }
        FailureClass::ExternalEnvironment => {
            Some("external environment must recover before retry".to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_common_error_codes() {
        assert_eq!(
            classify_error_code("E0433"),
            ErrorCategory::UnresolvedImport
        );
        assert_eq!(classify_error_code("E0308"), ErrorCategory::TypeMismatch);
        assert_eq!(classify_error_code("E0597"), ErrorCategory::Lifetime);
        assert_eq!(classify_error_code("E0063"), ErrorCategory::MissingMember);
        assert_eq!(classify_error_code("E0382"), ErrorCategory::Ownership);
        assert_eq!(classify_error_code("E9999"), ErrorCategory::Other);
    }

    #[test]
    fn classify_message_patterns() {
        assert_eq!(
            classify_message("cannot find value `foo` in this scope"),
            ErrorCategory::UnresolvedImport
        );
        assert_eq!(
            classify_message("expected `u32`, found `String`"),
            ErrorCategory::TypeMismatch
        );
        assert_eq!(
            classify_message("`x` does not live long enough"),
            ErrorCategory::Lifetime
        );
        assert_eq!(
            classify_message("missing field `name` in initializer"),
            ErrorCategory::MissingMember
        );
        assert_eq!(
            classify_message("value used here after move"),
            ErrorCategory::Other // "move" not "moved"
        );
    }

    #[test]
    fn parse_plain_stderr_basic() {
        let stderr = "\
warning: unused variable
error[E0433]: failed to resolve: use of undeclared crate
error: aborting due to 1 previous error
";
        let summary = parse_plain_stderr(stderr);
        assert_eq!(summary.error_count, 2);
        assert_eq!(summary.warning_count, 1);
        assert_eq!(summary.errors.len(), 2);
        assert_eq!(summary.errors[0].category, ErrorCategory::UnresolvedImport);
        assert_eq!(summary.errors[0].code.as_deref(), Some("E0433"));
    }

    #[test]
    fn parse_cargo_json_basic() {
        let json_line = r#"{"reason":"compiler-message","message":{"message":"cannot find value `foo`","code":{"code":"E0425","explanation":null},"level":"error","spans":[{"file_name":"src/main.rs","byte_start":0,"byte_end":3,"line_start":1,"line_end":1,"column_start":1,"column_end":4,"is_primary":true}],"children":[{"message":"consider importing this","level":"help"}]}}"#;
        let summary = parse_cargo_json(json_line);
        assert_eq!(summary.error_count, 1);
        assert_eq!(summary.errors[0].category, ErrorCategory::UnresolvedImport);
        assert_eq!(summary.errors[0].code.as_deref(), Some("E0425"));
        assert_eq!(summary.errors[0].file.as_deref(), Some("src/main.rs"));
        assert_eq!(summary.errors[0].line, Some(1));
        assert_eq!(
            summary.errors[0].suggestion.as_deref(),
            Some("consider importing this")
        );
    }

    #[test]
    fn parse_cargo_json_warnings_only() {
        let json_line = r#"{"reason":"compiler-message","message":{"message":"unused variable","code":null,"level":"warning","spans":[],"children":[]}}"#;
        let summary = parse_cargo_json(json_line);
        assert_eq!(summary.error_count, 0);
        assert_eq!(summary.warning_count, 1);
        assert!(summary.errors.is_empty());
    }

    #[test]
    fn summary_tracks_category_distribution() {
        let stderr = "\
error[E0433]: failed to resolve
error[E0433]: unresolved import
error[E0308]: mismatched types
";
        let summary = parse_plain_stderr(stderr);
        assert_eq!(
            summary.categories.get(&ErrorCategory::UnresolvedImport),
            Some(&2)
        );
        assert_eq!(
            summary.categories.get(&ErrorCategory::TypeMismatch),
            Some(&1)
        );
    }

    #[test]
    fn classifies_required_failure_classes() {
        let syntax = classify_gate_failure("compile:cargo", "error: unexpected token: `}`");
        assert_eq!(syntax.primary, FailureClass::SyntaxError);

        let import = classify_gate_failure("compile:cargo", "error[E0425]: cannot find value `x`");
        assert_eq!(import.primary, FailureClass::ImportError);

        let type_error = classify_gate_failure(
            "compile:cargo",
            "error[E0308]: expected `u32`, found `String`",
        );
        assert_eq!(type_error.primary, FailureClass::TypeError);

        let borrow = classify_gate_failure(
            "compile:cargo",
            "error[E0597]: borrowed value does not live long enough",
        );
        assert_eq!(borrow.primary, FailureClass::BorrowOrLifetime);
    }

    #[test]
    fn classifies_missing_dependency_feature_and_environment() {
        let dep = classify_gate_failure(
            "compile:cargo",
            "error[E0433]: failed to resolve: use of unresolved module or unlinked crate `serde_yaml`",
        );
        assert_eq!(dep.primary, FailureClass::MissingDependencyOrFeature);

        let feature = classify_gate_failure(
            "compile:cargo",
            "package `roko-cli` depends on `tokio` with feature `missing`, but `tokio` does not have that feature",
        );
        assert_eq!(feature.primary, FailureClass::MissingDependencyOrFeature);

        let env = classify_gate_failure("compile:cargo", "spawn failed: No such file or directory");
        assert_eq!(env.primary, FailureClass::ExternalEnvironment);
    }

    #[test]
    fn classifies_test_expectation_failures() {
        let failure = classify_gate_failure(
            "test:cargo",
            "thread 'foo' panicked at assertion failed: left == right\ntest result: FAILED. 9 passed; 1 failed",
        );
        assert_eq!(failure.primary, FailureClass::TestExpectationFailure);
        assert_eq!(failure.failure_kind, GateFailureKind::Permanent);
        assert!(failure.retry_policy.retryable);
        assert!(failure.retry_policy.include_error_digest);
        assert_eq!(failure.recommended_action, GateFailureAction::Retry);
        assert!(!failure.replan_candidate);
    }

    #[test]
    fn failure_kind_drives_retry_policy() {
        let transient = classify_gate_failure("shell:flake", "timed out after 100 ms");
        assert_eq!(transient.failure_kind, GateFailureKind::Transient);
        assert_eq!(transient.retry_policy.cooldown_secs, 2);
        assert!(!transient.retry_policy.include_error_digest);

        let resource =
            classify_gate_failure("compile:cargo", "out of memory: cannot allocate memory");
        assert_eq!(resource.failure_kind, GateFailureKind::Resource);
        assert!(!resource.retry_policy.retryable);

        let structural = classify_gate_failure(
            "verify_chain",
            "verify script impossible acceptance contract requires replan",
        );
        assert_eq!(structural.failure_kind, GateFailureKind::Structural);
        assert!(structural.retry_policy.regenerate_verify);
    }

    #[test]
    fn structured_gate_failure_preserves_retry_inputs() {
        let classification = structured_gate_failure(
            "compile:cargo",
            "error[E0308]: mismatched types",
            "error[E0308]: mismatched types",
            123,
        );

        assert_eq!(classification.summary, "error[E0308]: mismatched types");
        assert_eq!(classification.duration_ms, Some(123));
        assert_eq!(classification.failure_kind, GateFailureKind::Permanent);
        assert!(classification.retry_policy.generate_reflection);

        let rendered = render_failure_classification(&classification);
        let reparsed: GateFailureClassification =
            serde_json::from_str(&rendered).expect("rendered classification parses");
        assert_eq!(reparsed.duration_ms, Some(123));
        assert_eq!(reparsed.retry_policy, classification.retry_policy);
    }

    #[test]
    fn classifies_replan_and_human_needed_failures() {
        let replan = classify_gate_failure(
            "review:structured",
            "architectural conflict: cannot be solved by retry without changing the plan shape",
        );
        assert_eq!(
            replan.primary,
            FailureClass::ArchitecturalConflictRequiresReplan
        );
        assert_eq!(replan.recommended_action, GateFailureAction::NeedsReplan);
        assert!(replan.replan_candidate);
        assert!(!replan.blocking_findings.is_empty());

        let human = classify_gate_failure(
            "tool:dispatch",
            "role/tool permission issue: not allowed to use tool git",
        );
        assert_eq!(human.primary, FailureClass::RoleToolPermission);
        assert_eq!(human.recommended_action, GateFailureAction::NeedsHuman);
    }

    #[test]
    fn cargo_fix_candidate_requires_machine_suggestion() {
        let json_line = r#"{"reason":"compiler-message","message":{"message":"unused import: `foo`","code":{"code":"E0432","explanation":null},"level":"error","spans":[],"children":[{"message":"consider importing this","level":"help"}]}}"#;
        let classification = classify_gate_failure("compile:cargo", json_line);
        assert!(classification.cargo_fix_candidate);

        let dep = classify_gate_failure(
            "compile:cargo",
            "error[E0433]: use of unresolved module or unlinked crate `missing_crate`",
        );
        assert!(!dep.cargo_fix_candidate);
    }
}
