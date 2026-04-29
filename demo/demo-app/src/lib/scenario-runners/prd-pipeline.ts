// --- src/lib/scenario-runners/prd-pipeline.ts ---
import type { Scenario, ScenarioContext } from '../scenarios';
import { compactTime, pipelineEvent } from '../scenario-helpers';
import { enterWorkspace, showCmd, getRoko } from '../terminal-session';
import {
  type PipelineDemoState,
  type PipelinePhase,
  type PipelineScenarioExample,
  type PipelineStreamState,
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
  steps: [
    { label: 'Capture job', sublabel: 'prd idea' },
    { label: 'Generate PRD', sublabel: 'prd draft new' },
    { label: 'Publish PRD', sublabel: 'draft promote' },
    { label: 'Generate tasks', sublabel: 'prd plan' },
    { label: 'Execute gates', sublabel: 'plan run' },
  ],
  async run(ctx) {
    const { entries, playback, timeline, setMetric, setGate, logCommand } = ctx;
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
      main.clearTerminal();

      await playback.waitForStep();
      const ideaCmd = `${ROKO} prd idea "${example.idea}"`;
      playback.setProgress(1, 5, ideaCmd);
      timeline.setActive(0);
      ctx.patchPipeline({
        phase: 'idea',
        headline: `Capturing the job Roko will turn into a PRD`,
        currentCommand: ideaCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('idea', 'Idea captured into .roko/prd/ideas.md.'));
      await showCmd(main, ideaCmd, { timeout: 45000, onLog: logCommand });
      await refreshWorkflowSnapshot(ctx, dir, 'idea', 'Captured idea is visible to the workflow projection', ideaCmd);
      setMetric('tokens', 'idea');

      let livePrdSlug = example.slug;
      await playback.waitForStep();
      const draftCmd = `${ROKO} prd draft new "${example.prdTitle}"`;
      playback.setProgress(2, 5, draftCmd);
      timeline.setActive(1);
      ctx.patchPipeline({
        phase: 'draft',
        headline: 'Generating a structured PRD',
        currentCommand: draftCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('draft', 'Dispatching PRD writer agent.'));
      const draftResult = await showCmd(main, draftCmd, {
        timeout: 180000,
        onLog: logCommand,
        customDesc: 'Runs the PRD generator agent. The UI reads the resulting markdown from the workflow projection API.',
      });
      const draftSnapshot = await refreshWorkflowSnapshot(ctx, dir, 'draft', 'Structured PRD generated', draftCmd);
      if (!draftResult.ok || !draftSnapshot?.prd) {
        failPipeline(ctx, 'failed', 'PRD generation did not produce a draft', 'The workflow projection did not find a generated PRD draft.');
        return;
      }
      livePrdSlug = draftSnapshot.prd.slug || example.slug;
      setMetric('cost', '$ live');

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
      await showCmd(main, promoteCmd, { timeout: 120000, onLog: logCommand });
      await refreshWorkflowSnapshot(ctx, dir, 'published', 'PRD published and ready for planning', promoteCmd);

      await playback.waitForStep();
      const planCmd = `${ROKO} prd plan ${livePrdSlug}`;
      playback.setProgress(4, 5, planCmd);
      timeline.setActive(3);
      ctx.patchPipeline({
        phase: 'planning',
        headline: 'Generating plan directories and tasks.toml',
        currentCommand: planCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('planning', 'Planner agent is generating plan.md and tasks.toml.'));
      const planResult = await showCmd(main, planCmd, {
        timeout: 240000,
        onLog: logCommand,
        customDesc: 'Generates implementation plan directories under .roko/plans/ with modern tasks.toml metadata and verify commands.',
      });
      const planSnapshot = await refreshWorkflowSnapshot(
        ctx,
        dir,
        'tasks',
        'Generated tasks, gates, and routing are ready to execute',
        planCmd,
      );
      if (!planResult.ok || !planSnapshot || planSnapshot.plans.length === 0) {
        failPipeline(ctx, 'failed', 'Plan generation did not produce tasks', 'The workflow projection did not find generated plan/task artifacts.');
        return;
      }

      await playback.waitForStep();
      const runCmd = `${ROKO} plan run .roko/plans --max-retries 1`;
      playback.setProgress(5, 5, runCmd);
      timeline.setActive(4);
      ctx.patchPipeline({
        phase: 'implementing',
        headline: `Roko is implementing ${example.label}`,
        currentCommand: runCmd,
      });
      ctx.appendPipelineEvent(pipelineEvent('implementing', 'Starting real plan execution. Task states update from workflow events and artifacts.', 'success'));

      const runResult = await showCmd(main, runCmd, {
        timeout: 420000,
        onLog: logCommand,
        onGate: setGate,
        customDesc: 'Executes the generated plans through the Roko runner. The task board follows workflow SSE/WS state, gate events, and the workspace event log.',
      });
      const finalSnapshot = await refreshWorkflowSnapshot(
        ctx,
        dir,
        runResult.ok ? 'complete' : 'failed',
        runResult.ok ? 'Generated tasks completed' : 'Plan run ended with failures',
        runCmd,
      );
      if (!finalSnapshot) {
        failPipeline(ctx, 'failed', 'Workflow artifacts disappeared', 'The workflow projection could not load the final snapshot.');
      }
      ctx.appendPipelineEvent(
        pipelineEvent(
          runResult.ok ? 'complete' : 'failed',
          runResult.ok ? 'Implementation run finished.' : 'Implementation run returned a non-ready prompt or failure.',
          runResult.ok ? 'success' : 'error',
        ),
      );
      timeline.setActive(5);
    } finally {
      closeWorkflowStreams();
    }
  },
};
