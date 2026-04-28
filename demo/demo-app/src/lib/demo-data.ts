/**
 * Hardcoded demo data for offline investor demos.
 * Falls back to these when roko serve is unreachable.
 */

// Health
export const DEMO_HEALTH = {
  status: 'ok',
  uptime_secs: 14523,
  version: '0.9.2',
  active_plans: 2,
  active_agents: 5,
  active_runs: 1,
  providers: { healthy: 4, total: 5, unhealthy: 1 },
  statehub: {
    snapshot: {
      cost_usd_total: 1.42,
      episodes_total: 847,
      gates_passed: 791,
      gates_failed: 56,
    },
  },
};

// 5 agents (matches real /api/managed-agents shape)
export const DEMO_AGENTS = [
  { id: 'a1', label: 'rustsmith', domain_tags: ['systems'], status: 'registered', model: 'claude-sonnet', capabilities: ['rust', 'systems', 'testing'], reputation: 92, performance: { completed_tasks: 156, failed_tasks: 3, active_tasks: 1, reputation: 92 }, costs: { cumulative_usd: 0.34 }, last_seen_at: Math.floor(Date.now() / 1000) - 120 },
  { id: 'a2', label: 'ethdev', domain_tags: ['blockchain'], status: 'registered', model: 'claude-sonnet', capabilities: ['solidity', 'evm', 'defi'], reputation: 88, performance: { completed_tasks: 89, failed_tasks: 2, active_tasks: 0, reputation: 88 }, costs: { cumulative_usd: 0.28 }, last_seen_at: Math.floor(Date.now() / 1000) - 300 },
  { id: 'a3', label: 'fullstack', domain_tags: ['web'], status: 'registered', model: 'gpt-4o', capabilities: ['typescript', 'react', 'api'], reputation: 85, performance: { completed_tasks: 203, failed_tasks: 5, active_tasks: 0, reputation: 85 }, costs: { cumulative_usd: 0.41 }, last_seen_at: Math.floor(Date.now() / 1000) - 600 },
  { id: 'a4', label: 'researcher', domain_tags: ['research'], status: 'registered', model: 'claude-haiku', capabilities: ['research', 'papers', 'docs'], reputation: 90, performance: { completed_tasks: 312, failed_tasks: 1, active_tasks: 2, reputation: 90 }, costs: { cumulative_usd: 0.18 }, last_seen_at: Math.floor(Date.now() / 1000) - 60 },
  { id: 'a5', label: 'auditor', domain_tags: ['security'], status: 'registered', model: 'claude-opus', capabilities: ['security', 'audit', 'review'], reputation: 95, performance: { completed_tasks: 67, failed_tasks: 0, active_tasks: 0, reputation: 95 }, costs: { cumulative_usd: 0.21 }, last_seen_at: Math.floor(Date.now() / 1000) - 1800 },
];

// 18 knowledge entries across domains
export const DEMO_KNOWLEDGE_ENTRIES = [
  { id: 'k01', domain: 'gate', citations: 6, label: 'Gate Pipeline' },
  { id: 'k02', domain: 'agent', citations: 5, label: 'Cascade Router' },
  { id: 'k03', domain: 'knowledge', citations: 4, label: 'Episode Logger' },
  { id: 'k04', domain: 'plan', citations: 7, label: 'DAG Executor' },
  { id: 'k05', domain: 'agent', citations: 3, label: 'Tool Dispatcher' },
  { id: 'k06', domain: 'config', citations: 2, label: 'System Prompt Builder' },
  { id: 'k07', domain: 'gate', citations: 5, label: 'Adaptive Thresholds' },
  { id: 'k08', domain: 'agent', citations: 4, label: 'Process Supervisor' },
  { id: 'k09', domain: 'knowledge', citations: 8, label: 'Neuro Store' },
  { id: 'k10', domain: 'plan', citations: 3, label: 'Plan Revision' },
  { id: 'k11', domain: 'agent', citations: 6, label: 'MCP Integration' },
  { id: 'k12', domain: 'gate', citations: 4, label: 'Compile Gate' },
  { id: 'k13', domain: 'gate', citations: 3, label: 'Test Gate' },
  { id: 'k14', domain: 'config', citations: 2, label: 'Role Templates' },
  { id: 'k15', domain: 'knowledge', citations: 5, label: 'Dream Consolidation' },
  { id: 'k16', domain: 'plan', citations: 4, label: 'PRD Lifecycle' },
  { id: 'k17', domain: 'agent', citations: 3, label: 'Context Bidders' },
  { id: 'k18', domain: 'config', citations: 1, label: 'Safety Contracts' },
];

