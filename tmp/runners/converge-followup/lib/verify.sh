#!/usr/bin/env bash
# verify.sh — tiered verification: quick per-batch, full per-track, anti-pattern analysis
#
# The converge runner's fatal flaw: it ran cargo check -p <one-crate> per batch,
# which let cross-crate breakage accumulate. Then it ran no clippy, no tests,
# and no semantic analysis — so stubs-that-pass, dead code, duplicate traits,
# and computed-but-unused values all slipped through.
#
# This runner uses a tiered approach for efficiency:
#
#   PER-BATCH (fast, ~5-10s):
#     - cargo check -p <affected-crate>  (catches obvious compile errors fast)
#     - anti-pattern grep checks         (catches known bad patterns)
#
#   PER-TRACK GATE (thorough, ~1-2min, runs after all batches in a track):
#     - cargo check --workspace          (catches cross-crate breakage)
#     - cargo clippy --workspace         (catches lint issues)
#     - anti-pattern semantic analysis   (catches design-level problems)
#
#   END-OF-RUN (comprehensive, ~3-5min, runs once):
#     - cargo test --workspace           (catches behavioral regressions)
#
# The key efficiency insight: share a SINGLE CARGO_TARGET_DIR across the entire
# run. Incremental compilation means each check after the first is fast.

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

# ── Per-batch quick check ──
# Fast: only checks the affected crate(s). Catches 90% of issues in ~5s.

quick_check_batch() {
  local batch="$1" worktree="$2" run_id="$3" attempt="$4"
  local log_file target_dir
  log_file="$(run_log_file "$run_id" "$batch")"
  target_dir="$(run_target_dir "$run_id")"
  mkdir -p "$target_dir"

  # Determine which crates were affected from the scope
  local scope_files affected_crates=""
  scope_files="$(batch_scope "$batch")"
  for f in $scope_files; do
    # Extract crate name from path like "crates/roko-foo/src/bar.rs"
    local crate
    crate="$(echo "$f" | sed -n 's|^crates/\([^/]*\)/.*|\1|p')"
    if [[ -n "$crate" && ! " $affected_crates " =~ " $crate " ]]; then
      affected_crates="$affected_crates $crate"
    fi
  done

  if [[ -z "${affected_crates// /}" ]]; then
    # No crate identified from scope — fall back to workspace check
    affected_crates="--workspace"
  fi

  record_status "$run_id" "$batch" "$attempt" "quick_check" "$affected_crates"

  local output exit_code=0
  if [[ "$affected_crates" == "--workspace" ]]; then
    echo "[quick] cargo check --workspace" >> "$log_file"
    output="$(cd "$worktree" && env CARGO_TARGET_DIR="$target_dir" \
      cargo check --workspace 2>&1)" || exit_code=$?
  else
    for crate in $affected_crates; do
      echo "[quick] cargo check -p $crate" >> "$log_file"
      local crate_output crate_rc=0
      crate_output="$(cd "$worktree" && env CARGO_TARGET_DIR="$target_dir" \
        cargo check -p "$crate" 2>&1)" || crate_rc=$?
      output="${output}${crate_output}"$'\n'
      if (( crate_rc != 0 )); then exit_code=$crate_rc; fi
    done
  fi

  echo "$output" >> "$log_file"

  if (( exit_code != 0 )); then
    record_status "$run_id" "$batch" "$attempt" "quick_check_failed" "exit $exit_code"
    {
      echo "## Quick compilation check failed"
      echo
      echo "Affected crates: $affected_crates"
      echo
      echo '```'
      echo "$output" | grep -E '^error' | head -30
      echo '```'
      echo
      echo "Full output:"
      echo
      echo '```'
      echo "$output" | tail -60
      echo '```'
    } > "$(run_failure_file "$run_id" "$batch")"
    return 1
  fi

  record_status "$run_id" "$batch" "$attempt" "quick_check_ok" "$affected_crates"
  return 0
}

# ── Anti-pattern checks ──
# These catch the specific design-level problems the converge runner introduced.
# Each check is a targeted grep/analysis, not a generic lint.

