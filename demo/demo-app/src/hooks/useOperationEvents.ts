import { useEffect, useMemo, useState } from 'react';
import { useEventStreamContext } from '../contexts/EventStreamContext';
import type { ServerEvent } from '../transport/types';

export type OperationEventType =
  | 'run_started'
  | 'run_completed'
  | 'plan_started'
  | 'plan_completed'
  | 'task_started'
  | 'task_completed'
  | 'task_failed'
  | 'gate_result'
  | 'inference_started'
  | 'inference_completed'
  | 'inference_failed'
  | 'operation_started'
  | 'operation_completed'
  | 'agent_output'
  | 'agent_trace'
  | 'knowledge_ingested'
  | 'knowledge_consumed'
  | 'execution'
  | (string & {});

export type OperationEvent = Record<string, unknown> & {
  type: ServerEvent['type'] | OperationEventType;
  data?: Record<string, unknown>;
  event?: Record<string, unknown>;
};

export interface InferenceCostSummary {
  totalCost: number;
  totalTokens: number;
  inputTokens: number;
  outputTokens: number;
  calls: number;
}

export type PipelineStageId = 'classify' | 'plan' | 'execute' | 'gate' | 'done';
export type PipelineStageStatus = 'pending' | 'active' | 'complete' | 'failed';

export interface PipelineStageProgress {
  id: PipelineStageId;
  label: string;
  status: PipelineStageStatus;
}

export interface PipelineTaskProgress {
  id: string;
  title: string;
  status: 'running' | 'done' | 'failed';
  gates: PipelineGateProgress[];
}

export interface PipelineGateProgress {
  taskId?: string;
  gate: string;
  passed: boolean;
  message?: string;
}

export interface PipelineProgress {
  stages: PipelineStageProgress[];
  currentStage: PipelineStageId | null;
  tasks: PipelineTaskProgress[];
  gates: PipelineGateProgress[];
  started: boolean;
  completed: boolean;
  success: boolean | null;
  progress: number;
}

const MAX_OPERATION_EVENTS = 500;

const PIPELINE_EVENT_TYPES: OperationEventType[] = [
  'workflow_started',
  'phase_transition',
  'agent_spawned',
  'agent_completed',
  'agent_failed',
  'gate_started',
  'gate_passed',
  'gate_failed',
  'workflow_completed',
  'run_started',
  'plan_started',
  'task_started',
  'task_completed',
  'task_failed',
  'gate_result',
  'plan_completed',
  'run_completed',
  'execution',
];

const STAGE_LABELS: Record<PipelineStageId, string> = {
  classify: 'Classify',
  plan: 'Plan',
  execute: 'Execute',
  gate: 'Gate',
  done: 'Done',
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function normalizeEvent(event: unknown): OperationEvent | null {
  if (!isRecord(event)) return null;

  if (typeof event.type === 'string') {
    return event as OperationEvent;
  }

  if (typeof event.kind === 'string') {
    return { ...event, type: event.kind } as OperationEvent;
  }

  return null;
}

function nestedRecord(event: OperationEvent, key: 'data' | 'event'): Record<string, unknown> | null {
  const value = event[key];
  return isRecord(value) ? value : null;
}

function readStringFrom(
  record: Record<string, unknown> | null,
  keys: string[],
): string | undefined {
  if (!record) return undefined;

  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.length > 0) return value;
  }

  return undefined;
}

function readNumber(event: OperationEvent, keys: string[]): number {
  for (const record of [event, nestedRecord(event, 'data'), nestedRecord(event, 'event')]) {
    if (!record) continue;

    for (const key of keys) {
      const value = record[key];
      if (typeof value === 'number' && Number.isFinite(value)) return value;
    }
  }

  return 0;
}

function readBoolean(event: OperationEvent, keys: string[]): boolean | null {
  for (const record of [event, nestedRecord(event, 'data'), nestedRecord(event, 'event')]) {
    if (!record) continue;

    for (const key of keys) {
      const value = record[key];
      if (typeof value === 'boolean') return value;
    }
  }

  return null;
}

