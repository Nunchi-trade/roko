//! Tool dispatcher (§36.d) — runs a parsed [`ToolCall`] through the safety
//! funnel, invokes the handler with timeout + cancellation, truncates
//! oversized results, and returns a [`ToolResult`].
//!
//! # Pipeline (per call)
//!
//! 1. **Validate** args against the registry's JSON schema (§36.42).
//! 2. **Resolve** the [`ToolDef`] for the canonical name.
//! 3. **Authorize** — `def.permission.satisfied_by(&role_perms)` (§36.46).
//! 4. **Resolve handler** via the pluggable [`HandlerResolver`] trait.
//! 5. **Race** `handler.execute` against `ctx.timeout` + cancellation
//!    (§36.40, §36.45).
//! 6. **Truncate** oversized `Ok` content to `max_result_bytes`,
//!    preserving UTF-8 char boundaries (§36.43).
//!
//! # Batch (per turn)
//!
//! [`ToolDispatcher::dispatch_batch`] groups calls by
//! [`ToolConcurrency`](roko_core::tool::ToolConcurrency): `Parallel`
//! tools run via `futures::future::join_all`; `Serial` tools run
//! sequentially (preserves shell-state ordering, avoids write-write
//! races). Returns results with the parallel-bucket first, serial last.
//!
//! # Why [`HandlerResolver`] instead of depending on `roko-std`
//!
//! The built-in 16 handlers live in `roko-std`. Depending on `roko-std`
//! from `roko-agent` would invert the layering: backends would pull in
//! the entire standard library of handlers even when they only need the
//! dispatcher's plumbing. Callers pass their own resolver — typically
//! one that closes over `roko_std::tool::handler_for` — keeping this
//! crate free of that dependency. See M19 in MISTAKES-LEARNED.md.

use std::sync::Arc;
use std::time::Duration;

use roko_core::tool::{ToolCall, ToolContext, ToolError, ToolHandler, ToolRegistry, ToolResult};
use roko_core::{Body, Engram, Kind, Provenance, ToolPermissions};
use serde_json::{Value, json};

use crate::safety::SafetyLayer;

pub mod alert;
pub mod cancel;
/// Dispatch-level dedup cache for idempotent agent dispatch (DEPLOY-09).
pub mod dedup_cache;
pub mod emit_metric;
pub mod hook_chain;
pub mod parallel;
pub mod result_cache;
pub mod timeout;
pub mod tool_selector;
pub mod truncate;
pub mod validate;

use self::cancel::wait_cancelled;
use self::parallel::partition_by_concurrency;
use self::timeout::with_timeout;
use self::truncate::truncate_result;
use self::validate::validate;

/// Default cap on per-tool-result content bytes (§36.43).
pub const DEFAULT_MAX_RESULT_BYTES: usize = 16_384;

/// Pluggable handler lookup: maps a canonical tool name to a
/// [`ToolHandler`] instance.
///
/// The built-in resolver is [`roko_std::tool::handlers::handler_for`] in
/// the `roko-std` crate, but the dispatcher stays agnostic so custom
/// backends can ship their own (e.g. MCP-backed dynamic handlers).
pub trait HandlerResolver: Send + Sync {
    /// Look up the handler for `name`, if any.
    fn resolve(&self, name: &str) -> Option<Arc<dyn ToolHandler>>;
}

impl<F> HandlerResolver for F
where
    F: Fn(&str) -> Option<Arc<dyn ToolHandler>> + Send + Sync,
{
    fn resolve(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        (self)(name)
    }
}

/// Dispatches [`ToolCall`]s through validation → safety → authorization → handler.
pub struct ToolDispatcher {
    registry: Arc<dyn ToolRegistry>,
    resolver: Arc<dyn HandlerResolver>,
    max_result_bytes: usize,
    safety: Option<SafetyLayer>,
    /// Optional tool result cache for deterministic tools (AGT-10).
    tool_cache: Option<std::sync::Mutex<result_cache::ToolResultCache>>,
    /// Optional sequential safety hook chain (TOOL-02).
    ///
    /// When present, each tool call passes through every hook in order
    /// before the handler executes. Rejections short-circuit the chain.
    hook_chain: Option<hook_chain::SafetyHookChain>,
}

impl ToolDispatcher {
    /// Construct a dispatcher backed by the given tool registry and
    /// handler resolver.
    #[must_use]
    pub fn new(registry: Arc<dyn ToolRegistry>, resolver: Arc<dyn HandlerResolver>) -> Self {
        Self {
            registry,
            resolver,
            max_result_bytes: DEFAULT_MAX_RESULT_BYTES,
            safety: None,
            tool_cache: None,
            hook_chain: None,
        }
    }

