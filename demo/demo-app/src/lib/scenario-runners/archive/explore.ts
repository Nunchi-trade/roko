// --- src/lib/scenario-runners/explore.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../../scenarios';
import { showCmd, roko } from '../../terminal-session';

export const EXPLORE_COMMANDS: CommandDef[] = [
  { id: 'status',           command: 'roko status',                   description: 'Workspace status',        timeout: 45000, target: { pane: 0 } },
  { id: 'doctor',           command: 'roko doctor',                   description: 'Workspace diagnostics',   timeout: 45000, target: { pane: 0 } },
  { id: 'prd-list',         command: 'roko prd list',                 description: 'List PRDs',               timeout: 45000, target: { pane: 0 } },
  { id: 'learn-all',        command: 'roko learn all',                description: 'All learning state',      timeout: 45000, target: { pane: 1 } },
  { id: 'learn-efficiency', command: 'roko learn efficiency',         description: 'Efficiency metrics',      timeout: 45000, target: { pane: 1 } },
  { id: 'learn-tune',       command: 'roko learn tune gates',         description: 'Tune gate thresholds',    timeout: 45000, target: { pane: 1 } },
  { id: 'config-providers', command: 'roko config providers list',    description: 'List LLM providers',      timeout: 45000, target: { pane: 2 } },
  { id: 'config-models',    command: 'roko config models list',       description: 'List models',             timeout: 45000, target: { pane: 2 } },
  { id: 'config-validate',  command: 'roko config validate',          description: 'Validate config',         timeout: 45000, target: { pane: 2 } },
  { id: 'knowledge-stats',  command: 'roko knowledge stats',          description: 'Knowledge store stats',   timeout: 45000, target: { pane: 3 } },
  { id: 'knowledge-query',  command: 'roko knowledge query "routing"', description: 'Query knowledge store', timeout: 45000, target: { pane: 3 } },
  { id: 'explain',          command: 'roko explain "cascade routing"', description: 'Explain concept',        timeout: 45000, target: { pane: 3 } },
];

// pane index → capability family label
const PANE_LABELS = ['workspace', 'learning', 'config', 'knowledge'];

export const explore: ClickableScenario = {
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
  commands: EXPLORE_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const { entries, logCommand, logCommandComplete, signal } = ctx;

    const def = EXPLORE_COMMANDS.find(c => c.id === commandId);
    if (!def) return { ok: false, error: 'Unknown command' };

    const paneIndex = def.target && typeof def.target === 'object' && 'pane' in def.target
      ? def.target.pane
      : 0;
    const entry = entries[paneIndex];
    if (!entry) return { ok: false, error: `No pane ${paneIndex} connected (${PANE_LABELS[paneIndex]})` };

    const result = await showCmd(entry, roko(ctx, def.command.replace(/^roko\s+/, '')), {
      timeout: def.timeout,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      signal,
    });

    return { ok: result.ok, error: result.error };
  },
};