function operationIds(event: OperationEvent): string[] {
  const idKeys = [
    'operation_id',
    'operationId',
    'op_id',
    'opId',
    'run_id',
    'runId',
    'plan_id',
    'planId',
    'request_id',
    'requestId',
  ];
  const ids = new Set<string>();

  for (const record of [event, nestedRecord(event, 'data'), nestedRecord(event, 'event')]) {
    const id = readStringFrom(record, idKeys);
    if (id) ids.add(id);
  }

  return [...ids];
}

function eventType(event: OperationEvent): string {
  return event.type;
}

function logicalEventType(event: OperationEvent): string {
  const nested = nestedRecord(event, 'event');
  const nestedType = readStringFrom(nested, ['type']);
  return nestedType ?? eventType(event);
}

function eventMatchesTypes(event: OperationEvent, eventTypes?: readonly string[]): boolean {
  if (!eventTypes || eventTypes.length === 0) return true;
  const allowed = new Set(eventTypes);
  return allowed.has(eventType(event)) || allowed.has(logicalEventType(event));
}

function readTaskId(event: OperationEvent): string | undefined {
  return readStringFrom(event, ['task_id', 'taskId'])
    ?? readStringFrom(nestedRecord(event, 'data'), ['task_id', 'taskId'])
    ?? readStringFrom(nestedRecord(event, 'event'), ['task_id', 'taskId']);
}

function readTaskTitle(event: OperationEvent): string {
  return readStringFrom(event, ['title', 'description', 'task_name', 'taskName'])
    ?? readStringFrom(nestedRecord(event, 'data'), ['title', 'description', 'task_name', 'taskName'])
    ?? readStringFrom(nestedRecord(event, 'event'), ['title', 'description', 'task_name', 'taskName'])
    ?? readTaskId(event)
    ?? 'Task';
}

function readGate(event: OperationEvent): PipelineGateProgress {
  return {
    taskId: readTaskId(event),
    gate: readStringFrom(event, ['gate', 'gate_name', 'gateName', 'name'])
      ?? readStringFrom(nestedRecord(event, 'data'), ['gate', 'gate_name', 'gateName', 'name'])
      ?? readStringFrom(nestedRecord(event, 'event'), ['gate', 'gate_name', 'gateName', 'name'])
      ?? 'gate',
    passed: readBoolean(event, ['passed', 'success']) ?? false,
    message: readStringFrom(event, ['message', 'output', 'error'])
      ?? readStringFrom(nestedRecord(event, 'data'), ['message', 'output', 'error'])
      ?? readStringFrom(nestedRecord(event, 'event'), ['message', 'output', 'error']),
  };
}

function setStage(
  stages: Map<PipelineStageId, PipelineStageStatus>,
  id: PipelineStageId,
  status: PipelineStageStatus,
) {
  const current = stages.get(id);
  if (current === 'failed') return;
  if (current === 'complete' && status === 'active') return;
  stages.set(id, status);
}

function completeThrough(
  stages: Map<PipelineStageId, PipelineStageStatus>,
  ids: PipelineStageId[],
) {
  for (const id of ids) setStage(stages, id, 'complete');
}

function stageList(stages: Map<PipelineStageId, PipelineStageStatus>): PipelineStageProgress[] {
  return (Object.keys(STAGE_LABELS) as PipelineStageId[]).map((id) => ({
    id,
    label: STAGE_LABELS[id],
    status: stages.get(id) ?? 'pending',
  }));
}