// 28 edges connecting entries
export const DEMO_KNOWLEDGE_EDGES = [
  { source: 'k01', target: 'k07', frequency: 5 },
  { source: 'k01', target: 'k12', frequency: 4 },
  { source: 'k01', target: 'k13', frequency: 4 },
  { source: 'k02', target: 'k05', frequency: 3 },
  { source: 'k02', target: 'k08', frequency: 2 },
  { source: 'k03', target: 'k09', frequency: 5 },
  { source: 'k03', target: 'k15', frequency: 3 },
  { source: 'k04', target: 'k10', frequency: 4 },
  { source: 'k04', target: 'k16', frequency: 3 },
  { source: 'k05', target: 'k11', frequency: 4 },
  { source: 'k05', target: 'k17', frequency: 2 },
  { source: 'k06', target: 'k14', frequency: 3 },
  { source: 'k06', target: 'k18', frequency: 1 },
  { source: 'k07', target: 'k12', frequency: 3 },
  { source: 'k07', target: 'k13', frequency: 3 },
  { source: 'k08', target: 'k04', frequency: 2 },
  { source: 'k09', target: 'k15', frequency: 4 },
  { source: 'k09', target: 'k17', frequency: 3 },
  { source: 'k10', target: 'k01', frequency: 2 },
  { source: 'k11', target: 'k06', frequency: 2 },
  { source: 'k12', target: 'k13', frequency: 5 },
  { source: 'k14', target: 'k06', frequency: 2 },
  { source: 'k15', target: 'k09', frequency: 3 },
  { source: 'k16', target: 'k04', frequency: 4 },
  { source: 'k16', target: 'k10', frequency: 2 },
  { source: 'k17', target: 'k02', frequency: 3 },
  { source: 'k18', target: 'k05', frequency: 1 },
  { source: 'k03', target: 'k01', frequency: 2 },
];

