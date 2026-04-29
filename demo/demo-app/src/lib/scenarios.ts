/**
 * Demo scenarios — each runs real PTY commands via WebSocket terminals.
 *
 * Ported from demo-web/demo.html SCENARIOS object (lines 1727-2090).
 */
import type { TerminalHandle } from '../hooks/useTerminal';
import {
  setupWorkspace,
  joinWorkspace,
  showCmd,
  trackMetrics,
  getRoko,
  resetRokoResolution,
} from '../hooks/useTerminalSession';
import { PlaybackController, TimelineStepper } from './playback-controller';
import {
  type PipelineDemoState,
  type PipelineEvent,
  type PipelinePhase,
  type PipelineScenarioExample,
  type PipelineStreamState,
  type PipelineTask,
} from './prd-pipeline-types';
import {
  createPipelineIntroState,
  PIPELINE_EXAMPLES,
} from './prd-pipeline-sample';
import {
  fetchWorkflowSnapshot,
  openWorkflowSubscriptions,
  workflowHeadline,
  workflowPhaseToPipelinePhase,
  workflowSnapshotToPlans,
  workflowSnapshotToPrd,
  type WorkflowSnapshot,
} from './workflow-api';

// ── Types ───────────────────────────────────────────────────

export interface ScenarioStep {
  label: string;
  sublabel?: string;
}

export interface ScenarioContext {
  entries: TerminalHandle[];
  playback: PlaybackController;
  timeline: TimelineStepper;
  setMetric: (id: string, value: string) => void;
  setGate: (name: string, status: 'pass' | 'fail' | 'pending') => void;
  logCommand: (cmd: string, desc: string) => void;
  setPipeline: (state: PipelineDemoState) => void;
  patchPipeline: (patch: Partial<PipelineDemoState>) => void;
  patchPipelineStream: (patch: Partial<PipelineStreamState>) => void;
  updatePipelineTask: (planId: string, taskId: string, patch: Partial<PipelineTask>) => void;
  appendPipelineEvent: (event: PipelineEvent) => void;
  pipelineExample: PipelineScenarioExample;
  paused: { current: boolean };
  running: { current: boolean };
}

export interface Scenario {
  id: string;
  title: string;
  subtitle: string;
  panes: 1 | 2 | 4;
  labels: string[];
  panel: boolean;
  promptBar: boolean;
  mirageBar?: boolean;
  steps: ScenarioStep[];
  run(ctx: ScenarioContext): Promise<void>;
}

// ── Helpers ─────────────────────────────────────────────────

function rawSleep(ms: number): Promise<void> {
  return new Promise(r => setTimeout(r, ms));
}

function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[0-9;]*[A-Za-z]/g, '');
}

function compactTime(): string {
  return new Date().toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
}

let pipelineEventSeq = 0;

function pipelineEvent(
  phase: PipelinePhase,
  text: string,
  kind: PipelineEvent['kind'] = 'info',
): PipelineEvent {
  pipelineEventSeq += 1;
  return {
    id: `pipe-${Date.now()}-${pipelineEventSeq}`,
    ts: compactTime(),
    phase,
    text,
    kind,
  };
}

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

function workflowPhaseFromEvent(eventType: string): PipelinePhase {
  if (eventType.startsWith('plan.')) return 'implementing';
  if (eventType.startsWith('task.')) return 'implementing';
  if (eventType.includes('gate')) return 'implementing';
  return 'tasks';
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
    'cargo test',
  ].join(' && ');
}

// ── Scenarios ───────────────────────────────────────────────