function derivePipelineProgress(events: OperationEvent[]): PipelineProgress {
  const stages = new Map<PipelineStageId, PipelineStageStatus>(
    (Object.keys(STAGE_LABELS) as PipelineStageId[]).map((id) => [id, 'pending']),
  );
  const tasks = new Map<string, PipelineTaskProgress>();
  const gates: PipelineGateProgress[] = [];
  let started = false;
  let completed = false;
  let success: boolean | null = null;

  for (const event of events) {
    const type = logicalEventType(event);

    if (type === 'workflow_started' || type === 'run_started') {
      started = true;
      completeThrough(stages, ['classify']);
      setStage(stages, 'plan', 'active');
      continue;
    }

    if (type === 'phase_transition') {
      started = true;
      const phase = readStringFrom(event, ['to', 'phase'])
        ?? readStringFrom(nestedRecord(event, 'data'), ['to', 'phase']);

      if (phase === 'strategizing' || phase === 'planning') {
        completeThrough(stages, ['classify']);
        setStage(stages, 'plan', 'active');
      } else if (phase === 'implementing') {
        completeThrough(stages, ['classify', 'plan']);
        setStage(stages, 'execute', 'active');
      } else if (phase === 'gating') {
        completeThrough(stages, ['classify', 'plan', 'execute']);
        setStage(stages, 'gate', 'active');
      } else if (phase === 'reviewing' || phase === 'committing' || phase === 'complete') {
        completeThrough(stages, ['classify', 'plan', 'execute', 'gate']);
        setStage(stages, 'done', phase === 'complete' ? 'complete' : 'active');
      } else if (phase === 'halted' || phase === 'cancelled') {
        setStage(stages, 'done', 'failed');
        success = false;
      }
      continue;
    }

    if (type === 'plan_started') {
      started = true;
      completeThrough(stages, ['classify']);
      setStage(stages, 'plan', 'active');
      continue;
    }

    if (type === 'agent_spawned' || type === 'task_started') {
      started = true;
      completeThrough(stages, ['classify', 'plan']);
      setStage(stages, 'execute', 'active');

      const taskId = readTaskId(event);
      if (taskId) {
        const existing = tasks.get(taskId);
        tasks.set(taskId, {
          id: taskId,
          title: existing?.title ?? readTaskTitle(event),
          status: 'running',
          gates: existing?.gates ?? [],
        });
      }
      continue;
    }

    if (type === 'agent_completed' || type === 'task_completed') {
      completeThrough(stages, ['classify', 'plan', 'execute']);
      setStage(stages, 'gate', 'active');

      const taskId = readTaskId(event);
      if (taskId) {
        const existing = tasks.get(taskId);
        tasks.set(taskId, {
          id: taskId,
          title: existing?.title ?? readTaskTitle(event),
          status: 'done',
          gates: existing?.gates ?? [],
        });
      }
      continue;
    }

    if (type === 'agent_failed' || type === 'task_failed') {
      completeThrough(stages, ['classify', 'plan']);
      setStage(stages, 'execute', 'failed');
      setStage(stages, 'done', 'failed');
      success = false;

      const taskId = readTaskId(event);
      if (taskId) {
        const existing = tasks.get(taskId);
        tasks.set(taskId, {
          id: taskId,
          title: existing?.title ?? readTaskTitle(event),
          status: 'failed',
          gates: existing?.gates ?? [],
        });
      }
      continue;
    }

    if (type === 'gate_started') {
      completeThrough(stages, ['classify', 'plan', 'execute']);
      setStage(stages, 'gate', 'active');
      continue;
    }

    if (type === 'gate_passed' || type === 'gate_failed' || type === 'gate_result') {
      completeThrough(stages, ['classify', 'plan', 'execute']);
      const gate = type === 'gate_failed'
        ? { ...readGate(event), passed: false }
        : type === 'gate_passed'
          ? { ...readGate(event), passed: true }
          : readGate(event);
      gates.push(gate);
      setStage(stages, 'gate', gate.passed ? 'complete' : 'failed');
      if (!gate.passed) success = false;

      if (gate.taskId) {
        const existing = tasks.get(gate.taskId);
        tasks.set(gate.taskId, {
          id: gate.taskId,
          title: existing?.title ?? gate.taskId,
          status: existing?.status ?? 'running',
          gates: [...(existing?.gates ?? []), gate],
        });
      }
      continue;
    }

    if (type === 'plan_completed') {
      completeThrough(stages, ['classify', 'plan', 'execute']);
      setStage(stages, 'gate', gates.length > 0 ? 'complete' : 'active');
      continue;
    }

    if (type === 'workflow_completed' || type === 'run_completed') {
      completed = true;
      const outcome = readStringFrom(event, ['outcome'])
        ?? readStringFrom(nestedRecord(event, 'data'), ['outcome']);
      success = readBoolean(event, ['success', 'passed'])
        ?? (outcome ? !(outcome.includes('halted') || outcome.includes('cancelled')) : null);
      completeThrough(stages, ['classify', 'plan', 'execute', 'gate']);
      setStage(stages, 'done', success === false ? 'failed' : 'complete');
    }
  }

  const stageProgress = stageList(stages);
  const currentStage = stageProgress.find((stage) => stage.status === 'active')?.id
    ?? stageProgress.find((stage) => stage.status === 'failed')?.id
    ?? null;
  const progress = stageProgress.reduce((sum, stage) => {
    if (stage.status === 'complete') return sum + 1;
    if (stage.status === 'active') return sum + 0.5;
    return sum;
  }, 0) / stageProgress.length;

  return {
    stages: stageProgress,
    currentStage,
    tasks: [...tasks.values()],
    gates,
    started,
    completed,
    success,
    progress,
  };
}

