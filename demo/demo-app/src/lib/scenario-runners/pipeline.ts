// --- src/lib/scenario-runners/pipeline.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko } from '../terminal-session';

const PIPELINE_TASK = 'Build a Rust CLI that converts temperatures between Celsius and Fahrenheit';
export const PIPELINE_COMMAND = 'roko do "Build a Rust CLI that converts temperatures between Celsius and Fahrenheit"';

export const PIPELINE_COMMANDS: CommandDef[] = [
  {
    id: 'run',
    command: PIPELINE_COMMAND,
    description: 'Run idea-to-code pipeline',
    timeout: 300000,
    target: { pane: 0 },
  },
];

// ANSI escape code stripping for buffer analysis
function stripAnsi(s: string): string {
  // eslint-disable-next-line no-control-regex
  return s.replace(/\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])/g, '');
}

/**
 * Poll a terminal's output buffer for pipeline stage keywords.
 * Returns a cleanup function.
 */
function pollPipelineStages(
  entry: { outputBuffer: string },
  ctx: ScenarioContext,
  signal?: AbortSignal,
): () => void {
  const seen = new Set<string>();

  const emit = (key: string, value: string) => {
    const tag = `${key}:${value}`;
    if (seen.has(tag)) return;
    seen.add(tag);
    ctx.setMetric(key, value);
  };

  const interval = setInterval(() => {
    const text = stripAnsi(entry.outputBuffer);

    // Stage: classify — "workflow focused" or "workflow" keyword
    if (/\bworkflow\b/i.test(text)) {
      emit('pipeline-stage', 'classify');
    }

    // Stage: plan — "prompt" line appeared (roko received the task)
    if (/\bprompt\b/i.test(text) && seen.has('pipeline-stage:classify')) {
      emit('pipeline-stage', 'plan');
    }

    // Stage: execute — "model" line appeared (agent is running)
    if (/\bmodel\s+\S+/i.test(text) && seen.has('pipeline-stage:plan')) {
      emit('pipeline-stage', 'execute');
    }

    // Stage: gate — "workflow completed" (agent finished, gates next)
    if (/workflow completed/i.test(text) && seen.has('pipeline-stage:execute')) {
      emit('pipeline-stage', 'gate');
    }

    // All done — "Summary" block appeared
    if (/Summary/i.test(text) && seen.has('pipeline-stage:gate')) {
      emit('pipeline-stage-done', '1');
    }

    // Gate results
    if (/compile.*[✔✓]|[✔✓].*compile|compile.*\bpass\b/i.test(text)) emit('pipeline-gate-pass', 'compile');
    if (/compile.*[✖✗]|[✖✗].*compile/i.test(text)) emit('pipeline-gate-fail', 'compile');
    if (/clippy.*[✔✓]|[✔✓].*clippy|clippy.*\bpass\b/i.test(text)) emit('pipeline-gate-pass', 'clippy');
    if (/clippy.*[✖✗]|[✖✗].*clippy/i.test(text)) emit('pipeline-gate-fail', 'clippy');
    if (/\btest\b.*[✔✓]|[✔✓].*\btest\b|\btest\b.*\bpass\b/i.test(text)) emit('pipeline-gate-pass', 'test');
    if (/\btest\b.*[✖✗]|[✖✗].*\btest\b/i.test(text)) emit('pipeline-gate-fail', 'test');
    if (/gates\s+\(none configured\)/i.test(text)) emit('pipeline-gates-none', '1');

    // Model detection for pipeline-model metric
    const modelMatch = text.match(/\bmodel\s+([a-zA-Z0-9][\w./-]*)/i);
    if (modelMatch) emit('pipeline-model', modelMatch[1]);
  }, 400);

  const cleanup = () => clearInterval(interval);
  signal?.addEventListener('abort', cleanup, { once: true });
  return cleanup;
}

export const pipelineScenario: ClickableScenario = {
  id: 'pipeline',
  title: 'Pipeline',
  subtitle: 'One command turns a natural-language idea into working, tested Rust code.',
  panes: 1,
  labels: ['idea → code pipeline'],
  panel: true,
  promptBar: false,
  category: 'pipeline',
  features: ['Automatic classification', 'Task planning', 'Code generation', 'Compile + lint + test gates'],
  durationHint: '<2 min',
  accent: 'rose',
  icon: 'pipeline',
  steps: [
    { label: 'Classify', sublabel: 'understand the request' },
    { label: 'Plan', sublabel: 'break into steps' },
    { label: 'Execute', sublabel: 'write Rust code' },
    { label: 'Gate', sublabel: 'compile + lint + test' },
    { label: 'Done', sublabel: 'report results' },
  ],
  commands: PIPELINE_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    if (commandId !== 'run') return { ok: false, error: 'Unknown command' };
    const [entry] = ctx.entries;
    if (!entry) return { ok: false, error: 'Terminal pane is not connected' };

    // Mark classify as active immediately on start
    ctx.setMetric('pipeline-stage', 'classify');

    // Start real-time stage polling
    const stopPolling = pollPipelineStages(entry, ctx, ctx.signal);

    const result = await showCmd(entry, roko(ctx, `do "${PIPELINE_TASK}"`), {
      timeout: 300000,
      customDesc: 'Runs the redesigned one-command idea-to-code pipeline.',
      workspaceDir: ctx.workspaceDir,
      signal: ctx.signal,
    });

    stopPolling();

    // Ensure all stages marked complete on success
    if (result.ok) {
      ctx.setMetric('pipeline-stage-done', '1');
    }

    if (result.cost) ctx.setMetric('pipeline-cost', result.cost);
    if (result.tokens) ctx.setMetric('pipeline-tokens', result.tokens);
    ctx.setMetric('pipeline-elapsed', String(result.elapsed ?? 0));
    ctx.setMetric('pipeline-calls', '1');

    // Feed sidebar stats (model/cost/tokens/time) for provenance
    if (result.model) ctx.setMetric('model', result.model);
    if (result.cost) ctx.setMetric('cost', result.cost);
    if (result.tokens) ctx.setMetric('tokens', result.tokens);
    ctx.setMetric('time', `${(result.elapsed ?? 0).toFixed(1)}s`);

    return { ok: result.ok, error: result.error };
  },
};
