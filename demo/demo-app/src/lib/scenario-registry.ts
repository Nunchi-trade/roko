// --- src/lib/scenario-registry.ts ---
// Pure config metadata for each active scenario. No orchestration code.

export type ScenarioComplexity = 'simple' | 'medium' | 'complex';

export interface ScenarioMeta {
  id: string;            // matches Scenario.id in scenarios.ts
  label: string;         // human name for card title
  description: string;   // 1-2 sentence description for card body
  complexity: ScenarioComplexity;
  phases: string[];      // phase names for PhaseRail
  estimatedDuration: number; // seconds at 1x speed
  tags: string[];        // for filtering/grouping
}

export const SCENARIO_REGISTRY: ScenarioMeta[] = [
  {
    id: 'cost',
    label: 'Cost',
    description: 'Same task, same model class. Cascade routing is the variable.',
    complexity: 'medium',
    phases: ['Baseline', 'Cascade', 'Compare'],
    estimatedDuration: 120,
    tags: ['cost', 'model', 'comparison'],
  },
  {
    id: 'pipeline',
    label: 'Pipeline',
    description: 'One command takes an idea to working, validated code.',
    complexity: 'medium',
    phases: ['Classify', 'Plan', 'Execute', 'Gate', 'Done'],
    estimatedDuration: 120,
    tags: ['pipeline'],
  },
  {
    id: 'memory',
    label: 'Memory',
    description: 'Second run inherits useful knowledge from the first run.',
    complexity: 'medium',
    phases: ['Cold', 'Ingest', 'Warm', 'Delta'],
    estimatedDuration: 120,
    tags: ['knowledge', 'learning'],
  },
  {
    id: 'isfr',
    label: 'ISFR',
    description: 'Four specialized agents compute a DeFi risk-free rate.',
    complexity: 'complex',
    phases: ['Scout', 'Aggregate', 'Validate', 'Publish'],
    estimatedDuration: 120,
    tags: ['chain', 'defi', 'agents'],
  },
  {
    id: 'oracle',
    label: 'Oracle',
    description: 'On-chain data becomes reusable agent knowledge and a strategy recommendation.',
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
