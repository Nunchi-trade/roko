// --- src/lib/scenario-runners/explore.ts ---
import type { Scenario } from '../scenarios';
import { enterWorkspace, showCmd, roko } from '../terminal-session';

export const explore: Scenario = {
  id: 'explore',
  title: 'Explore',
  subtitle: '18 crates, 85 routes, 100+ commands. Four capability families at once.',
  panes: 4,
  labels: ['workspace', 'learning', 'config', 'knowledge'],
  panel: true,
  promptBar: false,
  category: 'exploration',
  features: ['18 crates', '85 routes', '100+ commands'],
  durationHint: '~120s',
  accent: 'violet',
  icon: 'explore',
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
  async run(ctx) {
    const { entries, playback, timeline, logCommand, logCommandComplete, signal, workspaceDir } = ctx;
    await enterWorkspace(entries[0], workspaceDir);
    await Promise.all(entries.slice(1).map(e => enterWorkspace(e, workspaceDir)));

    timeline.init(this.steps);

    const families = [
      [roko(ctx, 'status'), roko(ctx, 'doctor'), roko(ctx, 'prd list')],
      [roko(ctx, 'learn all'), roko(ctx, 'learn efficiency'), roko(ctx, 'learn tune gates')],
      [
        roko(ctx, 'config providers list'),
        roko(ctx, 'config models list'),
        roko(ctx, 'config validate'),
      ],
      [
        roko(ctx, 'knowledge stats'),
        roko(ctx, 'knowledge query "routing"'),
        roko(ctx, 'explain "cascade routing"'),
      ],
    ];

    await Promise.all(
      entries.map(async (e, i) => {
        for (let j = 0; j < families[i].length; j++) {
          if (signal.aborted) return;
          const globalStep = i * 3 + j;
          timeline.setActive(globalStep);
          playback.setProgress(globalStep + 1, 12, families[i][j]);
          await showCmd(e, families[i][j], { timeout: 45000, onLog: logCommand, onLogComplete: logCommandComplete, playback, signal });
        }
      }),
    );

    timeline.markAllComplete();
  },
};
