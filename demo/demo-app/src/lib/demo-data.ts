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

// 5 agents
export const DEMO_AGENTS = [
  { id: 'a1', name: 'rustsmith', domain: 'systems', status: 'active', model: 'claude-sonnet', capabilities: ['rust', 'systems', 'testing'], reputation: 92, stats: { tasks: 156, cost: 0.34, tokens: 42000 } },
  { id: 'a2', name: 'ethdev', domain: 'blockchain', status: 'active', model: 'claude-sonnet', capabilities: ['solidity', 'evm', 'defi'], reputation: 88, stats: { tasks: 89, cost: 0.28, tokens: 31000 } },
  { id: 'a3', name: 'fullstack', domain: 'web', status: 'idle', model: 'gpt-4o', capabilities: ['typescript', 'react', 'api'], reputation: 85, stats: { tasks: 203, cost: 0.41, tokens: 55000 } },
  { id: 'a4', name: 'researcher', domain: 'research', status: 'active', model: 'claude-haiku', capabilities: ['research', 'papers', 'docs'], reputation: 90, stats: { tasks: 312, cost: 0.18, tokens: 89000 } },
  { id: 'a5', name: 'auditor', domain: 'security', status: 'idle', model: 'claude-opus', capabilities: ['security', 'audit', 'review'], reputation: 95, stats: { tasks: 67, cost: 0.21, tokens: 28000 } },
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

// 20 sample episodes
export const DEMO_EPISODES = [
  { id: 'ep-001', kind: 'agent_turn', agent: 'rustsmith', task: 'wire-gate-pipeline', model: 'claude-sonnet', cost_usd: 0.018, timestamp: '2026-04-27T14:02:11Z' },
  { id: 'ep-002', kind: 'gate_result', agent: 'rustsmith', task: 'wire-gate-pipeline', model: 'claude-sonnet', cost_usd: 0.003, timestamp: '2026-04-27T14:02:44Z' },
  { id: 'ep-003', kind: 'tool_call', agent: 'rustsmith', task: 'wire-gate-pipeline', model: 'claude-sonnet', cost_usd: 0.001, timestamp: '2026-04-27T14:03:01Z' },
  { id: 'ep-004', kind: 'agent_turn', agent: 'ethdev', task: 'deploy-witness-contract', model: 'claude-sonnet', cost_usd: 0.024, timestamp: '2026-04-27T14:10:33Z' },
  { id: 'ep-005', kind: 'agent_turn', agent: 'researcher', task: 'enhance-prd-cascade', model: 'claude-haiku', cost_usd: 0.006, timestamp: '2026-04-27T14:15:22Z' },
  { id: 'ep-006', kind: 'gate_result', agent: 'ethdev', task: 'deploy-witness-contract', model: 'claude-sonnet', cost_usd: 0.002, timestamp: '2026-04-27T14:18:07Z' },
  { id: 'ep-007', kind: 'agent_turn', agent: 'fullstack', task: 'build-dashboard-ui', model: 'gpt-4o', cost_usd: 0.032, timestamp: '2026-04-27T14:22:45Z' },
  { id: 'ep-008', kind: 'tool_call', agent: 'fullstack', task: 'build-dashboard-ui', model: 'gpt-4o', cost_usd: 0.002, timestamp: '2026-04-27T14:23:18Z' },
  { id: 'ep-009', kind: 'agent_turn', agent: 'auditor', task: 'review-safety-layer', model: 'claude-opus', cost_usd: 0.045, timestamp: '2026-04-27T14:30:00Z' },
  { id: 'ep-010', kind: 'gate_result', agent: 'fullstack', task: 'build-dashboard-ui', model: 'gpt-4o', cost_usd: 0.003, timestamp: '2026-04-27T14:31:55Z' },
  { id: 'ep-011', kind: 'agent_turn', agent: 'rustsmith', task: 'wire-episode-logger', model: 'claude-sonnet', cost_usd: 0.015, timestamp: '2026-04-27T14:35:12Z' },
  { id: 'ep-012', kind: 'agent_turn', agent: 'researcher', task: 'analyze-cost-data', model: 'claude-haiku', cost_usd: 0.005, timestamp: '2026-04-27T14:40:03Z' },
  { id: 'ep-013', kind: 'tool_call', agent: 'rustsmith', task: 'wire-episode-logger', model: 'claude-sonnet', cost_usd: 0.001, timestamp: '2026-04-27T14:41:29Z' },
  { id: 'ep-014', kind: 'gate_result', agent: 'rustsmith', task: 'wire-episode-logger', model: 'claude-sonnet', cost_usd: 0.002, timestamp: '2026-04-27T14:42:50Z' },
  { id: 'ep-015', kind: 'agent_turn', agent: 'ethdev', task: 'wire-chain-witness', model: 'claude-sonnet', cost_usd: 0.021, timestamp: '2026-04-27T14:48:11Z' },
  { id: 'ep-016', kind: 'agent_turn', agent: 'auditor', task: 'audit-mcp-config', model: 'claude-opus', cost_usd: 0.038, timestamp: '2026-04-27T14:55:30Z' },
  { id: 'ep-017', kind: 'agent_turn', agent: 'researcher', task: 'research-model-routing', model: 'claude-haiku', cost_usd: 0.004, timestamp: '2026-04-27T15:01:42Z' },
  { id: 'ep-018', kind: 'gate_result', agent: 'ethdev', task: 'wire-chain-witness', model: 'claude-sonnet', cost_usd: 0.003, timestamp: '2026-04-27T15:05:18Z' },
  { id: 'ep-019', kind: 'tool_call', agent: 'auditor', task: 'audit-mcp-config', model: 'claude-opus', cost_usd: 0.002, timestamp: '2026-04-27T15:08:44Z' },
  { id: 'ep-020', kind: 'agent_turn', agent: 'fullstack', task: 'wire-tui-dashboard', model: 'gpt-4o', cost_usd: 0.028, timestamp: '2026-04-27T15:12:09Z' },
];

// Efficiency
export const DEMO_EFFICIENCY = {
  total_cost: 1.42,
  cost_per_task: 0.017,
  tasks: [
    { task_id: 'wire-gate-pipeline', cost_usd: 0.022, passed: true, tokens: 3200, duration_ms: 4500 },
    { task_id: 'deploy-witness-contract', cost_usd: 0.026, passed: true, tokens: 4100, duration_ms: 6200 },
    { task_id: 'enhance-prd-cascade', cost_usd: 0.006, passed: true, tokens: 1800, duration_ms: 2100 },
    { task_id: 'build-dashboard-ui', cost_usd: 0.037, passed: true, tokens: 5500, duration_ms: 8400 },
    { task_id: 'review-safety-layer', cost_usd: 0.047, passed: true, tokens: 6200, duration_ms: 9800 },
    { task_id: 'wire-episode-logger', cost_usd: 0.018, passed: true, tokens: 2800, duration_ms: 3900 },
    { task_id: 'analyze-cost-data', cost_usd: 0.005, passed: true, tokens: 1200, duration_ms: 1600 },
    { task_id: 'wire-chain-witness', cost_usd: 0.024, passed: false, tokens: 3800, duration_ms: 5700 },
    { task_id: 'audit-mcp-config', cost_usd: 0.040, passed: true, tokens: 5800, duration_ms: 8100 },
    { task_id: 'wire-tui-dashboard', cost_usd: 0.030, passed: true, tokens: 4600, duration_ms: 7000 },
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

// Router models
export const DEMO_ROUTER_MODELS = {
  models: [
    { model: 'claude-haiku', weight: 0.45, trials: 380 },
    { model: 'claude-sonnet', weight: 0.30, trials: 254 },
    { model: 'gpt-4o', weight: 0.15, trials: 127 },
    { model: 'claude-opus', weight: 0.10, trials: 86 },
  ],
  current_model: 'claude-haiku',
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
