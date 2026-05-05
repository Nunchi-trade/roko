// --- src/lib/scenario-runners/prd-pipeline.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../../scenarios';
import { showCmd, roko, getRoko } from '../../terminal-session';
import { fetchWorkflowSnapshot } from '../../workflow-api';

// ── PRD pipeline idea ────────────────────────────────────────

export const PRD_IDEA =
  'Build a CLI that fetches BTC funding rates from Binance, calculates average funding over 7 days, and alerts when funding exceeds 0.1%';

// ── Command templates (single source of truth) ──────────────

interface CommandTemplate {
  id: string;
  subcommand: string;
  display: string;
  description: string;
  timeout: number;
  needsModel: boolean;
}

const TEMPLATES: CommandTemplate[] = [
  { id: 'init',     subcommand: 'init',                                         display: 'roko init',                                                  description: 'Create workspace and config',    timeout: 10000,  needsModel: false },
  { id: 'idea',     subcommand: `prd idea "${PRD_IDEA}"`,                        display: `roko prd idea "${PRD_IDEA}"`,                                  description: 'Capture work item',              timeout: 10000,  needsModel: false },
  { id: 'draft',    subcommand: 'prd draft new "BTC Funding Alert CLI"',        display: 'roko prd draft new "BTC Funding Alert CLI"',                 description: 'Generate PRD via LLM',           timeout: 600000, needsModel: true  },
  { id: 'promote',  subcommand: 'prd draft promote btc-funding-alert-cli',      display: 'roko prd draft promote btc-funding-alert-cli',               description: 'Promote to published',           timeout: 10000,  needsModel: false },
  { id: 'plan',     subcommand: 'prd plan btc-funding-alert-cli',              display: 'roko prd plan btc-funding-alert-cli',                        description: 'Generate implementation plan',   timeout: 600000, needsModel: true  },
  { id: 'validate', subcommand: 'plan validate .roko/plans',                    display: 'roko plan validate .roko/plans',                             description: 'Lint the generated plan',        timeout: 10000,  needsModel: false },
  { id: 'run',      subcommand: 'plan run .roko/plans --max-retries 1',         display: 'roko plan run .roko/plans --max-retries 1',                  description: 'Execute: agents + gates',        timeout: 600000, needsModel: true  },
  { id: 'status',   subcommand: 'status',                                       display: 'roko status',                                                description: 'View results and costs',         timeout: 10000,  needsModel: false },
];

// ── Derived static commands (display layer, no ctx needed) ───

export const PRD_PIPELINE_COMMANDS: CommandDef[] = TEMPLATES.map(t => ({
  id: t.id,
  command: t.display,
  description: t.description,
  timeout: t.timeout,
}));

// ── Runtime command helpers ───────────────────────────────────

function runtimeCommand(ctx: ScenarioContext, tmpl: CommandTemplate): string {
  if (tmpl.needsModel) {
    return roko(ctx, tmpl.subcommand);
  }
  return `${getRoko()} ${tmpl.subcommand}`;
}

export function getRuntimeCmd(ctx: ScenarioContext, commandId: string): { command: string; description: string; timeout: number } | undefined {
  const tmpl = TEMPLATES.find(t => t.id === commandId);
  if (!tmpl) return undefined;
  return {
    command: runtimeCommand(ctx, tmpl),
    description: tmpl.description,
    timeout: tmpl.timeout,
  };
}

// ── Scenario ─────────────────────────────────────────────────

export const prdPipeline: ClickableScenario = {
  id: 'prd-pipeline',
  title: 'PRD Pipeline',
  subtitle: 'Click each command to walk through the full development pipeline',
  panes: 1,
  labels: ['Terminal'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'Init', sublabel: 'roko init' },
    { label: 'Capture idea', sublabel: 'prd idea' },
    { label: 'Generate PRD', sublabel: 'prd draft new' },
    { label: 'Promote', sublabel: 'draft promote' },
    { label: 'Generate plan', sublabel: 'prd plan' },
    { label: 'Validate', sublabel: 'plan validate' },
    { label: 'Execute plan', sublabel: 'plan run' },
    { label: 'Status', sublabel: 'results' },
  ],
  category: 'pipeline',
  features: ['PRD generation', 'Task planning', 'Gate validation'],
  durationHint: '2-5 min',
  accent: 'rose',
  icon: 'pipeline',
  commands: PRD_PIPELINE_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const resolved = getRuntimeCmd(ctx, commandId);
    if (!resolved) return { ok: false, error: 'Unknown command' };

    const [main] = ctx.entries;
    if (!main) return { ok: false, error: 'No terminal connected' };

    const result = await showCmd(main, resolved.command, {
      timeout: resolved.timeout,
      customDesc: resolved.description,
      workspaceDir: ctx.workspaceDir,
      signal: ctx.signal,
    });

    // Refresh workflow snapshot for commands that mutate state
    if (['draft', 'promote', 'plan', 'run'].includes(commandId)) {
      try {
        const snapshot = await fetchWorkflowSnapshot(ctx.workspaceDir);
        if (snapshot?.prd) {
          ctx.setMetric('prd-title', snapshot.prd.title ?? '');
        }
      } catch {
        // non-fatal
      }
    }

    return { ok: result.ok, error: result.error };
  },
};
