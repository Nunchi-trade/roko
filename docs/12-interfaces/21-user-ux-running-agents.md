# User UX: Running Agents

> **Abstract:** This chapter propagates `tmp/refinements/23-user-ux-running-agents.md` into the canonical docs tree. Roko should present one unified verb set across four surfaces, `CLI`, `TUI`, `Chat`, and `Web`, so users learn the interaction model once and carry that muscle memory anywhere they work. The goal is familiar-first onboarding: a first-time `roko` session should produce useful output in under 30 seconds.

**Topic**: [12-interfaces](./INDEX.md)  
**Prerequisites**: [00-cli-overview.md](./00-cli-overview.md), [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md), [05-http-api-roko-serve.md](./05-http-api-roko-serve.md), [06-websocket-streaming.md](./06-websocket-streaming.md), [08-tui-main-layout.md](./08-tui-main-layout.md), [13-web-portal.md](./13-web-portal.md), [14-agent-onboarding-flow.md](./14-agent-onboarding-flow.md), [17-accessibility-and-current-status.md](./17-accessibility-and-current-status.md), [19-rust-sdk-developer-ux.md](./19-rust-sdk-developer-ux.md), [../17-lifecycle/03-configuration-and-operator-model.md](../17-lifecycle/03-configuration-and-operator-model.md), [../17-lifecycle/05-knowledge-backup-export.md](../17-lifecycle/05-knowledge-backup-export.md), [../17-lifecycle/08-selective-restore.md](../17-lifecycle/08-selective-restore.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md)

## 1. Canonical Interaction Model

Roko's user UX is built around one verb set rendered four ways. The surface changes, but the verbs do not. That gives CLI users, terminal users, chat users, and web users the same mental model and keeps discovery consistent across sessions.

The canonical verb set is:

| Verb | Meaning | Typical output |
|---|---|---|
| `ask` | Run a single-turn question or task request | Direct answer, draft, or recommendation |
| `plan` | Propose a plan without executing it | Step list, scope, risks, and checkpoints |
| `do` | Execute a plan or single task | Live progress, tool calls, and final result |
| `watch` | Observe active work in real time | Streaming status, gate feedback, and banners |
| `inspect` | Drill into an episode, Engram, or heuristic | Full context, citations, and provenance |
| `replay` | Re-run a prior episode with the same or modified inputs | Replay controls, diffs, and comparison view |
| `learn` | Browse, curate, or challenge heuristics and playbooks | Heuristic library, calibration history, and review actions |
| `tune` | Change configuration, thresholds, routing, or defaults | Settings editor or config wizard |
| `connect` | Add a plugin, profile bundle, MCP server, credential, or integration | Connection flow, permission prompt, and diagnostics |

Every surface exposes those verbs, even when the chrome differs. A user who learns `roko ask`, `roko plan`, and `roko do` in CLI should be able to find the same actions in TUI tabs, Chat commands, and Web navigation without learning a second vocabulary.

## 2. The Four Surfaces

The current product already has four user-facing surfaces:

| Surface | Current role | What must be consistent |
|---|---|---|
| `CLI` | Primary scripting and automation surface | Command names, help text, output shape, error recovery |
| `TUI` | Interactive terminal control surface | The same verbs, the same session state, the same progress feed |
| `Chat` | Conversational surface for humans and agents | Slash commands, streaming, inline artifacts, permissions, and profile confirmation |
| `Web` | Browser surface on top of the HTTP control plane | The same verb set, live status, session continuity, and profile composition |

The key requirement is not visual uniformity. It is semantic uniformity. If a command exists in one surface, it needs a discoverable equivalent in the others. If a user pauses a task in Chat, the same session should be visible in TUI and Web. If a plan is inspectable in Web, the same underlying data should be reachable from CLI.

## 3. Familiar-First Defaults

The first-run experience should feel like a guided setup, not a documentation scavenger hunt. `roko init` should ask only the minimum required questions, recover from missing prerequisites, and leave the user with a working baseline even if they skip optional steps.

