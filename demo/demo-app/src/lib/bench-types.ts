/** Bench system types — mirrors Rust types in roko-serve. */

export type BenchRunKind = 'single' | 'suite' | 'comparison' | 'regression';
export type AgentStrategy = 'minimal' | 'context_enriched' | 'neuro_augmented' | 'full_cascade';
export type TaskStatus = 'pending' | 'running' | 'pass' | 'fail' | 'skipped';

export interface BenchGateConfig {
  compile: boolean;
  test: boolean;
  clippy: boolean;
  diff: boolean;
}

export interface BenchRunConfig {
  model: string;
  provider?: string;
  temperature?: number;
  max_tokens?: number;
  timeout_secs: number;
  strategy: AgentStrategy;
  retries: number;
  system_prompt_override?: string;
  gates?: BenchGateConfig;
  max_cost_usd?: number;
  parallel?: boolean;
}

export interface BenchTask {
  id: string;
  name: string;
  prompt: string;
  expected_outcome?: string;
  difficulty: number; // 1-5
  tags: string[];
  timeout_secs?: number;
}

export interface BenchSuite {
  id: string;
  name: string;
  description: string;
  tasks: BenchTask[];
  estimated_cost_usd: number;
  difficulty_range: [number, number];
}

export interface BenchGateVerdict {
  gate: string;
  passed: boolean;
  message?: string;
  duration_ms?: number;
}

export interface BenchTaskResult {
  task_id: string;
  task_name: string;
  status: TaskStatus;
  cost_usd: number;
  tokens_in: number;
  tokens_out: number;
  duration_ms: number;
  model: string;
  gate_verdicts: BenchGateVerdict[];
  error?: string;
  retries_used: number;
}

export interface BenchRunSummary {
  total_tasks: number;
  passed: number;
  failed: number;
  skipped: number;
  total_cost_usd: number;
  total_tokens: number;
  total_duration_ms: number;
  pass_rate: number;
  cost_per_success_usd: number;
  avg_duration_ms: number;
}

export interface BenchRun {
  id: string;
  kind: BenchRunKind;
  config: BenchRunConfig;
  suite_id: string;
  suite_name: string;
  status: 'pending' | 'running' | 'completed' | 'cancelled';
  results: BenchTaskResult[];
  summary?: BenchRunSummary;
  started_at: string;
  finished_at?: string;
}

export interface BenchModel {
  id: string;
  name: string;
  provider: string;
  cost_per_1k_input: number;
  cost_per_1k_output: number;
  max_tokens: number;
  context_window: number;
}

// SSE event types
export interface BenchTaskStartedEvent {
  type: 'BenchTaskStarted';
  bench_id: string;
  task_id: string;
  task_name: string;
}

export interface BenchTaskCompletedEvent {
  type: 'BenchTaskCompleted';
  bench_id: string;
  task_id: string;
  result: BenchTaskResult;
}

export interface BenchProgressEvent {
  type: 'BenchProgress';
  bench_id: string;
  completed: number;
  total: number;
  cost_so_far: number;
}

export interface BenchRunCompletedEvent {
  type: 'BenchRunCompleted';
  bench_id: string;
  summary: BenchRunSummary;
}

export type BenchSSEEvent =
  | BenchTaskStartedEvent
  | BenchTaskCompletedEvent
  | BenchProgressEvent
  | BenchRunCompletedEvent;
