// --- src/lib/scenario-runners/provider-race.ts ---
import type { Scenario } from '../scenarios';
import { enterWorkspace, showCmd, getRoko, trackMetrics } from '../terminal-session';

export const providerRace: Scenario = {
  id: 'provider-race',
  title: 'Provider Race',
  subtitle: '4 providers race on the same prompt. First to pass gates wins.',
  panes: 4,
  labels: ['anthropic (haiku)', 'openai (gpt-5.4-mini)', 'gemini (flash)', 'moonshot (v1)'],
  panel: true,
  promptBar: false,
  category: 'comparison',
  features: ['4-way provider race', 'Gate-based winner', 'Real-time progress'],
  durationHint: '~60s',
  accent: 'teal',
  icon: 'race',
  steps: [
    { label: 'Setup', sublabel: 'init workspaces' },
    { label: 'Race start', sublabel: 'dispatch all 4' },
    { label: 'Live tracking', sublabel: 'cost + gates' },
    { label: 'Winner', sublabel: 'first to pass' },
    { label: 'Cost summary', sublabel: 'compare totals' },
  ],
  async run({ entries, playback, timeline, setMetric, setGate, logCommand, logCommandComplete, workspaceDir }) {
    const providerNames = ['anthropic', 'openai', 'gemini', 'moonshot'];
    const providerModels = ['haiku', 'gpt-5.4-mini', 'flash', 'v1'];
    const providerLabels = ['anthropic (haiku)', 'openai (gpt-5.4-mini)', 'gemini (flash)', 'moonshot (v1)'];
    const costs: (string | null)[] = [null, null, null, null];
    const tokens: (string | null)[] = [null, null, null, null];
    const finishOrder: number[] = [];

    const renderCostSummary = () =>
      costs.map((cost, i) => `${providerNames[i]}:${cost ?? 'pending'}`).join(' | ');
    const renderTokenSummary = () =>
      tokens.map((value, i) => `${providerNames[i]}:${value ?? 'pending'}`).join(' | ');
    const refreshMetrics = () => {
      setMetric('cost', renderCostSummary());
      setMetric('tokens', renderTokenSummary());
    };
    const updateCost = (index: number, cost: string) => {
      costs[index] = cost;
      refreshMetrics();
    };
    const updateTokens = (index: number, value: string) => {
      tokens[index] = value;
      refreshMetrics();
    };
    const markGate = (index: number, name: string, status: 'pass' | 'fail') => {
      setGate(`${providerNames[index]}:${name}`, status);
      if (status === 'pass' && !finishOrder.includes(index)) {
        finishOrder.push(index);
      }
    };

    timeline.init(this.steps);
    timeline.setActive(0);
    playback.setProgress(0, 5, 'initializing workspaces...');

    await enterWorkspace(entries[0], workspaceDir);
    await Promise.all(entries.slice(1).map(e => enterWorkspace(e, workspaceDir)));

    const ROKO = getRoko();
    setMetric('model', 'provider race');
    refreshMetrics();
    logCommand('setup', `Workspace initialized for ${providerLabels.join(', ')}.`);

    await playback.waitForStep();
    timeline.setActive(1);
    playback.setProgress(1, 5, 'dispatching to all providers...');
    logCommand(
      'race start',
      `Dispatching "Build a Rust CLI that converts Celsius to Fahrenheit with tests" to ${providerLabels.join(', ')} simultaneously.`,
    );

    entries.forEach((_, index) => {
      setGate(providerNames[index], 'pending');
    });

    const trackers = entries.map((handle, index) =>
      trackMetrics(
        handle,
        {
          onCost: cost => updateCost(index, cost),
          onTokens: value => updateTokens(index, value),
          onGate: (name, status) => markGate(index, name, status),
        },
        250,
      ),
    );

    type RaceResult = {
      provider: string;
      model: string;
      label: string;
      ok: boolean;
      elapsed: number;
      gates: { name: string; status: 'pass' | 'fail' }[];
      cost: string | null;
      tokens: string | null;
    };

    const prompt = 'Build a Rust CLI that converts Celsius to Fahrenheit with tests';
    const racePromise = Promise.all(
      entries.map(async (handle, index): Promise<RaceResult> => {
        const result = await showCmd(handle, `${ROKO} run "${prompt}" --provider ${providerNames[index]}`, {
          playback,
          timeout: 240000,
          customDesc: `Racing ${providerLabels[index]} against the field.`,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          onGate: (name, status) => markGate(index, name, status),
          onCost: cost => updateCost(index, cost),
          onTokens: value => updateTokens(index, value),
        });

        if (result.gates.some(gate => gate.status === 'pass') && !finishOrder.includes(index)) {
          finishOrder.push(index);
        }

        const finalStatus = result.gates.some(gate => gate.status === 'fail')
          ? 'fail'
          : result.gates.some(gate => gate.status === 'pass')
            ? 'pass'
            : 'fail';
        setGate(providerNames[index], finalStatus);

        return {
          provider: providerNames[index],
          model: providerModels[index],
          label: providerLabels[index],
          ...result,
        };
      }),
    );

    timeline.setActive(2);
    playback.setProgress(2, 5, 'tracking live output...');
    await playback.waitForStep();

    let raceResults: RaceResult[] = [];
    try {
      raceResults = await racePromise;
    } finally {
      trackers.forEach(clearInterval);
    }

    const winnerIndex = finishOrder[0] ?? 0;
    const winner = raceResults[winnerIndex];

    timeline.setActive(3);
    playback.setProgress(3, 5, 'determining winner...');
    setMetric('model', `winner: ${winner.provider}`);
    logCommand(
      'winner',
      `${winner.provider} (${winner.model}) finished first in ${winner.elapsed.toFixed(1)}s — cost: ${winner.cost ?? 'unknown'}`,
    );

    await playback.waitForStep();
    timeline.setActive(4);
    playback.setProgress(4, 5, 'summarizing costs...');

    const summary = raceResults
      .map(r => `${r.provider}: ${r.cost ?? '?'} (${r.elapsed.toFixed(1)}s)`)
      .join(' | ');
    setMetric('cost', summary);
    setMetric(
      'tokens',
      raceResults.map(r => `${r.provider}:${r.tokens ?? 'pending'}`).join(' | '),
    );
    logCommand('summary', `Race results: ${summary}`);

    timeline.setActive(5);
  },
};