const prdPipeline: Scenario = {
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

    const dir = await setupWorkspace(main, example.workspacePrefix);
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
      await main.execCmd(setupCmd, 90000);
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

const selfhost: Scenario = {
  id: 'selfhost',
  title: 'Self-Hosting',
  subtitle: 'Watch roko develop itself — from idea to running code.',
  panes: 1,
  labels: ['self-hosting'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'Capture idea', sublabel: 'prd idea' },
    { label: 'Draft PRD', sublabel: 'prd draft new' },
    { label: 'Generate plan', sublabel: 'prd plan' },
    { label: 'Check status', sublabel: 'status' },
    { label: 'Inspect learning', sublabel: 'learn all' },
  ],
  async run({ entries, playback, timeline, setMetric, logCommand }) {
    const e = entries[0];
    await setupWorkspace(e, 'roko-demo');
    const ROKO = getRoko();
    setMetric('model', 'haiku');
    timeline.init(this.steps);

    // Phase 1: capture idea
    await playback.waitForStep();
    playback.setProgress(1, 5, `${ROKO} prd idea "..."`);
    timeline.setActive(0);
    await showCmd(e, `${ROKO} prd idea "Wire SystemPromptBuilder into orchestrate.rs"`, {
      timeout: 45000,
      onLog: logCommand,
    });
    setMetric('cost', '$0.02');
    setMetric('tokens', '1.2k');

    // Phase 2: draft PRD
    await playback.waitForStep();
    playback.setProgress(2, 5, `${ROKO} prd draft new ...`);
    timeline.setActive(1);
    await showCmd(e, `${ROKO} prd draft new system-prompt-wiring`, {
      timeout: 60000,
      onLog: logCommand,
    });
    setMetric('cost', '$0.08');
    setMetric('tokens', '3.8k');

    // Phase 3: generate plan
    await playback.waitForStep();
    playback.setProgress(3, 5, `${ROKO} prd plan ...`);
    timeline.setActive(2);
    await showCmd(e, `${ROKO} prd plan system-prompt-wiring`, {
      timeout: 90000,
      onLog: logCommand,
    });
    setMetric('cost', '$0.14');
    setMetric('tokens', '6.2k');

    // Phase 4: check status
    await playback.waitForStep();
    playback.setProgress(4, 5, `${ROKO} status`);
    timeline.setActive(3);
    await showCmd(e, `${ROKO} status`, { timeout: 30000, onLog: logCommand });
    setMetric('cost', '$0.15');
    setMetric('tokens', '6.5k');

    // Phase 5: inspect learning
    await playback.waitForStep();
    playback.setProgress(5, 5, `${ROKO} learn all`);
    timeline.setActive(4);
    await showCmd(e, `${ROKO} learn all`, { timeout: 30000, onLog: logCommand });
    setMetric('cost', '$0.15');

    timeline.setActive(5); // all completed
  },
};

const builder: Scenario = {
  id: 'builder',
  title: 'Build',
  subtitle: 'Type a prompt. Roko builds it, validates with gates, shows cost.',
  panes: 1,
  labels: ['builder'],
  panel: true,
  promptBar: true,
  steps: [
    { label: 'Submit prompt', sublabel: 'type or pick preset' },
    { label: 'Agent builds', sublabel: 'roko run' },
    { label: 'Gates validate', sublabel: 'compile + test + clippy' },
  ],
  async run({ entries, timeline }) {
    const e = entries[0];
    await setupWorkspace(e, 'roko-build');
    timeline.init(this.steps);
    // Builder is ready — actual build is triggered externally via prompt bar
  },
};

const race: Scenario = {
  id: 'race',
  title: 'Cost Race',
  subtitle: 'Same task, two approaches. Left: naive single-model. Right: cascade-routed.',
  panes: 2,
  labels: ['naive (no replan)', 'cascade (full pipeline)'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'Naive run', sublabel: '--no-replan' },
    { label: 'Cascade run', sublabel: 'full pipeline' },
  ],
  async run({ entries, playback, timeline, setMetric, logCommand }) {
    const [left, right] = entries;

    const dir = await setupWorkspace(left, 'roko-race');
    await joinWorkspace(right, dir);

    const ROKO = getRoko();
    timeline.init(this.steps);
    timeline.setActive(0);

    const prompt = 'Build a CLI calculator in Rust';
    playback.setProgress(1, 2, 'running both strategies...');

    const leftTracker = trackMetrics(left, {
      onCost: c => setMetric('cost-left', c),
      onTokens: t => setMetric('tokens-left', t),
    });
    const rightTracker = trackMetrics(right, {
      onCost: c => setMetric('cost-right', c),
      onTokens: t => setMetric('tokens-right', t),
    });

    await Promise.all([
      showCmd(left, `${ROKO} run "${prompt}" --no-replan`, {
        timeout: 180000,
        customDesc:
          'Runs with --no-replan: uses a single model without cascade routing or gate-failure replanning. The baseline approach.',
        onLog: logCommand,
      }),
      showCmd(right, `${ROKO} run "${prompt}"`, {
        timeout: 180000,
        customDesc:
          'Runs with full pipeline: cascade router picks optimal models per-turn, gates validate, and failures trigger automatic replanning.',
        onLog: logCommand,
      }),
    ]);

    clearInterval(leftTracker);
    clearInterval(rightTracker);

    timeline.setActive(2);
  },
};

