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
  backend?: string;
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
  expected_output?: string;
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
  output_preview?: string;
  difficulty?: number;
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
  status: 'pending' | 'running' | 'completed' | 'cancelled' | 'failed';
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
export interface BenchRunStartedEvent {
  type: 'BenchRunStarted';
  bench_id: string;
  suite_id: string;
  suite_name: string;
  total_tasks: number;
  model: string;
}

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

export interface BenchLearningEvent {
  type: 'BenchLearning' | 'BenchLearningEvent';
  bench_id: string;
  task_id?: string;
  insight?: string;
  metric?: string;
  before?: number;
  after?: number;
  confidence?: number;
  playbooks_created?: number;
  anti_patterns_created?: number;
  total_playbooks?: number;
  total_anti_patterns?: number;
}

// ── Matrix types ──

export interface MatrixLane {
  model: string;
  backend?: string;
  strategy: AgentStrategy;
  label?: string;
  overrides: Partial<BenchRunConfig>;
}

export interface MatrixRun {
  id: string;
  suite_id: string;
  lane_ids: string[];
  status: 'running' | 'completed' | 'cancelled' | 'partial_failure';
  started_at: string;
  finished_at?: string;
  label?: string;
}

export interface ConfigPreset {
  id: string;
  label: string;
  strategy: AgentStrategy;
  temperature?: number;
  maxTokens?: number;
  description: string;
}

// Matrix SSE events

export interface MatrixRunStartedEvent {
  type: 'MatrixRunStarted';
  matrix_id: string;
  suite_id: string;
  lane_ids: string[];
  total_lanes: number;
}

export interface MatrixLaneCompletedEvent {
  type: 'MatrixLaneCompleted';
  matrix_id: string;
  lane_id: string;
  pass_rate: number;
  cost_usd: number;
}

export interface MatrixRunCompletedEvent {
  type: 'MatrixRunCompleted';
  matrix_id: string;
  summary: MatrixLaneSummary[];
}

export interface MatrixLaneSummary {
  lane_id: string;
  model: string;
  strategy: AgentStrategy;
  pass_rate: number;
  cost_usd: number;
  duration_ms: number;
  total_tasks: number;
  passed: number;
  failed: number;
}

// ── SWE-bench types ──

export interface SweDataset {
  id: string;
  name: string;
  total_instances: number;
  description?: string;
}

export interface SweInstance {
  instance_id: string;
  repo: string;
  resolved: boolean;
  duration_ms: number;
  error?: string;
}

export interface SweRun {
  id: string;
  dataset: string;
  status: 'running' | 'completed' | 'cancelled';
  agent_mode: string;
  total_instances: number;
  resolved: number;
  instances: SweInstance[];
  started_at: string;
  finished_at?: string;
}

export interface SweRunStartedEvent {
  type: 'SweRunStarted';
  run_id: string;
  dataset: string;
  total_instances: number;
}

export interface SweInstanceCompletedEvent {
  type: 'SweInstanceCompleted';
  run_id: string;
  instance_id: string;
  resolved: boolean;
  duration_ms: number;
}

export interface SweRunCompletedEvent {
  type: 'SweRunCompleted';
  run_id: string;
  resolved: number;
  total: number;
  pass_rate: number;
}

// ── Pareto types ──

export interface ParetoFrontierPoint {
  run_id: string;
  label?: string;
  model?: string;
  provider?: string;
  cost_usd: number;
  total_cost_usd?: number;
  pass_rate: number;
  duration_ms?: number;
}

export interface ParetoFrontierResponse {
  points: ParetoFrontierPoint[];
  frontier?: ParetoFrontierPoint[];
  generated_at?: string;
}

export interface BenchAgentOutputEvent {
  type: 'BenchAgentOutput';
  bench_id: string;
  task_id: string;
  agent_id: string;
  content: string;
  done: boolean;
  tool_calls?: any[];
  reasoning?: string;
}

export interface BenchGateVerdictEvent {
  type: 'BenchGateVerdict';
  bench_id: string;
  task_id: string;
  gate: string;
  passed: boolean;
  message?: string;
  duration_ms: number;
}

export interface BenchTokenVelocityEvent {
  type: 'BenchTokenVelocity';
  bench_id: string;
  task_id: string;
  tokens_per_second: number;
  tokens_in: number;
  tokens_out: number;
  duration_ms: number;
}

export type BenchSSEEvent =
  | BenchRunStartedEvent
  | BenchTaskStartedEvent
  | BenchTaskCompletedEvent
  | BenchProgressEvent
  | BenchRunCompletedEvent
  | BenchLearningEvent
  | BenchAgentOutputEvent
  | BenchGateVerdictEvent
  | BenchTokenVelocityEvent
  | MatrixRunStartedEvent
  | MatrixLaneCompletedEvent
  | MatrixRunCompletedEvent
  | SweRunStartedEvent
  | SweInstanceCompletedEvent
  | SweRunCompletedEvent;