    /// Override the default result-byte cap.
    #[must_use]
    pub const fn with_max_result_bytes(mut self, n: usize) -> Self {
        self.max_result_bytes = n;
        self
    }

    /// Attach a [`SafetyLayer`] so every dispatched call passes through
    /// pre-execution safety checks and post-execution output scrubbing.
    #[must_use]
    pub fn with_safety(mut self, layer: SafetyLayer) -> Self {
        self.safety = Some(layer);
        self
    }

    /// Returns the configured safety layer, if any.
    #[must_use]
    pub const fn safety(&self) -> Option<&SafetyLayer> {
        self.safety.as_ref()
    }

    /// Attach a sequential safety hook chain (TOOL-02).
    ///
    /// When attached, every dispatched tool call passes through each hook
    /// in order before the handler executes. The first rejection
    /// short-circuits the chain and returns `ToolError::PermissionDenied`.
    ///
    /// Audit records from each hook decision are emitted as Engram signals.
    #[must_use]
    pub fn with_hook_chain(mut self, chain: hook_chain::SafetyHookChain) -> Self {
        self.hook_chain = Some(chain);
        self
    }

    /// Returns the hook chain, if one is attached.
    #[must_use]
    pub const fn hook_chain(&self) -> Option<&hook_chain::SafetyHookChain> {
        self.hook_chain.as_ref()
    }

    /// Enable cross-turn tool result caching for deterministic tools (AGT-10).
    ///
    /// When enabled, results from deterministic tools (Read, Glob, Grep) are
    /// cached by argument hash. Write/Edit calls invalidate affected entries.
    #[must_use]
    pub fn with_tool_cache(mut self, cache: result_cache::ToolResultCache) -> Self {
        self.tool_cache = Some(std::sync::Mutex::new(cache));
        self
    }

    /// Returns tool cache statistics, if caching is enabled.
    #[must_use]
    pub fn cache_stats(&self) -> Option<(u64, u64, f64)> {
        let cache = self.tool_cache.as_ref()?.lock().ok()?;
        Some((cache.hits(), cache.misses(), cache.hit_rate()))
    }

    /// Configured cap on content bytes for a single `Ok` result.
    #[must_use]
    pub const fn max_result_bytes(&self) -> usize {
        self.max_result_bytes
    }

    /// Backing registry (exposed for advanced callers).
    #[must_use]
    pub fn registry(&self) -> &Arc<dyn ToolRegistry> {
        &self.registry
    }