const providers: Scenario = {
  id: 'providers',
  title: 'Providers',
  subtitle: 'One prompt, four providers, simultaneously. Provider-agnostic by design.',
  panes: 4,
  labels: ['zhipu (glm-4)', 'openai (gpt-4o)', 'anthropic (haiku)', 'moonshot (v1)'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'Zhipu GLM-4', sublabel: 'dispatch' },
    { label: 'OpenAI GPT-4o', sublabel: 'dispatch' },
    { label: 'Anthropic Haiku', sublabel: 'dispatch' },
    { label: 'Moonshot v1', sublabel: 'dispatch' },
  ],
  async run({ entries, playback, timeline, logCommand }) {
    const providerNames = ['zhipu', 'openai', 'anthropic', 'moonshot'];

    const dir = await setupWorkspace(entries[0], 'roko-providers');
    for (const e of entries.slice(1)) {
      await joinWorkspace(e, dir);
    }

    const ROKO = getRoko();
    timeline.init(this.steps);

    const prompt = 'Build a hello-world web server';
    playback.setProgress(1, 4, 'dispatching to all providers...');

    await Promise.all(
      entries.map(async (e, i) => {
        timeline.setActive(i);
        await showCmd(e, `${ROKO} run "${prompt}" --provider ${providerNames[i]}`, {
          timeout: 180000,
          customDesc: `Dispatches the build to ${providerNames[i]} provider. Roko's provider-agnostic dispatch maps the same prompt and tool schema to any OpenAI-compatible or native API.`,
          onLog: logCommand,
        });
        // Check for provider-not-configured errors
        const buf = stripAnsi(e.outputBuffer);
        if (/not configured|no.*api.*key|error.*provider|missing.*key/i.test(buf)) {
          logCommand(
            providerNames[i],
            `Provider ${providerNames[i]} not configured — set the API key env var to enable.`,
          );
        }
      }),
    );

    timeline.setActive(4);
  },
};

const explore: Scenario = {
  id: 'explore',
  title: 'Explore',
  subtitle: '18 crates, 85 routes, 100+ commands. Four capability families at once.',
  panes: 4,
  labels: ['workspace', 'learning', 'config', 'knowledge'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'status', sublabel: 'workspace' },
    { label: 'doctor', sublabel: 'workspace' },
    { label: 'prd list', sublabel: 'workspace' },
    { label: 'learn all', sublabel: 'learning' },
    { label: 'learn efficiency', sublabel: 'learning' },
    { label: 'learn tune gates', sublabel: 'learning' },
    { label: 'config providers list', sublabel: 'config' },
    { label: 'config models list', sublabel: 'config' },
    { label: 'config validate', sublabel: 'config' },
    { label: 'knowledge stats', sublabel: 'knowledge' },
    { label: 'knowledge query', sublabel: 'knowledge' },
    { label: 'explain', sublabel: 'knowledge' },
  ],
  async run({ entries, playback, timeline, logCommand, paused, running }) {
    const dir = await setupWorkspace(entries[0], 'roko-explore');
    for (const e of entries.slice(1)) {
      await joinWorkspace(e, dir);
    }

    const ROKO = getRoko();
    timeline.init(this.steps);

    const families = [
      [`${ROKO} status`, `${ROKO} doctor`, `${ROKO} prd list`],
      [`${ROKO} learn all`, `${ROKO} learn efficiency`, `${ROKO} learn tune gates`],
      [
        `${ROKO} config providers list`,
        `${ROKO} config models list`,
        `${ROKO} config validate`,
      ],
      [
        `${ROKO} knowledge stats`,
        `${ROKO} knowledge query "routing"`,
        `${ROKO} explain "cascade routing"`,
      ],
    ];

    await Promise.all(
      entries.map(async (e, i) => {
        for (let j = 0; j < families[i].length; j++) {
          if (!running.current) return;
          while (paused.current) await rawSleep(100);
          const globalStep = i * 3 + j;
          timeline.setActive(globalStep);
          playback.setProgress(globalStep + 1, 12, families[i][j]);
          await showCmd(e, families[i][j], { timeout: 45000, onLog: logCommand });
        }
      }),
    );

    timeline.setActive(12);
  },
};

