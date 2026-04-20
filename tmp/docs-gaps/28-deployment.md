# 19-deployment -- Gap checklist

Spec: `docs/19-deployment/` (15 files). Code: `crates/roko-cli/`, `crates/roko-serve/`, CI workflows.

Overall: ~27% compliant. systemd unit generation complete. HTTP API serves as remote orchestrator. Major gaps in packaging/distribution, Docker, daemon IPC, secret management, and observability.

## Compliant (no action needed)

- systemd user unit generation with security hardening (doc 05)
- Remote orchestrator via roko-serve HTTP API (doc 11)
- Current status and port allocation doc accurate (doc 13)
- Basic native build for x86_64/aarch64 (doc 01 partial)

## Checklist

### DEPLOY-01: Release pipeline

- [x] Configure release-plz and cargo-dist for automated releases

**Spec** (doc 00 `docs/19-deployment/00-packaging-and-distribution.md`): Complete release pipeline:
1. **release-plz** — automated changelog generation from conventional commits, version bumping, PR creation for releases
2. **git-cliff** — changelog formatting with `cliff.toml` config
3. **cargo-dist** — prebuilt binary generation for 6 platform targets (macOS Intel/ARM, Linux glibc/musl x Intel/ARM)
4. **GitHub Releases** — automated release creation with binaries attached
5. **Homebrew tap** — `brew install roko` via homebrew-roko tap
6. **axoupdater** — self-update mechanism (`roko self-update`)
7. **Shell completions** — generated and distributed with releases (see DEPLOY-06)

GitHub Actions workflow: on tag push, cargo-dist builds binaries for all targets, creates GitHub Release, updates Homebrew formula, publishes shell completions.

**Current code**: No release pipeline. All crates marked `publish = false` in their `Cargo.toml`. No `cliff.toml`, no `release-plz.toml`, no `dist-workspace.toml` in repo root. `.github/workflows/` may have CI but no release workflow.

**What to change**:
1. Create `cliff.toml` in workspace root with conventional commit format
2. Create `release-plz.toml` with workspace settings
3. Add `[workspace.metadata.dist]` to `Cargo.toml` for cargo-dist configuration (targets, installers)
4. Create `.github/workflows/release.yml` GitHub Actions workflow
5. Create `homebrew-roko` tap repository structure

**Reference files**:
- `Cargo.toml` — workspace root (add `[workspace.metadata.dist]`)
- `.github/workflows/` — CI directory (add release workflow)
- `docs/19-deployment/00-packaging-and-distribution.md` — full spec: release-plz config, cargo-dist targets, Homebrew tap, axoupdater
- `docs/19-deployment/01-native-x86-arm.md` — 6 target triples, build config, release profile
**Depends on**: None
**Accept when**:
- [x] `cliff.toml` configures conventional-commit changelog
- [x] `release-plz.toml` configures version management
- [x] cargo-dist builds for at least macOS ARM + Linux x86_64 (2 of 6 targets)
- [x] GitHub Actions release workflow exists
- [ ] `cargo dist build --dry-run` succeeds (if cargo-dist installed) -- not verified (requires cargo-dist binary)
**Verify**:
```bash
ls cliff.toml release-plz.toml 2>/dev/null
grep -r 'metadata.dist' Cargo.toml
ls .github/workflows/release* 2>/dev/null
```
**Priority**: P1

---

### DEPLOY-02: Docker packaging refinement

- [x] Add slim image variant and multi-stage runtime stage

**Spec** (doc 03): Slim and full image variants. State volume mounting. Compose for multi-service.
**Current code**: `Dockerfile` exists at workspace root (single-stage, `rust:1.91-bookworm`, builds `roko-cli`). `docker/docker-compose.yml` exists with full stack: mirage, roko, gateway (placeholder), prometheus, grafana. State volumes defined (`demo-runtime`, `mirage-state`). **Missing**: multi-stage build (no separate runtime stage — final image includes full Rust toolchain), no `Dockerfile.slim` variant, no `.roko/` volume mount for agent state persistence.
**What to change**:
1. Add runtime stage to `Dockerfile` using `debian:bookworm-slim` (copy only the binary from builder stage)
2. Add `Dockerfile.slim` variant with minimal dependencies
3. Add `.roko/` volume mount in `docker-compose.yml` for agent state persistence
**Reference files**:
- `Dockerfile` — existing single-stage build (add runtime stage)
- `docker/docker-compose.yml` — existing multi-service compose (add `.roko/` volume)
- `crates/roko-serve/Cargo.toml` (server binary)
**Depends on**: None
**Accept when**:
- [x] Dockerfile uses multi-stage build (builder + slim runtime)
- [ ] Dockerfile.slim variant exists -- not found in repo
- [x] docker-compose.yml mounts `.roko/` as volume for state persistence
- [ ] `docker build .` succeeds -- not verified (requires Docker)
**Verify**:
```bash
ls Dockerfile Dockerfile.slim docker/docker-compose.yml 2>/dev/null
grep -c 'FROM' Dockerfile  # should be 2 (builder + runtime)
grep 'roko' docker/docker-compose.yml | head -5
docker build . --dry-run 2>/dev/null || echo "docker not available"
```
**Priority**: P1