    /// Dispatch a single tool call end-to-end.
    #[allow(clippy::too_many_lines)]
    pub async fn dispatch(&self, call: ToolCall, ctx: &ToolContext) -> ToolResult {
        let timeout = ctx.timeout;
        let timeout_ms = duration_to_ms(timeout);

        // 1. Validate args.
        if let Err(e) = validate(&call, self.registry.as_ref()) {
            Self::emit_audit(
                ctx,
                &call,
                "validation",
                "failed",
                &json!({
                    "error": e.to_string(),
                    "error_kind": tool_error_kind(&e),
                }),
            );
            Self::emit_terminal_audit(ctx, &call, &ToolResult::err(e.clone()), timeout_ms);
            return ToolResult::err(e);
        }
        Self::emit_audit(ctx, &call, "validation", "passed", &argument_summary(&call));
        // 2. Resolve the def.
        let Some(def) = self.registry.get(&call.name) else {
            let err = ToolError::Other(format!("unknown tool: {}", call.name));
            Self::emit_audit(
                ctx,
                &call,
                "handler",
                "missing_definition",
                &json!({
                    "error": err.to_string(),
                    "error_kind": tool_error_kind(&err),
                }),
            );
            Self::emit_terminal_audit(ctx, &call, &ToolResult::err(err.clone()), timeout_ms);
            return ToolResult::err(err);
        };
        // 3. Apply task-level tool filters before capability checks.
        if let Some(reason) = tool_filter_block_reason(
            &call.name,
            ctx.allowed_tools.as_deref(),
            ctx.denied_tools.as_deref(),
        ) {
            let err = ToolError::PermissionDenied(reason.clone());
            Self::emit_audit(
                ctx,
                &call,
                "tool_filter",
                "denied",
                &json!({
                    "tool": call.name,
                    "allowed_tools": ctx.allowed_tools.clone(),
                    "denied_tools": ctx.denied_tools.clone(),
                    "error": err.to_string(),
                    "error_kind": tool_error_kind(&err),
                }),
            );
            Self::emit_terminal_audit(ctx, &call, &ToolResult::err(err.clone()), timeout_ms);
            return ToolResult::err(err);
        }
        // 4. Authorize against the role's capabilities. The `satisfied_by`
        //    method wants `ToolPermissions` (what the role grants); we
        //    build one from `ctx.capabilities` (a `ToolPermission` — same
        //    flags, different type).
        let role_perms = ToolPermissions {
            read: ctx.capabilities.read,
            write: ctx.capabilities.write,
            exec: ctx.capabilities.exec,
            git: ctx.capabilities.git,
            network: ctx.capabilities.network,
        };
        if !def.permission.satisfied_by(&role_perms) {
            let err = ToolError::PermissionDenied(format!(
                "{} requires {:?}, role grants {:?}",
                call.name, def.permission, role_perms
            ));
            Self::emit_audit(
                ctx,
                &call,
                "permission",
                "denied",
                &json!({
                    "required": format!("{:?}", def.permission),
                    "granted": format!("{:?}", role_perms),
                    "error": err.to_string(),
                    "error_kind": tool_error_kind(&err),
                }),
            );
            Self::emit_terminal_audit(ctx, &call, &ToolResult::err(err.clone()), timeout_ms);
            return ToolResult::err(err);
        }
        Self::emit_audit(
            ctx,
            &call,
            "permission",
            "granted",
            &json!({
                "required": format!("{:?}", def.permission),
                "granted": format!("{:?}", role_perms),
            }),
        );
        // 3b. Safety checks — if a SafetyLayer is attached, run all
        //     pre-execution policies. First failure short-circuits.
        if let Some(ref safety) = self.safety {
            if let Err(e) = safety.check_pre_execution(&call, ctx) {
                Self::emit_audit(
                    ctx,
                    &call,
                    "safety",
                    "blocked",
                    &json!({
                        "error": e.to_string(),
                        "error_kind": tool_error_kind(&e),
                    }),
                );
                Self::emit_terminal_audit(ctx, &call, &ToolResult::err(e.clone()), timeout_ms);
                return ToolResult::err(e);
            }
            if let Err(e) = safety.check_contract(&call, ctx) {
                Self::emit_audit(
                    ctx,
                    &call,
                    "contract",
                    "blocked",
                    &json!({
                        "error": e.to_string(),
                        "error_kind": tool_error_kind(&e),
                    }),
                );
                Self::emit_terminal_audit(ctx, &call, &ToolResult::err(e.clone()), timeout_ms);
                return ToolResult::err(e);
            }
        }
        // 3c. Safety hook chain — if a chain is attached, run each hook
        //     sequentially. The first rejection short-circuits. Audit records
        //     are emitted for every hook decision.
        if let Some(ref chain) = self.hook_chain {
            match chain.evaluate(&def, call.arguments.clone(), ctx).await {
                Ok((_params, audit_records)) => {
                    for record in &audit_records {
                        Self::emit_audit(
                            ctx,
                            &call,
                            "hook_chain",
                            match &record.decision {
                                crate::safety::hooks::HookDecision::Allow => "allow",
                                crate::safety::hooks::HookDecision::AllowModified(_) => "modified",
                                crate::safety::hooks::HookDecision::Reject(_) => "rejected",
                            },
                            &json!({
                                "hook": record.hook_name,
                                "decision": format!("{:?}", record.decision),
                            }),
                        );
                    }
                }
                Err((err, audit_records)) => {
                    for record in &audit_records {
                        Self::emit_audit(
                            ctx,
                            &call,
                            "hook_chain",
                            match &record.decision {
                                crate::safety::hooks::HookDecision::Allow => "allow",
                                crate::safety::hooks::HookDecision::AllowModified(_) => "modified",
                                crate::safety::hooks::HookDecision::Reject(_) => "rejected",
                            },
                            &json!({
                                "hook": record.hook_name,
                                "decision": format!("{:?}", record.decision),
                                "reason": record.reason.as_deref().unwrap_or(""),
                            }),
                        );
                    }
                    Self::emit_terminal_audit(ctx, &call, &ToolResult::err(err.clone()), timeout_ms);
                    return ToolResult::err(err);
                }
            }
        }
        // 4. Resolve handler.
        let Some(handler) = self.resolver.resolve(&call.name) else {
            let err = ToolError::Other(format!("no handler: {}", call.name));
            Self::emit_audit(
                ctx,
                &call,
                "handler",
                "missing",
                &json!({
                    "error": err.to_string(),
                    "error_kind": tool_error_kind(&err),
                }),
            );
            Self::emit_terminal_audit(ctx, &call, &ToolResult::err(err.clone()), timeout_ms);
            return ToolResult::err(err);
        };
        let handler_name = handler.name().to_string();
        Self::emit_audit(
            ctx,
            &call,
            "handler",
            "started",
            &json!({
                "handler": handler_name,
                "timeout_ms": timeout_ms,
            }),
        );
        // 5. Race handler.execute against timeout + cancellation.
        let call_for_exec = call.clone();
        let exec_fut = async move { handler.execute(call_for_exec, ctx).await };
        let result = tokio::select! {
            r = with_timeout(timeout, exec_fut) => r,
            () = wait_cancelled(ctx.cancel_token.as_ref()) => {
                ToolResult::err(ToolError::Cancelled)
            }
        };
        // 6. Truncate oversized output.
        let result = truncate_result(result, self.max_result_bytes);
        // 7. Scrub secrets from output.
        let result = if let Some(ref safety) = self.safety {
            safety.scrub_output(result)
        } else {
            result
        };
        let result = if let Some(ref safety) = self.safety {
            match safety.check_recovery(&result) {
                Ok(()) => result,
                Err(err) => ToolResult::err(err),
            }
        } else {
            result
        };
        Self::emit_terminal_audit(ctx, &call, &result, timeout_ms);
        result
    }

