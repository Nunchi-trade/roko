export type PipelinePhase =
  | 'idle'
  | 'setup'
  | 'idea'
  | 'draft'
  | 'published'
  | 'planning'
  | 'tasks'
  | 'implementing'
  | 'complete'
  | 'failed';

export type PipelineSource = 'empty' | 'live';

export type PipelineTaskStatus = 'pending' | 'active' | 'done' | 'failed' | 'blocked';

export type PipelineRouteTier = 'T1' | 'T2' | 'T3';

export type PipelineExampleId = 'simple-status' | 'release-watch' | 'funding-alert';

export interface PipelineExampleSummary {
  id: PipelineExampleId;
  label: string;
  complexity: string;
  prdTitle: string;
  idea: string;
  why: string[];
  stageQuote?: string;
}

export interface PipelineScenarioExample extends PipelineExampleSummary {
  slug: string;
  workspacePrefix: string;
  repoName: string;
  setupDescription: string;
  /** Pre-seeded PRD markdown for reliable demo playback */
  seedPrd?: string;
  /** Pre-seeded tasks.toml content */
  seedTasksToml?: string;
  /** Pre-seeded plan.md content */
  seedPlanMd?: string;
  /** Pre-seeded implementation files: { relativePath: content } */
  seedFiles?: Record<string, string>;
}

export interface PipelinePrd {
  slug: string;
  title: string;
  path?: string;
  status: 'idea' | 'draft' | 'published' | 'planned';
  excerpt: string;
  requirements: string[];
  acceptance: string[];
}

export interface PipelineVerifyStep {
  phase: string;
  command: string;
  failMsg?: string;
  status?: 'pending' | 'passed' | 'failed';
}

export interface PipelineTask {
  id: string;
  title: string;
  description?: string;
  status: PipelineTaskStatus;
  rawStatus?: string;
  routeTier?: PipelineRouteTier;
  tier?: string;
  role?: string;
  modelHint?: string;
  maxLoc?: number;
  files: string[];
  dependsOn: string[];
  dependsOnPlan?: string[];
  verify: PipelineVerifyStep[];
  phase?: string;
  agentId?: string;
}

export interface PipelinePlan {
  id: string;
  title: string;
  path?: string;
  status: 'pending' | 'active' | 'complete' | 'failed';
  excerpt: string;
  estimatedMinutes?: number;
  tasks: PipelineTask[];
}

export interface PipelineEvent {
  id: string;
  ts: string;
  phase: PipelinePhase;
  text: string;
  kind?: 'info' | 'success' | 'warning' | 'error';
}

export type PipelineStreamConnection = 'idle' | 'connecting' | 'live' | 'error' | 'closed';

export interface PipelineStreamState {
  sse: PipelineStreamConnection;
  ws: PipelineStreamConnection;
  workdir?: string;
  workflowId?: string;
  cursor?: number;
  message?: string;
}

export interface PipelineDemoState {
  source: PipelineSource;
  phase: PipelinePhase;
  headline: string;
  example?: PipelineExampleSummary;
  currentCommand?: string;
  prd?: PipelinePrd;
  plans: PipelinePlan[];
  events: PipelineEvent[];
  lastUpdated?: string;
  stream?: PipelineStreamState;
}

export const EMPTY_PIPELINE_STATE: PipelineDemoState = {
  source: 'empty',
  phase: 'idle',
  headline: 'Waiting for a live Roko pipeline run',
  plans: [],
  events: [],
  stream: {
    sse: 'idle',
    ws: 'idle',
  },
};

export function normalizePipelineTaskStatus(status?: string): PipelineTaskStatus {
  const s = (status ?? '').trim().toLowerCase();
  if (['done', 'complete', 'completed', 'passed'].includes(s)) return 'done';
  if (['active', 'running', 'in_progress', 'implementing', 'working'].includes(s)) return 'active';
  if (['failed', 'fail', 'error'].includes(s)) return 'failed';
  if (['blocked', 'waiting'].includes(s)) return 'blocked';
  return 'pending';
}

export function normalizePipelineRouteTier(
  tier?: string,
  modelHint?: string,
  role?: string,
  maxLoc?: number,
  verifyCount = 0,
): PipelineRouteTier {
  const explicit = (tier ?? '').trim().toUpperCase();
  if (explicit.startsWith('T1')) return 'T1';
  if (explicit.startsWith('T2')) return 'T2';
  if (explicit.startsWith('T3')) return 'T3';

  const model = (modelHint ?? '').toLowerCase();
  const roleText = (role ?? '').toLowerCase();
  if (model.includes('opus') || model.includes('gpt-5') || roleText.includes('security')) return 'T3';
  if (model.includes('sonnet') || model.includes('codex') || roleText.includes('integr')) return 'T2';
  if (model.includes('haiku') || roleText.includes('verify') || roleText.includes('test')) return 'T1';

  if ((maxLoc ?? 0) >= 100 || verifyCount >= 4) return 'T3';
  if ((maxLoc ?? 0) >= 45 || verifyCount >= 2) return 'T2';
  return 'T1';
}
