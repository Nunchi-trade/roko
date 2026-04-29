// --- src/lib/scenario-runners/explore.ts ---
import type { Scenario } from '../scenarios';
import { rawSleep } from '../scenario-helpers';
import { enterWorkspace, showCmd, getRoko } from '../terminal-session';

export const explore: Scenario = {
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
  async run({ entries, playback, timeline, logCommand, paused, running, workspaceDir }) {
    await enterWorkspace(entries[0], workspaceDir);
    await Promise.all(entries.slice(1).map(e => enterWorkspace(e, workspaceDir)));

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
