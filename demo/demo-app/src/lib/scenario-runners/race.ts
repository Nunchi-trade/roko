// --- src/lib/scenario-runners/race.ts ---
import type { Scenario } from '../scenarios';
import { enterWorkspace, showCmd, getRoko, trackMetrics } from '../terminal-session';

export const race: Scenario = {
  id: 'race',
  title: 'Cost Race',
  subtitle: 'Same task, two approaches. Left: naive single-model. Right: cascade-routed.',
  panes: 2,
  labels: ['naive (no replan)', 'cascade (full pipeline)'],
  panel: true,
  promptBar: false,
  category: 'comparison',
  features: ['Naive vs cascade routing', 'Cost comparison', 'Side-by-side'],
  durationHint: '~60s',
  accent: 'teal',
  icon: 'race',
  steps: [
    { label: 'Naive run', sublabel: '--no-replan' },
    { label: 'Cascade run', sublabel: 'full pipeline' },
  ],
  async run({ entries, playback, timeline, setMetric, logCommand, logCommandComplete, workspaceDir }) {
    const [left, right] = entries;

    await enterWorkspace(left, workspaceDir);
    await enterWorkspace(right, workspaceDir);

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
        onLogComplete: logCommandComplete,
        playback,
      }),
      showCmd(right, `${ROKO} run "${prompt}"`, {
        timeout: 180000,
        customDesc:
          'Runs with full pipeline: cascade router picks optimal models per-turn, gates validate, and failures trigger automatic replanning.',
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        playback,
      }),
    ]);

    clearInterval(leftTracker);
    clearInterval(rightTracker);

    timeline.markAllComplete();
  },
};