run_antipattern_checks() {
  local batch="$1" worktree="$2" run_id="$3" attempt="$4"
  local log_file
  log_file="$(run_log_file "$run_id" "$batch")"
  local scope_files
  scope_files="$(batch_scope "$batch")"
  local failed=0

  echo "[antipattern] checking batch $batch scope files" >> "$log_file"

  for f in $scope_files; do
    local full_path="$worktree/$f"
    [[ -f "$full_path" ]] || continue

    # AP-1: Stubs that silently succeed
    # Functions that return Ok(()), Verdict::pass, or true without doing real work
    # Look for: fn that contains only Ok(()), pass(), or true with nothing else
    if grep -n 'Verdict::pass.*stub\|Verdict::pass.*always\|Verdict::pass.*noop\|Verdict::pass.*todo\|Verdict::pass.*placeholder' "$full_path" >> "$log_file" 2>&1; then
      log_err "$batch" "AP-1: Stub that silently passes in $f"
      record_status "$run_id" "$batch" "$attempt" "antipattern" "AP-1: silent-pass stub in $f"
      failed=1
    fi

    # AP-2: block_on inside async (will panic in tokio)
    if grep -n 'futures::executor::block_on\|futures::executor::LocalPool\|block_on.*\.await' "$full_path" >> "$log_file" 2>&1; then
      log_err "$batch" "AP-2: block_on inside async context in $f"
      record_status "$run_id" "$batch" "$attempt" "antipattern" "AP-2: block_on in async in $f"
      failed=1
    fi

    # AP-3: Duplicate trait definitions
    # If a scope file defines a pub trait that already exists in foundation.rs
    if [[ "$f" != *"foundation.rs" ]]; then
      local defined_traits
      defined_traits="$(grep -oP 'pub trait \K\w+' "$full_path" 2>/dev/null || true)"
      for trait_name in $defined_traits; do
        if grep -q "pub trait $trait_name" "$worktree/crates/roko-core/src/foundation.rs" 2>/dev/null; then
          log_err "$batch" "AP-3: Duplicate trait $trait_name (also in foundation.rs) in $f"
          record_status "$run_id" "$batch" "$attempt" "antipattern" "AP-3: duplicate trait $trait_name in $f"
          failed=1
        fi
      done
    fi

    # AP-4: Computed value never used
    # Pattern: let modulation = ...; (and modulation never appears again)
    # This is a heuristic — checks for let bindings where the variable appears only once
    local -a let_vars=()
    mapfile -t let_vars < <(grep -oP 'let (?:mut )?\K\w+(?= *=)' "$full_path" 2>/dev/null || true)
    for var in "${let_vars[@]}"; do
      [[ -z "$var" ]] && continue
      [[ "$var" == "_" || "$var" == "_"* ]] && continue
      local count
      count="$(grep -c "\b${var}\b" "$full_path" 2>/dev/null || echo 0)"
      if (( count == 1 )); then
        # Variable defined but never referenced again — possible dead computation
        local line_context
        line_context="$(grep -n "let.*\b${var}\b.*=" "$full_path" | head -1)"
        echo "[antipattern] AP-4 candidate: $var used once in $f: $line_context" >> "$log_file"
        # Don't fail — just warn. Too many false positives for destructuring etc.
      fi
    done

    # AP-5: Shell out to claude (bypass provider abstraction)
    if grep -n 'Command::new.*"claude"\|Command::new.*"codex"' "$full_path" >> "$log_file" 2>&1; then
      log_err "$batch" "AP-5: Shelling out to claude/codex CLI in $f"
      record_status "$run_id" "$batch" "$attempt" "antipattern" "AP-5: shell out to CLI in $f"
      failed=1
    fi

    # AP-6: Inline prompt strings (should use PromptAssemblyService)
    if grep -n 'format!.*"You are a\|format!.*"You are an\|"You are a .*assistant\|"You are an .*agent' "$full_path" >> "$log_file" 2>&1; then
      log_err "$batch" "AP-6: Inline prompt string in $f"
      record_status "$run_id" "$batch" "$attempt" "antipattern" "AP-6: inline prompt in $f"
      failed=1
    fi

    # AP-7: std::sync::Mutex guard held across .await
    # Heuristic: MutexGuard in scope + .await in same function
    if grep -q 'std::sync::Mutex\|use std::sync::Mutex' "$full_path" 2>/dev/null; then
      # Check if any function has both .lock() and .await
      if awk '/fn /{found_lock=0; found_await=0} /\.lock\(\)/{found_lock=1} /\.await/{found_await=1} found_lock && found_await{print FILENAME":"NR": std Mutex + await"; exit 1}' "$full_path" >> "$log_file" 2>&1; then
        log_err "$batch" "AP-7: std::sync::Mutex held across .await in $f"
        record_status "$run_id" "$batch" "$attempt" "antipattern" "AP-7: std Mutex + await in $f"
        failed=1
      fi
    fi
  done

  if (( failed )); then
    record_status "$run_id" "$batch" "$attempt" "antipattern_failed" "see above"
    return 1
  fi

  record_status "$run_id" "$batch" "$attempt" "antipattern_ok" ""
  return 0
}