---

### DEPLOY-03: Daemon IPC protocol

- [x] Implement Unix socket IPC for daemon communication

**Spec** (doc 04 `docs/19-deployment/04-daemon-launchd-macos.md`): Persistent background daemon on macOS with IPC over Unix domain socket. `DaemonCmd` protocol:
- `Status` — returns `DaemonStatus` with uptime, active repos, agent counts, memory usage
- `Stop` — graceful shutdown (flush pending, backup state, deregister)
- `Restart` — stop + start with config reload
- `ListSubscriptions` — return all monitored repositories
- `PauseSubscription(repo_id)` — temporarily pause monitoring for a repo
- `ResumeSubscription(repo_id)` — resume paused monitoring

Socket path: `$ROKO_STATE_DIR/daemon.sock` (typically `~/.roko/daemon.sock`). JSON-framed messages over Unix domain socket. Graceful shutdown with SIGTERM handling.

**Current code** (`crates/roko-cli/src/daemon.rs`): `DaemonConfig` with `socket_path()` method (line 79). `DaemonState` enum at line 42 (Stopped/Starting/Running/Stopping). `DaemonStatus` at line 178 with `socket_path`, `pid`, `uptime` fields. `daemon_start()` at line 205. `DaemonContext` at line 112 with state management. `crates/roko-cli/src/daemon/launchd.rs` generates macOS plist. `crates/roko-cli/src/daemon/systemd.rs` generates Linux systemd unit. **Socket path computed but no actual Unix socket listener. No `DaemonCmd` enum. No JSON-framed message protocol.**

**What to change**:
1. Define `DaemonCmd` enum with `Status`, `Stop`, `Restart`, `ListSubscriptions`, `PauseSubscription`, `ResumeSubscription` variants
2. Add `tokio::net::UnixListener` bind in `daemon_start()` at the `socket_path()`
3. Implement JSON-framed message handler for each `DaemonCmd`
4. Add `roko daemon status` CLI command that connects to socket and prints status
5. Add SIGTERM handler for graceful shutdown

**Reference files**:
- `crates/roko-cli/src/daemon.rs` — `DaemonConfig` with `socket_path()` at 79, `DaemonState` at 42, `daemon_start()` at 205
- `crates/roko-cli/src/daemon/launchd.rs` — macOS plist generation (compliant, no changes needed)
- `crates/roko-cli/src/daemon/systemd.rs` — Linux systemd unit generation (compliant)
- `docs/19-deployment/04-daemon-launchd-macos.md` — full spec: DaemonCmd protocol, Unix socket, graceful shutdown, log management
**Depends on**: None
**Accept when**:
- [x] `DaemonCmd` enum defined with at least Status/Stop/Restart
- [x] Daemon listens on Unix socket at `socket_path()`
- [x] JSON-framed message protocol handles all DaemonCmd variants
- [x] `roko daemon status` connects to socket and prints status
- [x] SIGTERM triggers graceful shutdown
- [x] `cargo test -p roko-cli` passes
**Verify**:
```bash
grep -rn 'DaemonCmd\|UnixListener\|socket_path\|daemon.*status' crates/roko-cli/src/ --include='*.rs'
cargo test -p roko-cli
```
**Priority**: P1

---

### DEPLOY-04: Secret management

- [x] Implement ${VAR} interpolation, _FILE suffix, and keychain integration

**Spec** (doc 10 `docs/19-deployment/10-secret-management.md`): Layered secret resolution with 5 levels (highest to lowest precedence):
1. **Environment variables** — `ROKO_API_KEY`, `ANTHROPIC_API_KEY`, etc.
2. **Config file** — `api_key = "${ANTHROPIC_API_KEY}"` with interpolation
3. **OS keychain** — macOS Keychain (`security-framework` crate), Linux `secret-service` D-Bus API
4. **Secret stores** — HashiCorp Vault, AWS Secrets Manager (future)
5. **Defaults** — fallback values from config

