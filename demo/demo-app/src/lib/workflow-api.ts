import { SERVE_URL, WS_BASE } from './serve-url';
import {
  normalizePipelineRouteTier,
  normalizePipelineTaskStatus,
  type PipelinePhase,
  type PipelinePlan,
  type PipelinePrd,
  type PipelineStreamState,
  type PipelineTask,
} from './prd-pipeline-types';

type Transport = 'sse' | 'ws';

export interface WorkflowVerifyStep {
  phase?: string;
  command?: string;
  fail_msg?: string;
  status?: string;
}

export interface WorkflowTask {
  id: string;
  title: string;
  description?: string;
  status?: string;
  raw_status?: string;
  route_tier?: string;
  routing_tier?: string;
  tier?: string;
  role?: string;
  model_hint?: string;
  selected_model?: string;
  max_loc?: number;
  files?: string[];
  depends_on?: string[];
  depends_on_plan?: string[];
  verify?: WorkflowVerifyStep[];
  phase?: string;
  agent_id?: string;
}

export interface WorkflowPlan {
  id: string;
  title: string;
  path?: string;
  status?: string;
  excerpt?: string;
  estimated_minutes?: number;
  tasks: WorkflowTask[];
}

export interface WorkflowPrd {
  slug: string;
  title: string;
  path?: string;
  status: 'idea' | 'draft' | 'published' | 'planned';
  excerpt?: string;
  requirements?: string[];
  acceptance?: string[];
}

export interface WorkflowSnapshot {
  id: string;
  title: string;
  phase: string;
  workdir: string;
  updated_at_millis?: number;
  prd?: WorkflowPrd;
  plans: WorkflowPlan[];
  live?: {
    events?: WorkflowLiveEvent[];
  };
}

interface WorkflowLiveEvent {
  timestamp_ms: number;
  event_type: string;
  plan_id: string;
  task_id: string;
  message: string;
}

interface WorkflowFrame {
  type: 'state' | 'delta' | 'ack' | 'error' | 'pong';
  channel?: string;
  cursor?: number;
  workflow_id?: string | null;
  workdir?: string;
  data?: WorkflowSnapshot | null;
  event?: unknown;
  message?: string;
}

export interface WorkflowSubscriptionHandlers {
  onSnapshot: (snapshot: WorkflowSnapshot, transport: Transport, cursor?: number) => void;
  onStatus: (patch: Partial<PipelineStreamState>) => void;
  onLiveEvent?: (event: WorkflowLiveEvent) => void;
  onError?: (message: string) => void;
}

function workflowQuery(root: string): string {
  return `root=${encodeURIComponent(root)}`;
}

export async function fetchWorkflowSnapshot(root: string, id = 'latest'): Promise<WorkflowSnapshot | null> {
  const res = await fetch(`${SERVE_URL}/api/workflows/${encodeURIComponent(id)}?${workflowQuery(root)}`);
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`workflow snapshot failed: ${res.status}`);
  return await res.json() as WorkflowSnapshot;
}

export function openWorkflowSubscriptions(root: string, handlers: WorkflowSubscriptionHandlers): () => void {
  let closed = false;
  let lastSnapshot = '';
  let latestLiveEventKey = '';
  const patchStatus = (patch: Partial<PipelineStreamState>) => {
    if (!closed) handlers.onStatus(patch);
  };

  const handleFrame = (frame: WorkflowFrame, transport: Transport) => {
    if (frame.type === 'error') {
      const message = frame.message ?? `${transport} workflow stream error`;
      handlers.onError?.(message);
      patchStatus({ message });
      return;
    }

    patchStatus({
      workflowId: frame.workflow_id ?? undefined,
      workdir: frame.workdir ?? root,
      cursor: frame.cursor,
      message: transport === 'sse' ? 'SSE projection stream connected' : 'WebSocket projection stream connected',
    });

    const snapshot = frame.data;
    if (!snapshot) return;
    const serialized = JSON.stringify(snapshot);
    if (serialized !== lastSnapshot) {
      lastSnapshot = serialized;
      handlers.onSnapshot(snapshot, transport, frame.cursor);
    }

    const liveEvents = snapshot.live?.events ?? [];
    const liveEvent = liveEvents[liveEvents.length - 1];
    if (liveEvent) {
      const key = `${liveEvent.timestamp_ms}:${liveEvent.event_type}:${liveEvent.plan_id}:${liveEvent.task_id}:${liveEvent.message}`;
      if (key !== latestLiveEventKey) {
        latestLiveEventKey = key;
        handlers.onLiveEvent?.(liveEvent);
      }
    }
  };

  const sse = new EventSource(`${SERVE_URL}/api/workflows/latest/stream?${workflowQuery(root)}`);
  patchStatus({ sse: 'connecting', ws: 'connecting', workdir: root, message: 'Connecting workflow streams' });

  sse.onopen = () => patchStatus({ sse: 'live', message: 'SSE projection stream connected' });
  sse.onerror = () => patchStatus({ sse: closed ? 'closed' : 'error', message: 'SSE stream disconnected' });
  sse.addEventListener('state', (event) => {
    try {
      handleFrame(JSON.parse((event as MessageEvent).data) as WorkflowFrame, 'sse');
    } catch (err) {
      handlers.onError?.(err instanceof Error ? err.message : String(err));
    }
  });
  sse.addEventListener('delta', (event) => {
    try {
      handleFrame(JSON.parse((event as MessageEvent).data) as WorkflowFrame, 'sse');
    } catch (err) {
      handlers.onError?.(err instanceof Error ? err.message : String(err));
    }
  });

  const ws = new WebSocket(`${WS_BASE}/api/workflow/ws`);
  ws.onopen = () => {
    patchStatus({ ws: 'live', message: 'WebSocket projection stream connected' });
    ws.send(JSON.stringify({
      type: 'subscribe',
      root,
      projections: ['workflow.artifacts', 'workflow.execution', 'workflow.gates', 'workflow.agents'],
    }));
  };
  ws.onerror = () => patchStatus({ ws: 'error', message: 'WebSocket workflow stream error' });
  ws.onclose = () => patchStatus({ ws: closed ? 'closed' : 'error', message: 'WebSocket workflow stream closed' });
  ws.onmessage = (event) => {
    try {
      handleFrame(JSON.parse(event.data) as WorkflowFrame, 'ws');
    } catch (err) {
      handlers.onError?.(err instanceof Error ? err.message : String(err));
    }
  };

  return () => {
    closed = true;
    sse.close();
    ws.close();
    patchStatus({ sse: 'closed', ws: 'closed', message: 'Workflow streams closed' });
  };
}