# ── Track gate: full workspace verification ──
# Runs after all batches in a track complete. Catches cross-crate breakage.

run_track_gate() {
  local track="$1" run_id="$2" worktree="$3"
  local target_dir
  target_dir="$(run_target_dir "$run_id")"
  local gate_log="$LOG_ROOT/$run_id/gate-${track}.log"

  log_header "TRACK GATE: $track"

  # Level 1: cargo check --workspace
  log_info "gate" "cargo check --workspace"
  record_status "$run_id" "gate:$track" "1" "gate_compile" "cargo check --workspace"

  local output exit_code=0
  output="$(cd "$worktree" && env CARGO_TARGET_DIR="$target_dir" \
    cargo check --workspace 2>&1)" || exit_code=$?
  echo "$output" > "$gate_log"

  if (( exit_code != 0 )); then
    record_status "$run_id" "gate:$track" "1" "gate_compile_failed" ""
    log_err "gate" "Workspace compilation failed after track $track"
    log_err "gate" "$(echo "$output" | grep '^error' | head -5)"
    return 1
  fi
  log_ok "gate" "Workspace compiles"

  # Level 2: cargo clippy --workspace
  log_info "gate" "cargo clippy --workspace"
  record_status "$run_id" "gate:$track" "1" "gate_clippy" "cargo clippy"

  output="$(cd "$worktree" && env CARGO_TARGET_DIR="$target_dir" \
    cargo clippy --workspace --no-deps -- -D warnings 2>&1)" || exit_code=$?
  echo "$output" >> "$gate_log"

  if (( exit_code != 0 )); then
    record_status "$run_id" "gate:$track" "1" "gate_clippy_failed" ""
    log_err "gate" "Clippy failed after track $track"
    log_err "gate" "$(echo "$output" | grep '^error' | head -5)"
    return 1
  fi
  log_ok "gate" "Clippy clean"

  # Level 3: Cross-crate anti-pattern sweep
  # Check ALL files changed in this track for design-level issues
  log_info "gate" "Anti-pattern sweep"
  local cum_dir
  cum_dir="$(run_cumulative_dir "$run_id")"
  local all_changed_files=()
  for batch in "${ALL_BATCHES[@]}"; do
    [[ "$(batch_group "$batch")" == "$track" ]] || continue
    local files_list="$cum_dir/${batch}.files"
    [[ -f "$files_list" ]] || continue
    while IFS= read -r f; do
      [[ -z "$f" ]] && continue
      all_changed_files+=("$f")
    done < "$files_list"
  done

  local ap_failed=0
  local -A seen_traits=()
  for f in "${all_changed_files[@]}"; do
    local full_path="$worktree/$f"
    [[ -f "$full_path" ]] || continue
    [[ "$f" == *.rs ]] || continue

    # Collect all pub trait definitions across the track
    while IFS= read -r trait_line; do
      local trait_name
      trait_name="$(echo "$trait_line" | grep -oP 'pub trait \K\w+')"
      [[ -z "$trait_name" ]] && continue
      if [[ -n "${seen_traits[$trait_name]:-}" && "${seen_traits[$trait_name]}" != "$f" ]]; then
        log_err "gate" "Duplicate trait $trait_name in $f AND ${seen_traits[$trait_name]}"
        ap_failed=1
      fi
      seen_traits["$trait_name"]="$f"
    done < <(grep 'pub trait ' "$full_path" 2>/dev/null || true)
  done

  if (( ap_failed )); then
    record_status "$run_id" "gate:$track" "1" "gate_antipattern_failed" ""
    log_err "gate" "Anti-pattern issues found in track $track"
    return 1
  fi

  record_status "$run_id" "gate:$track" "1" "gate_ok" "compile + clippy + antipattern"
  log_ok "gate" "Track $track gate passed"
  return 0
}

