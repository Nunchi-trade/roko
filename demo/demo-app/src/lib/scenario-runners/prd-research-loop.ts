// --- src/lib/scenario-runners/prd-research-loop.ts ---
import type { Scenario } from '../scenarios';
import { enterWorkspace, showCmd, getRoko } from '../terminal-session';

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
  async run({ entries, playback, timeline, setMetric, setGate, logCommand, logCommandComplete, workspaceDir }) {
    const e = entries[0];
    await enterWorkspace(e, workspaceDir);
    const ROKO = getRoko();
    setMetric('model', 'cascade');
    timeline.init(this.steps);

    // Phase 1: capture idea
    await playback.waitForStep();
    playback.setProgress(1, 8, `${ROKO} prd idea "..."`);
    timeline.setActive(0);
    await showCmd(e, `${ROKO} prd idea "Add config validation with schema checking and helpful error messages"`, {
      playback,
      timeout: 45000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Captures a raw work item into the PRD backlog. This is the seed for the full pipeline.',
    });
    setMetric('cost', '$0.02');
    setMetric('tokens', '1.2k');

    // Phase 2: draft PRD
    await playback.waitForStep();
    playback.setProgress(2, 8, `${ROKO} prd draft new ...`);
    timeline.setActive(1);
    await showCmd(e, `${ROKO} prd draft new cli-config-validation`, {
      playback,
      timeout: 120000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Agent expands the idea into a structured PRD with motivation, design, tasks, and success criteria.',
    });
    setMetric('cost', '$0.08');
    setMetric('tokens', '3.8k');

    // Phase 3: research enhance -- the new step
    await playback.waitForStep();
    playback.setProgress(3, 8, `${ROKO} research enhance-prd cli-config-validation`);
    timeline.setActive(2);
    logCommand(
      'research enhance-prd',
      'Enriching the PRD with research: prior art, implementation references, and architectural context. This step makes the generated plan more informed.',
    );
    await showCmd(e, `${ROKO} research enhance-prd cli-config-validation`, {
      playback,
      timeout: 180000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc:
        'Research agent searches for relevant prior art, patterns, and references, then weaves findings into the PRD. The subsequent plan generation benefits from this enriched context.',
    });
    setMetric('cost', '$0.18');
    setMetric('tokens', '8.5k');

    // Phase 4: generate plan (now informed by research)
    await playback.waitForStep();
    playback.setProgress(4, 8, `${ROKO} prd plan cli-config-validation`);
    timeline.setActive(3);
    await showCmd(e, `${ROKO} prd plan cli-config-validation`, {
      playback,
      timeout: 180000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Generates tasks.toml from the research-enhanced PRD. The plan quality is higher because the PRD now contains prior art and implementation references.',
    });
    setMetric('cost', '$0.28');
    setMetric('tokens', '12k');

    // Phase 5: execute plan
    await playback.waitForStep();
    playback.setProgress(5, 8, `${ROKO} plan run .roko/plans --max-retries 1`);
    timeline.setActive(4);
    setGate('compile', 'pending');
    setGate('test', 'pending');
    setGate('clippy', 'pending');
    const runResult = await showCmd(e, `${ROKO} plan run .roko/plans --max-retries 1`, {
      playback,
      timeout: 300000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      onGate: (name, status) => setGate(name, status),
      customDesc: 'Executes the generated plan through the Roko runner. Agents implement tasks, gates validate each one.',
    });
    setMetric('cost', '$0.52');

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
    setMetric('tokens', `${runResult.elapsed.toFixed(0)}s elapsed`);

    // Phase 7: learn -- show what the system learned
    await playback.waitForStep();
    playback.setProgress(7, 8, `${ROKO} learn all`);
    timeline.setActive(6);
    e.clearTerminal();
    await showCmd(e, `${ROKO} learn all`, {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Full learning state: cascade router weights, prompt experiments, adaptive gate thresholds, and efficiency metrics.',
    });
    await showCmd(e, `${ROKO} learn tune routing`, {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Cascade router tuning: shows model confidence scores and routing decisions based on this execution.',
    });
    setMetric('cost', '$0.53');

    // Phase 8: summary -- status + efficiency
    await playback.waitForStep();
    playback.setProgress(8, 8, `${ROKO} status`);
    timeline.setActive(7);
    e.clearTerminal();
    await showCmd(e, `${ROKO} status`, {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Workspace status: signal counts, episode count, and overall health.',
    });
    await showCmd(e, `${ROKO} learn efficiency`, {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Per-turn efficiency events: tokens used, cost, latency, and model selection decisions across all steps of the pipeline.',
    });
    setMetric('model', 'loop complete');

    timeline.setActive(8); // all completed
  },
};
