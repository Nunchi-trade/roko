// --- src/lib/scenario-registry.ts ---
// Pure config metadata for each scenario. No orchestration code.

export type ScenarioComplexity = 'simple' | 'medium' | 'complex';

export interface ScenarioMeta {
  id: string;            // matches Scenario.id in scenarios.ts
  label: string;         // human name for card title
  description: string;   // 1-2 sentence description for card body
  complexity: ScenarioComplexity;
  phases: string[];      // phase names for PhaseRail
  estimatedDuration: number; // seconds at 1x speed
  tags: string[];        // for filtering/grouping: 'prd', 'bench', 'model', 'knowledge', 'chain'
}

export const SCENARIO_REGISTRY: ScenarioMeta[] = [
  {
    id: 'prd-pipeline',
    label: 'PRD Pipeline',
    description: 'Full idea-to-code workflow: draft PRD, research, generate plan, execute tasks, validate with gates.',
    complexity: 'complex',
    phases: ['Idea', 'Draft', 'Research', 'Plan', 'Execute', 'Gate', 'Done'],
    estimatedDuration: 45,
    tags: ['prd'],
  },
  {
    id: 'prd-research-loop',
    label: 'Research Loop',
    description: 'Research-enhanced PRD generation with Perplexity-powered context enrichment.',
    complexity: 'medium',
    phases: ['Draft', 'Research', 'Enhance', 'Done'],
    estimatedDuration: 30,
    tags: ['prd'],
  },
  {
    id: 'race',
    label: 'Model Race',
    description: 'Side-by-side model comparison on identical prompts with cost and quality tracking.',
    complexity: 'medium',
    phases: ['Configure', 'Race', 'Score', 'Done'],
    estimatedDuration: 25,
    tags: ['model', 'bench'],
  },
  {
    id: 'gate-retry',
    label: 'Gate Retry',
    description: 'Demonstrates gate failure detection and automatic replan-retry loop.',
    complexity: 'medium',
    phases: ['Execute', 'Gate Fail', 'Replan', 'Retry', 'Pass'],
    estimatedDuration: 30,
    tags: ['prd'],
  },
  {
    id: 'providers',
    label: 'Provider Health',
    description: 'Iterates all configured providers and checks health, latency, and model availability.',
    complexity: 'simple',
    phases: ['Scan', 'Test', 'Report'],
    estimatedDuration: 15,
    tags: ['model'],
  },
  {
    id: 'provider-race',
    label: 'Provider Race',
    description: 'Concurrent provider benchmark: same prompt to multiple backends, first-to-finish wins.',
    complexity: 'medium',
    phases: ['Configure', 'Race', 'Compare', 'Done'],
    estimatedDuration: 20,
    tags: ['model', 'bench'],
  },
  {
    id: 'explore',
    label: 'Code Explorer',
    description: 'Code intelligence walkthrough: index build, semantic search, dependency graph.',
    complexity: 'simple',
    phases: ['Index', 'Search', 'Graph', 'Done'],
    estimatedDuration: 20,
    tags: ['knowledge'],
  },
  {
    id: 'knowledge-accumulation',
    label: 'Knowledge Accumulation',
    description: 'Shows neuro store ingestion, distillation, tier progression, and query.',
    complexity: 'medium',
    phases: ['Ingest', 'Distill', 'Promote', 'Query', 'Done'],
    estimatedDuration: 25,
    tags: ['knowledge'],
  },
  {
    id: 'dream-consolidation',
    label: 'Dream Consolidation',
    description: 'Offline dream cycle: hypnagogia, imagination, consolidation, journal entry.',
    complexity: 'complex',
    phases: ['Hypnagogia', 'Imagine', 'Consolidate', 'Journal', 'Done'],
    estimatedDuration: 35,
    tags: ['knowledge'],
  },
  {
    id: 'chat',
    label: 'Agent Chat',
    description: 'Interactive chat session with streaming response and tool calls.',
    complexity: 'simple',
    phases: ['Connect', 'Chat', 'Done'],
    estimatedDuration: 15,
    tags: ['model'],
  },
  {
    id: 'knowledge-transfer',
    label: 'Knowledge Transfer',
    description: 'Mesh knowledge sync between agents with custody verification.',
    complexity: 'complex',
    phases: ['Source', 'Transfer', 'Verify', 'Done'],
    estimatedDuration: 30,
    tags: ['knowledge', 'chain'],
  },
  {
    id: 'chain-intelligence',
    label: 'Chain Intelligence',
    description: 'Chain witness anchoring with HDC fingerprints and custody audit.',
    complexity: 'complex',
    phases: ['Fingerprint', 'Anchor', 'Verify', 'Done'],
    estimatedDuration: 35,
    tags: ['chain'],
  },
  {
    id: 'mirage',
    label: 'Mirage Deploy',
    description: 'Deploy pipeline: build, test, containerize, deploy to Mirage endpoint.',
    complexity: 'medium',
    phases: ['Build', 'Test', 'Container', 'Deploy', 'Done'],
    estimatedDuration: 30,
    tags: ['chain'],
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
