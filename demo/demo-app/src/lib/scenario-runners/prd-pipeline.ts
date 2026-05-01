// --- src/lib/scenario-runners/prd-pipeline.ts ---
import type { Scenario, ScenarioContext } from '../scenarios';
import { compactTime, pipelineEvent } from '../scenario-helpers';
import { enterWorkspace, showCmd, getRoko } from '../terminal-session';
import {
  type PipelineDemoState,
  type PipelinePhase,
  type PipelineScenarioExample,
} from '../prd-pipeline-types';
import {
  createPipelineIntroState,
  PIPELINE_EXAMPLES,
} from '../prd-pipeline-sample';
import {
  fetchWorkflowSnapshot,
  openWorkflowSubscriptions,
  workflowHeadline,
  workflowPhaseToPipelinePhase,
  workflowSnapshotToPlans,
  workflowSnapshotToPrd,
  type WorkflowSnapshot,
} from '../workflow-api';

// ── Pipeline-specific helpers ───────────────────────────────

function applyWorkflowSnapshot(
  ctx: ScenarioContext,
  snapshot: WorkflowSnapshot,
  phase?: PipelinePhase,
  headline?: string,
  currentCommand?: string,
) {
  const patch: Partial<PipelineDemoState> = {
    source: 'live',
    phase: phase ?? workflowPhaseToPipelinePhase(snapshot),
    headline: headline ?? workflowHeadline(snapshot),
    example: ctx.pipelineExample,
    prd: workflowSnapshotToPrd(snapshot),
    plans: workflowSnapshotToPlans(snapshot),
    lastUpdated: compactTime(),
  };
  if (currentCommand !== undefined) patch.currentCommand = currentCommand;
  ctx.patchPipeline(patch);
  ctx.patchPipelineStream({
    workdir: snapshot.workdir,
    workflowId: snapshot.id,
  });
}

async function refreshWorkflowSnapshot(
  ctx: ScenarioContext,
  root: string,
  phase: PipelinePhase,
  headline: string,
  currentCommand?: string,
): Promise<WorkflowSnapshot | null> {
  const snapshot = await fetchWorkflowSnapshot(root);
  if (snapshot) applyWorkflowSnapshot(ctx, snapshot, phase, headline, currentCommand);
  return snapshot;
}

function workflowPhaseFromEvent(eventType: string): PipelinePhase {
  if (eventType.startsWith('plan.')) return 'implementing';
  if (eventType.startsWith('task.')) return 'implementing';
  if (eventType.includes('gate')) return 'implementing';
  return 'tasks';
}

function startWorkflowSubscriptions(ctx: ScenarioContext, root: string): () => void {
  return openWorkflowSubscriptions(root, {
    onSnapshot: (snapshot) => applyWorkflowSnapshot(ctx, snapshot),
    onStatus: ctx.patchPipelineStream,
    onLiveEvent: (event) => {
      ctx.appendPipelineEvent(pipelineEvent(
        workflowPhaseFromEvent(event.event_type),
        event.message || `${event.event_type} ${event.plan_id}${event.task_id ? `:${event.task_id}` : ''}`,
        event.event_type.includes('failed') || event.event_type.includes('error') ? 'error' : 'info',
      ));
    },
    onError: (message) => {
      ctx.appendPipelineEvent(pipelineEvent('failed', message, 'warning'));
    },
  });
}

function failPipeline(ctx: ScenarioContext, phase: PipelinePhase, headline: string, text: string) {
  ctx.patchPipeline({
    source: 'live',
    phase,
    headline,
    example: ctx.pipelineExample,
    lastUpdated: compactTime(),
  });
  ctx.appendPipelineEvent(pipelineEvent(phase, text, 'error'));
}

function rustSetupCommand(example: PipelineScenarioExample): string {
  const name = example.repoName.replace(/[^a-zA-Z0-9_]/g, '_');
  return [
    'mkdir -p src',
    `printf '[package]\\nname = "${name}"\\nversion = "0.1.0"\\nedition = "2021"\\n\\n[dependencies]\\n' > Cargo.toml`,
    `printf 'fn main() {\\n    println!("${example.label} demo");\\n}\\n' > src/main.rs`,
    `printf '#[test]\\nfn it_works() { assert_eq!(2 + 2, 4); }\\n' > src/lib.rs`,
  ].join(' && ');
}

/**
 * Write a string to a file inside the terminal PTY using base64 to avoid
 * shell escaping issues with quotes, newlines, and special chars.
 */
async function writeFileViaPty(
  handle: { execCmd(cmd: string, timeout?: number): Promise<unknown> },
  path: string,
  content: string,
): Promise<void> {
  // btoa() only handles Latin1 — encode via TextEncoder for Unicode safety.
  const bytes = new TextEncoder().encode(content);
  let binary = '';
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  const b64 = btoa(binary);
  await handle.execCmd(
    `echo '${b64}' | base64 -D > '${path}' 2>/dev/null || echo '${b64}' | base64 -d > '${path}'`,
    5000,
  );
}

