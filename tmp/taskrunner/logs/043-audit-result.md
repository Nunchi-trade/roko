# Task 043: Audit sync Mutex usage in async contexts

## Summary

No changes needed. All inspected mutexes are correctly typed for their usage patterns.

## Files Inspected

### `crates/roko-serve/src/state.rs`

- **`affect_engine: Mutex<DaimonState>`** (line 360)
  - Already uses `tokio::sync::Mutex` (imported on line 18)
  - Callers use `.lock().await` (e.g., `dreams.rs:233`, `dispatch.rs:2460`)
  - **Verdict**: Correct. No change needed.

### `crates/roko-learn/src/runtime_feedback.rs`

All five fields use `parking_lot::Mutex`. Each lock is acquired and released within
purely synchronous code — no `.await` occurs while any guard is held.

| Field | Line(s) | Critical section | `.await` while held? |
|-------|---------|------------------|---------------------|
| `affect_engine` | 2346 | `appraise()` calls + `query()` | No |
| `pattern_miner` | 2257 | `ingest_episode()` | No |
| `experiment_store` | 2292–2311 | `iter()`, `record_outcome()`, `save()`, explicit `drop(store)` | No |
| `local_rewards` | 1569, 1578, 1587 | `score()`, `observe()`, serialize+write | No |
| `section_effectiveness` | 2574 | `record_outcome()`, `save()` | No |

- **Verdict**: All correct. `parking_lot::Mutex` is the right choice for short synchronous
  critical sections in an async context — it avoids the overhead of `tokio::sync::Mutex`
  when no `.await` is needed inside the lock.

## Conclusion

No migration from `parking_lot::Mutex` to `tokio::sync::Mutex` is required. The sync
mutexes in `runtime_feedback.rs` protect fast, CPU-bound operations that complete without
yielding. The async mutex in `state.rs` is already correctly `tokio::sync::Mutex`.
