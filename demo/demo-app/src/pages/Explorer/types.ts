export interface HealthData {
  status: string;
  uptime_secs?: number;
  version?: string;
  active_plans?: number;
  active_agents?: number;
  active_runs?: number;
  providers?: Record<string, { healthy: boolean; latency_ms?: number }>;
}

export interface Episode {
  id: string;
  kind: string;
  agent_id?: string;
  task_id?: string;
  model?: string;
  status?: string;
  success?: boolean;
  usage?: { cost_usd?: number; input_tokens?: number; output_tokens?: number };
  timestamp_ms?: number;
  duration_secs?: number;
  turns?: number;
  gate_verdicts?: Array<{ gate: string; passed: boolean }>;
  [key: string]: unknown;
}

export interface StateEvent {
  type: string;
  payload: unknown;
  timestamp: string;
}

export const KIND_COLORS: Record<string, string> = {
  agent_turn: 'var(--rose)',
  gate_result: 'var(--success)',
  tool_call: 'var(--bone)',
  plan_step: 'var(--dream)',
};

export function kindColor(kind: string): string {
  return KIND_COLORS[kind] ?? 'var(--dream)';
}

export function getProviders(health: HealthData | null): Record<string, { healthy: boolean }> {
  if (!health) return {};
  const prov = health.providers;
  if (prov && typeof prov === 'object') {
    const keys = Object.keys(prov);
    if (keys.length > 0 && keys.some((k) => k !== 'healthy' && k !== 'total' && k !== 'unhealthy')) {
      return prov as Record<string, { healthy: boolean }>;
    }
    return {};
  }
  return {};
}
