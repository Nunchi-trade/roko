# Task 041: Redesign Trigger Trait + Wire BusTrigger into Event Subscriptions

```toml
id = 41
title = "Redesign Trigger trait to async with TriggerBinding + implement BusTrigger + wire into roko config subscriptions"
track = "v2-core-abstractions"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-core/src/traits.rs",
    "crates/roko-core/src/lib.rs",
    "crates/roko-core/src/bus_trigger.rs",
    "crates/roko-core/src/config/subscriptions.rs",
    "crates/roko-cli/src/subscriptions.rs",
    "crates/roko-cli/src/commands/config_cmd.rs",
    "crates/roko-cli/src/main.rs",
]
exclusive_files = []
estimated_minutes = 180
```

## Context

The Trigger trait exists in `roko-core/src/traits.rs` but it's a minimal stub:

```rust
pub trait Trigger: crate::cell::Cell {
    fn arm(&self) -> Result<()>;
    fn disarm(&self) -> Result<()>;
}
```

This has three problems:
1. It's synchronous — triggers often need async I/O for watching topics
2. It has no `check()` method — no way to evaluate if the trigger should fire
3. It returns nothing from `arm()` — no way to know what the trigger is watching

The v2 spec gives Trigger a check-and-fire model: arm returns a TriggerBinding describing
what it watches, check evaluates incoming pulses and optionally fires output signals.

This task redesigns Trigger, implements `BusTrigger` that watches Bus topics, and wires it
into the event subscription system that already exists in `roko config subscriptions`.

Checklist items: P1-13, P1-14.

## Background

Read these files before starting:

1. `crates/roko-core/src/traits.rs` — current Trigger stub (lines 417-425)
2. `crates/roko-core/src/pulse.rs` — Pulse struct, TopicFilter enum
3. `crates/roko-core/src/config/subscriptions.rs` — SubscriptionConfig type
4. `crates/roko-cli/src/main.rs` — find the `config subscriptions` subcommand handler
5. `tmp/v2-refactoring/06-NEW-PROTOCOLS.md` — the v2 spec for Trigger

Also understand the existing subscription system:
```bash
grep -rn 'SubscriptionConfig\|subscriptions' crates/roko-core/src/config/ --include='*.rs' | head -20
grep -rn 'subscription\|Subscription' crates/roko-cli/src/main.rs --include='*.rs' | head -20
grep -rn 'TopicFilter' crates/roko-core/src/pulse.rs | head -10
```

And check how the PRD auto-plan trigger currently works (an existing trigger-like pattern):
```bash
grep -rn 'prd_publish_subscriber\|auto_plan' crates/roko-serve/src/ --include='*.rs' | head -10
```

Current source notes that supersede the illustrative snippets below:

- `TopicFilter` is in `crates/roko-core/src/pulse.rs` with variants
  `Exact(Topic)`, `Prefix(String)`, and `All`, plus `matches(&Topic)`.
- `Pulse::new` currently takes `(seq: u64, topic: Topic, kind: Kind, body: Body)`.
  Do not assume a string-only constructor exists.
- `SubscriptionConfig` has `template`, `trigger`, `trigger_config`, filters, and
  scheduling fields. It does not have `name` or `topic_filter`.
- The active subscription list path is
  `crates/roko-cli/src/commands/config_cmd.rs` `ConfigSubscriptionCmd::List` ->
  `roko_cli::subscriptions::cmd_list`.
- `roko_cli::subscriptions::cmd_list` loads
  `roko_serve::dispatch::SubscriptionRegistry` and maps runtime subscriptions to
  rows with `id`, `template`, `trigger`, and `enabled`. Wire the visible Trigger
  call there unless a deeper central pulse-dispatch call site is already ready.
- `Engram::builder` currently requires a `Kind`; there is no `Kind::Event` and
  no `.author(...)`. Use `Kind::Metric` or `Kind::Custom("trigger.fired".into())`,
  `Body::Json(...)`, and `Provenance::agent(...)`, or the `Signal` alias if
  Task 037 has landed.

## What to Change

### 1. Add TriggerBinding struct to roko-core

```rust
use crate::TopicFilter;
use serde::{Serialize, Deserialize};

/// Describes what a Trigger is watching after being armed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerBinding {
    /// The topic filter this trigger matches against.
    pub filter: TopicFilter,
    /// Human-readable description of what this trigger watches.
    pub description: String,
}

impl TriggerBinding {
    pub fn new(filter: TopicFilter, description: impl Into<String>) -> Self {
        Self { filter, description: description.into() }
    }
}
```

### 2. Redesign Trigger trait in `crates/roko-core/src/traits.rs`

Replace the existing Trigger trait:

```rust
/// Event-driven activation — watches for conditions and fires when criteria are met.
///
/// Triggers subscribe to Bus topics or poll external state, then emit activation
/// Signals when their condition is satisfied. They're the reactive complement to
/// Observe (which is pull-based).
///
/// # Lifecycle
///
/// ```text
/// arm() -> TriggerBinding -> [waiting] -> check(pulses) -> Some(signals) -> [fire!]
///                                      -> check(pulses) -> None            -> [continue waiting]
/// disarm() -> [inactive]
/// ```
#[async_trait]
pub trait Trigger: Cell {
    /// Arm the trigger. Returns a TriggerBinding describing what it will watch.
    async fn arm(&self, ctx: &Context) -> Result<TriggerBinding>;

