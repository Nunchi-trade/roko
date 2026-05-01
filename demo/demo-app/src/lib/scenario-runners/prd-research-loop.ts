// --- src/lib/scenario-runners/prd-research-loop.ts ---
import type { Scenario } from '../scenarios';
import { enterWorkspace, showCmd, roko, trackMetrics } from '../terminal-session';

export const prdResearchLoop: Scenario = {
  id: 'prd-research-loop',
  title: 'Research Loop',
  subtitle: 'Full pipeline: idea, draft, research, plan, execute, gates, learn.',
  panes: 1,
  labels: ['full pipeline'],
  panel: true,
  promptBar: false,
  category: 'pipeline',
  features: ['Full PRD lifecycle', 'Research enhancement', '7 gates'],
  durationHint: '~90s',
  accent: 'rose',
  icon: 'pipeline',
  steps: [
    { label: 'Capture idea', sublabel: 'prd idea' },
    { label: 'Draft PRD', sublabel: 'prd draft new' },
    { label: 'Research enhance', sublabel: 'research enhance-prd' },
    { label: 'Generate plan', sublabel: 'prd plan' },
    { label: 'Execute plan', sublabel: 'plan run' },
    { label: 'Gate results', sublabel: 'compile + test + clippy' },
    { label: 'Learn', sublabel: 'learn all' },
    { label: 'Summary', sublabel: 'status + efficiency' },
  ],
  async run(ctx) {
    const { entries, playback, timeline, setMetric, setGate, logCommand, logCommandComplete, workspaceDir } = ctx;
    const e = entries[0];
    await enterWorkspace(e, workspaceDir);
    setMetric('model', '--');
    timeline.init(this.steps);

    // Live metric tracking from terminal output
    const tracker = trackMetrics(e, {
      onCost: (c) => setMetric('cost', c),
      onTokens: (t) => setMetric('tokens', t),
    }, 250);
    const stopTracking = () => clearInterval(tracker);

    try {

    // Phase 1: capture idea
    await playback.waitForStep();
    playback.setProgress(1, 8, roko(ctx, 'prd idea "..."'));
    timeline.setActive(0);
    const ideaResult = await showCmd(e, roko(ctx, 'prd idea "Add config validation with schema checking and helpful error messages"'), {
      playback,
      timeout: 45000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Captures a raw work item into the PRD backlog. This is the seed for the full pipeline.',
    });
    if (ideaResult.cost) setMetric('cost', ideaResult.cost);
    if (ideaResult.tokens) setMetric('tokens', ideaResult.tokens);

    // Phase 2: draft PRD
    await playback.waitForStep();
    playback.setProgress(2, 8, roko(ctx, 'prd draft new ...'));
    timeline.setActive(1);
    const draftResult = await showCmd(e, roko(ctx, 'prd draft new cli-config-validation'), {
      playback,
      timeout: 120000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Agent expands the idea into a structured PRD with motivation, design, tasks, and success criteria.',
    });
    if (draftResult.cost) setMetric('cost', draftResult.cost);
    if (draftResult.tokens) setMetric('tokens', draftResult.tokens);

    // Phase 3: research enhance -- the new step
    await playback.waitForStep();
    playback.setProgress(3, 8, roko(ctx, 'research enhance-prd cli-config-validation'));
    timeline.setActive(2);
    logCommand(
      'research enhance-prd',
      'Enriching the PRD with research: prior art, implementation references, and architectural context. This step makes the generated plan more informed.',
    );
    const researchResult = await showCmd(e, roko(ctx, 'research enhance-prd cli-config-validation'), {
      playback,
      timeout: 180000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc:
        'Research agent searches for relevant prior art, patterns, and references, then weaves findings into the PRD. The subsequent plan generation benefits from this enriched context.',
    });
    if (researchResult.cost) setMetric('cost', researchResult.cost);
    if (researchResult.tokens) setMetric('tokens', researchResult.tokens);

    // Phase 4: generate plan (now informed by research)
    await playback.waitForStep();
    playback.setProgress(4, 8, roko(ctx, 'prd plan cli-config-validation'));
    timeline.setActive(3);
    const planResult = await showCmd(e, roko(ctx, 'prd plan cli-config-validation'), {
      playback,
      timeout: 180000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Generates tasks.toml from the research-enhanced PRD. The plan quality is higher because the PRD now contains prior art and implementation references.',
    });
    if (planResult.cost) setMetric('cost', planResult.cost);
    if (planResult.tokens) setMetric('tokens', planResult.tokens);

    // Phase 5: execute plan
    await playback.waitForStep();
    playback.setProgress(5, 8, roko(ctx, 'plan run .roko/plans --max-retries 1'));
    timeline.setActive(4);
    setGate('compile', 'pending');
    setGate('test', 'pending');
    setGate('clippy', 'pending');
    const runResult = await showCmd(e, roko(ctx, 'plan run .roko/plans --max-retries 1'), {
      playback,
      timeout: 300000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      onGate: (name, status) => setGate(name, status),
      customDesc: 'Executes the generated plan through the Roko runner. Agents implement tasks, gates validate each one.',
    });
    if (runResult.cost) setMetric('cost', runResult.cost);

    // Phase 6: gate results
    timeline.setActive(5);
    playback.setProgress(6, 8, 'gate results');
    if (runResult.gates.length > 0) {
      logCommand(
        'gates',
        runResult.gates.map(gate => `${gate.name}: ${gate.status}`).join(', '),
      );
    } else if (runResult.ok) {
      logCommand('gates', 'Run completed, but no gate verdicts were detected in the output.');
    } else {
      logCommand('gates', 'Plan run failed before gate verdicts were fully reported.');
    }
    if (runResult.tokens) setMetric('tokens', runResult.tokens);

    // Phase 7: learn -- show what the system learned
    await playback.waitForStep();
    playback.setProgress(7, 8, roko(ctx, 'learn all'));
    timeline.setActive(6);
    e.clearTerminal();
    await showCmd(e, roko(ctx, 'learn all'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Full learning state: cascade router weights, prompt experiments, adaptive gate thresholds, and efficiency metrics.',
    });
    const tuneResult = await showCmd(e, roko(ctx, 'learn tune routing'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Cascade router tuning: shows model confidence scores and routing decisions based on this execution.',
    });
    if (tuneResult.cost) setMetric('cost', tuneResult.cost);

    // Phase 8: summary -- status + efficiency
    await playback.waitForStep();
    playback.setProgress(8, 8, roko(ctx, 'status'));
    timeline.setActive(7);
    e.clearTerminal();
    await showCmd(e, roko(ctx, 'status'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Workspace status: signal counts, episode count, and overall health.',
    });
    await showCmd(e, roko(ctx, 'learn efficiency'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Per-turn efficiency events: tokens used, cost, latency, and model selection decisions across all steps of the pipeline.',
    });
    setMetric('model', 'done');

    timeline.markAllComplete();

    } finally {
      stopTracking();
    }
  },
};