# ── End-of-run test gate ──

run_test_gate() {
  local run_id="$1" worktree="$2"
  local target_dir
  target_dir="$(run_target_dir "$run_id")"

  log_header "TEST GATE"
  log_info "test" "cargo test --workspace"
  record_status "$run_id" "gate:test" "1" "test_start" ""

  local output exit_code=0
  output="$(cd "$worktree" && env CARGO_TARGET_DIR="$target_dir" \
    cargo test --workspace 2>&1)" || exit_code=$?

  echo "$output" > "$LOG_ROOT/$run_id/gate-test.log"

  if (( exit_code != 0 )); then
    record_status "$run_id" "gate:test" "1" "test_failed" ""
    log_err "test" "Tests failed"
    echo "$output" | grep -E '^test .* FAILED|^failures:' | head -20
    return 1
  fi

  local test_count
  test_count="$(echo "$output" | grep -oP '\d+ passed' | tail -1)"
  record_status "$run_id" "gate:test" "1" "test_ok" "$test_count"
  log_ok "test" "All tests passed ($test_count)"
  return 0
}

# ── Per-batch verification (fast path) ──

verify_batch() {
  local batch="$1" run_id="$2" worktree="$3" attempt="${4:-1}"
  local mode
  mode="$(batch_verify_mode "$batch")"

  # Quick compile check (per-crate, fast)
  if ! quick_check_batch "$batch" "$worktree" "$run_id" "$attempt"; then
    return 1
  fi

  # Anti-pattern checks (grep-based, instant)
  if ! run_antipattern_checks "$batch" "$worktree" "$run_id" "$attempt"; then
    return 1
  fi

  record_status "$run_id" "$batch" "$attempt" "verify_ok" "quick ($mode)"
  log_ok "$batch" "Verified (quick)"
  return 0
}

# ── Commit ──

commit_batch() {
  local batch="$1" worktree="$2" run_id="$3" attempt="${4:-1}"
  local title
  title="$(batch_title "$batch")"

  rm -rf "$worktree/.cargo-target" "$worktree/target"
  git -C "$worktree" add -A

  if git -C "$worktree" diff --cached --quiet; then
    record_status "$run_id" "$batch" "$attempt" "commit_noop" ""
    log_warn "$batch" "No changes"
    return 10
  fi

  git -C "$worktree" commit -m "$(printf 'followup(%s): %s\n\nAutomated via tmp/runners/converge-followup/run.sh' "$batch" "$title")" >/dev/null

  local hash
  hash="$(git -C "$worktree" rev-parse --short HEAD)"
  record_status "$run_id" "$batch" "$attempt" "commit_ok" "$hash"
  log_ok "$batch" "Committed $hash"
  return 0
}

# ── Worktree management ──

backup_worktree() {
  local run_id="$1" batch="$2" attempt="$3" worktree="$4" label="$5"
  local dir
  dir="$(run_backups_dir "$run_id")"
  ensure_dir "$dir"
  git -C "$worktree" diff > "$dir/${batch}-a${attempt}-${label}.patch" 2>/dev/null || true
  git -C "$worktree" status --short > "$dir/${batch}-a${attempt}-${label}.status" 2>/dev/null || true
}

reset_worktree() {
  git -C "$1" reset --hard HEAD >/dev/null 2>&1 || true
  git -C "$1" clean -fd >/dev/null 2>&1 || true
}