// 20 sample episodes (matches real /api/episodes shape)
export const DEMO_EPISODES = [
  { id: 'ep-001', kind: 'agent_turn', agent_id: 'rustsmith', task_id: 'wire-gate-pipeline', model: 'claude-sonnet', usage: { cost_usd: 0.018 }, timestamp_ms: 1777388531000, gate_verdicts: [{ gate: 'compile', passed: true }], duration_secs: 12, turns: 3 },
  { id: 'ep-002', kind: 'gate_result', agent_id: 'rustsmith', task_id: 'wire-gate-pipeline', model: 'claude-sonnet', usage: { cost_usd: 0.003 }, timestamp_ms: 1777388564000, gate_verdicts: [{ gate: 'compile', passed: true }, { gate: 'test', passed: true }], duration_secs: 4, turns: 1 },
  { id: 'ep-003', kind: 'tool_call', agent_id: 'rustsmith', task_id: 'wire-gate-pipeline', model: 'claude-sonnet', usage: { cost_usd: 0.001 }, timestamp_ms: 1777388581000, gate_verdicts: [], duration_secs: 2, turns: 1 },
  { id: 'ep-004', kind: 'agent_turn', agent_id: 'ethdev', task_id: 'deploy-witness-contract', model: 'claude-sonnet', usage: { cost_usd: 0.024 }, timestamp_ms: 1777389033000, gate_verdicts: [{ gate: 'compile', passed: true }], duration_secs: 18, turns: 4 },
  { id: 'ep-005', kind: 'agent_turn', agent_id: 'researcher', task_id: 'enhance-prd-cascade', model: 'claude-haiku', usage: { cost_usd: 0.006 }, timestamp_ms: 1777389322000, gate_verdicts: [], duration_secs: 8, turns: 2 },
  { id: 'ep-006', kind: 'gate_result', agent_id: 'ethdev', task_id: 'deploy-witness-contract', model: 'claude-sonnet', usage: { cost_usd: 0.002 }, timestamp_ms: 1777389487000, gate_verdicts: [{ gate: 'compile', passed: true }, { gate: 'test', passed: true }], duration_secs: 3, turns: 1 },
  { id: 'ep-007', kind: 'agent_turn', agent_id: 'fullstack', task_id: 'build-dashboard-ui', model: 'gpt-4o', usage: { cost_usd: 0.032 }, timestamp_ms: 1777389765000, gate_verdicts: [{ gate: 'compile', passed: true }], duration_secs: 22, turns: 5 },
  { id: 'ep-008', kind: 'tool_call', agent_id: 'fullstack', task_id: 'build-dashboard-ui', model: 'gpt-4o', usage: { cost_usd: 0.002 }, timestamp_ms: 1777389798000, gate_verdicts: [], duration_secs: 1, turns: 1 },
  { id: 'ep-009', kind: 'agent_turn', agent_id: 'auditor', task_id: 'review-safety-layer', model: 'claude-opus', usage: { cost_usd: 0.045 }, timestamp_ms: 1777390200000, gate_verdicts: [{ gate: 'compile', passed: true }, { gate: 'clippy', passed: true }], duration_secs: 30, turns: 6 },
  { id: 'ep-010', kind: 'gate_result', agent_id: 'fullstack', task_id: 'build-dashboard-ui', model: 'gpt-4o', usage: { cost_usd: 0.003 }, timestamp_ms: 1777390315000, gate_verdicts: [{ gate: 'test', passed: true }], duration_secs: 5, turns: 1 },
  { id: 'ep-011', kind: 'agent_turn', agent_id: 'rustsmith', task_id: 'wire-episode-logger', model: 'claude-sonnet', usage: { cost_usd: 0.015 }, timestamp_ms: 1777390512000, gate_verdicts: [{ gate: 'compile', passed: true }], duration_secs: 10, turns: 3 },
  { id: 'ep-012', kind: 'agent_turn', agent_id: 'researcher', task_id: 'analyze-cost-data', model: 'claude-haiku', usage: { cost_usd: 0.005 }, timestamp_ms: 1777390803000, gate_verdicts: [], duration_secs: 6, turns: 2 },
  { id: 'ep-013', kind: 'tool_call', agent_id: 'rustsmith', task_id: 'wire-episode-logger', model: 'claude-sonnet', usage: { cost_usd: 0.001 }, timestamp_ms: 1777390889000, gate_verdicts: [], duration_secs: 1, turns: 1 },
  { id: 'ep-014', kind: 'gate_result', agent_id: 'rustsmith', task_id: 'wire-episode-logger', model: 'claude-sonnet', usage: { cost_usd: 0.002 }, timestamp_ms: 1777390970000, gate_verdicts: [{ gate: 'compile', passed: true }, { gate: 'test', passed: true }, { gate: 'clippy', passed: true }], duration_secs: 4, turns: 1 },
  { id: 'ep-015', kind: 'agent_turn', agent_id: 'ethdev', task_id: 'wire-chain-witness', model: 'claude-sonnet', usage: { cost_usd: 0.021 }, timestamp_ms: 1777391291000, gate_verdicts: [{ gate: 'compile', passed: false }], duration_secs: 15, turns: 4 },
  { id: 'ep-016', kind: 'agent_turn', agent_id: 'auditor', task_id: 'audit-mcp-config', model: 'claude-opus', usage: { cost_usd: 0.038 }, timestamp_ms: 1777391730000, gate_verdicts: [{ gate: 'compile', passed: true }, { gate: 'clippy', passed: true }], duration_secs: 25, turns: 5 },
  { id: 'ep-017', kind: 'agent_turn', agent_id: 'researcher', task_id: 'research-model-routing', model: 'claude-haiku', usage: { cost_usd: 0.004 }, timestamp_ms: 1777392102000, gate_verdicts: [], duration_secs: 7, turns: 2 },
  { id: 'ep-018', kind: 'gate_result', agent_id: 'ethdev', task_id: 'wire-chain-witness', model: 'claude-sonnet', usage: { cost_usd: 0.003 }, timestamp_ms: 1777392318000, gate_verdicts: [{ gate: 'compile', passed: true }, { gate: 'test', passed: true }], duration_secs: 4, turns: 1 },
  { id: 'ep-019', kind: 'tool_call', agent_id: 'auditor', task_id: 'audit-mcp-config', model: 'claude-opus', usage: { cost_usd: 0.002 }, timestamp_ms: 1777392524000, gate_verdicts: [], duration_secs: 2, turns: 1 },
  { id: 'ep-020', kind: 'agent_turn', agent_id: 'fullstack', task_id: 'wire-tui-dashboard', model: 'gpt-4o', usage: { cost_usd: 0.028 }, timestamp_ms: 1777392729000, gate_verdicts: [{ gate: 'compile', passed: true }], duration_secs: 20, turns: 4 },
];