    /// Dispatch a batch of tool calls, grouping by concurrency policy.
    ///
    /// Parallel-safe tools run via `futures::future::join_all`; serial
    /// tools run sequentially. The returned vec has parallel results
    /// first (in the order they completed, not input order), then
    /// serial results (in input order).
    pub async fn dispatch_batch(
        &self,
        calls: Vec<ToolCall>,
        ctx: &ToolContext,
    ) -> Vec<(ToolCall, ToolResult)> {
        let (parallel, serial) = partition_by_concurrency(calls, self.registry.as_ref());

        // Parallel bucket: fan out with join_all.
        let par_futs = parallel.into_iter().map(|call| async {
            let name = call.clone();
            let res = self.dispatch(call, ctx).await;
            (name, res)
        });
        let mut out = futures::future::join_all(par_futs).await;

        // Serial bucket: sequential loop so calls observe each other's side effects.
        for call in serial {
            let call_copy = call.clone();
            let res = self.dispatch(call, ctx).await;
            out.push((call_copy, res));
        }

        out
    }

    fn emit_audit(
        ctx: &ToolContext,
        call: &ToolCall,
        phase: &'static str,
        status: &'static str,
        details: &Value,
    ) {
        let signal = Engram::builder(Kind::ToolInvocation)
            .body(audit_body(call, phase, status, details))
            .provenance(Provenance::trusted("tool_dispatcher"))
            .tag("call_id", &call.id)
            .tag("tool", &call.name)
            .tag("phase", phase)
            .tag("status", status)
            .build();
        ctx.audit_sink.emit(signal);
    }

    fn emit_terminal_audit(
        ctx: &ToolContext,
        call: &ToolCall,
        result: &ToolResult,
        timeout_ms: u64,
    ) {
        match result {
            ToolResult::Ok {
                content,
                artifacts,
                is_structured,
            } => Self::emit_audit(
                ctx,
                call,
                "completion",
                "succeeded",
                &json!({
                    "content_bytes": content.len(),
                    "artifacts": artifacts.len(),
                    "is_structured": is_structured,
                    "timeout_ms": timeout_ms,
                }),
            ),
            ToolResult::Err(err) => Self::emit_audit(
                ctx,
                call,
                "completion",
                "failed",
                &json!({
                    "error": err.to_string(),
                    "error_kind": tool_error_kind(err),
                    "timeout_ms": timeout_ms,
                }),
            ),
        }
    }
}

fn audit_body(call: &ToolCall, phase: &str, status: &str, details: &Value) -> Body {
    let payload = json!({
        "call_id": call.id,
        "tool": call.name,
        "phase": phase,
        "status": status,
        "request_ts_ms": call.request_ts_ms,
        "details": details,
    });
    Body::from_json(&payload).unwrap_or_else(|_| Body::text(payload.to_string()))
}

fn argument_summary(call: &ToolCall) -> Value {
    match &call.arguments {
        Value::Object(map) => {
            let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
            keys.sort_unstable();
            json!({
                "argument_kind": "object",
                "argument_keys": keys,
                "argument_count": map.len(),
            })
        }
        Value::Array(items) => json!({
            "argument_kind": "array",
            "argument_count": items.len(),
        }),
        Value::Null => json!({
            "argument_kind": "null",
            "argument_count": 0,
        }),
        other => json!({
            "argument_kind": json_value_kind(other),
            "argument_count": 1,
        }),
    }
}

const fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::missing_const_for_fn
)]
fn duration_to_ms(duration: Duration) -> u64 {
    let millis = duration.as_millis();
    if millis > u128::from(u64::MAX) {
        u64::MAX
    } else {
        millis as u64
    }
}

