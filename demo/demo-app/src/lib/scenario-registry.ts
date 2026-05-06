// --- src/lib/scenario-registry.ts ---
// Pure config metadata for each active scenario. No orchestration code.

export type ScenarioComplexity = 'simple' | 'medium' | 'complex';

export interface ScenarioMeta {
  id: string;            // matches Scenario.id in scenarios.ts
  label: string;         // human name for card title
  description: string;   // 1-2 sentence jargon-free description
  headline: string;      // plain-English what-this-does (one line)
  narrative: string;     // what the viewer will see happen
  complexity: ScenarioComplexity;
  phases: string[];      // phase names for PhaseRail
  estimatedDuration: number; // seconds at 1x speed
  tags: string[];        // for filtering/grouping
}

export const SCENARIO_REGISTRY: ScenarioMeta[] = [
  {
    id: 'cost',
    label: 'Cost Comparison',
    description: 'Two agents solve the same coding task. One uses a fixed expensive model. The other uses cascade routing — starting cheap, escalating only when needed.',
    headline: 'See how smart model routing cuts costs',
    narrative: 'The left pane runs with a single expensive model. The right pane uses cascade routing — it starts with a cheap model and only escalates if the task needs it. Compare cost, tokens, and time side by side.',
    complexity: 'medium',
    phases: ['Baseline', 'Cascade', 'Compare'],
    estimatedDuration: 120,
    tags: ['cost', 'model', 'comparison'],
  },
  {
    id: 'pipeline',
    label: 'Pipeline',
    description: 'A single command takes a natural-language idea through classification, planning, code generation, and automated validation — producing working, tested code.',
    headline: 'From English prompt to working Rust code',
    narrative: 'Watch one command classify the request, break it into tasks, generate Rust code, then validate it with compile, lint, and test gates — all automatically.',
    complexity: 'medium',
    phases: ['Classify', 'Plan', 'Execute', 'Gate', 'Done'],
    estimatedDuration: 120,
    tags: ['pipeline'],
  },
  {
    id: 'memory',
    label: 'Memory',
    description: 'The first agent solves a task from scratch and saves what it learned. The second agent tackles a similar task but starts with that knowledge — solving it faster and cheaper.',
    headline: 'Agents that learn from past runs',
    narrative: 'The left pane solves a CSV-to-JSON task from scratch. Its learnings are saved. The right pane tackles a similar TOML-to-JSON task with that prior knowledge — compare cost and speed to see the difference.',
    complexity: 'medium',
    phases: ['First Run', 'Save Knowledge', 'Second Run', 'Compare'],
    estimatedDuration: 120,
    tags: ['knowledge', 'learning'],
  },
  {
    id: 'isfr',
    label: 'ISFR',
    description: 'Four specialized AI agents work together to compute a composite DeFi benchmark rate from lending, staking, and structured yield data.',
    headline: 'Agent swarm computes a DeFi benchmark rate',
    narrative: 'Four agents run in parallel: a rate keeper polls sources, lending and staking scouts analyze yields, and an oracle synthesizes everything into a single composite rate.',
    complexity: 'complex',
    phases: ['Scout', 'Aggregate', 'Validate', 'Publish'],
    estimatedDuration: 120,
    tags: ['chain', 'defi', 'agents'],
  },
  {
    id: 'oracle',
    label: 'Oracle',
    description: 'One agent reads live DeFi data from an Ethereum fork and writes structured analysis. A second agent reads that analysis and produces an investment strategy.',
    headline: 'Chain data to investment strategy via knowledge',
    narrative: 'The left pane agent reads DeFi lending rates from a local Ethereum fork and writes analysis to the knowledge store. The right pane agent reads that analysis and recommends a USDC allocation strategy.',
    complexity: 'complex',
    phases: ['Connect', 'Scan', 'Write', 'Recommend'],
    estimatedDuration: 120,
    tags: ['chain', 'defi', 'knowledge'],
  },
];

/** Look up a scenario by ID. Returns undefined if not found. */
export function getScenarioMeta(id: string): ScenarioMeta | undefined {
  return SCENARIO_REGISTRY.find((s) => s.id === id);
}

/** Get scenarios filtered by tag. */
export function getScenariosByTag(tag: string): ScenarioMeta[] {
  return SCENARIO_REGISTRY.filter((s) => s.tags.includes(tag));
}
