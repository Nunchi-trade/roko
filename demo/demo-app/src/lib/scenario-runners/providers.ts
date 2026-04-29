// --- src/lib/scenario-runners/providers.ts ---
import type { Scenario } from '../scenarios';
import { stripAnsi } from '../scenario-helpers';
import { enterWorkspace, showCmd, getRoko } from '../terminal-session';

export const providers: Scenario = {
  id: 'providers',
  title: 'Providers',
  subtitle: 'One prompt, four providers, simultaneously. Provider-agnostic by design.',
  panes: 4,
  labels: ['zhipu (glm-4)', 'openai (gpt-5.4-mini)', 'anthropic (haiku)', 'moonshot (v1)'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'Zhipu GLM-4', sublabel: 'dispatch' },
    { label: 'OpenAI GPT-5.4-Mini', sublabel: 'dispatch' },
    { label: 'Anthropic Haiku', sublabel: 'dispatch' },
    { label: 'Moonshot v1', sublabel: 'dispatch' },
  ],
  async run({ entries, playback, timeline, logCommand, workspaceDir }) {
    const providerNames = ['zhipu', 'openai', 'anthropic', 'moonshot'];

    await enterWorkspace(entries[0], workspaceDir);
    await Promise.all(entries.slice(1).map(e => enterWorkspace(e, workspaceDir)));

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
