export const CMD_DESCRIPTIONS: Record<string, string> = {
  'prd idea':
    'Captures a raw work item idea into the PRD backlog. Stored as a one-liner that can be expanded into a full PRD later.',
  'prd draft new':
    'Dispatches an agent to expand an idea into a structured PRD document with sections for motivation, design, tasks, and success criteria.',
  'prd plan':
    'Generates a concrete implementation plan (tasks.toml) from a published PRD. The plan contains a DAG of tasks with dependencies.',
  'status':
    'Queries the signal store and reports counts across episodes, efficiency metrics, and workspace health.',
  'init':
    'Bootstraps a .roko/ directory with default config, signal store, and learning state. Required once per workspace.',
  'run':
    'Executes the universal loop: compose a system prompt, dispatch to an agent, run gate validation (compile/test/clippy), persist results.',
  'doctor':
    'Diagnoses workspace state: checks config, providers, dependencies, and reports any missing or misconfigured components.',
  'learn all':
    'Displays all learning state: cascade router weights, prompt experiments, gate thresholds, and efficiency metrics.',
  'learn efficiency':
    'Shows per-turn efficiency events: tokens used, cost, latency, and model selection decisions.',
  'config providers list':
    'Lists all configured LLM providers with their status, API key presence, and available models.',
  'config models list':
    'Shows the full model catalog across all providers, with routing tiers and capability tags.',
  'knowledge stats':
    'Reports durable knowledge store statistics: entry count, tiers, memory usage, and last distillation time.',
  'knowledge query':
    'Searches the neuro knowledge store by topic. Returns relevant entries ranked by relevance and recency.',
  'agent list':
    'Lists all registered agents with their status, domain, and last activity timestamp.',
  'agent create':
    'Creates a new agent from a manifest with a name, domain, and optional tool/model constraints.',
  'bench demo':
    'Runs a simulated benchmark comparing naive single-model execution against cascade-routed optimization.',
  'prd list':
    'Lists all PRDs in the workspace with their lifecycle stage (idea/draft/published/planned).',
  'research topic':
    'Dispatches a research agent to investigate a topic using web search and returns structured findings with citations.',
  'research enhance-prd':
    'Enriches an existing PRD with research findings, adding context, prior art, and implementation references.',
  'research analyze':
    'Analyzes execution data (episodes, efficiency events) and produces insights about agent performance patterns.',
  'plan list':
    'Lists all implementation plans in the workspace with their completion status and task counts.',
  'plan run':
    'Executes a plan — runs tasks in dependency order, dispatches agents, validates with gates, persists results. The main orchestration loop.',
  'plan validate':
    'Lints tasks.toml without executing. Checks DAG validity, dependency cycles, and missing fields.',
  'dashboard':
    'Opens the interactive ratatui TUI with F1-F7 tabs for monitoring agents, plans, episodes, and metrics.',
  'prd status':
    'Coverage report across all PRDs — shows how many have plans, how many tasks are completed.',
  'prd consolidate':
    'Scans PRDs for gaps and duplicates across the entire backlog.',
  'learn tune gates':
    'Adjusts adaptive gate thresholds based on recent pass/fail rates.',
  'learn tune routing':
    'Updates cascade router model weights based on cost/quality tradeoffs.',
  'config validate':
    'Validates roko.toml against the schema. Reports any invalid fields or missing required sections.',
  'config set-secret':
    'Securely stores an API key or secret, encrypted at rest.',
  'replay':
    'Walks the signal DAG from a given hash, showing the chain of transforms that produced it.',
  'explain':
    'Concept explainer with 3 depth levels (brief/standard/deep). Uses the knowledge store for context.',
};

export function lookupCmdDesc(cmd: string): string | null {
  const stripped = cmd.replace(/^(\.\/target\/(release|debug)\/)?roko\s+/, '');
  for (const [pattern, desc] of Object.entries(CMD_DESCRIPTIONS)) {
    if (stripped.startsWith(pattern)) return desc;
  }
  return null;
}