The proposed onboarding path is interactive:

```bash
$ roko init
Welcome to Roko. Let's get you set up.

Which profile would you like to install or activate?
  [x] Coding
  [ ] Research
  [ ] Ops
  [ ] Compose multiple profiles

Which models would you like to use?
  [x] Claude
  [ ] OpenAI
  [ ] Local Ollama
  [ ] Other configured provider

Would you like a starter heuristic library?
  [x] Import the starter kit
  [ ] Start from scratch

Should Roko look for existing MCP servers?
  [x] Yes, auto-discover
  [ ] No, manual config only
```

The important properties are:

- `roko init` should never dead-end. Every prompt needs a `skip` or `configure later` path.
- `roko init` should make profile install and profile composition explicit, not hidden behind a generic template chooser.
- Missing API keys should yield a literal next command, not a generic failure.
- Partial success should be durable. If the user stops midway, already-completed steps remain committed.
- Remote checks should time out quickly and offer retry or bypass choices.
- The result should point to the next useful command, such as `roko ask`, `roko dashboard`, or `roko plugin list`.
- The selected profile should carry a `TypedContext` schema and any `Custody` expectations into later inspect and replay flows.

This is intentionally modeled on familiar-first agent UX: the user should be able to do something useful immediately, then deepen setup only when needed.

## 4. Live Progress Through StateHub

Users should not stare at a spinner while the agent works. Every surface should render the same live progress feed, but REF26 makes the contract more specific: surfaces subscribe to shared `StateHub` projections that fold `Bus` Pulses together with durable `Substrate` state.

The live feed should include:

- Token streaming from the model.
- Tool call banners that show what the agent is reading or editing.
- Gate outcomes as they happen.
- Plan updates as task state changes.
- Heuristic application notices when a belief affects the decision.
- Profile activation notices when a bundle is installed, composed, or revalidated.

In practice, `watch` should compose from canonical projections such as:

- `active_tasks` for task progress and ETA.
- `agent_trails` for token chunks, tool banners, and current action.
- `gate_pipeline` for rung status and pass/fail counts.
- `recent_episodes` for completed or resumed work.
- `cohort_health` for c-factor and roster context when work spans a fleet.

The important design point is that this is one state model, not four different status systems. CLI prints it linearly, TUI renders it in panes, Chat interleaves it with conversation, and Web turns it into a real-time page. That keeps the user oriented when they move between surfaces mid-task.

`watch` should therefore follow a `query + subscribe` pattern: fetch current state first, then fold projection deltas as they arrive. That is what makes surface handoff, replay, reconnect, and shareable sessions behave like one product instead of four shells around the same runtime. See [22-statehub-projection-layer.md](./22-statehub-projection-layer.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), and [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md).

## 5. Checkpoints And Permissions

Roko should treat dangerous or uncertain actions as explicit checkpoints. The user stays in control, but the system should remember reasonable answers within a session so repeated work does not become noisy.

| Action class | Default behavior | Remembered scope |
|---|---|---|
| Read project files | Allow | n/a |
| Read files outside the project root | Ask | Session |
| Write project files | Ask once | Session or directory scope |
| Delete files | Always ask | Never |
| Run unrecognized local commands | Ask | Session and command |
| Network access to a new host | Ask | Session and host |
| Install or upgrade a plugin or profile | Ask | Never |
| Send a message or email on the user's behalf | Always ask | Never |
| Override a failing gate | Ask | Session and gate rung |

Every denial should include the literal command or action the user can take next. Every approval should still emit a visible Pulse so the UI shows what happened, and sensitive actions should attach custody metadata so inspect and replay can reconstruct who approved what and why. The permissions model is part of the UX, not a hidden backend rule.

## 6. Undo, Replay, And Sessions

Users need low-friction recovery. The chapter-level rule is simple: make reversal and replay cheap enough that users feel safe exploring.

Three undo levels matter:

