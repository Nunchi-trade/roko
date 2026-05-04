// --- src/lib/scenario-runners/prd-pipeline.ts ---
import type { Scenario, ScenarioContext } from '../scenarios';
import { compactTime, pipelineEvent } from '../scenario-helpers';
import { enterWorkspace, ensureWorkspaceCwd, showCmd, roko, trackMetrics } from '../terminal-session';
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
  durationHint: '~180s',
  accent: 'rose',
  icon: 'pipeline',
  steps: [
    { label: 'Capture job', sublabel: 'prd idea' },
    { label: 'Generate PRD', sublabel: 'prd draft new' },
    { label: 'Publish PRD', sublabel: 'draft promote' },
    { label: 'Generate plan', sublabel: 'prd plan' },
    { label: 'Validate tasks', sublabel: 'plan validate' },
    { label: 'Execute plan', sublabel: 'plan run' },
    { label: 'Results', sublabel: 'gates + learn' },
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
    timeline.init(this.steps);
    setMetric('model', '--');

    // Live metric tracking from terminal output
    const tracker = trackMetrics(main, {
      onCost: (c) => setMetric('cost', c),
      onTokens: (t) => setMetric('tokens', t),
    }, 250);
    const stopTracking = () => clearInterval(tracker);

    try {
      // ── Invisible setup: scaffold a minimal Rust workspace ────
      const setupCmd = rustSetupCommand(example);
      playback.setProgress(0, 7, `preparing ${example.label}`);
      ctx.patchPipeline({
        phase: 'setup',
        headline: `Scaffolding ${example.setupDescription}`,
        currentCommand: setupCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('setup', `${example.setupDescription} Creating a minimal Rust scaffold.`));
      logCommand('prepare workspace', 'Creates a small Rust CLI so the generated PRD and plan target real files.');
      if (!(await ensureWorkspaceCwd(main, dir))) {
        failPipeline(ctx, 'failed', 'Workspace setup failed', `Could not enter workspace ${dir}.`);
        return;
      }
      await main.execCmd(setupCmd, 15000);
      if (!(await ensureWorkspaceCwd(main, dir))) {
        failPipeline(ctx, 'failed', 'Workspace setup failed', `Could not re-enter workspace ${dir}.`);
        return;
      }
      await main.execCmd(`${roko(ctx, 'init')} 2>/dev/null; true`, 15000);
      main.clearTerminal();

      // ── Step 1: prd idea ────────────────────────────────────
      await playback.waitForStep();
      const ideaCmd = roko(ctx, `prd idea "${example.idea}"`);
      playback.setProgress(1, 7, ideaCmd);
      timeline.setActive(0);
      ctx.patchPipeline({
        phase: 'idea',
        headline: 'Capturing the job Roko will turn into a PRD',
        currentCommand: ideaCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('idea', 'Idea captured into .roko/prd/ideas.md.'));
      const ideaResult = await showCmd(main, ideaCmd, {
        timeout: 45000,
        workspaceDir: dir,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        playback,
      });
      if (!ideaResult.ok) {
        failPipeline(ctx, 'failed', 'prd idea command failed', 'prd idea returned a non-zero exit code.');
        return;
      }
      await refreshWorkflowSnapshot(ctx, dir, 'idea', 'Captured idea is visible to the workflow projection', ideaCmd);
      if (ideaResult.tokens) setMetric('tokens', ideaResult.tokens);

      // ── Step 2: prd draft new (LLM generates the PRD) ──────
      await playback.waitForStep();
      const draftCmd = roko(ctx, `prd draft new "${example.prdTitle}"`);
      playback.setProgress(2, 7, draftCmd);
      timeline.setActive(1);
      ctx.patchPipeline({
        phase: 'draft',
        headline: 'LLM generating a structured PRD',
        currentCommand: draftCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('draft', 'Generating PRD via LLM agent.'));
      const draftResult = await showCmd(main, draftCmd, {
        timeout: 180000,
        workspaceDir: dir,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        playback,
        customDesc: 'Agent generates a full PRD with requirements, acceptance criteria, and task breakdown.',
      });
      const draftSnapshot = await refreshWorkflowSnapshot(ctx, dir, 'draft', 'Structured PRD generated', draftCmd);
      if (!draftResult.ok) {
        failPipeline(ctx, 'failed', 'PRD draft command failed', 'prd draft new returned a non-zero exit code.');
        return;
      }
      const livePrdSlug = draftSnapshot?.prd?.slug || example.slug;
      if (draftResult.cost) setMetric('cost', draftResult.cost);
      if (draftResult.tokens) setMetric('tokens', draftResult.tokens);

      // ── Step 3: draft promote ───────────────────────────────
      await playback.waitForStep();
      const promoteCmd = roko(ctx, `prd draft promote ${livePrdSlug}`);
      playback.setProgress(3, 7, promoteCmd);
      timeline.setActive(2);
      ctx.patchPipeline({
        phase: 'published',
        headline: 'Publishing the PRD',
        currentCommand: promoteCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('published', 'Promoting draft PRD into the published set.'));
      await showCmd(main, promoteCmd, {
        timeout: 30000,
        workspaceDir: dir,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        playback,
      });
      await refreshWorkflowSnapshot(ctx, dir, 'published', 'PRD published and ready for planning', promoteCmd);

      // ── Step 4: prd plan (LLM generates tasks.toml) ────────
      await playback.waitForStep();
      const planCmd = roko(ctx, `prd plan ${livePrdSlug}`);
      playback.setProgress(4, 7, planCmd);
      timeline.setActive(3);
      ctx.patchPipeline({
        phase: 'planning',
        headline: 'LLM generating implementation plan',
        currentCommand: planCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('planning', 'Generating tasks.toml from the published PRD via LLM agent.'));
      const planResult = await showCmd(main, planCmd, {
        timeout: 300000,
        workspaceDir: dir,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        playback,
        customDesc: 'Agent analyzes the PRD and generates a structured tasks.toml with task dependencies, tiers, and verify steps.',
      });
      await refreshWorkflowSnapshot(ctx, dir, 'tasks', 'Implementation plan generated', planCmd);
      if (!planResult.ok) {
        failPipeline(ctx, 'failed', 'Plan generation failed', 'prd plan returned a non-zero exit code.');
        return;
      }
      if (planResult.cost) setMetric('cost', planResult.cost);
      if (planResult.tokens) setMetric('tokens', planResult.tokens);

      // ── Step 5: plan validate ───────────────────────────────
      await playback.waitForStep();
      const validateCmd = roko(ctx, 'plan validate .roko/plans');
      playback.setProgress(5, 7, validateCmd);
      timeline.setActive(4);
      ctx.patchPipeline({
        phase: 'tasks',
        headline: 'Validating generated tasks.toml',
        currentCommand: validateCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('tasks', 'Validating plan structure and task metadata.'));
      const validateResult = await showCmd(main, validateCmd, {
        timeout: 30000,
        workspaceDir: dir,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        playback,
        customDesc: 'Validates the LLM-generated tasks.toml for structural correctness.',
      });
      await refreshWorkflowSnapshot(ctx, dir, 'tasks', 'Tasks validated and ready for execution', validateCmd);
      if (!validateResult.ok) {
        failPipeline(ctx, 'failed', 'Plan validation failed', 'roko plan validate found errors in tasks.toml.');
        return;
      }

      // ── Step 6: plan run (agents execute tasks, gates validate) ──
      await playback.waitForStep();
      const runCmd = roko(ctx, 'plan run .roko/plans --max-retries 1');
      playback.setProgress(6, 7, runCmd);
      timeline.setActive(5);
      ctx.patchPipeline({
        phase: 'implementing',
        headline: `Agents implementing ${example.label}`,
        currentCommand: runCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('implementing', 'Executing plan: agents implement tasks, gates validate each one.'));
      const runResult = await showCmd(main, runCmd, {
        timeout: 600000,
        workspaceDir: dir,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        onGate: setGate,
        playback,
        customDesc: 'Executes the generated plan. Agents implement tasks, gates (compile, test, clippy) validate each one.',
      });
      if (runResult.cost) setMetric('cost', runResult.cost);
      if (runResult.tokens) setMetric('tokens', runResult.tokens);

      // ── Step 7: Results ─────────────────────────────────────
      await playback.waitForStep();
      playback.setProgress(7, 7, 'results');
      timeline.setActive(6);

      // Show gate results from the plan run
      if (runResult.gates.length > 0) {
        logCommand('gates', runResult.gates.map(gate => `${gate.name}: ${gate.status}`).join(', '));
      }

      // Show learning state
      await showCmd(main, roko(ctx, 'learn all'), {
        timeout: 30000,
        workspaceDir: dir,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        playback,
        customDesc: 'Full learning state: cascade router weights, efficiency metrics.',
      });

      const finalSnapshot = await refreshWorkflowSnapshot(
        ctx,
        dir,
        runResult.ok ? 'complete' : 'failed',
        runResult.ok ? 'Pipeline complete — all tasks implemented and gates run' : 'Pipeline finished with failures',
        'done',
      );
      if (!finalSnapshot) {
        // Non-fatal — workflow projection may not track all commands
      }
      ctx.appendPipelineEvent(
        pipelineEvent(
          runResult.ok ? 'complete' : 'failed',
          runResult.ok ? 'Pipeline complete — implementation verified by gates.' : 'Pipeline finished with gate failures.',
          runResult.ok ? 'success' : 'error',
        ),
      );
      timeline.markAllComplete();
    } finally {
      stopTracking();
      closeWorkflowStreams();
    }
  },
};
