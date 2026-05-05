# Demo UI — Known Issues & Improvement Backlog

## Critical: CLI Startup Latency

**Problem:** Every `roko <subcommand>` invocation takes 3-15 seconds before producing output.

**Root cause:** The CLI loads the full config + state stack on every invocation:

1. **Env loading** — reads `~/.roko/.env` + `./.roko/.env` via dotenvy
2. **Layered config** — parses global + project `roko.toml`, canonicalizes all repo paths
3. **Background serve** — bare `roko` (no subcommand) spawns a background `roko serve` which triggers a full state bootstrap:
   - Reads *entire* `.roko/episodes.jsonl` (can be huge)
   - Reads *entire* `.roko/learn/efficiency.jsonl`
   - Reads *entire* `.roko/learn/c-factor.jsonl`
   - Scans `.roko/jobs/*.json`, `.roko/prd/{drafts,published}/*.md`
   - Queries neuro store (`query("*", 200)`) — potentially thousands of JSONL entries
4. **Auth detection** — if env vars not set, spawns `claude --version` subprocess

**Impact on demo:** Each command in a scenario has ~5-10s dead air before output.

**Fix ideas (CLI-side, not demo-side):**
- Lazy-load DashboardSnapshot — only bootstrap what's needed for the current subcommand
- Stream JSONL files (read last N lines) instead of loading entire files
- Cache config validation with mtime checks
- Skip `spawn_background_serve()` for subcommands that don't need it (init, prd idea, status)
- Skip auth detection if any API key env var is set

**Workaround in demo:** All infrastructure (`resolveRoko`, `mkdir`, `cd`, `roko init`) runs via `execCmd` (instant send), terminal is cleared after setup so audience only sees show commands.

---

## Fixed Issues

### Scrollbars visible
**Status:** Fixed (2026-04-27)
Global `scrollbar-width: none` + `::-webkit-scrollbar { display: none }`.

### Gate icons render as broken emoji
**Status:** Fixed (2026-04-27)
Replaced `⏳` (U+23F3) with `○` (U+25CB) for pending state.

### Prompt regex doesn't match all shell prompts
**Status:** Fixed (2026-04-27)
Regex matches `❯ $ % > #`. `stripAnsi()` strips ANSI escapes before testing.

### Self-Hosting scenario runs bare `roko` instead of `roko init`
**Status:** Fixed (2026-04-27)
`setupWorkspace()` clears terminal after probe/mkdir so audience never sees infrastructure noise.

### waitForPrompt clears buffer prematurely
**Status:** Fixed (2026-04-27)
Callers (`execCmd`/`showCmd`) clear the buffer. `waitForPrompt` just polls.

### Builder preset cards don't work after tab switch
**Status:** Fixed (2026-04-27)
`submitBuild()` now checks `builderReady` flag. If workspace was destroyed by tab switch, it re-runs `setupWorkspace` + `roko init` transparently before executing the build.

### Terminal WebSocket reconnection
**Status:** Fixed (2026-04-27)
`ws.onclose` auto-reconnects after 2s if the pane is still mounted in the DOM. Server restart no longer leaves terminals permanently dead.

### xterm.js fit on tab switch
**Status:** Fixed (2026-04-27)
`ResizeObserver` uses 80ms debounce. `switchScenario` calls `fitAddon.fit()` 120ms after layout settles.

### Tab transitions are hard cuts
**Status:** Fixed (2026-04-27)
Terminal area fades out (150ms) before destroying, fades back in on new scenario.

### Status bar doesn't show scenario state
**Status:** Fixed (2026-04-27)
Added `sb-state` element with "setting up..." / scenario name / "paused" / "complete" states. Rose color when running.

### Pane border doesn't indicate active command
**Status:** Fixed (2026-04-27)
`.pane.active-cmd` class added during `showCmd()`, gives rose-dim border glow while a command is running.

### Per-pane timing not shown
**Status:** Fixed (2026-04-27)
Each pane header now has a timer element that shows elapsed time after each command completes.

### Cost/token scraping from output
**Status:** Fixed (2026-04-27)
`detectFromOutput` now scrapes `$X.XX` and `N tokens` patterns from terminal output and updates the metrics panel automatically.

### No keyboard shortcuts for tabs
**Status:** Fixed (2026-04-27)
Number keys 1-5 switch tabs. R resets the current scenario.

---

## Remaining Issues

### Per-command startup overhead
Each `showCmd` pays the full roko startup tax (~5-15s). For the Explore scenario with 8 commands across 4 panes, that's significant dead time.

**Possible fix:** Use `roko` in REPL mode if available, or batch commands in a single invocation.

### Provider scenario: unconfigured providers show errors
**Status:** Fixed (2026-04-27)
When an API key is missing, `roko run --provider X` errors out. Now auto-detects error output patterns and shows a styled overlay ("not configured") on the pane instead of leaving raw error text.

---