/**
 * Pre-seed the workspace with PRD, plan, and implementation files so
 * demo commands that detect existing artifacts complete instantly instead
 * of dispatching LLM agents.
 */
async function seedWorkspace(
  handle: { execCmd(cmd: string, timeout?: number): Promise<unknown> },
  example: PipelineScenarioExample,
): Promise<void> {
  if (!example.seedPrd) return;

  await handle.execCmd(
    'mkdir -p .roko/prd/drafts .roko/prd/published .roko/prd/ideas',
    5000,
  );
  await writeFileViaPty(handle, `.roko/prd/drafts/${example.slug}.md`, example.seedPrd);

  if (example.seedTasksToml) {
    await handle.execCmd(`mkdir -p .roko/plans/${example.slug}`, 5000);
    await writeFileViaPty(
      handle,
      `.roko/plans/${example.slug}/tasks.toml`,
      example.seedTasksToml,
    );
    if (example.seedPlanMd) {
      await writeFileViaPty(
        handle,
        `.roko/plans/${example.slug}/plan.md`,
        example.seedPlanMd,
      );
    }
  }

  if (example.seedFiles) {
    for (const [path, content] of Object.entries(example.seedFiles)) {
      const dir = path.includes('/') ? path.substring(0, path.lastIndexOf('/')) : null;
      if (dir) await handle.execCmd(`mkdir -p '${dir}'`, 3000);
      await writeFileViaPty(handle, path, content);
    }
  }
}

// ── Scenario ────────────────────────────────────────────────

