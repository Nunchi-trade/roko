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
  chat,
  mirage,
];

export const SCENARIO_MAP: Record<string, Scenario> = Object.fromEntries(
  SCENARIOS.map(s => [s.id, s]),
);

/** Reset shared state when switching scenarios. */
export function resetScenarioState() {
  resetRokoResolution();
}