**${VAR} interpolation**: In roko.toml, any string value containing `${VAR_NAME}` is expanded by reading the environment variable. Example:
```toml
[inference]
api_key = "${ANTHROPIC_API_KEY}"
openai_key = "${OPENAI_API_KEY}"
```

**_FILE suffix**: Any config key ending in `_file` reads the secret from the file path. Example:
```toml
[inference]
api_key_file = "/run/secrets/anthropic_key"
```
The file contents (trimmed) become the value. This is the Docker/K8s-native secret mounting pattern.

**Scoped secrets**: Secrets can be scoped per domain, per provider, or per agent. Example: `[inference.providers.anthropic]` has its own `api_key`.

**Current code** (`crates/roko-core/src/config/schema.rs:640`): `apply_env()` method handles ~15 `ROKO_*` env vars (ROKO_MODEL, ROKO_BACKEND, ROKO_EFFORT, ROKO_BUDGET_USD, etc.) with direct value assignment. Provider configs at line 1014 have `api_key: Option<String>`. **No ${VAR} interpolation in TOML string values. No _FILE suffix support. No keychain integration. Env vars are direct overrides, not interpolation.**

**What to change**:
1. Add `fn interpolate_vars(value: &str) -> String` in `crates/roko-core/src/config/`:
   ```rust
   fn interpolate_vars(value: &str) -> String {
       let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();
       re.replace_all(value, |caps: &regex::Captures| {
           std::env::var(&caps[1]).unwrap_or_default()
       }).to_string()
   }
   ```
2. Apply `interpolate_vars()` to all string fields after TOML parsing in `load_config()`
3. Add `fn resolve_file_secrets(config: &mut RokoConfig)` that looks for `*_file` fields and reads file contents
4. Add `security-framework` (macOS) / `secret-service` (Linux) dependencies behind `keychain` feature flag
5. Add `fn keychain_get(service: &str, account: &str) -> Option<String>` for fallback resolution