export function workflowSnapshotToPrd(snapshot: WorkflowSnapshot): PipelinePrd | undefined {
  if (!snapshot.prd) return undefined;
  return {
    slug: snapshot.prd.slug,
    title: snapshot.prd.title,
    path: snapshot.prd.path,
    status: snapshot.prd.status,
    excerpt: snapshot.prd.excerpt ?? '',
    requirements: snapshot.prd.requirements ?? [],
    acceptance: snapshot.prd.acceptance ?? [],
  };
}

export function workflowSnapshotToPlans(snapshot: WorkflowSnapshot): PipelinePlan[] {
  return snapshot.plans.map((plan) => {
    const tasks: PipelineTask[] = plan.tasks.map((task) => ({
      id: task.id,
      title: task.title,
      description: task.description,
      status: normalizePipelineTaskStatus(task.status),
      rawStatus: task.raw_status ?? task.status,
      routeTier: normalizePipelineRouteTier(
        task.route_tier ?? task.routing_tier ?? task.tier,
        task.selected_model ?? task.model_hint,
        task.role,
        task.max_loc,
        task.verify?.length ?? 0,
      ),
      tier: task.tier,
      role: task.role,
      modelHint: task.selected_model ?? task.model_hint,
      maxLoc: task.max_loc,
      files: task.files ?? [],
      dependsOn: task.depends_on ?? [],
      dependsOnPlan: task.depends_on_plan ?? [],
      phase: task.phase,
      agentId: task.agent_id,
      verify: (task.verify ?? [])
        .filter((step) => step.command)
        .map((step) => ({
          phase: step.phase ?? 'verify',
          command: step.command ?? '',
          failMsg: step.fail_msg,
          status: step.status === 'passed' || step.status === 'failed' ? step.status : 'pending',
        })),
    }));
    const done = tasks.filter((task) => task.status === 'done').length;
    const active = tasks.some((task) => task.status === 'active');
    const failed = tasks.some((task) => task.status === 'failed');
    return {
      id: plan.id,
      title: plan.title,
      path: plan.path,
      status: failed ? 'failed' : done === tasks.length && tasks.length > 0 ? 'complete' : active ? 'active' : 'pending',
      excerpt: plan.excerpt ?? '',
      estimatedMinutes: plan.estimated_minutes,
      tasks,
    };
  });
}

export function workflowPhaseToPipelinePhase(snapshot: WorkflowSnapshot): PipelinePhase {
  const phase = (snapshot.phase ?? '').toLowerCase();
  if (phase === 'idea') return 'idea';
  if (phase === 'draft') return 'draft';
  if (phase === 'published') return 'published';
  if (phase === 'planning') return 'planning';
  if (phase === 'tasks') return 'tasks';
  if (phase === 'implementing') return 'implementing';
  if (phase === 'complete') return 'complete';
  if (phase === 'failed') return 'failed';
  if (snapshot.plans.length > 0) return 'tasks';
  if (snapshot.prd) return snapshot.prd.status === 'planned' ? 'tasks' : snapshot.prd.status;
  return 'idle';
}

export function workflowHeadline(snapshot: WorkflowSnapshot): string {
  const phase = workflowPhaseToPipelinePhase(snapshot);
  if (phase === 'implementing') return `Implementing ${snapshot.title}`;
  if (phase === 'complete') return `Completed ${snapshot.title}`;
  if (phase === 'failed') return `Workflow failed for ${snapshot.title}`;
  if (phase === 'tasks') return 'Generated tasks, gates, and routing are ready';
  if (phase === 'published') return 'PRD published and ready for planning';
  if (phase === 'draft') return 'Structured PRD generated';
  return snapshot.title;
}