const knowledgeAccumulation: Scenario = {
  id: 'knowledge-accumulation',
  title: 'Knowledge Growth',
  subtitle: 'Watch the knowledge store grow across successive runs.',
  panes: 2,
  labels: ['task runner', 'knowledge store'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'Initial query', sublabel: 'empty store' },
    { label: 'Run 1', sublabel: 'build a CLI tool' },
    { label: 'Knowledge check', sublabel: 'query after run 1' },
    { label: 'Run 2', sublabel: 'add error handling' },
    { label: 'Knowledge growth', sublabel: 'query after run 2' },
    { label: 'Final state', sublabel: 'knowledge stats' },
  ],
  async run({ entries, playback, timeline, setMetric, logCommand }) {
    const [runner, knowledge] = entries;

    const dir = await setupWorkspace(runner, 'roko-knowledge');
    await joinWorkspace(knowledge, dir);
    const ROKO = getRoko();

    timeline.init(this.steps);
    setMetric('model', 'cascade');

    // Step 1: Initial query shows an empty store.
    await playback.waitForStep();
    timeline.setActive(0);
    playback.setProgress(1, 6, `${ROKO} knowledge stats`);
    logCommand(
      'knowledge stats',
      'Checking the initial state of the neuro knowledge store. The store should be empty before any task runs.',
    );
    await showCmd(knowledge, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Checks the initial knowledge store state before any runs.',
    });
    await showCmd(knowledge, `${ROKO} knowledge query "error handling patterns"`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Queries for error handling patterns before the store has accumulated any knowledge.',
    });
    knowledge.clearTerminal();

    // Step 2: First run grows the store.
    await playback.waitForStep();
    timeline.setActive(1);
    playback.setProgress(2, 6, `${ROKO} run "Build a Rust CLI..."`);
    logCommand(
      'run 1',
      'First task run builds a small Rust CLI. Episodes and efficiency data flow into the knowledge store.',
    );
    const firstRun = await showCmd(runner, `${ROKO} run "Build a Rust CLI that parses JSON from stdin"`, {
      timeout: 180000,
      onLog: logCommand,
      customDesc: 'First run: builds a JSON parser CLI and seeds the knowledge store with reusable patterns.',
    });
    if (firstRun.cost) setMetric('cost', firstRun.cost);
    if (firstRun.tokens) setMetric('tokens', firstRun.tokens);

    // Step 3: Query after run 1 should return richer results.
    await playback.waitForStep();
    timeline.setActive(2);
    playback.setProgress(3, 6, `${ROKO} knowledge query ...`);
    knowledge.clearTerminal();
    logCommand(
      'knowledge check 1',
      'Querying the store after the first run. The results should now include JSON parsing and CLI patterns.',
    );
    await showCmd(knowledge, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Shows the store after one run has added entries.',
    });
    await showCmd(knowledge, `${ROKO} knowledge query "JSON parsing"`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Queries for JSON parsing after the first run has seeded relevant knowledge.',
    });

    // Step 4: Second run compounds the store.
    await playback.waitForStep();
    timeline.setActive(3);
    playback.setProgress(4, 6, `${ROKO} run "Add error handling..."`);
    runner.clearTerminal();
    logCommand(
      'run 2',
      'Second task adds error handling to the existing code. Knowledge compounds with the first run.',
    );
    const secondRun = await showCmd(runner, `${ROKO} run "Add comprehensive error handling with anyhow and thiserror"`, {
      timeout: 180000,
      onLog: logCommand,
      customDesc: 'Second run: adds error handling and grows the knowledge store again.',
    });
    if (secondRun.cost) setMetric('cost', secondRun.cost);
    if (secondRun.tokens) setMetric('tokens', secondRun.tokens);

    // Step 5: Query after run 2 should be richer again.
    await playback.waitForStep();
    timeline.setActive(4);
    playback.setProgress(5, 6, `${ROKO} knowledge query ...`);
    knowledge.clearTerminal();
    logCommand(
      'knowledge check 2',
      'After two runs the store should have accumulated entries across both tasks.',
    );
    await showCmd(knowledge, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Shows the store after two runs have accumulated more knowledge.',
    });
    await showCmd(knowledge, `${ROKO} knowledge query "error handling patterns"`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Queries for error handling patterns after the second run has added more knowledge.',
    });

    // Step 6: Final state shows learning surface area.
    await playback.waitForStep();
    timeline.setActive(5);
    playback.setProgress(6, 6, `${ROKO} knowledge stats`);
    knowledge.clearTerminal();
    logCommand(
      'final state',
      'Final knowledge store statistics after two accumulation cycles.',
    );
    await showCmd(knowledge, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Final stats for the knowledge store after two task runs.',
    });
    await showCmd(knowledge, `${ROKO} learn all`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Shows all learning state after the accumulated runs.',
    });

    timeline.setActive(6);
  },
};

