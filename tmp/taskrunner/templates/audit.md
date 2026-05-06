+++
task_id = "000"
auditor = ""
audit_time = ""
result = ""  # pass | fail | partial
+++

# Audit Report: Task {ID}

## Wire Check

**Wire target**: `cargo run -p roko-cli -- <command>`

**Did the wire target execute the new code?**
- [ ] Command runs without error
- [ ] Output matches expected behavior described in task
- [ ] New code path is exercised (not just compiled)

## Callsite Check

```bash
grep -rn '<key_function>' crates/ --include='*.rs' | grep -v test | grep -v target/
```

**Results**:
<!-- Paste grep output -->

**At least one non-test callsite?** Yes / No

## Test Check

```bash
cargo test -p <crate>
```

**All tests pass?** Yes / No

## Regression Check

```bash
cargo build --workspace && cargo test --workspace
```

**Full workspace builds and tests pass?** Yes / No

## Verdict

- [ ] **PASS** — Task is verified, mark as `done`
- [ ] **FAIL** — Issues found (describe below)
- [ ] **PARTIAL** — Works but needs additional wiring (describe below)

## Notes

<!-- Any issues, observations, or follow-up items -->