- `roko undo last` reverts the most recent file-changing task.
- `roko replay <episode>` re-runs a prior session from its recorded state.
- `roko revert <episode>` undoes the diffs associated with a prior episode.

Sessions should also be first-class objects. Named sessions let a user attach to a recurring context such as `research-q2`, and the same session should be visible in CLI, TUI, Chat, and Web. Shareable sessions should export to a portable transcript format and replay later on another machine or by another user.

This is where lifecycle and interface docs meet: the lifecycle chapter covers export and restore mechanics, while this chapter defines the user-facing affordances for naming, resuming, and sharing work.

## 7. Accessibility And Shortcut Discipline

The surfaces must stay usable for keyboard-only users, screen-reader users, and people who rely on predictable terminal ergonomics.

Minimum expectations:

- High-contrast themes in TUI and Web.
- Screen-reader-friendly markup in Web.
- Keyboard-only navigation everywhere that is interactive.
- Every critical action exposed as a CLI command, even if a richer surface exists.
- Internationalizable strings in user-facing paths.

Roko should also adopt familiar key conventions instead of inventing its own where there is no benefit:

- `:` for command palette.
- `/` for slash commands.
- `?` for contextual help.
- `q` to quit.
- `j` and `k` for navigation.
- `Ctrl+R` for prompt history.
- `#` for anchoring a thought, task, or session reference.

Those shortcuts are not just convenience. They are part of the familiar-first contract.

## 8. Surface-Specific Notes

### CLI

CLI remains the most direct automation surface. It should prioritize predictable flags, concise success output, and error messages that always include a next step. Help pages should teach the next command, not just document the current one, and profile install/compose flows should be one command away from the first prompt.

### TUI

TUI should be a control surface, not a read-only dashboard. If the CLI can do it, the TUI should provide a binding or action for it. That includes replaying episodes, editing plans, adjusting thresholds, jumping between active sessions, and selecting or composing profiles with a visible collision summary. The TUI should consume the same in-process StateHub projections as the remote surfaces rather than maintaining a private dashboard-only state tree.

### Chat

Chat should support multi-agent interaction, live streaming, slash commands, and inline artifacts. If the agent emits a code block or a diff, the surface should offer affordances to apply, copy, or inspect it instead of flattening everything into text. Profile install, profile activation, and custody confirmation should be represented as explicit chat actions, not hidden side effects.

### Web

Web should behave like the browser counterpart to the same runtime state, not a separate product. The browser view should show active sessions, live progress, plans, episodes, heuristics, settings, and profile composition without inventing alternate semantics. The same `TypedContext` and `Custody` summary that CLI can inspect should be visible in the browser, and the browser should consume the same named StateHub projections that the TUI and external dashboards read.

## 9. Related Refinements

- [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md) — canonical source for this chapter.
- [tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md) — profile install, TypedContext, and Custody contracts.
- [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md) — projections that carry live progress, permissions, and cross-surface continuity.
- [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md) — streaming transport for `watch` and browser updates.
- [tmp/refinements/28-cli-parity-familiar-workflows.md](../../tmp/refinements/28-cli-parity-familiar-workflows.md) — command palette, slash-command, and history conventions.
- [tmp/refinements/30-rich-ux-primitives.md](../../tmp/refinements/30-rich-ux-primitives.md) — rendering primitives for tool banners, uncertainty, and heuristic annotations.
- [tmp/refinements/22-developer-ux-rust.md](../../tmp/refinements/22-developer-ux-rust.md) — the Rust SDK path for power users who outgrow the shipped surfaces.

## 10. Implementation Notes

This chapter is the canonical user-facing framing for REF23. It intentionally uses the current vocabulary from the naming glossary: `Engram`, `Pulse`, `Bus`, `StateHub`, `Topic`, and `Neuro` where relevant. It also treats named sessions as durable user objects and makes replay, share, and undo part of the normal interaction model rather than exceptional recovery paths. REF25 adds the interface contract for domain profiles: surfaces should expose the same profile chooser, the same `TypedContext` summary, and the same `Custody` trail wherever setup or inspection happens.