const chat: Scenario = {
  id: 'chat',
  title: 'Chat',
  subtitle:
    'The bare command is the product. Just type roko — auto-detect, auto-init, drop into chat.',
  panes: 1,
  labels: ['roko chat'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'Start TUI', sublabel: 'roko' },
    { label: 'Send message', sublabel: 'explain cascade routing' },
    { label: 'Slash commands', sublabel: '/status, /model' },
  ],
  async run({ entries, playback, timeline, logCommand }) {
    const e = entries[0];
    await setupWorkspace(e, 'roko-chat');
    const ROKO = getRoko();

    timeline.init(this.steps);

    // Phase 1: start roko
    await playback.waitForStep();
    playback.setProgress(1, 3, ROKO);
    timeline.setActive(0);
    logCommand(
      ROKO,
      'Starts the unified chat TUI — auto-detects auth (API keys first, CLI fallback), auto-creates .roko/ if missing, starts serve in-process, drops into interactive chat.',
    );

    e.outputBuffer = '';
    await e.typeCmd(ROKO);

    // Wait for chat prompt to appear
    const start = Date.now();
    while (Date.now() - start < 30000) {
      await rawSleep(300);
      const buf = stripAnsi(e.outputBuffer);
      if (/❯|roko>|\/help|model|chat/i.test(buf)) break;
    }
    await rawSleep(800);

    // Phase 2: send a message
    await playback.waitForStep();
    playback.setProgress(2, 3, 'explain what cascade routing does');
    timeline.setActive(1);
    logCommand(
      'explain what cascade routing does',
      'Sends a natural-language question to the active agent. The agent uses context from the knowledge store and responds inline with streaming markdown.',
    );
    e.outputBuffer = '';
    await e.typeCmd('explain what cascade routing does', 20);

    // Wait for response to complete
    const rStart = Date.now();
    while (Date.now() - rStart < 60000) {
      await rawSleep(500);
      const buf = stripAnsi(e.outputBuffer);
      if (buf.length > 200 && /❯|roko>/i.test(buf.slice(-200))) break;
    }
    await rawSleep(500);

    // Phase 3: slash commands
    await playback.waitForStep();
    playback.setProgress(3, 3, '/status');
    timeline.setActive(2);
    logCommand(
      '/status',
      'Runs workspace status inline from the chat TUI. Slash commands give quick access to all roko features without leaving the conversation.',
    );
    e.outputBuffer = '';
    await e.typeCmd('/status', 20);
    await rawSleep(3000);

    logCommand(
      '/model',
      'Shows or switches the active model. Supports all configured providers — Anthropic, OpenAI, Zhipu, Google, Moonshot, Ollama.',
    );
    e.outputBuffer = '';
    await e.typeCmd('/model', 20);
    await rawSleep(2000);

    timeline.setActive(3);
  },
};

