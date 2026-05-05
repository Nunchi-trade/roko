// -- Execution sub-events (nested inside ServerEvent::Execution) ----
export type ExecutionEvent =
  | { type: 'plan_started' }
  | { type: 'task_started'; taskId: string; title: string; phase: string }
  | { type: 'task_phase_changed'; taskId: string; oldPhase: string; newPhase: string }
  | { type: 'gate_result'; taskId: string; gate: string; passed: boolean; message: string }
  | { type: 'task_completed'; taskId: string; outcome: string }
  | { type: 'plan_completed'; outcome: string; stats: Record<string, unknown> }
  | { type: 'replan_triggered'; taskId: string; strategy: string }
  | { type: 'watcher_alert'; watcher: string; message: string };

// -- Top-level ServerEvent discriminated union ----------------------
// 50 variants. Field names are camelCase conversions of Rust snake_case.
// Variants with #[serde(rename = "...")] use the RENAMED value as the type tag.
export type ServerEvent =
  // Plan execution
  | { type: 'plan_started'; planId: string }
  | { type: 'plan_completed'; planId: string; success: boolean }
  | { type: 'phase_transition'; planId: string; from: string; to: string }
  | { type: 'execution'; planId: string; event: ExecutionEvent }
  | { type: 'episode'; planId: string; taskId: string; passed: boolean }
  | { type: 'efficiency_event'; planId: string; taskId: string;
       metric: string; value: number }
  // Task lifecycle (dashboard-facing)
  | { type: 'task_started'; planId: string; taskId: string;
       description: string }
  | { type: 'task_completed'; planId: string; taskId: string;
       success: boolean }
  | { type: 'task_failed'; planId: string; taskId: string; error: string;
       gateFailure: boolean }
  // Agent lifecycle
  | { type: 'agent_spawned'; agentId: string; role: string; model: string }
  | { type: 'agent_output'; agentId: string; runId?: string;
       content: string; done: boolean;
       metadata?: Record<string, unknown> }
  | { type: 'agent_trace'; agentId: string; runId?: string;
       turn?: number; content?: string; toolCalls?: unknown[];
       reasoning?: string; usage?: Record<string, unknown>; done?: boolean }
  | { type: 'agent_started'; agentId: string }
  | { type: 'agent_stopped'; agentId: string; reason: string }
  // Gate results
  | { type: 'gate_result'; planId: string; taskId: string;
       gate: string; rung: number; passed: boolean }
  // Inference tracking
  | { type: 'inference_started'; requestId: string; model: string;
       agentId: string; autoRouted: boolean }
  | { type: 'inference_completed'; requestId: string; model: string;
       agentId: string; inputTokens: number; outputTokens: number;
       costUsd: number; durationMs: number }
  | { type: 'inference_failed'; requestId: string; model: string;
       agentId: string; error: string }
  // One-shot runs
  | { type: 'run_started'; runId: string; prompt: string;
       complexity: string }
  | { type: 'run_completed'; runId: string; success: boolean;
       costUsd: number; durationMs: number }
  // Knowledge
  | { type: 'knowledge_ingested'; entryId: string; topic: string;
       sourceAgent: string }
  | { type: 'knowledge_consumed'; entryId: string; topic: string;
       consumingAgent: string }
  // Generic operations
  | { type: 'operation_started'; opId: string; kind: string }
  | { type: 'operation_completed'; opId: string; kind: string;
       success: boolean }
  // Somatic / affect
  | { type: 'somatic_marker_fired'; planId: string; taskId: string;
       valence: number; intensity: number; sourceEpisodes: string[];
       strategyParam: string }
  // Deployment lifecycle
  | { type: 'deployment_created'; id: string; name: string }
  | { type: 'deployment_ready'; id: string; url: string }
  | { type: 'deployment_failed'; id: string; reason: string }
  | { type: 'deployment_torn_down'; id: string }
  // Job marketplace
  | { type: 'job_created'; job: Record<string, unknown> }
  | { type: 'job_posted_to_candidate'; jobId: string; agentId: string;
       reward: string }
  | { type: 'job_updated'; job: Record<string, unknown> }
  | { type: 'job_transitioned'; jobId: string; from: string; to: string;
       assignedTo?: string }
  | { type: 'job_execution_started'; jobId: string; jobType: string;
       agentId: string }
  | { type: 'job_progress'; jobId: string; percent: number;
       message: string }
  | { type: 'job_agent_output'; jobId: string; agentId: string;
       content: string; done: boolean }
  | { type: 'job_submitted'; jobId: string; agentId: string }
  | { type: 'job_evaluated'; jobId: string; accepted: boolean;
       feedback: string }
  | { type: 'job_state_changed'; jobId: string; from: string;
       to: string }
  // Worker
  | { type: 'worker_task_started'; deploymentId: string;
       taskId: string }
  | { type: 'worker_task_completed'; deploymentId: string;
       taskId: string; success: boolean }
  // Chain triage
  | { type: 'chain_triage_result'; jobId: string; eventCount: number;
       anomalyCount: number; summary: string }
  // Heartbeat
  | { type: 'heartbeat_received'; senderId: string;
       activeTasks: number; activeAgents: number }
  | { type: 'heartbeat'; agentId: string; blockNumber?: number }
  // Config / strategy reload
  | { type: 'config_reloaded'; appliedSections: string[];
       restartRequired: string[] }
  | { type: 'strategy_reloaded'; goalsCount: number;
       tacticsCount: number }
  // Vision loop
  | { type: 'vision_loop_iteration'; runId: string; iteration: number;
       score: number; notes: string }
  | { type: 'vision_loop_completed'; runId: string; iterations: number;
       bestScore: number; stopReason: string }
  // Webhook
  | { type: 'webhook_received'; signal: Record<string, unknown> }
  // Bench (PascalCase type tags -- server uses #[serde(rename)])
  | { type: 'BenchRunStarted'; benchId: string; suiteId: string;
       totalTasks: number }
  | { type: 'BenchTaskStarted'; benchId: string; taskId: string;
       taskName: string; taskIndex: number; totalTasks: number }
  | { type: 'BenchTaskCompleted'; benchId: string; taskId: string;
       result: Record<string, unknown> }
  | { type: 'BenchLearningEvent'; benchId: string; taskId: string;
       playbooksCreated: number; antiPatternsCreated: number;
       totalPlaybooks: number; totalAntiPatterns: number }
  | { type: 'BenchProgress'; benchId: string; completed: number;
       total: number; costSoFar: number }
  | { type: 'BenchRunCompleted'; benchId: string;
       summary: Record<string, unknown> }
  | { type: 'BenchGateVerdict'; benchId: string; taskId: string;
       gate: string; passed: boolean; message?: string;
       durationMs: number }
  | { type: 'BenchTokenVelocity'; benchId: string; taskId: string;
       tokensPerSecond: number; tokensIn: number; tokensOut: number;
       durationMs: number }
  | { type: 'BenchAgentOutput'; benchId: string; taskId: string;
       agentId: string; content: string; done: boolean;
       toolCalls?: unknown[]; reasoning?: string }
  // Matrix bench
  | { type: 'MatrixRunStarted'; matrixId: string; suiteId: string;
       laneIds: string[]; totalLanes: number }
  | { type: 'MatrixLaneCompleted'; matrixId: string; laneId: string;
       passRate: number; costUsd: number }
  | { type: 'MatrixRunCompleted'; matrixId: string;
       summary: Record<string, unknown>[] }
  // SWE-bench
  | { type: 'SweRunStarted'; runId: string; dataset: string;
       totalInstances: number }
  | { type: 'SweInstanceCompleted'; runId: string; instanceId: string;
       resolved: boolean; durationMs: number }
  | { type: 'SweRunCompleted'; runId: string; resolved: number;
       total: number; passRate: number }
  // System
  | { type: 'server_shutdown' }
  | { type: 'error'; message: string };

/**
 * Recursively convert snake_case keys to camelCase in a plain object.
 * Arrays are traversed element-wise; non-object values pass through.
 */
function snakeToCamelObj(obj: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(obj)) {
    const camel = key.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
    const val = obj[key];
    if (val !== null && typeof val === 'object' && !Array.isArray(val)) {
      out[camel] = snakeToCamelObj(val as Record<string, unknown>);
    } else {
      out[camel] = val;
    }
  }
  return out;
}

/**
 * Parse a raw JSON object (from SSE `e.data`) into a typed ServerEvent.
 * Converts snake_case field names to camelCase.
 * Returns null if the `type` field is missing.
 */
export function parseServerEvent(
  raw: Record<string, unknown>,
): ServerEvent | null {
  if (typeof raw.type !== 'string') return null;

  const converted = snakeToCamelObj(raw);
  // Preserve the original `type` tag (do NOT camelCase it -- Bench events
  // use PascalCase like `BenchRunStarted`, and snake_case events like
  // `plan_started` must stay as-is).
  converted.type = raw.type;
  return converted as unknown as ServerEvent;
}
