# Task 021: Demo App Scenario Redesign (14 → 5 Scenarios)

```toml
id = 21
title = "Redesign demo app: collapse 14 scenarios to 5 with custom panels and SSE streaming"
track = "demo-ui"
wave = "wave-2"
priority = "high"
blocked_by = []
touches = [
    "demo/demo-app/src/lib/scenarios.ts",
    "demo/demo-app/src/lib/scenario-runners/",
    "demo/demo-app/src/pages/Demo/ScenarioSlot.tsx",
    "demo/demo-app/src/pages/Demo/index.tsx",
    "demo/demo-app/src/components/",
]
exclusive_files = []
estimated_minutes = 480
```

## Context

The demo app is being collapsed from the old 14-scenario set to the redesigned 5-scenario
set. The original task text used stale names and stale doc paths. The authoritative source
for this task is now:

- `tmp/solutions/demo-running/next-phase/SCENARIO-REDESIGN.md`
- `tmp/solutions/demo-running/next-phase/BATCH-GAPS.md`
- `tmp/solutions/demo-running/next-phase/SCENARIO-DETAILS.md` for supplemental behavior
- `tmp/solutions/demo-running/CURRENT-STATE.md`
- `tmp/solutions/demo-running/next-phase/04-DEMO-UI-REDESIGN.md`
- `tmp/solutions/demo-running/next-phase/06-STREAMING-DESIGN.md`

The 5 scenarios per `SCENARIO-REDESIGN.md` are:

1. **Cost** — naive vs cascaded infer comparison, with live cost/speed/quality metrics.
2. **Pipeline** — one-command `roko do` workflow, with stage progress.
3. **Memory** — cold vs warm workspace, with knowledge transfer/efficiency visualization.
4. **ISFR** — chain health plus agent collaboration panes.
5. **Oracle** — data gathering plus strategy synthesis, with knowledge/flow visualization.

Do not implement the stale `Explore`, `Build`, `Race`, `Learn`, or `Dream` names from the
old task text.

## Background

Read these files first:
1. `tmp/solutions/demo-running/next-phase/SCENARIO-REDESIGN.md` — target design.
2. `tmp/solutions/demo-running/next-phase/SCENARIO-DETAILS.md` — supplemental panel
   and command behavior. Treat `SCENARIO-REDESIGN.md` as authoritative when names differ.
3. `demo/demo-app/src/lib/scenarios.ts` — scenario definitions and command targets.
4. `demo/demo-app/src/lib/scenario-runners/index.ts` — runner dispatch.
5. `demo/demo-app/src/lib/scenario-runners/{cost,pipeline,memory,isfr,oracle}.ts` —
   current runner implementations.
6. `demo/demo-app/src/pages/Demo/ScenarioSlot.tsx` — clickable scenario layout and
   current sidebar routing.
7. `demo/demo-app/src/pages/Demo/index.tsx` — scenario list/page composition.
8. `demo/demo-app/src/hooks/useEventStream.ts`,
   `demo/demo-app/src/hooks/useOperationEvents.ts`,
   `demo/demo-app/src/hooks/useInferenceTrace.ts`, and
   `demo/demo-app/src/lib/EventStreamContext.tsx` — existing SSE plumbing.
9. `demo/demo-app/src/components/PipelineStagesPanel.tsx` and nearby panel components
   before creating new panel code.
10. `demo/demo-app/e2e/demo-all-scenarios.spec.ts` — currently stale if it still expects
    old scenario ids.

Current branch note: the runner index may already export exactly
`costScenario`, `pipelineScenario`, `memoryScenario`, `isfrScenario`, and
`oracleScenario`, and old runner files may already be archived under
`scenario-runners/archive/`. If so, do not redo that work; focus on remaining gaps:
runner fidelity, scenario-specific sidebars, SSE-driven panels, and e2e updates.

## What to Change

1. **Keep the public scenario set exactly five.**
   `scenario-runners/index.ts` must export Cost, Pipeline, Memory, ISFR, and Oracle in
   that order unless design docs are explicitly updated. `scenarios.ts` and the visible
   tab/list UI must not expose old 14-scenario ids.

2. **Bring runner behavior in line with the redesign.**
   - Cost: two panes, naive command with `--no-cascade`, cascaded command without it.
     Both panes should make the comparison obvious in labels and output titles.
   - Pipeline: single `roko do` command that exercises the pipeline stages shown in
     `PipelineStagesPanel`.
   - Memory: model cold vs warm workspaces. If the current implementation reuses one
     workspace, split the setup or explicitly perform the knowledge-transfer step before
     the warm run. The UI should show transfer/efficiency, not just two unrelated runs.
   - ISFR: include a chain-health/doctor-style command before or alongside the agent
     collaboration panes. Preserve four panes for the collaboration view.
   - Oracle: use two panes, one for data collection and one for strategy/synthesis. Do
     not send every command to pane 0.

