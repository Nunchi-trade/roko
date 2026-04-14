# Self-Development Pipeline: Status & Bugs

## Pipeline Overview

```
roko prd idea → prd draft new → prd draft promote → prd plan → plan run → dashboard
```

## Test Run: 2026-04-13

### Bugs Found & Fixed

#### Bug 1: `--bare` flag doesn't exist in Claude CLI
- **File**: `crates/roko-agent/src/claude_cli_agent.rs:302`
- **Symptom**: `exit 1: error: unknown option '--bare'`
- **Cause**: Code passes `--bare` to Claude CLI but the flag was removed/never existed
- **Fix**: Removed the `--bare` argument from `build_command()`. The `bare_mode` field still exists on the struct but is now a no-op.
- **Status**: Fixed

#### Bug 2: `prd draft new` fails if scaffold exists from failed run
- **File**: `crates/roko-cli/src/main.rs:4323`
- **Symptom**: `Draft already exists: .roko/prd/drafts/test-doc-comment.md` — even though the draft is just an empty scaffold
- **Cause**: Previous `new` run created the scaffold file but the agent failed (bug 1), leaving an empty scaffold. Next `new` run sees the file and bails.
- **Fix**: Added skeleton detection — if the existing draft has no content beyond YAML frontmatter and section headers, treat it as empty and overwrite.
- **Status**: Fixed

#### Bug 3: Agent output not written to PRD file
- **File**: `crates/roko-cli/src/agent_exec.rs` + `crates/roko-cli/src/main.rs:4366`
- **Symptom**: Agent runs successfully, produces output, but the PRD file remains an empty scaffold
- **Cause**: `run_agent()` captures the agent's text output and prints it to stdout, but never writes it to the target file. For Claude CLI agents, the prompt tells the agent to write the file directly, but it relies on the agent having tool access AND choosing to use file-write tools.
- **Fix**:
  - Added `run_agent_capture()` variant that returns `(exit_code, output_text)`
  - PRD `new` handler now: (1) snapshots file mtime, (2) runs agent, (3) checks if agent modified the file directly, (4) if not, writes agent text output to the file
  - Works for all provider types: CLI agents write files directly, API agents return text
- **Status**: Fixed

#### Bug 4: No progress output during agent execution
- **File**: `crates/roko-agent/src/claude_cli_agent.rs:421-467`
- **Symptom**: Terminal hangs with no output for minutes while agent works
- **Cause**: Both stdout and stderr are piped and only read after process exits. No real-time streaming.
- **Fix**: Spawn a tokio task that reads stderr line-by-line and prints each line prefixed with `[agent-name]` in real time. Stdout is still accumulated for parsing (it's stream-json format, not human-readable).
- **Status**: Fixed

### Bugs Not Yet Fixed (Known Issues)

#### Issue: Claude CLI nesting
- **Symptom**: `Error: Claude Code cannot be launched inside another Claude Code session`
- **Cause**: roko's default provider is `claude_cli`, which spawns `claude --print`. This can't run inside an existing Claude Code session.
- **Workaround**: Run roko commands in a separate terminal, not inside Claude Code
- **Real fix**: Support `anthropic_api` provider as fallback, or detect nesting and switch provider automatically

#### Issue: `prd draft edit` prompt assumes file tools
- **Symptom**: Prompt says "Update the file in place" but API-only providers can't write files
- **Fix needed**: Same pattern as `prd draft new` — detect provider type, capture output, write back

#### Issue: Plan generator lacks naming context
- **Symptom**: Generated plans may use old names (Signal, Bardo, etc.)
- **Fix needed**: Inject `docs/00-architecture/01-naming-and-glossary.md` into `PLAN_GENERATOR_SYSTEM_PROMPT`

#### Issue: Runtime safety is still not universal
- **Symptom**: provider-backed construction now defaults a real safety layer at factory time, and raw `ExecAgent` fallback runs under that same contract, but some native/provider-specific backends still bypass the shared `ToolDispatcher` chain
- **Cause**: backend-specific paths such as Claude CLI, Gemini-native, embeddings, and async deep-research still own more of their execution loop instead of flowing through one universal dispatcher
- **Impact**: Medium — the provider factory no longer has an unscoped fallback seam, but full backend-universal enforcement is still incomplete

## What Works Now

| Step | Command | Status |
|------|---------|--------|
| Capture idea | `roko prd idea "text"` | Works |
| Create draft | `roko prd draft new "slug"` | Works (after fixes) |
| Promote draft | `roko prd draft promote slug` | Wired; auto-plan path supported |
| Generate plan | `roko prd plan slug` | Wired; direct agent-exec episodes now logged |
| Execute plan | `roko plan run .roko/plans/` | Works (tested in earlier sessions) |
| Resume plan | `roko plan run .roko/plans/ --resume STATE` | Works |
| Dashboard | `roko dashboard` | Works (text mode) |
| Status | `roko status` | Works |

## Provider Compatibility Matrix

| Provider | Agent Type | File Tools | System Prompt | Text Output |
|----------|-----------|------------|---------------|-------------|
| `claude_cli` | ClaudeCliAgent | Yes (built-in) | Via --append-system-prompt | Yes (stream-json) |
| `anthropic_api` | ClaudeAgent | No | Yes (API field) | Yes |
| `openai_compat` | CodexAgent/ToolLoopAgent | Only if supports_tools | Yes | Yes |
| `gemini_api` | GeminiNativeAgent | No | Yes | Yes |
| Fallback | ExecAgent | No | No | Yes (stdout) |

The output-capture approach now works for all providers:
- CLI agents: write files directly (detected via mtime check), fallback to output capture
- API agents: return text content, which is written to the target file

PRD/research/plan-generation runs now also emit learning episodes via `agent_exec.rs`, and `agent_exec.rs`, `run.rs`, orchestrate fallback paths, and raw `ExecAgent` fallback all enter the current scoped safety surface by default.