export function useOperationEvents(
  operationId: string | null,
  eventTypes?: readonly OperationEventType[],
): OperationEvent[] {
  const { manager } = useEventStreamContext();
  const [events, setEvents] = useState<OperationEvent[]>([]);
  const typeFilterKey = eventTypes?.join('\u0000') ?? '*';
  const eventTypeFilters = useMemo(
    () => (eventTypes && eventTypes.length > 0
      ? typeFilterKey.split('\u0000')
      : undefined),
    [typeFilterKey],
  );

  useEffect(() => {
    setEvents([]);
    if (!manager || !operationId) return;

    return manager.subscribe(['*'], (event: unknown) => {
      const normalized = normalizeEvent(event);
      if (!normalized) return;
      if (!operationIds(normalized).includes(operationId)) return;
      if (!eventMatchesTypes(normalized, eventTypeFilters)) return;

      setEvents((prev) => [...prev.slice(-(MAX_OPERATION_EVENTS - 1)), normalized]);
    });
  }, [manager, operationId, eventTypeFilters]);

  return events;
}

export function useInferenceCosts(operationId: string | null): InferenceCostSummary {
  const events = useOperationEvents(operationId, ['inference_completed']);

  return useMemo(
    () => events.reduce<InferenceCostSummary>((acc, event) => {
      const inputTokens = readNumber(event, ['input_tokens', 'inputTokens', 'tokens_in', 'tokensIn']);
      const outputTokens = readNumber(event, ['output_tokens', 'outputTokens', 'tokens_out', 'tokensOut']);
      const fallbackTokens = readNumber(event, ['total_tokens', 'totalTokens', 'tokens', 'tokens_used', 'tokensUsed']);

      return {
        totalCost: acc.totalCost + readNumber(event, ['cost_usd', 'costUsd', 'cost']),
        totalTokens: acc.totalTokens + (inputTokens + outputTokens || fallbackTokens),
        inputTokens: acc.inputTokens + inputTokens,
        outputTokens: acc.outputTokens + outputTokens,
        calls: acc.calls + 1,
      };
    }, {
      totalCost: 0,
      totalTokens: 0,
      inputTokens: 0,
      outputTokens: 0,
      calls: 0,
    }),
    [events],
  );
}

export function usePipelineProgress(operationId: string | null): PipelineProgress {
  const events = useOperationEvents(operationId, PIPELINE_EVENT_TYPES);

  return useMemo(() => derivePipelineProgress(events), [events]);
}