3. **Replace generic sidebars with scenario-specific panels.**
   In `ScenarioSlot.tsx`, route by scenario id and render:
   - Cost: `CostComparisonPanel` or equivalent comparison panel.
   - Pipeline: existing `PipelineStagesPanel`.
   - Memory: `MemoryTransferPanel`, `KnowledgeFlowPanel`, or equivalent transfer panel.
   - ISFR: `ISFRPanel`, chain-health panel, or equivalent collaboration/chain panel.
   - Oracle: `OracleFlowPanel`, `KnowledgeFlowPanel`, or equivalent flow panel.

4. **Use existing SSE infrastructure.**
   Panels should consume `EventStreamProvider`, `useEventStreamContext`,
   `useOperationEvents`, `usePipelineProgress`, `useInferenceTrace`, or comparable
   existing hooks. Do not create a separate `EventSource` per panel unless the existing
   shared manager cannot support the required event type.

5. **Use React/local runner state, not module-level mutable state.**
   Scenario runner modules should export data/functions and use the provided runner
   context. Avoid `let` state that persists across resets or across scenario runs.

6. **Update tests and e2e expectations.**
   `demo-all-scenarios.spec.ts` should expect the five new scenarios, their pane counts,
   and scenario-specific sidebars. Remove old 14-scenario assertions.

## Existing Code Map

- `demo/demo-app/src/lib/scenario-runners/cost.ts` already demonstrates the intended
  naive-vs-cascade split.
- `demo/demo-app/src/lib/scenario-runners/pipeline.ts` already has a one-command
  pipeline runner.
- `demo/demo-app/src/lib/scenario-runners/memory.ts` may still need explicit workspace
  transfer semantics.
- `demo/demo-app/src/lib/scenario-runners/isfr.ts` may still need an explicit chain
  health step.
- `demo/demo-app/src/lib/scenario-runners/oracle.ts` may still be configured for one
  pane; redesign requires two panes.
- `demo/demo-app/src/pages/Demo/ScenarioSlot.tsx` currently contains the clickable layout
  and is the right place to replace any hardcoded pipeline-only sidebar branch with a
  scenario-specific switch/helper.
- `demo/demo-app/src/components/SidebarRenderer.tsx` may still reference old ids such as
  `prd-pipeline`, `knowledge-transfer`, `chain-intelligence`, and `isfr-agents`. Either
  update it for new ids or bypass it cleanly from `ScenarioSlot.tsx`; do not leave active
  routing tied to the old ids.

## Mechanical Implementation Order

1. Confirm `scenario-runners/index.ts` and `scenarios.ts` expose exactly the five target
   scenarios.
2. Fix runner details for Memory, ISFR, and Oracle before changing UI tests.
3. Add or adapt panel components using existing panel styles and hooks.
4. Replace `ScenarioSlot.tsx` sidebar selection with a typed scenario-id switch.
5. Update `demo-all-scenarios.spec.ts` to the five-scenario world.
6. Run TypeScript/build/e2e checks, then remove any stale references that only existed for
   active routing. Keep archived old runners archived; do not delete them.

## What NOT to Do

- Don't fix bugs in the old 14 scenarios (they're being replaced).
- Don't add new backend functionality (this is frontend-only).
- Don't change the roko-serve SSE format.
- Don't remove the terminal component — scenarios still use the terminal.
- Don't implement `Explore`, `Build`, `Race`, `Learn`, or `Dream`.
- Don't add per-component raw `new EventSource(...)` calls when shared SSE context is
  already mounted in `main.tsx`.
- Don't leave old e2e tests asserting the 14 archived scenarios.
- Don't introduce module-scope mutable variables in runner modules.

## Wire Target

```bash
cd demo/demo-app && npm run build
# Should compile without errors

cd demo/demo-app && npm run dev
# Open http://localhost:5173 — should show 5 scenarios with custom sidebars
```

Expected observable behavior:
- The first screen shows five scenario choices/tabs: Cost, Pipeline, Memory, ISFR, Oracle.
- Each scenario shows terminal panes plus its own metrics/flow panel, not a generic
  label-only `ContextPanel`.
- Streaming panels update from the shared event stream when backend events arrive, while
  still rendering a stable empty/loading state when no events have arrived.
- Old scenarios remain only under `scenario-runners/archive/` or non-active historical
  references.

## Verification

- [ ] `cd demo/demo-app && npx tsc --noEmit` — passes
- [ ] `cd demo/demo-app && npm run build` — builds successfully
- [ ] `cd demo/demo-app && npx playwright test e2e/demo-all-scenarios.spec.ts` — passes
- [ ] 5 scenarios visible in the demo UI
- [ ] Each scenario has its own custom sidebar panel (not generic ContextPanel)
- [ ] No module-level mutable state in new scenario runners
- [ ] Old scenario files archived in `scenario-runners/archive/`
- [ ] `rg "explore|provider-race|gate-retry|dream-consolidation|prd-pipeline" demo/demo-app/src/lib demo/demo-app/src/pages/Demo demo/demo-app/e2e` shows no active route/test dependency on old ids

## Status Log

| Time | Agent | Action |
|------|-------|--------|