fn tool_filter_block_reason(
    tool_name: &str,
    allowed_tools: Option<&[String]>,
    denied_tools: Option<&[String]>,
) -> Option<String> {
    if let Some(denied_tools) = denied_tools {
        if denied_tools.iter().any(|name| name == tool_name) {
            return Some(format!(
                "tool '{tool_name}' is blocked because it is listed in denied_tools: [{}]",
                denied_tools.join(", ")
            ));
        }
    }

    if let Some(allowed_tools) = allowed_tools {
        if !allowed_tools.iter().any(|name| name == tool_name) {
            return Some(format!(
                "tool '{tool_name}' is blocked because it is not listed in allowed_tools: [{}]",
                allowed_tools.join(", ")
            ));
        }
    }

    None
}

const fn tool_error_kind(err: &ToolError) -> &'static str {
    match err {
        ToolError::PermissionDenied(_) => "permission_denied",
        ToolError::SchemaInvalid(_) => "schema_invalid",
        ToolError::HandlerPanic(_) => "handler_panic",
        ToolError::Timeout { .. } => "timeout",
        ToolError::PathOutsideWorktree(_) => "path_outside_worktree",
        ToolError::CommandNotAllowed(_) => "command_not_allowed",
        ToolError::NetworkBlocked(_) => "network_blocked",
        ToolError::Cancelled => "cancelled",
        _ => "other",
    }
}