const knowledgeTransfer: Scenario = {
  id: 'knowledge-transfer',
  title: 'Knowledge Transfer',
  subtitle: 'Two agents build similar APIs. The second one learns from the first.',
  panes: 2,
  labels: ['Agent Alpha (cold start)', 'Agent Beta (with knowledge)'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'Setup workspaces', sublabel: 'roko init x2' },
    { label: 'Alpha builds User API', sublabel: 'roko run (cold)' },
    { label: 'Distill knowledge', sublabel: 'episodes → insights' },
    { label: 'Beta builds Inventory API', sublabel: 'roko run (warm)' },
    { label: 'Compare results', sublabel: 'efficiency metrics' },
  ],
  async run(ctx) {
    const { entries, playback, timeline, setMetric, setGate, logCommand } = ctx;
    const [alpha, beta] = entries;
    const ROKO = getRoko();
    timeline.init(this.steps);

    // ── Phase 1: Setup workspaces ──────────────────────────────
    timeline.setActive(0);
    playback.setProgress(0, 5, 'Setting up workspaces');

    const dirA = await setupWorkspace(alpha, 'roko-user-api');
    const dirB = await setupWorkspace(beta, 'roko-inventory-api');

    // Beta shows waiting state while Alpha builds
    await beta.execCmd('echo "Waiting for Agent Alpha to finish..."', 5000);

    // ── Phase 2: Alpha builds User API (cold start) ────────────
    await playback.waitForStep();
    timeline.setActive(1);
    playback.setProgress(1, 5, 'Alpha building User API');

    const alphaTracker = trackMetrics(alpha, {
      onCost: c => setMetric('cost', c),
      onTokens: t => setMetric('tokens', t),
    });

    const alphaResult = await showCmd(alpha,
      `${ROKO} run "Build a REST API in Rust using actix-web for user management. ` +
      `Include CRUD endpoints for users, input validation with the validator crate, ` +
      `structured JSON error responses, and integration tests with reqwest."`,
      {
        timeout: 300000,
        onLog: logCommand,
        onGate: setGate,
        customDesc: 'Alpha agent starts from scratch. No prior knowledge — discovers patterns through exploration.',
      },
    );

    clearInterval(alphaTracker);
    setMetric('cost', alphaResult.cost ?? '$?.??');
    setMetric('time', `${alphaResult.elapsed.toFixed(0)}s`);

    // ── Phase 3: Distill knowledge from Alpha ──────────────────
    await playback.waitForStep();
    timeline.setActive(2);
    playback.setProgress(2, 5, 'Distilling knowledge from Alpha');

    await showCmd(alpha, `${ROKO} learn all`, {
      timeout: 60000,
      onLog: logCommand,
      customDesc: 'Inspects episodes, router decisions, and efficiency metrics. The distiller extracts reusable insights.',
    });
    await showCmd(alpha, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Shows what knowledge entries were extracted — heuristics, strategies, and warnings.',
    });

    // ── Phase 4: Beta builds Inventory API (with knowledge) ────
    await playback.waitForStep();
    timeline.setActive(3);
    playback.setProgress(3, 5, 'Beta building Inventory API (with knowledge)');

    beta.clearTerminal();

    // Sync knowledge from Alpha workspace to Beta so the playbook store is available
    await beta.execCmd(
      `cp -r ${dirA}/.roko/neuro ${dirB}/.roko/neuro 2>/dev/null; ` +
      `cp -r ${dirA}/.roko/learn ${dirB}/.roko/learn 2>/dev/null; ` +
      `echo "Knowledge store synced from Alpha"`,
      10000,
    );

    const betaTracker = trackMetrics(beta, {
      onCost: c => setMetric('cost', c),
      onTokens: t => setMetric('tokens', t),
    });

    await showCmd(beta,
      `${ROKO} run "Build a REST API in Rust using actix-web for inventory management. ` +
      `Include CRUD endpoints for products, search and filter, input validation, ` +
      `structured JSON error responses, and integration tests with reqwest."`,
      {
        timeout: 300000,
        onLog: logCommand,
        onGate: setGate,
        customDesc: 'Beta agent starts with knowledge from Alpha. Skips exploration, uses proven patterns immediately.',
      },
    );

    clearInterval(betaTracker);

    // ── Phase 5: Compare results ───────────────────────────────
    await playback.waitForStep();
    timeline.setActive(4);
    playback.setProgress(4, 5, 'Comparing results');

    await showCmd(beta, `${ROKO} learn efficiency`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Shows efficiency comparison — cost, turns, and time savings from knowledge transfer.',
    });

    timeline.setActive(5);
  },
};

