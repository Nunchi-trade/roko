// --- src/lib/scenario-runners/prd-pipeline.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko } from '../terminal-session';
import { fetchWorkflowSnapshot } from '../workflow-api';

// ── PRD pipeline idea ────────────────────────────────────────

const PRD_IDEA =
  'Build a CLI that fetches BTC funding rates from Binance, calculates average funding over 7 days, and alerts when funding exceeds 0.1%';

// ── Static command definitions (display layer, no ctx needed) ─

export const PRD_PIPELINE_COMMANDS: CommandDef[] = [
  { id: 'init',     command: 'roko init',                                                  description: 'Create workspace and config',    timeout: 10000  },
  { id: 'idea',     command: `roko prd idea "..."`,                                        description: 'Capture work item',              timeout: 10000  },
  { id: 'draft',    command: 'roko prd draft new "BTC Funding Alert CLI"',                 description: 'Generate PRD via LLM',           timeout: 180000 },
  { id: 'promote',  command: 'roko prd draft promote btc-funding-alert-cli',               description: 'Promote to published',           timeout: 10000  },
  { id: 'plan',     command: 'roko prd plan btc-funding-alert-cli',                        description: 'Generate implementation plan',   timeout: 300000 },
  { id: 'validate', command: 'roko plan validate .roko/plans',                             description: 'Lint the generated plan',        timeout: 10000  },
  { id: 'run',      command: 'roko plan run .roko/plans --max-retries 1',                  description: 'Execute: agents + gates',        timeout: 600000 },
  { id: 'status',   command: 'roko status',                                                description: 'View results and costs',         timeout: 10000  },
];

// ── Runtime commands factory (ctx-aware, actual command strings) ─

function prdCommands(ctx: ScenarioContext): CommandDef[] {
  return [
    { id: 'init',     command: roko(ctx, 'init'),                                              description: 'Create workspace and config',    timeout: 10000  },
    { id: 'idea',     command: roko(ctx, `prd idea "${PRD_IDEA}"`),                            description: 'Capture work item',              timeout: 10000  },
    { id: 'draft',    command: roko(ctx, 'prd draft new "BTC Funding Alert CLI"'),             description: 'Generate PRD via LLM',           timeout: 180000 },
    { id: 'promote',  command: roko(ctx, 'prd draft promote btc-funding-alert-cli'),           description: 'Promote to published',           timeout: 10000  },
    { id: 'plan',     command: roko(ctx, 'prd plan btc-funding-alert-cli'),                    description: 'Generate implementation plan',   timeout: 300000 },
    { id: 'validate', command: roko(ctx, 'plan validate .roko/plans'),                         description: 'Lint the generated plan',        timeout: 10000  },
    { id: 'run',      command: roko(ctx, 'plan run .roko/plans --max-retries 1'),              description: 'Execute: agents + gates',        timeout: 600000 },
    { id: 'status',   command: roko(ctx, 'status'),                                            description: 'View results and costs',         timeout: 10000  },
  ];
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

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<boolean> {
    const commands = prdCommands(ctx);
    const cmd = commands.find(c => c.id === commandId);
    if (!cmd) return false;

    const [main] = ctx.entries;
    if (!main) return false;

    const result = await showCmd(main, cmd.command, {
      timeout: cmd.timeout ?? 60000,
      customDesc: cmd.description,
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

    return result.ok;
  },
};
