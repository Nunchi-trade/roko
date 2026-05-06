# Task 017: Wire or Build JSONL Rotation for Episode/Efficiency Logs

```toml
id = 17
title = "Wire or build JSONL log rotation for episodes and efficiency logs"
track = "wiring"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-learn/src/episode_logger.rs",
    "crates/roko-learn/src/jsonl_rotation.rs",
    "crates/roko-learn/src/runtime_feedback.rs",
    "crates/roko-core/src/config/learning.rs",
    "crates/roko-cli/src/config.rs",
]
exclusive_files = []
estimated_minutes = 60
```

## Context

Episode and efficiency JSONL logs grow unbounded. Need rotation at a configurable size threshold.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` — DCA-4: Wire jsonl_rotation

## Background

Read these files first:
1. `crates/roko-learn/src/jsonl_rotation.rs`
   - Rotation already exists.
   - Public constants: `DEFAULT_ROTATION_THRESHOLD_BYTES = 10 * 1024 * 1024`, `MAX_ROTATED_FILES = 5`.
   - Public helpers: `rotation_path(...)`, `rotate_if_needed(path, threshold_bytes)`.
   - Existing tests cover missing/small/no-op files, threshold rotation, chain shifting, and max-file capping.
2. `crates/roko-learn/src/episode_logger.rs`
   - `EpisodeLogger::append(...)` already calls `jsonl_rotation::rotate_if_needed(...)` before appending.
3. `crates/roko-learn/src/runtime_feedback.rs`
   - `append_jsonl_record(...)` already rotates generic feedback logs such as `efficiency.jsonl`, `efficiency-summaries.jsonl`, `gate_outcomes.jsonl`, and retry/cfactor logs before appending.
4. `crates/roko-core/src/config/learning.rs`
   - `LearningConfig` does not currently expose rotation settings.
5. `crates/roko-cli/src/config.rs`
   - `LearningLayer` does not currently expose layered config overrides for rotation settings.

The remaining gap is configurability/testability, not inventing the rotation algorithm from scratch.

## What to Change

1. Keep the existing rename scheme in `jsonl_rotation.rs`; do not duplicate it in loggers.
2. Add configurable rotation settings with defaults:
   - `learning.rotation_threshold_bytes`, default `10 * 1024 * 1024`
   - `learning.rotation_max_files`, default `5`
3. Thread those settings through the logging call sites:
   - `EpisodeLogger` should have a default constructor that preserves current behavior and a test/configurable constructor such as `with_rotation(path, threshold_bytes, max_files)`.
   - `runtime_feedback::append_jsonl_record(...)` should use the same rotation configuration for `efficiency.jsonl` and related feedback JSONL logs.
4. Update `jsonl_rotation.rs` so max files can be supplied by config without breaking existing callers. Acceptable shapes:
   - Add `JsonlRotationConfig { threshold_bytes, max_files }` and `rotate_if_needed_with_config(...)`, keeping `rotate_if_needed(...)` as a default wrapper.
   - Or add a new helper that takes `(path, threshold_bytes, max_files)` and update callers.
5. Update config structs and layered config parsing:
   - Add fields to `LearningConfig` in `crates/roko-core/src/config/learning.rs`.
   - Add matching optional fields to `LearningLayer` in `crates/roko-cli/src/config.rs` and merge them into the effective config.
6. Add tests that use a tiny threshold:
   - `jsonl_rotation` still rotates and caps at the configured max file count.
   - `EpisodeLogger::append(...)` creates `episodes.jsonl.1` and continues writing valid JSONL to the live file.
   - Runtime feedback append path rotates `efficiency.jsonl` (or a generic test JSONL path) using the same helper.
   - Config parsing accepts `[learning] rotation_threshold_bytes = 1024` and `rotation_max_files = 3`.

## What NOT to Do

- Don't change the JSONL format.
- Don't delete old rotated files (keep up to max_files).
- Don't add compression (keep it simple).
- Don't rotate after writing if that can leave an oversized record stranded until the next append; keep the existing pre-append behavior unless a test proves a different order is required.
- Don't hard-code a 1KB threshold outside tests.
- Don't make episode logs and efficiency logs use different rotation algorithms.

## Wire Target

```bash
# Verify rotation works with a small threshold:
# (Set threshold to 1KB in test, write enough episodes)
cargo test -p roko-learn -- rotation
# OR manually:
ls -la .roko/learn/episodes.jsonl*
# Should show .jsonl and .jsonl.1 etc. after enough writes
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test -p roko-learn jsonl_rotation`
- [ ] `cargo test -p roko-learn episode_logger`
- [ ] `cargo test -p roko-core learning`
- [ ] `cargo test --workspace`
- [ ] `rg -n 'rotate_if_needed|rotation_threshold_bytes|rotation_max_files' crates/roko-learn crates/roko-core crates/roko-cli --glob '*.rs'`
- [ ] Rotation triggers at configured threshold
- [ ] Status Log documents whether rotation was found or built

## Status Log

| Time | Agent | Action |
|------|-------|--------|