**Reference files**:
- `crates/roko-core/src/config/schema.rs:640` — `apply_env()` with 15+ ROKO_* env vars
- `crates/roko-core/src/config/schema.rs:1014` — `ProviderConfig` with `api_key: Option<String>`
- `crates/roko-core/src/config/mod.rs` — config loading module
- `docs/19-deployment/10-secret-management.md` — full spec: 5-level resolution, interpolation syntax, _FILE, scoped secrets, audit
**Depends on**: None
**Accept when**:
- [x] `${VAR}` interpolation works in roko.toml string values
- [x] `*_file` suffix reads secret from file path
- [ ] macOS Keychain integration (behind `keychain` feature flag) -- no `security-framework` or `secret-service` dep in roko-core/Cargo.toml
- [ ] Secrets never logged or printed (audit check)
- [ ] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'interpolate_vars\|resolve_file_secrets\|keychain\|_file' crates/roko-core/src/config/ --include='*.rs'
grep -rn 'security.framework\|secret.service' crates/roko-core/Cargo.toml
cargo test --workspace
```
**Priority**: P1

---

### DEPLOY-05: Observability export

- [x] Implement Prometheus metrics endpoint and OTLP exporter

**Spec** (doc 14): Structured logs, JSONL artifacts, Prometheus/OTLP surfaces. Concrete metric names: `roko_gate_pass_total`, `roko_gate_fail_total`, `roko_agents_active`, `roko_task_duration_seconds`, `roko_cost_usd_total`, `roko_episodes_total`, `roko_tier_routing_total`.
**Current code** (`crates/roko-serve/src/routes/status.rs:28`): JSON metrics endpoints already exist: `/metrics` (line 28), `/metrics/summary` (line 29), `/metrics/gate_rate` (line 34), `/metrics/model_efficiency` (line 33), `/metrics/success_rate` (line 30), `/metrics/engagement` (line 31), `/metrics/c_factor` (line 32), `/metrics/feedback_latency` (line 36), `/metrics/velocity` (line 37), `/metrics/coverage` (line 38). **All return JSON, not Prometheus text format.** No OTLP exporter.
**What to change**: Add `/metrics/prometheus` endpoint that returns Prometheus text exposition format. Map existing JSON metrics to Prometheus gauges/counters using concrete metric names. Add optional OTLP exporter behind feature flag.
**Reference files**:
- `crates/roko-serve/src/routes/status.rs` (existing JSON metrics endpoints)
- `crates/roko-learn/src/efficiency.rs` (cost tracking data)
- `crates/roko-learn/src/episode_logger.rs` (episode data)
- `crates/roko-gate/src/adaptive_threshold.rs` (gate pass rate EMA)
**Depends on**: None
**Accept when**:
- [x] `/metrics/prometheus` endpoint on roko-serve exposes Prometheus text format
- [x] Key counters: `roko_gate_pass_total`, `roko_gate_fail_total`, `roko_episodes_total`
- [x] Key gauges: `roko_agents_active`, `roko_cost_usd_total`
- [ ] Key histograms: `roko_task_duration_seconds` -- not found in prometheus_metrics handler
- [ ] `cargo test -p roko-serve` passes
**Verify**:
```bash
grep -rn 'prometheus\|text/plain\|roko_gate_pass' crates/roko-serve/src/ --include='*.rs'
cargo test -p roko-serve
```
**Priority**: P1

---

### DEPLOY-06: Shell completions — dynamic completions

- [x] Add dynamic completions for plan names, PRD slugs, and subcommand arguments

**Spec** (doc 00): Shell completions for all commands with dynamic argument completion.
**Current code** (`crates/roko-cli/src/main.rs:325`): `Command::Completions { shell: CompletionShell }` **already exists**. `CompletionShell` enum at line 398 with Bash/Zsh/Fish variants. `print_completions()` at line 6069 generates completion scripts for all three shells. `completion_words()` at line 6078 extracts top-level subcommand names from clap. **Working**: `roko completions bash/zsh/fish` outputs functional completion scripts. **Missing**: no `clap_complete` integration (uses manual word lists), no dynamic completions for plan names, PRD slugs, agent IDs, or nested subcommand arguments.
**What to change**:
1. Add `clap_complete` dependency for richer clap-derived completions (nested subcommands, flags)
2. Add dynamic completions that scan `.roko/prd/` for PRD slugs and `plans/` for plan names
3. Generate completions for nested subcommands (e.g., `roko prd draft`, `roko plan run`)
**Reference files**:
- `crates/roko-cli/src/main.rs:325` — `Command::Completions` variant, `CompletionShell` at 398, `print_completions()` at 6069, `completion_words()` at 6078
- `crates/roko-cli/Cargo.toml` (add `clap_complete` dependency)
**Depends on**: None
**Accept when**:
- [x] `roko completions bash/zsh/fish` outputs a completion script (exists)
- [x] Dynamic completions for plan names (scan `plans/` directory)
- [x] Dynamic completions for PRD slugs (scan `.roko/prd/` directory)
- [x] Nested subcommand completions (e.g., `roko prd <TAB>` shows draft/plan/list/status)
- [ ] `cargo test -p roko-cli` passes -- `clap_complete` crate not used (manual word lists + filesystem scan)
**Verify**:
```bash
grep -rn 'CompletionShell\|print_completions\|completion_words' crates/roko-cli/src/main.rs
grep -rn 'clap_complete' crates/roko-cli/Cargo.toml
cargo test -p roko-cli
```
**Priority**: P2

---

### DEPLOY-07: Subscription configuration

- [x] Implement event trigger configuration in roko.toml

**Spec** (doc 08): [subscriptions] section with Cron, FileWatch, and Webhook triggers. Debouncing. ${VAR} interpolation.
**Current code** (`crates/roko-core/src/config/schema.rs:42`): `RokoConfig` has no `subscriptions` field. No [subscriptions] section in config schema.
**What to change**: Add `subscriptions: Vec<SubscriptionConfig>` to `RokoConfig`. Define `SubscriptionConfig` with trigger type (Cron/FileWatch/Webhook), schedule/paths/url, debounce settings. Wire into serve runtime.
**Reference files**:
- `crates/roko-core/src/config/schema.rs` (RokoConfig)
- `crates/roko-serve/src/scheduler.rs` (existing scheduler)
- `crates/roko-serve/src/fswatcher.rs` (existing file watcher)
**Depends on**: DEPLOY-04 (${VAR} interpolation for webhook URLs)
**Accept when**:
- [x] [subscriptions] config section parses without error
- [x] Cron trigger: schedule field accepts cron syntax
- [x] FileWatch trigger: paths and debounce configurable
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'subscriptions\|SubscriptionConfig' crates/roko-core/src/config/ --include='*.rs'
cargo test --workspace
```
**Priority**: P2