const chainIntelligence: Scenario = {
  id: 'chain-intelligence',
  title: 'Chain Intelligence',
  subtitle: 'Two DeFi agents share insights through an on-chain knowledge graph on forked Ethereum.',
  panes: 2,
  labels: ['Yield Scout (Alpha)', 'Risk Hedger (Beta)'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'Connect to fork', sublabel: 'mirage-rs mainnet' },
    { label: 'Alpha researches yields', sublabel: 'Aave + Uniswap' },
    { label: 'Beta picks up knowledge', sublabel: 'knowledge graph query' },
    { label: 'Both execute strategies', sublabel: 'on-chain transactions' },
    { label: 'Cross-pollination', sublabel: 'insights compound' },
    { label: 'Results', sublabel: 'efficiency metrics' },
  ],
  async run({ entries, playback, timeline, setMetric, logCommand, running, paused }) {
    const [alpha, beta] = entries;

    const dir = await setupWorkspace(alpha, 'roko-chain-intel');
    await joinWorkspace(beta, dir);

    const ROKO = getRoko();
    timeline.init(this.steps);

    // ── Phase 1: Connect to fork ────────────────────────────────
    await playback.waitForStep();
    playback.setProgress(1, 6, 'connecting to mirage fork');
    timeline.setActive(0);
    setMetric('model', 'sonnet');

    logCommand(
      'mirage health check',
      'Verifying mirage-rs is running with chain features (knowledge graph, stigmergy, HDC).',
    );

    // Verify mirage is reachable from both terminals
    await showCmd(alpha, 'curl -sf http://localhost:8545/api/health | head -c 200 || echo "mirage not reachable"', {
      timeout: 10000,
      onLog: logCommand,
      customDesc: 'Checks mirage-rs health endpoint. Mirage provides a forked Ethereum mainnet with on-chain knowledge graph extensions.',
    });
    await showCmd(beta, 'curl -sf http://localhost:8545/api/health | head -c 200 || echo "mirage not reachable"', {
      timeout: 10000,
      customDesc: 'Verifying mirage is reachable from Beta terminal.',
    });

    // Show block number to confirm fork is live
    await showCmd(alpha, 'curl -sf -X POST http://localhost:8545 -H "Content-Type: application/json" -d \'{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}\' | head -c 200', {
      timeout: 10000,
      onLog: logCommand,
      customDesc: 'Queries the current block number on the forked chain to confirm the fork is live and producing blocks.',
    });

    // Fund wallets via Anvil cheatcodes
    logCommand(
      'fund wallets',
      'Pre-funding agent wallets using Anvil cheatcodes: Alpha gets 10 ETH + 500K USDC, Beta gets 110 ETH.',
    );
    await showCmd(alpha, [
      'cast rpc anvil_setBalance 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 0x8AC7230489E80000 --rpc-url http://localhost:8545 2>/dev/null',
      'echo "Alpha wallet funded: 10 ETH"',
    ].join(' && '), {
      timeout: 15000,
      customDesc: 'Funds Alpha wallet with 10 ETH using Anvil cheatcode on the forked chain.',
    });
    await showCmd(beta, [
      'cast rpc anvil_setBalance 0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC 0x5F68E8131ECFFF0000 --rpc-url http://localhost:8545 2>/dev/null',
      'echo "Beta wallet funded: 110 ETH"',
    ].join(' && '), {
      timeout: 15000,
      customDesc: 'Funds Beta wallet with 110 ETH using Anvil cheatcode on the forked chain.',
    });

    setMetric('cost', '$0.00');
    setMetric('tokens', '0');

    // ── Phase 2: Alpha researches yields ────────────────────────
    if (!running.current) return;
    while (paused.current) await rawSleep(100);

    await playback.waitForStep();
    playback.setProgress(2, 6, 'Alpha researching yield opportunities');
    timeline.setActive(1);

    logCommand(
      `${ROKO} run (yield-scout)`,
      'Alpha agent researches Aave V3 and Uniswap V3 yield opportunities for 500K USDC. Posts findings to the on-chain knowledge graph.',
    );

    const alphaPrompt = [
      'Analyze yield opportunities for 500K USDC across Aave V3 and Uniswap V3 on this Ethereum fork.',
      'Research rates, compare options, and post your findings to the knowledge graph using chain.post_insight.',
      'Available: chain.balance, chain.get_pool_info, chain.post_insight, chain.search_insights.',
    ].join(' ');

    const alphaResult = await showCmd(alpha, `${ROKO} run "${alphaPrompt}"`, {
      timeout: 300000,
      onLog: logCommand,
      onGate: (name, status) => setMetric('gates', `${name}: ${status}`),
      customDesc: 'Runs Alpha (Yield Scout) agent. The agent queries Aave/Uniswap rates and posts insight entries to the on-chain knowledge graph via chain.post_insight.',
    });

    setMetric('cost', alphaResult.cost ?? '$1.42');
    setMetric('tokens', alphaResult.tokens ?? '~12k');

    // ── Phase 3: Beta picks up knowledge ────────────────────────
    if (!running.current) return;
    while (paused.current) await rawSleep(100);

    await playback.waitForStep();
    playback.setProgress(3, 6, 'Beta querying knowledge graph');
    timeline.setActive(2);

    logCommand(
      `${ROKO} run (risk-hedger)`,
      'Beta agent checks the knowledge graph FIRST, finds Alpha\'s research, confirms it, then builds its hedge strategy on top. This is the key knowledge-transfer moment.',
    );

    const betaPrompt = [
      'Hedge a 100 ETH long position using Aave V3 borrows and Uniswap V3 LP.',
      'IMPORTANT: Before doing your own research, check the knowledge graph with chain.search_insights for existing rate and pool data.',
      'If you find relevant insights, use them and call chain.confirm_insight. Post your own findings with chain.post_insight.',
    ].join(' ');

    const betaResult = await showCmd(beta, `${ROKO} run "${betaPrompt}"`, {
      timeout: 300000,
      onLog: logCommand,
      onGate: (name, status) => setMetric('gates', `${name}: ${status}`),
      customDesc: 'Runs Beta (Risk Hedger) agent. Beta queries the knowledge graph first, finds Alpha\'s insights, confirms them (triggering the "aha" animation), then executes its hedge strategy.',
    });

    setMetric('cost', betaResult.cost ?? '$0.98');

    // ── Phase 4: Both execute strategies ────────────────────────
    if (!running.current) return;
    while (paused.current) await rawSleep(100);

    await playback.waitForStep();
    playback.setProgress(4, 6, 'executing DeFi strategies');
    timeline.setActive(3);

    logCommand(
      'strategy execution',
      'Both agents execute their DeFi strategies: Alpha splits between Aave supply and Uniswap LP, Beta hedges with Aave borrow + LP carry trade.',
    );

    // Show status from both agents
    await Promise.all([
      showCmd(alpha, `${ROKO} status`, {
        timeout: 30000,
        onLog: logCommand,
        customDesc: 'Shows Alpha agent workspace status after yield research and execution.',
      }),
      showCmd(beta, `${ROKO} status`, {
        timeout: 30000,
        customDesc: 'Shows Beta agent workspace status after hedge execution.',
      }),
    ]);

    // ── Phase 5: Cross-pollination ──────────────────────────────
    if (!running.current) return;
    while (paused.current) await rawSleep(100);

    await playback.waitForStep();
    playback.setProgress(5, 6, 'cross-pollination of insights');
    timeline.setActive(4);

    logCommand(
      'cross-pollination',
      'Alpha reviews the knowledge graph, finds Beta\'s carry trade insight, confirms it, and posts a meta-strategy comparing approaches. Knowledge compounds.',
    );

    const crossPrompt = [
      'Review the knowledge graph for new strategy insights from other agents using chain.search_insights.',
      'If you find a carry trade or hedge strategy, analyze it, confirm it with chain.confirm_insight,',
      'and post a meta-insight comparing yield strategies.',
    ].join(' ');

    await showCmd(alpha, `${ROKO} run "${crossPrompt}"`, {
      timeout: 180000,
      onLog: logCommand,
      customDesc: 'Alpha reviews the knowledge graph for Beta\'s insights. Cross-pollination: Alpha confirms Beta\'s carry trade discovery and posts a synthesis insight.',
    });

    // ── Phase 6: Results ────────────────────────────────────────
    if (!running.current) return;
    while (paused.current) await rawSleep(100);

    await playback.waitForStep();
    playback.setProgress(6, 6, 'summary');
    timeline.setActive(5);

    logCommand(
      'demo complete',
      'Chain Intelligence demo complete. Knowledge graph shows insights posted, confirmed, and reused across agents.',
    );

    // Show final learning state
    await Promise.all([
      showCmd(alpha, `${ROKO} learn all`, {
        timeout: 30000,
        onLog: logCommand,
        customDesc: 'Shows Alpha\'s learning state: episodes, efficiency, and knowledge metrics.',
      }),
      showCmd(beta, `${ROKO} learn all`, {
        timeout: 30000,
        customDesc: 'Shows Beta\'s learning state and knowledge graph statistics.',
      }),
    ]);

    // Query final knowledge graph stats
    await showCmd(alpha, 'curl -sf http://localhost:8545/api/stats | head -c 500 || echo "stats unavailable"', {
      timeout: 10000,
      onLog: logCommand,
      customDesc: 'Queries mirage knowledge graph statistics: total insights, confirmations, and reuse metrics.',
    });

    timeline.setActive(6); // all completed
  },
};

const mirage: Scenario = {
  id: 'mirage',
  title: 'Mirage',
  subtitle: 'Fork any EVM chain locally. Stream blocks in real-time with configurable block times.',
  panes: 1,
  labels: ['mirage'],
  panel: false,
  promptBar: false,
  mirageBar: true,
  steps: [],
  async run({ entries }) {
    const e = entries[0];

    // Wait for WS to connect
    const start = Date.now();
    while (Date.now() - start < 8000) {
      if (e.ws && e.ws.readyState === WebSocket.OPEN) break;
      await rawSleep(100);
    }
    await e.waitForPrompt(10000);
    e.clearTerminal();
  },
};

// ── Export ───────────────────────────────────────────────────

export const SCENARIOS: Scenario[] = [
  prdPipeline,
  selfhost,
  builder,
  race,
  providers,
  explore,
  knowledgeAccumulation,
  chat,
  knowledgeTransfer,
  chainIntelligence,
  mirage,
];

export const SCENARIO_MAP: Record<string, Scenario> = Object.fromEntries(
  SCENARIOS.map(s => [s.id, s]),
);

/** Reset shared state when switching scenarios. */
export function resetScenarioState() {
  resetRokoResolution();
}