    /// Check if the trigger should fire given a batch of pulses.
    /// Returns Some(signals) if the trigger fires, None if it doesn't.
    async fn check(&self, pulses: &[Pulse], ctx: &Context) -> Result<Option<Vec<Engram>>>;

    /// Disarm the trigger, stopping all watches.
    async fn disarm(&self, ctx: &Context) -> Result<()>;
}
```

### 3. Implement BusTrigger

Create `crates/roko-core/src/bus_trigger.rs`:

```rust
use crate::{
    Cell, CellVersion, Context, Engram, Kind, Body, Provenance, Pulse, TopicFilter,
    traits::Trigger,
    error::Result,
};
use async_trait::async_trait;

/// A Trigger that watches Bus topics and fires when matching pulses arrive.
///
/// This is the simplest useful Trigger: it watches for pulses matching a
/// TopicFilter and emits a signal when one arrives.
pub struct BusTrigger {
    /// Unique name for this trigger instance.
    name: String,
    /// The topic filter to match against.
    filter: TopicFilter,
    /// Whether this trigger is currently armed.
    armed: std::sync::atomic::AtomicBool,
}

impl BusTrigger {
    pub fn new(name: impl Into<String>, filter: TopicFilter) -> Self {
        Self {
            name: name.into(),
            filter,
            armed: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Cell for BusTrigger {
    fn cell_id(&self) -> &str { &self.name }
    fn cell_name(&self) -> &str { &self.name }
    fn protocols(&self) -> &[&str] { &["Trigger"] }
}

#[async_trait]
impl Trigger for BusTrigger {
    async fn arm(&self, _ctx: &Context) -> Result<TriggerBinding> {
        self.armed.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(TriggerBinding::new(
            self.filter.clone(),
            format!("BusTrigger '{}' watching for matching pulses", self.name),
        ))
    }

    async fn check(&self, pulses: &[Pulse], _ctx: &Context) -> Result<Option<Vec<Engram>>> {
        if !self.armed.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(None);
        }

        let matching: Vec<&Pulse> = pulses.iter()
            .filter(|p| self.filter.matches(&p.topic))
            .collect();

        if matching.is_empty() {
            return Ok(None);
        }

        // Create a trigger-fired signal containing the matching pulse info
        let body = serde_json::json!({
            "trigger": self.name,
            "matched_count": matching.len(),
            "topics": matching.iter().map(|p| p.topic.to_string()).collect::<Vec<_>>(),
        });

        let signal = Engram::builder(Kind::Custom("trigger.fired".into()))
            .body(Body::Json(body))
            .provenance(Provenance::agent(&self.name))
            .build();

        Ok(Some(vec![signal]))
    }

    async fn disarm(&self, _ctx: &Context) -> Result<()> {
        self.armed.store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}
```

**Important**: Check how `TopicFilter::matches()` works. It's defined in `pulse.rs`. Make
sure the method signature matches what you're calling.

### 4. Wire into `roko config subscriptions`

The existing `roko config subscriptions list` command shows configured event subscriptions.
Add trigger status information:

Add mapping helpers in `crates/roko-core/src/config/subscriptions.rs`:

```rust
pub fn topic_filter_from_trigger(trigger: &str) -> TopicFilter {
    let trigger = trigger.trim();
    if trigger.is_empty() || trigger == "*" {
        TopicFilter::All
    } else if let Some(prefix) = trigger.strip_suffix('*') {
        TopicFilter::Prefix(prefix.to_string())
    } else {
        TopicFilter::Exact(Topic::new(trigger))
    }
}

impl SubscriptionConfig {
    pub fn trigger_topic_filter(&self) -> TopicFilter {
        topic_filter_from_trigger(&self.trigger)
    }
}
```

Then update `crates/roko-cli/src/subscriptions.rs::cmd_list`:

```rust
use roko_core::traits::Trigger;

// For each subscription config, create a BusTrigger and show its binding info
for sub in &subscriptions {
    let filter = topic_filter_from_trigger(&sub.trigger);
    let trigger = BusTrigger::new(&sub.id, filter);
    let ctx = Context::now();
    let binding = trigger.arm(&ctx).await?;
    // include binding.description or binding.filter in SubscriptionRow
}
```

Because `Trigger::arm` is async, change `cmd_list` to async and update the
`ConfigSubscriptionCmd::List` dispatch arm in `commands/config_cmd.rs` from
`roko_cli::subscriptions::cmd_list(...)?` to
`roko_cli::subscriptions::cmd_list(...).await?`.

If the command shape has moved, check:
```bash
grep -rn 'subscriptions.*list\|cmd_subscriptions\|SubscriptionCmd' crates/roko-cli/src/ --include='*.rs' | head -20
```

The command already exists in the current tree; do not add a second list command.

### 5. Add integration test for BusTrigger

```rust
#[tokio::test]
async fn bus_trigger_fires_on_matching_pulse() {
    let trigger = BusTrigger::new("test-trigger", TopicFilter::Exact(Topic::new("test.event")));
    let ctx = Context::now();

    // Arm
    let binding = trigger.arm(&ctx).await.unwrap();
    assert!(binding.description.contains("test-trigger"));

    // Check with non-matching pulse
    let wrong_pulse = Pulse::new(1, Topic::new("other.event"), Kind::Metric, Body::Json(serde_json::json!({})));
    let result = trigger.check(&[wrong_pulse], &ctx).await.unwrap();
    assert!(result.is_none());

    // Check with matching pulse
    let right_pulse = Pulse::new(2, Topic::new("test.event"), Kind::Metric, Body::Json(serde_json::json!({})));
    let result = trigger.check(&[right_pulse], &ctx).await.unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().len(), 1);

    // Disarm
    trigger.disarm(&ctx).await.unwrap();
    let result = trigger.check(&[right_pulse], &ctx).await.unwrap();
    assert!(result.is_none()); // Should not fire when disarmed
}
```

**Note**: Check how `Pulse::new()` works. The constructor may differ from what's shown.
Read `pulse.rs` carefully.

### 6. Export from lib.rs

Add to `crates/roko-core/src/lib.rs`:
```rust
pub mod bus_trigger;
pub use bus_trigger::BusTrigger;
pub use traits::TriggerBinding;  // if defined in traits.rs
```

## Mechanical Implementation Plan

1. Add `TriggerBinding` and update `Trigger` to async `arm`, `check`, and `disarm`.
2. Add `bus_trigger.rs`, implement `Cell` and `Trigger`, and export `BusTrigger`.
3. Add `topic_filter_from_trigger` and `SubscriptionConfig::trigger_topic_filter`.
4. Update subscription list rows to include a trigger binding/filter field in both
   human and JSON output.
5. Make `roko_cli::subscriptions::cmd_list` async and await `Trigger::arm`.
6. Update the config dispatch arm to await `cmd_list`.
7. Add unit tests for filter mapping and BusTrigger lifecycle, then update
   subscription list formatting tests.

Expected visible runtime path:

`roko config subscriptions list` -> `commands/config_cmd.rs::dispatch_config` ->
`roko_cli::subscriptions::cmd_list(...).await` -> `SubscriptionRegistry::load` ->
`topic_filter_from_trigger(&subscription.trigger)` -> `BusTrigger::new(...)` ->
`Trigger::arm(&trigger, &ctx).await` -> output row includes binding/filter.

## What NOT to Do

- Do NOT build a TriggerRegistry or TriggerScheduler. The Graph engine (Phase 2) manages that.
- Do NOT implement Trigger for file watchers, webhooks, or cron yet. Just BusTrigger.
- Do NOT modify the PRD auto-plan trigger in roko-serve. That's a separate system that may
  eventually be reimplemented as a Trigger, but not in this task.
- Do NOT add background Trigger polling. BusTrigger is checked on-demand via check().
  Background polling comes with Hot Graphs in Phase 4.
- Do NOT add Trigger to the universal loop. Triggers are event-driven, not tick-driven.
- Do NOT add `name` or `topic_filter` fields to `SubscriptionConfig`; map from the
  existing `trigger` string.
- Do NOT create a nested Tokio runtime or `block_on` in subscription list.
- Do NOT add a second subscription list command. Wire the existing
  `roko_cli::subscriptions::cmd_list` path.
- Do NOT use nonexistent APIs such as `Kind::Event`, string-only `Pulse::new`, or
  `Engram::builder()` without a `Kind`.

## Wire Target

```bash
cargo run -p roko-cli -- config subscriptions list
# Should show subscription entries with trigger binding information
# Example output:
#   prd-auto-plan:
#     filter: prd.published
#     trigger: BusTrigger 'prd-auto-plan' watching for matching pulses
```

If `config subscriptions list` doesn't exist, the wire target is the integration test:

```bash
cargo test -p roko-core -- bus_trigger
# Should show: test for arm/check/disarm lifecycle passing
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `cargo test -p roko-core -- bus_trigger` — lifecycle test passes
- [ ] `grep -rn 'BusTrigger' crates/ --include='*.rs' | grep -v target/ | grep -v test` — shows at least one non-test callsite
- [ ] `grep -rn 'TriggerBinding' crates/roko-core/src/ --include='*.rs' | grep -v target/` — struct exists and is exported
- [ ] `grep -rn 'async fn arm\|async fn check\|async fn disarm' crates/roko-core/src/traits.rs` — 3 async methods on Trigger
- [ ] TopicFilter::matches() is called correctly (verify the method signature matches usage)
- [ ] `grep -rn 'Trigger::arm\|\.arm(&ctx).*await\|BusTrigger' crates/roko-cli/src/subscriptions.rs crates/roko-cli/src/commands/config_cmd.rs --include='*.rs'` — shows a non-test visible callsite

## Status Log

| Time | Agent | Action |
|------|-------|--------|