### No autocomplete in prompt bar
**Status:** Fixed (2026-04-27)
Builder prompt input now has autocomplete dropdown with 15 suggestions. Fuzzy-matches as you type (min 2 chars). Arrow keys navigate, Enter selects, Escape dismisses. Dropdown opens upward (above the input) to not occlude the terminal.

### No model selector
**Status:** Fixed (2026-04-27)
Model dropdown in top bar controls. Shows all configured providers (Anthropic, OpenAI, Zhipu, Google, Moonshot, Perplexity) with model variants grouped by provider. "auto" mode (default) lets cascade router decide. Selected model is passed via `--model` flag to `roko run` in builder, race, and provider scenarios.

### `roko chat` model switching only works for OpenAI-compat providers
**Status:** Fixed (2026-04-27)

### Only 5 demo scenarios → redesigned to 7 focused tabs
**Status:** Fixed (2026-04-27)
Full demo.html redesign. Consolidated 9 tabs to 7 narrative-driven tabs:
1. **Self-Hosting** — idea → draft → plan → status → learn (auto-plays, core value prop, 5 phases)
2. **Build** — interactive prompt bar with presets and gates
3. **Cost Race** — side-by-side naive vs cascade with comparison metrics panel
4. **Providers** — 4 providers simultaneously from shared workspace, overlay on unconfigured
5. **Explore** — 4 command families with 3 commands each (workspace, learning, config, knowledge)
6. **Chat** — bare `roko` command entering the unified chat TUI, slash commands, architecture showcase
7. **Mirage** — EVM fork with live block streaming

Key infrastructure changes:
- `setupWorkspace` includes `roko init` + 200ms settle delay + clear (no init noise visible)
- `joinWorkspace` helper for shared multi-pane workspaces (no duplicate init)
- Narrative overlay for slow commands (pulsing context messages)
- Comparison panel mode for Cost Race with per-pane metric tracking
- Numbered tab badges, rose-tinted card borders, rose glow on active panes
- Scenario description bar shows title + italic subtitle

### No command log / description panel
**Status:** Fixed (2026-04-27)
Side panel now splits into two halves: top = context (metrics/gates/files), bottom = scrolling command log. Each command logged with description explaining what it does and why. Click-to-copy individual commands, "copy" button exports all with comments. Descriptions auto-match from `CMD_DESCRIPTIONS` lookup table (20+ entries).

### No Mirage (EVM fork) demo tab
**Status:** Fixed (2026-04-27)
Added Tab 6 "Mirage" with:
- Config bar: network selector (Mainnet, Arbitrum, Optimism, Base), block time input (default 50ms)
- Terminal pane running `mirage --fork-url <rpc> --block-time <secs>`
- Live block stream panel: connects via WebSocket `eth_subscribe("newHeads")`, renders block number, hash, gas, parent hash with slide-in animation
- Stats header: current block, network, hash, blocks/sec rate
- Start/Stop controls, Ctrl+C to terminal on stop

### Redundant setup visible in multi-pane scenarios
**Status:** Fixed (2026-04-27)
`setupWorkspace` now does init inside itself and clears after a 200ms settle delay. Multi-pane scenarios use `joinWorkspace` to share ONE workspace directory — no duplicate `roko init`.

### Race scenario resolves roko only in first pane
**Status:** Fixed (2026-04-27)
`resolveRoko` is global state — resolves once in setupWorkspace, joinWorkspace reuses it. All panes in a scenario share the same binary path.

---

### No Chat TUI demo tab
**Status:** Fixed (2026-04-27)
Added Tab 6 "Chat" showcasing the unified `roko` command — the bare command as product:
- Side panel with session info + architecture tips
- Enters chat TUI, sends a prompt, demonstrates `/status` and `/model` slash commands
- Shows the single-process architecture (chat + dispatch + serve + gates + learning)

### No loading spinner during workspace setup
**Status:** Fixed (2026-04-27)
`showPaneSpinner()` / `hidePaneSpinner()` display a CSS spinner overlay on each pane during `setupWorkspace`. All scenarios now show spinners during initialization, replacing dead terminal time.

### Connect screen doesn't auto-retry
**Status:** Fixed (2026-04-27)
Connect screen now auto-retries every 5s with countdown timer. On successful reconnect, auto-switches to current scenario and starts playback.

### Explore tab only has 2 commands per pane
**Status:** Fixed (2026-04-27)
Expanded to 3 commands per pane: workspace (status/doctor/prd list), learning (learn all/efficiency/tune gates), config (providers list/models list/validate), knowledge (stats/query/explain). CMD_DESCRIPTIONS expanded from 20 to 32 entries.

### Self-Hosting tab too short
**Status:** Fixed (2026-04-27)
Self-Hosting now runs 5 phases with labeled narrative: idea → draft → plan → status → learn. Phase indicators ("Phase 3/5") show progress context.

---

## Visual Polish (P2)

- Metrics card should animate value changes (counter roll-up)
- Preset cards should highlight/disable while a build is running
- Side panel file list should show file count badge in header
