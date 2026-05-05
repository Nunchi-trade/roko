// --- src/lib/scenario-runners/provider-race.ts ---
import type { ClickableScenario, CommandDef, CommandTarget, ScenarioContext } from '../../scenarios';
import { showCmd, roko, trackMetrics } from '../../terminal-session';

export const PROVIDER_RACE_COMMANDS: CommandDef[] = [
  {
    id: 'race-anthropic',
    command: 'roko run "Build a Rust CLI that converts Celsius to Fahrenheit with tests" --provider anthropic',
    description: 'Race anthropic',
    timeout: 240000,
    target: { pane: 0 } as CommandTarget,
  },
  {
    id: 'race-openai',
    command: 'roko run "Build a Rust CLI that converts Celsius to Fahrenheit with tests" --provider openai',
    description: 'Race openai',
    timeout: 240000,
    target: { pane: 1 } as CommandTarget,
  },
  {
    id: 'race-gemini',
    command: 'roko run "Build a Rust CLI that converts Celsius to Fahrenheit with tests" --provider gemini',
    description: 'Race gemini',
    timeout: 240000,
    target: { pane: 2 } as CommandTarget,
  },
  {
    id: 'race-moonshot',
    command: 'roko run "Build a Rust CLI that converts Celsius to Fahrenheit with tests" --provider moonshot',
    description: 'Race moonshot',
    timeout: 240000,
    target: { pane: 3 } as CommandTarget,
  },
  {
    id: 'summary',
    command: 'roko learn efficiency',
    description: 'Cost summary',
    timeout: 30000,
    target: 'all' as CommandTarget,
  },
];

const providerNames = ['anthropic', 'openai', 'gemini', 'moonshot'];

const state = {
  costs: [null, null, null, null] as (string | null)[],
  tokens: [null, null, null, null] as (string | null)[],
  finishOrder: [] as number[],
  results: [] as { elapsed: number; cost: string | null; tokens: string | null }[],
};

export function resetProviderRaceState() {
  state.costs = [null, null, null, null];
  state.tokens = [null, null, null, null];
  state.finishOrder = [];
  state.results = [];
}

export const providerRace: ClickableScenario = {
  id: 'provider-race',
  title: 'Provider Race',
  subtitle: '4 providers race on the same prompt. First to pass gates wins.',
  panes: 4,
  labels: ['provider 1', 'provider 2', 'provider 3', 'provider 4'],
  panel: true,
  promptBar: false,
  category: 'comparison',
  features: ['4-way provider race', 'Gate-based winner', 'Real-time progress'],
  durationHint: '~60s',
  accent: 'teal',
  icon: 'race',
  resetState: resetProviderRaceState,
  steps: [
    { label: 'Setup', sublabel: 'init workspaces' },
    { label: 'Race start', sublabel: 'dispatch all 4' },
    { label: 'Live tracking', sublabel: 'cost + gates' },
    { label: 'Winner', sublabel: 'first to pass' },
    { label: 'Cost summary', sublabel: 'compare totals' },
  ],
  commands: PROVIDER_RACE_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const { entries, setMetric, setGate } = ctx;
    const prompt = 'Build a Rust CLI that converts Celsius to Fahrenheit with tests';

    const paneIndex = providerNames.indexOf(commandId.replace('race-', ''));

    if (paneIndex >= 0) {
      const pane = entries[paneIndex];
      if (!pane) return { ok: false, error: `No pane ${paneIndex} connected` };

      // Reset state when the first race command runs
      if (state.finishOrder.length === 0 && state.costs.every(c => c === null)) {
        resetProviderRaceState();
      }

      const provider = providerNames[paneIndex];
      setGate(provider, 'pending');

      const tracker = trackMetrics(pane, {
        onCost: cost => {
          state.costs[paneIndex] = cost;
          setMetric(
            'cost',
            state.costs.map((c, i) => `${providerNames[i]}:${c ?? 'pending'}`).join(' | '),
          );
        },
        onTokens: value => {
          state.tokens[paneIndex] = value;
          setMetric(
            'tokens',
            state.tokens.map((t, i) => `${providerNames[i]}:${t ?? 'pending'}`).join(' | '),
          );
        },
        onGate: (name, status) => {
          setGate(`${provider}:${name}`, status);
          if (status === 'pass' && !state.finishOrder.includes(paneIndex)) {
            state.finishOrder.push(paneIndex);
          }
        },
      });

      try {
        const result = await showCmd(
          pane,
          roko(ctx, `run "${prompt}" --provider ${provider}`),
          {
            timeout: 240000,
            customDesc: `Race ${provider}`,
            workspaceDir: ctx.workspaceDir,
            signal: ctx.signal,
            onGate: (name, status) => {
              setGate(`${provider}:${name}`, status);
              if (status === 'pass' && !state.finishOrder.includes(paneIndex)) {
                state.finishOrder.push(paneIndex);
              }
            },
            onCost: cost => {
              state.costs[paneIndex] = cost;
            },
            onTokens: value => {
              state.tokens[paneIndex] = value;
            },
          },
        );

        state.results[paneIndex] = {
          elapsed: result.elapsed,
          cost: result.cost ?? state.costs[paneIndex],
          tokens: result.tokens ?? state.tokens[paneIndex],
        };

        const finalStatus = result.gates.some(g => g.status === 'fail')
          ? 'fail'
          : result.gates.some(g => g.status === 'pass')
            ? 'pass'
            : result.ok
              ? 'pass'
              : 'fail';
        setGate(provider, finalStatus);

        if (finalStatus === 'pass' && !state.finishOrder.includes(paneIndex)) {
          state.finishOrder.push(paneIndex);
        }

        // Update winner metric if this is the first finisher
        if (state.finishOrder[0] === paneIndex) {
          setMetric('model', `winner: ${provider}`);
        }

        return { ok: result.ok, error: result.error };
      } finally {
        clearInterval(tracker);
      }
    }

    if (commandId === 'summary') {
      const winnerIndex = state.finishOrder[0];
      const winnerName = winnerIndex !== undefined ? providerNames[winnerIndex] : 'unknown';
      const winnerResult = winnerIndex !== undefined ? state.results[winnerIndex] : undefined;

      if (winnerResult) {
        setMetric(
          'model',
          `winner: ${winnerName} (${winnerResult.elapsed.toFixed(1)}s, ${winnerResult.cost ?? '?'})`,
        );
      } else {
        setMetric('model', `winner: ${winnerName}`);
      }

      const panes = entries.filter(Boolean);
      if (panes.length === 0) return { ok: false, error: 'No panes connected' };

      const results = await Promise.all(
        panes.map(pane =>
          showCmd(pane, roko(ctx, 'learn efficiency'), {
            timeout: 30000,
            customDesc: 'Cost summary across all providers.',
            workspaceDir: ctx.workspaceDir,
            signal: ctx.signal,
          }),
        ),
      );

      const failed = results.find(r => !r.ok);
      return failed ? { ok: false, error: failed.error } : { ok: true };
    }

    return { ok: false, error: 'Unknown command' };
  },
};