export const prdPipeline: Scenario = {
  id: 'prd-pipeline',
  title: 'PRD Pipeline',
  subtitle: 'Pick an example, generate the PRD, generate tasks.toml, then watch routing and gates.',
  panes: 1,
  labels: ['roko commands'],
  panel: true,
  promptBar: false,
  category: 'pipeline',
  features: ['PRD generation', 'Task planning', 'Gate validation'],
  durationHint: '~30s',
  accent: 'rose',
  icon: 'pipeline',
  steps: [
    { label: 'Capture job', sublabel: 'prd idea' },
    { label: 'Generate PRD', sublabel: 'prd draft new' },
    { label: 'Publish PRD', sublabel: 'draft promote' },
    { label: 'Validate tasks', sublabel: 'plan validate' },
    { label: 'Run gates', sublabel: 'cargo test + clippy' },
  ],
  async run(ctx) {
    const { entries, playback, timeline, setMetric, setGate, logCommand, logCommandComplete } = ctx;
    const [main] = entries;
    const example = ctx.pipelineExample ?? PIPELINE_EXAMPLES[0];

    ctx.setPipeline({
      ...createPipelineIntroState(example),
      source: 'live',
      phase: 'setup',
      headline: `Creating a workspace for ${example.label}`,
      events: [pipelineEvent('setup', `Starting live PRD pipeline for ${example.label}.`)],
    });

    const dir = ctx.workspaceDir;
    await enterWorkspace(main, dir);
    const closeWorkflowStreams = startWorkflowSubscriptions(ctx, dir);
    const ROKO = getRoko();
    timeline.init(this.steps);
    setMetric('model', 'T1/T2/T3');

    try {
      // ── Invisible setup: scaffold + seed ────────────────────
      const setupCmd = rustSetupCommand(example);
      playback.setProgress(0, 5, `preparing ${example.label}`);
      ctx.patchPipeline({
        phase: 'setup',
        headline: `Seeding ${example.setupDescription}`,
        currentCommand: setupCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('setup', `${example.setupDescription} This is setup, not the customer-facing demo step.`));
      logCommand('prepare workspace', 'Creates a small Rust CLI so the generated PRD and plan target real files.');
      await main.execCmd(setupCmd, 10000);
      await seedWorkspace(main, example);
      await main.execCmd(`${ROKO} init 2>/dev/null; true`, 10000);
      // Wipe all setup noise so the visible demo starts on a clean terminal.
      main.clearTerminal();

      // ── Step 1: prd idea ────────────────────────────────────
      await playback.waitForStep();
      const ideaCmd = `${ROKO} prd idea "${example.idea}"`;
      playback.setProgress(1, 5, ideaCmd);
      timeline.setActive(0);
      ctx.patchPipeline({
        phase: 'idea',
        headline: 'Capturing the job Roko will turn into a PRD',
        currentCommand: ideaCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('idea', 'Idea captured into .roko/prd/ideas.md.'));
      const ideaResult = await showCmd(main, ideaCmd, { timeout: 45000, onLog: logCommand, onLogComplete: logCommandComplete, playback });
      if (!ideaResult.ok) {
        failPipeline(ctx, 'failed', 'prd idea command failed', 'prd idea returned a non-zero exit code.');
        return;
      }
      await refreshWorkflowSnapshot(ctx, dir, 'idea', 'Captured idea is visible to the workflow projection', ideaCmd);
      setMetric('tokens', 'idea');

      // ── Step 2: prd draft new (detects existing → instant) ──
      await playback.waitForStep();
      const draftCmd = `${ROKO} prd draft new "${example.prdTitle}"`;
      playback.setProgress(2, 5, draftCmd);
      timeline.setActive(1);
      ctx.patchPipeline({
        phase: 'draft',
        headline: 'Generating a structured PRD',
        currentCommand: draftCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('draft', 'PRD draft detected — reusing pre-seeded content.'));
      const draftResult = await showCmd(main, draftCmd, {
        timeout: 30000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        playback,
        customDesc: 'Detects existing PRD draft and confirms it. No LLM call needed.',
      });
      const draftSnapshot = await refreshWorkflowSnapshot(ctx, dir, 'draft', 'Structured PRD generated', draftCmd);
      if (!draftResult.ok) {
        failPipeline(ctx, 'failed', 'PRD draft command failed', 'prd draft new returned a non-zero exit code.');
        return;
      }
      const livePrdSlug = draftSnapshot?.prd?.slug || example.slug;
      setMetric('cost', '$ live');

      // ── Step 3: draft promote ───────────────────────────────
      await playback.waitForStep();
      const promoteCmd = `${ROKO} prd draft promote ${livePrdSlug}`;
      playback.setProgress(3, 5, promoteCmd);
      timeline.setActive(2);
      ctx.patchPipeline({
        phase: 'published',
        headline: 'Publishing the PRD',
        currentCommand: promoteCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('published', 'Promoting draft PRD into the published set.'));
      await showCmd(main, promoteCmd, { timeout: 30000, onLog: logCommand, onLogComplete: logCommandComplete, playback });
      await refreshWorkflowSnapshot(ctx, dir, 'published', 'PRD published and ready for planning', promoteCmd);

      // ── Step 4: plan validate ───────────────────────────────
      await playback.waitForStep();
      const validateCmd = `${ROKO} plan validate .roko/plans`;
      playback.setProgress(4, 5, validateCmd);
      timeline.setActive(3);
      ctx.patchPipeline({
        phase: 'tasks',
        headline: 'Validating pre-generated tasks.toml',
        currentCommand: validateCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('tasks', 'Validating plan structure and task metadata.'));
      const validateResult = await showCmd(main, validateCmd, {
        timeout: 30000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        playback,
        customDesc: 'Validates the pre-seeded tasks.toml without LLM calls.',
      });
      await refreshWorkflowSnapshot(
        ctx,
        dir,
        'tasks',
        'Tasks validated and ready for gate execution',
        validateCmd,
      );
      if (!validateResult.ok) {
        failPipeline(ctx, 'failed', 'Plan validation failed', 'roko plan validate found errors in tasks.toml.');
        return;
      }

      // ── Step 5: cargo test + clippy (real gates) ────────────
      await playback.waitForStep();
      const gateCmd = 'cargo test && cargo clippy -- -D warnings';
      playback.setProgress(5, 5, gateCmd);
      timeline.setActive(4);
      ctx.patchPipeline({
        phase: 'implementing',
        headline: `Running gates on ${example.label}`,
        currentCommand: gateCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('implementing', 'Running cargo test and clippy gates on the seeded implementation.', 'success'));

      const gateResult = await showCmd(main, gateCmd, {
        timeout: 120000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        onGate: setGate,
        playback,
        customDesc: 'Runs real cargo test and cargo clippy gates against the pre-seeded implementation.',
      });

      // Supplement gate detection from raw output
      const output = main.outputBuffer ?? '';
      if (/test result: ok/i.test(output) || /passing/i.test(output)) {
        setGate('test', 'pass');
      }
      if (gateResult.ok) {
        setGate('compile', 'pass');
        setGate('clippy', 'pass');
      }

      const finalSnapshot = await refreshWorkflowSnapshot(
        ctx,
        dir,
        gateResult.ok ? 'complete' : 'failed',
        gateResult.ok ? 'All gates passed' : 'Gate execution failed',
        gateCmd,
      );
      if (!finalSnapshot) {
        // Non-fatal — workflow projection may not track raw cargo commands
      }
      ctx.appendPipelineEvent(
        pipelineEvent(
          gateResult.ok ? 'complete' : 'failed',
          gateResult.ok ? 'All gates passed — implementation verified.' : 'Gate execution returned failures.',
          gateResult.ok ? 'success' : 'error',
        ),
      );
      timeline.markAllComplete();
    } finally {
      closeWorkflowStreams();
    }
  },
};