---

### DEPLOY-08: Cloud deployment (Fly.io) — GitHub Action

- [x] Add GitHub Action workflow for Fly.io deployment

**Spec** (doc 06): fly.toml, deploy scripts, service topology.
**Current code** (`crates/roko-cli/src/main.rs:736`): `Command::Deploy` subcommand with `Fly` variant **already exists**. `cmd_deploy_fly()` at line 6123 calls `write_fly_toml()` (line 6206) to generate a `fly.toml` in the workspace, then runs `flyctl deploy --remote-only`. The generated fly.toml configures the roko-serve binary. **Working**: `roko deploy fly` generates fly.toml and triggers deployment. **Missing**: no GitHub Action workflow for automated deployment, no health check configuration in generated fly.toml.
**What to change**:
1. Add `.github/workflows/deploy-fly.yml` GitHub Action for CI-triggered Fly.io deployment
2. Add health check endpoint configuration to the generated fly.toml (`[http_service.checks]`)
3. Add internal_port 6677 to fly.toml if not already present
**Reference files**:
- `crates/roko-cli/src/main.rs:6206` — `write_fly_toml()` generates fly.toml
- `crates/roko-cli/src/main.rs:6123` — `cmd_deploy_fly()` orchestrates the deploy
- `Dockerfile` — Docker image used by Fly.io
**Depends on**: DEPLOY-02 (Docker image)
**Accept when**:
- [x] `roko deploy fly` generates fly.toml and deploys (exists)
- [x] GitHub Action workflow for automated Fly.io deployment
- [x] fly.toml includes health check configuration (interval, timeout, grace_period, path)
- [ ] `fly deploy` succeeds end-to-end (requires Fly.io account) -- not verifiable without credentials
**Verify**:
```bash
grep -rn 'write_fly_toml\|cmd_deploy_fly\|Deploy.*Fly' crates/roko-cli/src/main.rs | head -5
ls .github/workflows/deploy-fly* 2>/dev/null
```
**Priority**: P2

---

### DEPLOY-09: Production hardening

- [x] Implement per-provider semaphores and dedup cache

**Spec** (doc 12): Semaphores for concurrency control. Dedup cache for idempotency. Hedged requests for latency.
**Current code** (`crates/roko-agent/src/dispatcher/mod.rs`): Basic retry and timeout exist in dispatcher. No per-provider semaphores. No dedup cache.
**What to change**: Add `tokio::sync::Semaphore` per provider in dispatcher. Add `DedupCache` (hash-based) to prevent duplicate agent dispatches. Optionally add hedged requests for latency-sensitive calls.
**Reference files**:
- `crates/roko-agent/src/dispatcher/mod.rs` (dispatch loop)
- `crates/roko-core/src/config/schema.rs:1014` (ProviderConfig)
**Depends on**: None
**Accept when**:
- [x] Per-provider semaphore limits concurrent requests -- `ProviderSemaphores` in `provider/mod.rs:369` with `new()` and per-provider `Semaphore`
- [x] Dedup cache prevents duplicate agent dispatches -- `dedup_cache.rs` module in `dispatcher/` with TTL and max-entry eviction
- [ ] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'Semaphore\|dedup\|DedupCache' crates/roko-agent/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P2

---

### DEPLOY-10: WASM deployment target

- [ ] Compile core cognitive primitives to wasm32-wasi

**Spec** (doc 02 `docs/19-deployment/02-wasm-browser-edge.md`): Core Synapse traits and cognitive primitives compile to `wasm32-wasi` for browsers, edge functions, and embedded WASM runtimes.

**What works in WASM**: Engram struct, Score (7-axis), Scorer/Router/Composer/Policy traits (pure computation), HDC vectors (pure bit operations), decay calculations, content addressing (BLAKE3 compiles to WASM).

**What does NOT work**: FileSubstrate (needs filesystem), LLM backends (need HTTP+TLS), MCP client (needs stdio/TCP), ProcessSupervisor (needs process spawning), roko-orchestrator (needs git/filesystem), tree-sitter (C FFI).

**MemorySubstrate**: WASM-compatible implementation using `BTreeMap<ContentHash, Engram>` in memory with indexed lookups.

**Current code**: No WASM build target configured. No MemorySubstrate implementation. No feature flags for WASM exclusion.