impl std::fmt::Debug for ToolDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolDispatcher")
            .field("max_result_bytes", &self.max_result_bytes)
            .field("registry", &"Arc<dyn ToolRegistry>")
            .field("resolver", &"Arc<dyn HandlerResolver>")
            .field("safety", &self.safety.is_some())
            .field("hook_chain", &self.hook_chain)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use roko_core::tool::{
        AtomicCancel, AuditSink, CancelToken, NoopMetricsSink, NoopTraceSink, ToolCall,
        ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
        ToolPermission, ToolResult, VecToolRegistry,
    };
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    // ─── Mock handlers ────────────────────────────────────────────────

    struct EchoHandler;
    #[async_trait]
    impl ToolHandler for EchoHandler {
        fn name(&self) -> &str {
            "echo"
        }
        async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
            ToolResult::text(call.arguments.to_string())
        }
    }

    struct SleepHandler {
        ms: u64,
    }
    #[async_trait]
    impl ToolHandler for SleepHandler {
        fn name(&self) -> &str {
            "sleep"
        }
        async fn execute(&self, _call: ToolCall, _ctx: &ToolContext) -> ToolResult {
            tokio::time::sleep(Duration::from_millis(self.ms)).await;
            ToolResult::text("done")
        }
    }

    struct HugeHandler {
        payload_bytes: usize,
    }
    #[async_trait]
    impl ToolHandler for HugeHandler {
        fn name(&self) -> &str {
            "huge"
        }
        async fn execute(&self, _call: ToolCall, _ctx: &ToolContext) -> ToolResult {
            ToolResult::text("x".repeat(self.payload_bytes))
        }
    }

    // ─── Resolver helpers ─────────────────────────────────────────────

    fn resolver_from<const N: usize>(
        entries: [(&'static str, Arc<dyn ToolHandler>); N],
    ) -> Arc<dyn HandlerResolver> {
        let map: Vec<(&'static str, Arc<dyn ToolHandler>)> = entries.to_vec();
        Arc::new(move |name: &str| {
            map.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, h)| Arc::clone(h))
        })
    }

    fn tool(name: &str, perm: ToolPermission, conc: ToolConcurrency) -> ToolDef {
        ToolDef::new(name, "x", ToolCategory::Meta, perm).with_concurrency(conc)
    }

    // ─── Registry that always rejects args ────────────────────────────

    /// Drop-in registry that proxies `get`/`all` to an inner one but
    /// forces `validate_args` to fail with a schema error for any known
    /// tool — used to exercise the SchemaInvalid branch.
    struct RejectingRegistry {
        inner: VecToolRegistry,
    }
    impl ToolRegistry for RejectingRegistry {
        fn get(&self, name: &str) -> Option<&ToolDef> {
            self.inner.get(name)
        }
        fn all(&self) -> &[ToolDef] {
            self.inner.all()
        }
        fn validate_args(
            &self,
            name: &str,
            _args: &serde_json::Value,
        ) -> roko_core::error::Result<()> {
            if self.inner.get(name).is_some() {
                Err(roko_core::error::RokoError::invalid(
                    "missing required field: path",
                ))
            } else {
                Err(roko_core::error::RokoError::invalid(format!(
                    "unknown tool: {name}"
                )))
            }
        }
    }

    #[derive(Debug, Default)]
    struct CollectAuditSink {
        signals: Mutex<Vec<Engram>>,
    }

    impl CollectAuditSink {
        fn snapshot(&self) -> Vec<Engram> {
            self.signals.lock().expect("audit signals lock").clone()
        }
    }

    impl AuditSink for CollectAuditSink {
        fn emit(&self, signal: Engram) {
            self.signals
                .lock()
                .expect("audit signals lock")
                .push(signal);
        }
    }

    fn status_phases(signals: &[Engram]) -> Vec<(String, String)> {
        signals
            .iter()
            .map(|signal| {
                (
                    signal.tag("phase").unwrap_or_default().to_string(),
                    signal.tag("status").unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    // ─── Tests ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn unknown_tool_returns_other_error() {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "echo",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([("echo", Arc::new(EchoHandler) as Arc<dyn ToolHandler>)]);
        let d = ToolDispatcher::new(registry, resolver);
        let call = ToolCall::new("c", "no_such_tool", serde_json::json!({}));
        let res = d.dispatch(call, &ToolContext::testing("/tmp")).await;
        match res {
            ToolResult::Err(ToolError::Other(msg)) => assert!(msg.contains("no_such_tool")),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_args_returns_schema_invalid() {
        let inner = VecToolRegistry::from_tools(vec![tool(
            "echo",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]);
        let registry: Arc<dyn ToolRegistry> = Arc::new(RejectingRegistry { inner });
        let resolver = resolver_from([("echo", Arc::new(EchoHandler) as Arc<dyn ToolHandler>)]);
        let d = ToolDispatcher::new(registry, resolver);
        let call = ToolCall::new("c", "echo", serde_json::json!({}));
        let res = d.dispatch(call, &ToolContext::testing("/tmp")).await;
        match res {
            ToolResult::Err(ToolError::SchemaInvalid(msg)) => {
                assert!(msg.contains("missing required field"));
            }
            other => panic!("expected SchemaInvalid, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_permission_returns_permission_denied() {
        // Tool requires write, context only grants read.
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "echo",
            ToolPermission::writes(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([("echo", Arc::new(EchoHandler) as Arc<dyn ToolHandler>)]);
        let d = ToolDispatcher::new(registry, resolver);
        let call = ToolCall::new("c", "echo", serde_json::json!({}));

        let read_only_perms = ToolPermission::read_only();
        let ctx = ToolContext::new(
            "/tmp",
            Duration::from_secs(5),
            read_only_perms,
            Arc::new(roko_core::tool::NoopAuditSink),
            Arc::new(roko_core::tool::NoopTraceSink),
            Arc::new(roko_core::tool::NoopMetricsSink),
            Arc::new(roko_core::tool::NeverCancel),
        );
        let res = d.dispatch(call, &ctx).await;
        match res {
            ToolResult::Err(ToolError::PermissionDenied(msg)) => {
                assert!(msg.contains("echo"), "msg should name the tool: {msg}");
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn allowlist_blocks_unlisted_tool_with_clear_error() {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "echo",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([("echo", Arc::new(EchoHandler) as Arc<dyn ToolHandler>)]);
        let d = ToolDispatcher::new(registry, resolver);
        let ctx = ToolContext::testing("/tmp")
            .with_allowed_tools(Some(vec!["read_file".into(), "grep".into()]));
        let res = d
            .dispatch(ToolCall::new("c", "echo", serde_json::json!({})), &ctx)
            .await;
        match res {
            ToolResult::Err(ToolError::PermissionDenied(msg)) => {
                assert!(msg.contains("echo"), "msg should name the tool: {msg}");
                assert!(
                    msg.contains("allowed_tools"),
                    "msg should explain the allowlist reason: {msg}"
                );
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn denylist_blocks_listed_tool_with_clear_error() {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "echo",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([("echo", Arc::new(EchoHandler) as Arc<dyn ToolHandler>)]);
        let d = ToolDispatcher::new(registry, resolver);
        let ctx = ToolContext::testing("/tmp")
            .with_allowed_tools(Some(vec!["echo".into(), "grep".into()]))
            .with_denied_tools(Some(vec!["echo".into()]));
        let res = d
            .dispatch(ToolCall::new("c", "echo", serde_json::json!({})), &ctx)
            .await;
        match res {
            ToolResult::Err(ToolError::PermissionDenied(msg)) => {
                assert!(msg.contains("echo"), "msg should name the tool: {msg}");
                assert!(
                    msg.contains("denied_tools"),
                    "msg should explain the denylist reason: {msg}"
                );
            }
            other => panic!("expected PermissionDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handler_timeout_returns_timeout_error_with_ms() {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "sleep",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([(
            "sleep",
            Arc::new(SleepHandler { ms: 500 }) as Arc<dyn ToolHandler>,
        )]);
        let d = ToolDispatcher::new(registry, resolver);
        let call = ToolCall::new("c", "sleep", serde_json::json!({}));
        let ctx = ToolContext::new(
            "/tmp",
            Duration::from_millis(50),
            ToolPermission::read_only(),
            Arc::new(roko_core::tool::NoopAuditSink),
            Arc::new(roko_core::tool::NoopTraceSink),
            Arc::new(roko_core::tool::NoopMetricsSink),
            Arc::new(roko_core::tool::NeverCancel),
        );
        let res = d.dispatch(call, &ctx).await;
        match res {
            ToolResult::Err(ToolError::Timeout { after_ms }) => {
                assert!(
                    after_ms < 400,
                    "after_ms={after_ms} should be near 50ms cap, not ~500ms handler sleep"
                );
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancellation_returns_cancelled() {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "sleep",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([(
            "sleep",
            Arc::new(SleepHandler { ms: 2_000 }) as Arc<dyn ToolHandler>,
        )]);
        let d = ToolDispatcher::new(registry, resolver);
        let cancel = Arc::new(AtomicCancel::new());
        let ctx = ToolContext::new(
            "/tmp",
            Duration::from_secs(5),
            ToolPermission::read_only(),
            Arc::new(roko_core::tool::NoopAuditSink),
            Arc::new(roko_core::tool::NoopTraceSink),
            Arc::new(roko_core::tool::NoopMetricsSink),
            cancel.clone() as Arc<dyn CancelToken>,
        );
        let call = ToolCall::new("c", "sleep", serde_json::json!({}));

        let tripper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            tripper.cancel();
        });
        let res = d.dispatch(call, &ctx).await;
        assert!(
            matches!(res, ToolResult::Err(ToolError::Cancelled)),
            "expected Cancelled, got {res:?}"
        );
    }

    #[tokio::test]
    async fn successful_call_returns_ok() {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "echo",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([("echo", Arc::new(EchoHandler) as Arc<dyn ToolHandler>)]);
        let d = ToolDispatcher::new(registry, resolver);
        let call = ToolCall::new("c", "echo", serde_json::json!({"x": 1}));
        let res = d.dispatch(call, &ToolContext::testing("/tmp")).await;
        match res {
            ToolResult::Ok { content, .. } => assert!(content.contains("\"x\"")),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn successful_call_emits_audit_signals_for_each_phase() {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "echo",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([("echo", Arc::new(EchoHandler) as Arc<dyn ToolHandler>)]);
        let d = ToolDispatcher::new(registry, resolver);
        let audit_sink = Arc::new(CollectAuditSink::default());
        let ctx = ToolContext::testing("/tmp").with_audit_sink(audit_sink.clone());

        let res = d
            .dispatch(
                ToolCall::new("c", "echo", serde_json::json!({"x": 1})),
                &ctx,
            )
            .await;
        assert!(res.is_ok(), "expected successful tool call, got {res:?}");

        let phases = status_phases(&audit_sink.snapshot());
        assert_eq!(
            phases,
            vec![
                ("validation".to_string(), "passed".to_string()),
                ("permission".to_string(), "granted".to_string()),
                ("handler".to_string(), "started".to_string()),
                ("completion".to_string(), "succeeded".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn permission_denial_emits_failure_audit_signals() {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "echo",
            ToolPermission::writes(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([("echo", Arc::new(EchoHandler) as Arc<dyn ToolHandler>)]);
        let d = ToolDispatcher::new(registry, resolver);
        let audit_sink = Arc::new(CollectAuditSink::default());
        let ctx = ToolContext::new(
            "/tmp",
            Duration::from_secs(5),
            ToolPermission::read_only(),
            audit_sink.clone(),
            Arc::new(NoopTraceSink),
            Arc::new(NoopMetricsSink),
            Arc::new(roko_core::tool::NeverCancel),
        );

        let res = d
            .dispatch(ToolCall::new("c", "echo", serde_json::json!({})), &ctx)
            .await;
        assert!(matches!(
            res,
            ToolResult::Err(ToolError::PermissionDenied(_))
        ));

        let phases = status_phases(&audit_sink.snapshot());
        assert_eq!(
            phases,
            vec![
                ("validation".to_string(), "passed".to_string()),
                ("permission".to_string(), "denied".to_string()),
                ("completion".to_string(), "failed".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn oversized_content_truncated_with_marker() {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "huge",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([(
            "huge",
            Arc::new(HugeHandler {
                payload_bytes: 5_000,
            }) as Arc<dyn ToolHandler>,
        )]);
        let d = ToolDispatcher::new(registry, resolver).with_max_result_bytes(1_024);
        let call = ToolCall::new("c", "huge", serde_json::json!({}));
        let res = d.dispatch(call, &ToolContext::testing("/tmp")).await;
        match res {
            ToolResult::Ok { content, .. } => {
                assert!(content.contains("[truncated]"));
                assert!(
                    content.len() < 5_000,
                    "content should be shorter than the handler output"
                );
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn truncation_respects_utf8_char_boundary() {
        // Handler emits "日本語" repeated many times (each char is 3 bytes).
        struct MultibyteHandler;
        #[async_trait]
        impl ToolHandler for MultibyteHandler {
            fn name(&self) -> &str {
                "mb"
            }
            async fn execute(&self, _call: ToolCall, _ctx: &ToolContext) -> ToolResult {
                let chunk = "日本語";
                ToolResult::text(chunk.repeat(500)) // 500*9 = 4500 bytes
            }
        }
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "mb",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([("mb", Arc::new(MultibyteHandler) as Arc<dyn ToolHandler>)]);
        let d = ToolDispatcher::new(registry, resolver).with_max_result_bytes(100);
        let call = ToolCall::new("c", "mb", serde_json::json!({}));
        let res = d.dispatch(call, &ToolContext::testing("/tmp")).await;
        match res {
            ToolResult::Ok { content, .. } => {
                // Must be valid UTF-8.
                let _ = std::str::from_utf8(content.as_bytes())
                    .expect("truncated multibyte content must be valid UTF-8");
                assert!(content.contains("[truncated]"));
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_batch_runs_parallel_tools_concurrently() {
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "sleep",
            ToolPermission::read_only(),
            ToolConcurrency::Parallel,
        )]));
        let resolver = resolver_from([(
            "sleep",
            Arc::new(SleepHandler { ms: 100 }) as Arc<dyn ToolHandler>,
        )]);
        let d = ToolDispatcher::new(registry, resolver);
        let ctx = ToolContext::testing("/tmp");
        let calls = vec![
            ToolCall::new("a", "sleep", serde_json::json!({})),
            ToolCall::new("b", "sleep", serde_json::json!({})),
            ToolCall::new("c", "sleep", serde_json::json!({})),
        ];
        let started = Instant::now();
        let out = d.dispatch_batch(calls, &ctx).await;
        let elapsed = started.elapsed();
        assert_eq!(out.len(), 3);
        assert!(
            out.iter().all(|(_, r)| r.is_ok()),
            "all three should succeed"
        );
        // Parallel: wall time should be well under 2× single-call time.
        assert!(
            elapsed < Duration::from_millis(200),
            "expected ~100ms parallel wall-time, got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn dispatch_batch_runs_serial_tools_sequentially() {
        // Handler increments a shared counter AFTER sleeping, so if the
        // dispatcher ran calls concurrently the counter observations
        // would interleave; with serial dispatch each call's own
        // "before sleep" counter read equals the number of previous
        // completions.
        static COUNTER: AtomicUsize = AtomicUsize::new(0);

        struct SerialHandler;
        #[async_trait]
        impl ToolHandler for SerialHandler {
            fn name(&self) -> &str {
                "ser"
            }
            async fn execute(&self, _call: ToolCall, _ctx: &ToolContext) -> ToolResult {
                let observed = COUNTER.load(Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(60)).await;
                COUNTER.fetch_add(1, Ordering::SeqCst);
                ToolResult::text(observed.to_string())
            }
        }
        COUNTER.store(0, Ordering::SeqCst);
        let registry: Arc<dyn ToolRegistry> = Arc::new(VecToolRegistry::from_tools(vec![tool(
            "ser",
            ToolPermission::read_only(),
            ToolConcurrency::Serial,
        )]));
        let resolver = resolver_from([("ser", Arc::new(SerialHandler) as Arc<dyn ToolHandler>)]);
        let d = ToolDispatcher::new(registry, resolver);
        let ctx = ToolContext::testing("/tmp");
        let calls = vec![
            ToolCall::new("a", "ser", serde_json::json!({})),
            ToolCall::new("b", "ser", serde_json::json!({})),
            ToolCall::new("c", "ser", serde_json::json!({})),
        ];
        let started = Instant::now();
        let out = d.dispatch_batch(calls, &ctx).await;
        let elapsed = started.elapsed();
        assert_eq!(out.len(), 3);
        // Serial wall time ≈ 3 × 60ms = 180ms; allow slack but must be
        // substantially more than a single handler's sleep.
        assert!(
            elapsed >= Duration::from_millis(150),
            "expected serial wall-time ≥ 150ms, got {elapsed:?}"
        );
        // Each call's observed counter should be strictly increasing,
        // proving they ran one-after-the-other.
        let observations: Vec<usize> = out
            .iter()
            .map(|(_, r)| match r {
                ToolResult::Ok { content, .. } => content.parse().expect("observation is usize"),
                ToolResult::Err(e) => panic!("handler failed: {e}"),
            })
            .collect();
        assert_eq!(observations, vec![0, 1, 2]);
    }
}
