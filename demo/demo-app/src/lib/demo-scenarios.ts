export interface ScenarioStep {
  label: string;
  sublabel: string;
}

export interface Scenario {
  id: string;
  title: string;
  subtitle: string;
  panes: 1 | 2 | 4;
  panel: boolean;
  promptBar: boolean;
  labels: string[];
  steps: ScenarioStep[];
}

export const SCENARIOS: Scenario[] = [
  {
    id: 'selfhost',
    title: 'Self-Hosting',
    subtitle: 'Watch roko develop itself — from idea to running code.',
    panes: 1,
    panel: true,
    promptBar: false,
    labels: ['self-hosting'],
    steps: [
      { label: 'Capture idea', sublabel: 'prd idea' },
      { label: 'Draft PRD', sublabel: 'prd draft new' },
      { label: 'Generate plan', sublabel: 'prd plan' },
      { label: 'Check status', sublabel: 'status' },
      { label: 'Inspect learning', sublabel: 'learn all' },
    ],
  },
  {
    id: 'builder',
    title: 'Build',
    subtitle: 'Type a prompt. Roko builds it, validates with gates, shows cost.',
    panes: 1,
    panel: true,
    promptBar: true,
    labels: ['builder'],
    steps: [
      { label: 'Submit prompt', sublabel: 'type or pick preset' },
      { label: 'Agent builds', sublabel: 'roko run' },
      { label: 'Gates validate', sublabel: 'compile + test + clippy' },
    ],
  },
  {
    id: 'race',
    title: 'Cost Race',
    subtitle: 'Same task, two approaches. Left: naive single-model. Right: cascade-routed.',
    panes: 2,
    panel: true,
    promptBar: false,
    labels: ['naive (no replan)', 'cascade (full pipeline)'],
    steps: [
      { label: 'Naive run', sublabel: '--no-replan' },
      { label: 'Cascade run', sublabel: 'full pipeline' },
    ],
  },
  {
    id: 'providers',
    title: 'Providers',
    subtitle: 'One prompt, four providers, simultaneously. Provider-agnostic by design.',
    panes: 4,
    panel: true,
    promptBar: false,
    labels: ['zhipu (glm-4)', 'openai (gpt-4o)', 'anthropic (haiku)', 'moonshot (v1)'],
    steps: [
      { label: 'Zhipu GLM-4', sublabel: 'dispatch' },
      { label: 'OpenAI GPT-4o', sublabel: 'dispatch' },
      { label: 'Anthropic Haiku', sublabel: 'dispatch' },
      { label: 'Moonshot v1', sublabel: 'dispatch' },
    ],
  },
  {
    id: 'explore',
    title: 'Explore',
    subtitle: '18 crates, 85 routes, 100+ commands. Four capability families at once.',
    panes: 4,
    panel: true,
    promptBar: false,
    labels: ['workspace', 'learning', 'config', 'knowledge'],
    steps: [
      { label: 'status', sublabel: 'workspace' },
      { label: 'doctor', sublabel: 'workspace' },
      { label: 'prd list', sublabel: 'workspace' },
      { label: 'learn all', sublabel: 'learning' },
      { label: 'learn efficiency', sublabel: 'learning' },
      { label: 'learn tune gates', sublabel: 'learning' },
      { label: 'providers list', sublabel: 'config' },
      { label: 'models list', sublabel: 'config' },
      { label: 'config validate', sublabel: 'config' },
      { label: 'knowledge stats', sublabel: 'knowledge' },
      { label: 'knowledge query', sublabel: 'knowledge' },
      { label: 'explain', sublabel: 'knowledge' },
    ],
  },
  {
    id: 'chat',
    title: 'Chat',
    subtitle: 'The bare command is the product. Just type roko — auto-detect, auto-init, drop into chat.',
    panes: 1,
    panel: true,
    promptBar: false,
    labels: ['roko chat'],
    steps: [
      { label: 'Start TUI', sublabel: 'roko' },
      { label: 'Send message', sublabel: 'explain cascade routing' },
      { label: 'Slash commands', sublabel: '/status, /model' },
    ],
  },
  {
    id: 'mirage',
    title: 'Mirage',
    subtitle: 'Fork any EVM chain locally. Stream blocks in real-time with configurable block times.',
    panes: 1,
    panel: false,
    promptBar: false,
    labels: ['mirage'],
    steps: [],
  },
];
