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

  // Prefix-match map for model tier inference (replaces substring matching)
  const MODEL_TIER_MAP: Record<string, PipelineRouteTier> = {
    'claude-opus':    'T3',
    'claude-sonnet':  'T2',
    'claude-haiku':   'T1',
    'gpt-5.4':        'T3',
    'gpt-5.4-mini':   'T2',
    'gemini-2.5-pro': 'T3',
    'gemini-2.5':     'T2',
    'codex':          'T2',
  };
  const modelKey = Object.keys(MODEL_TIER_MAP).find(k => model.startsWith(k));
  if (modelKey) return MODEL_TIER_MAP[modelKey];

  if (roleText.includes('security')) return 'T3';
  if (roleText.includes('integr')) return 'T2';
  if (roleText.includes('verify') || roleText.includes('test')) return 'T1';

  if ((maxLoc ?? 0) >= 100 || verifyCount >= 4) return 'T3';
  if ((maxLoc ?? 0) >= 45 || verifyCount >= 2) return 'T2';
  return 'T1';
}
