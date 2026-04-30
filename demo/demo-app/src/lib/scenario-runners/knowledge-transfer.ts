// --- src/lib/scenario-runners/knowledge-transfer.ts ---
import type { Scenario } from '../scenarios';
import { enterWorkspace, showCmd, getRoko, trackMetrics } from '../terminal-session';

export const knowledgeTransfer: Scenario = {
  id: 'knowledge-transfer',
  title: 'Knowledge Transfer',
  subtitle: 'Two agents build similar APIs. The second one learns from the first.',
  panes: 2,
  labels: ['Agent Alpha (cold start)', 'Agent Beta (with knowledge)'],
  panel: true,
  promptBar: false,
  category: 'learning',
  features: ['Cross-agent learning', 'Cold vs warm start', 'Knowledge reuse'],
  durationHint: '~90s',
  accent: 'emerald',
  icon: 'transfer',
  steps: [
    { label: 'Setup workspaces', sublabel: 'roko init x2' },
    { label: 'Alpha builds User API', sublabel: 'roko run (cold)' },
    { label: 'Distill knowledge', sublabel: 'episodes → insights' },
    { label: 'Beta builds Inventory API', sublabel: 'roko run (warm)' },
    { label: 'Compare results', sublabel: 'efficiency metrics' },
  ],
  async run(ctx) {
    const { entries, playback, timeline, setMetric, setGate, logCommand, logCommandComplete } = ctx;
    const [alpha, beta] = entries;
    const ROKO = getRoko();
    timeline.init(this.steps);

    // -- Phase 1: Setup workspaces --
    timeline.setActive(0);
    playback.setProgress(0, 5, 'Setting up workspaces');

    const dirA = ctx.workspaceDir;
    const dirB = await ctx.createWorkspace('roko-inventory-api');
    await Promise.all([
      enterWorkspace(alpha, dirA),
      enterWorkspace(beta, dirB),
    ]);

    // Beta shows waiting state while Alpha builds
    await beta.execCmd('echo "Waiting for Agent Alpha to finish..."', 5000);

    // -- Phase 2: Alpha builds User API (cold start) --
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
        playback,
        timeout: 300000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        onGate: setGate,
        customDesc: 'Alpha agent starts from scratch. No prior knowledge — discovers patterns through exploration.',
      },
    );

    clearInterval(alphaTracker);
    setMetric('cost', alphaResult.cost ?? '$?.??');
    setMetric('time', `${alphaResult.elapsed.toFixed(0)}s`);

    // -- Phase 3: Distill knowledge from Alpha --
    await playback.waitForStep();
    timeline.setActive(2);
    playback.setProgress(2, 5, 'Distilling knowledge from Alpha');

    await showCmd(alpha, `${ROKO} learn all`, {
      playback,
      timeout: 60000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Inspects episodes, router decisions, and efficiency metrics. The distiller extracts reusable insights.',
    });
    await showCmd(alpha, `${ROKO} knowledge stats`, {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Shows what knowledge entries were extracted — heuristics, strategies, and warnings.',
    });

    // -- Phase 4: Beta builds Inventory API (with knowledge) --
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
        playback,
        timeout: 300000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        onGate: setGate,
        customDesc: 'Beta agent starts with knowledge from Alpha. Skips exploration, uses proven patterns immediately.',
      },
    );

    clearInterval(betaTracker);

    // -- Phase 5: Compare results --
    await playback.waitForStep();
    timeline.setActive(4);
    playback.setProgress(4, 5, 'Comparing results');

    await showCmd(beta, `${ROKO} learn efficiency`, {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Shows efficiency comparison — cost, turns, and time savings from knowledge transfer.',
    });

    timeline.markAllComplete();
  },
};