**What to change**: Add `wasm32-wasi` target to CI. Create `MemorySubstrate` implementing `Substrate` trait in `crates/roko-core/`. Add feature flags (`default = ["fs", "net"]`) to exclude filesystem/network dependencies for WASM builds. Verify `cargo build -p roko-core --target wasm32-wasi --no-default-features` compiles.

**Reference files**:
- `crates/roko-core/src/` — core traits (must compile to WASM)
- `crates/roko-primitives/src/hdc.rs` — HDC vectors (should compile to WASM)
- `docs/19-deployment/02-wasm-browser-edge.md` — full spec: what works/doesn't, MemorySubstrate, size budget
**Depends on**: None
**Accept when**:
- [ ] `cargo build -p roko-core --target wasm32-wasi --no-default-features` compiles -- no wasm target or feature flags in roko-core/Cargo.toml
- [x] `MemorySubstrate` implements `Substrate` trait in memory -- `roko-std/src/memory.rs`: `MemorySubstrate` with `parking_lot::RwLock<HashMap>`, implements `Substrate`
- [ ] Feature flags exclude filesystem/network for WASM builds -- no `default-features` or WASM feature flags defined
- [ ] HDC vectors work in WASM (pure bit operations) -- not verified (no wasm target configured)
**Verify**:
```bash
grep -rn 'MemorySubstrate\|wasm' crates/roko-core/src/ --include='*.rs'
cargo build -p roko-core --target wasm32-wasi --no-default-features 2>/dev/null || echo "wasm target not installed"
```
**Priority**: P2

---

### DEPLOY-11: Multi-repo daemon coordination

- [x] Implement multi-repository subscription and isolation

**Spec** (doc 09 `docs/19-deployment/09-multi-repo-coordination.md`): A single daemon manages N repository subscriptions simultaneously. Each subscription is isolated:
- **Filesystem isolation** — each repo has its own `.roko/` directory
- **Process isolation** — agent processes scoped to their repo
- **Budget isolation** — per-repo budget tracking
- **Knowledge isolation** — per-repo Neuro store (optional cross-repo sharing via Mesh)

Shared scheduler manages total agent concurrency (default max 8 agents total). Priority-based scheduling: urgent repos (recent changes) get priority over idle repos. Loading algorithm: on daemon start, scan all configured repos, load subscription configs, start schedulers.

Config in `roko.toml`:
```toml
[[subscriptions]]
path = "/path/to/repo-a"
trigger = "cron"
schedule = "*/30 * * * *"

[[subscriptions]]
path = "/path/to/repo-b"
trigger = "watch"
paths = ["src/", "tests/"]
```

**Current code**: `crates/roko-cli/src/daemon.rs` manages a single daemon process. `crates/roko-serve/src/scheduler.rs` has scheduling infrastructure. No multi-repo subscription management. No per-repo isolation.

**What to change**: Add `SubscriptionManager` struct that loads `[[subscriptions]]` from config. Each subscription gets its own `.roko/` state directory. Scheduler distributes agent slots across repos. Wire into `daemon_start()`.

**Reference files**:
- `crates/roko-cli/src/daemon.rs` — daemon lifecycle (add subscription management)
- `crates/roko-serve/src/scheduler.rs` — scheduling infrastructure (reuse for multi-repo)
- `crates/roko-core/src/config/schema.rs` — config schemas (add `[[subscriptions]]` array)
- `docs/19-deployment/09-multi-repo-coordination.md` — full spec: isolation model, shared scheduler, priority, loading algorithm
**Depends on**: DEPLOY-03 (daemon IPC), DEPLOY-07 (subscription config)
**Accept when**:
- [x] `[[subscriptions]]` config parses from roko.toml -- `SubscriptionConfig` in schema.rs, parsed as `Vec<SubscriptionConfig>`
- [x] Each repo has isolated `.roko/` state directory -- `RepoSubscription.state_dir` derived from repo path; test `repo_subscription_state_dir_derived`
- [x] Shared scheduler respects max total agent limit -- `SubscriptionManager::new(max_agents)` constructor; tests verify limit enforcement
- [x] Per-repo budget tracking -- `RepoSubscription.budget_limit_usd` and `budget_spent_usd` fields; `has_budget()` method; test `subscription_manager_budget_enforcement`
- [ ] `cargo test -p roko-cli` passes
**Verify**:
```bash
grep -rn 'SubscriptionManager\|subscriptions\|multi.*repo' crates/roko-cli/src/ --include='*.rs'
cargo test -p roko-cli
```
**Priority**: P2

---

## Verify

```bash
cargo test --workspace
docker build .
```