// Efficiency (real API does not include `passed` field)
export const DEMO_EFFICIENCY = {
  total_cost: 1.42,
  cost_per_task: 0.017,
  tasks: [
    { task_id: 'wire-gate-pipeline', cost_usd: 0.022, tokens: 3200, duration_ms: 4500 },
    { task_id: 'deploy-witness-contract', cost_usd: 0.026, tokens: 4100, duration_ms: 6200 },
    { task_id: 'enhance-prd-cascade', cost_usd: 0.006, tokens: 1800, duration_ms: 2100 },
    { task_id: 'build-dashboard-ui', cost_usd: 0.037, tokens: 5500, duration_ms: 8400 },
    { task_id: 'review-safety-layer', cost_usd: 0.047, tokens: 6200, duration_ms: 9800 },
    { task_id: 'wire-episode-logger', cost_usd: 0.018, tokens: 2800, duration_ms: 3900 },
    { task_id: 'analyze-cost-data', cost_usd: 0.005, tokens: 1200, duration_ms: 1600 },
    { task_id: 'wire-chain-witness', cost_usd: 0.024, tokens: 3800, duration_ms: 5700 },
    { task_id: 'audit-mcp-config', cost_usd: 0.040, tokens: 5800, duration_ms: 8100 },
    { task_id: 'wire-tui-dashboard', cost_usd: 0.030, tokens: 4600, duration_ms: 7000 },
  ],
};

// C-Factor
export const DEMO_CFACTOR = {
  composite: { overall: 0.847, episode_count: 847 },
  sub_metrics: {
    gate_pass_rate: 0.931,
    cost_efficiency: 0.872,
    speed: 0.814,
    reuse_rate: 0.756,
    learning_rate: 0.863,
  },
};

// Router models (matches real /api/learn/cascade-router shape)
export const DEMO_ROUTER_MODELS = {
  model_slugs: ['haiku', 'sonnet', 'opus', 'gpt-4o'],
  role_table: {
    implementer: 'claude-sonnet-4-20250514',
    researcher: 'claude-haiku-3-20250414',
    reviewer: 'claude-opus-4-20250414',
    planner: 'claude-sonnet-4-20250514',
  },
  confidence_stats: {
    'claude-haiku-3-20250414': { successes: 342, trials: 380 },
    'claude-sonnet-4-20250514': { successes: 236, trials: 254 },
    'gpt-4o': { successes: 108, trials: 127 },
    'claude-opus-4-20250414': { successes: 82, trials: 86 },
  },
  total_observations: 847,
};

// Gates summary
export const DEMO_GATES_SUMMARY = { pass_rate: 0.931 };

// Status
export const DEMO_STATUS = {
  signals: 1247,
  episodes: 847,
  agents: 5,
  plans_completed: 23,
  plans_active: 2,
};

// Events
export const DEMO_EVENTS = [
  { type: 'plan_started', payload: { plan: 'wire-gate-pipeline', tasks: 4 }, timestamp: '2026-04-27T14:00:00Z' },
  { type: 'agent_dispatched', payload: { agent: 'rustsmith', model: 'claude-sonnet' }, timestamp: '2026-04-27T14:02:11Z' },
  { type: 'gate_passed', payload: { gate: 'compile', task: 'wire-gate-pipeline' }, timestamp: '2026-04-27T14:02:44Z' },
  { type: 'agent_dispatched', payload: { agent: 'ethdev', model: 'claude-sonnet' }, timestamp: '2026-04-27T14:10:33Z' },
  { type: 'route_selected', payload: { model: 'claude-haiku', task: 'enhance-prd-cascade' }, timestamp: '2026-04-27T14:15:22Z' },
  { type: 'gate_passed', payload: { gate: 'test', task: 'build-dashboard-ui' }, timestamp: '2026-04-27T14:31:55Z' },
  { type: 'episode_logged', payload: { count: 847, cost_total: 1.42 }, timestamp: '2026-04-27T14:35:12Z' },
  { type: 'threshold_updated', payload: { gate: 'clippy', new_threshold: 0.92 }, timestamp: '2026-04-27T14:42:50Z' },
  { type: 'plan_completed', payload: { plan: 'wire-gate-pipeline', success: true }, timestamp: '2026-04-27T15:05:18Z' },
  { type: 'router_updated', payload: { model: 'claude-haiku', weight: 0.45 }, timestamp: '2026-04-27T15:12:09Z' },
];

// Dashboard
export const DEMO_DASHBOARD = {
  total_cost: 1.42,
  cache_hit_rate: 0.73,
  routing_distribution: {
    'claude-haiku': 45,
    'claude-sonnet': 30,
    'gpt-4o': 15,
    'claude-opus': 10,
  },
  gate_pass_rate: 0.931,
};
