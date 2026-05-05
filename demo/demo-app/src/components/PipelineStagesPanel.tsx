import { useEffect, useMemo, useState } from 'react';
import { useEventStreamContext } from '../contexts/EventStreamContext';
import {
  useInferenceCosts,
  useOperationEvents,
  usePipelineProgress,
} from '../hooks/useOperationEvents';
import './PipelineStagesPanel.css';

const PIPELINE_PROMPT = 'Build a Rust CLI that converts temperatures between Celsius and Fahrenheit';

const PIPELINE_EVENT_TYPES = [
  'run_started',
  'task_started',
  'task_completed',
  'gate_result',
  'run_completed',
  'inference_completed',
] as const;

type RecordValue = Record<string, unknown>;

type PipelineEvent = RecordValue & {
  data?: RecordValue;
  event?: RecordValue;
};

type TaskStatus = 'pending' | 'running' | 'done' | 'failed';
type StageStatus = 'pending' | 'active' | 'complete' | 'failed';

interface PipelineStagesPanelProps {
  operationId?: string | null;
  prompt?: string;
  isRunning?: boolean;
}

function isRecord(value: unknown): value is RecordValue {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function nested(event: PipelineEvent, key: 'data' | 'event'): RecordValue | null {
  return isRecord(event[key]) ? event[key] : null;
}

function records(event: PipelineEvent): RecordValue[] {
  return [event, nested(event, 'data'), nested(event, 'event')].filter(isRecord);
}

function readString(event: PipelineEvent, keys: string[]): string | undefined {
  for (const record of records(event)) {
    for (const key of keys) {
      const value = record[key];
      if (typeof value === 'string' && value.length > 0) return value;
    }
  }
  return undefined;
}

function readNumber(event: PipelineEvent, keys: string[]): number | undefined {
  for (const record of records(event)) {
    for (const key of keys) {
      const value = record[key];
      if (typeof value === 'number' && Number.isFinite(value)) return value;
    }
  }
  return undefined;
}

function eventType(event: PipelineEvent): string | undefined {
  return readString(event, ['type', 'kind']);
}

function eventOpId(event: PipelineEvent): string | undefined {
  return readString(event, [
    'operation_id',
    'operationId',
    'op_id',
    'opId',
    'run_id',
    'runId',
    'plan_id',
    'planId',
  ]);
}

function promptMatches(event: PipelineEvent, prompt: string): boolean {
  const eventPrompt = readString(event, ['prompt', 'idea', 'input']);
  return Boolean(eventPrompt && eventPrompt.includes(prompt));
}

function formatCost(value: number): string {
  if (value <= 0) return '--';
  return `$${value.toFixed(value < 0.01 ? 4 : 3)}`;
}

function formatTokens(value: number): string {
  if (value <= 0) return '--';
  return value >= 1000 ? `${(value / 1000).toFixed(1)}k` : String(value);
}

function formatTime(ms?: number): string {
  if (!ms || ms <= 0) return '--';
  return ms < 1000 ? `${Math.round(ms)}ms` : `${(ms / 1000).toFixed(1)}s`;
}

function statusLabel(status: TaskStatus | StageStatus): string {
  switch (status) {
    case 'complete':
    case 'done':
      return 'DONE';
    case 'active':
    case 'running':
      return 'RUN';
    case 'failed':
      return 'FAIL';
    default:
      return 'WAIT';
  }
}

function stageToTaskStatus(status: StageStatus): TaskStatus {
  if (status === 'complete') return 'done';
  if (status === 'active') return 'running';
  if (status === 'failed') return 'failed';
  return 'pending';
}

function gateState(gateName: string, gates: Array<{ gate: string; passed: boolean }>): TaskStatus {
  const matches = gates.filter((gate) => gate.gate.toLowerCase().includes(gateName));
  if (matches.some((gate) => !gate.passed)) return 'failed';
  if (matches.some((gate) => gate.passed)) return 'done';
  return 'pending';
}

function metricFromEvents(metricEvents: PipelineEvent[]) {
  let model = '--';
  let runDurationMs: number | undefined;
  let inferenceDurationMs = 0;

  for (const event of metricEvents) {
    const type = eventType(event);
    if (type === 'inference_completed') {
      model = readString(event, ['model']) ?? model;
      inferenceDurationMs += readNumber(event, ['duration_ms', 'durationMs']) ?? 0;
    }
    if (type === 'run_completed') {
      runDurationMs = readNumber(event, ['duration_ms', 'durationMs']) ?? runDurationMs;
    }
  }

  return {
    model,
    durationMs: runDurationMs ?? inferenceDurationMs,
  };
}

export default function PipelineStagesPanel({
  operationId = null,
  prompt = PIPELINE_PROMPT,
  isRunning = false,
}: PipelineStagesPanelProps) {
  const { connected, manager } = useEventStreamContext();
  const [discoveredOpId, setDiscoveredOpId] = useState<string | null>(null);
  const opId = operationId ?? discoveredOpId;
  const progress = usePipelineProgress(opId);
  const costs = useInferenceCosts(opId);
  const metricEvents = useOperationEvents(opId, ['inference_completed', 'run_completed']) as PipelineEvent[];

  useEffect(() => {
    if (operationId) {
      setDiscoveredOpId(operationId);
      return;
    }

    if (!manager) return;

    return manager.subscribe(['*'], (rawEvent: unknown) => {
      if (!isRecord(rawEvent)) return;
      const event = rawEvent as PipelineEvent;
      const type = eventType(event);
      if (type !== 'run_started' && type !== 'workflow_started') return;
      if (!promptMatches(event, prompt)) return;

      const nextOpId = eventOpId(event);
      if (nextOpId) setDiscoveredOpId(nextOpId);
    });
  }, [manager, operationId, prompt]);

  useEffect(() => {
    if (!isRunning) return;
    setDiscoveredOpId(operationId ?? null);
  }, [isRunning, operationId]);

  const stages = useMemo(() => {
    if (!opId || progress.started || progress.completed) return progress.stages;
    return progress.stages.map((stage) => {
      if (stage.id === 'classify') return { ...stage, status: 'complete' as const };
      if (stage.id === 'plan') return { ...stage, status: 'active' as const };
      return stage;
    });
  }, [opId, progress.completed, progress.started, progress.stages]);

  const tasks = useMemo(() => {
    if (progress.tasks.length > 0) {
      return progress.tasks.map((task) => ({
        id: task.id,
        title: task.title,
        status: task.status as TaskStatus,
      }));
    }

    const stageStatus = Object.fromEntries(
      stages.map((stage) => [stage.id, stage.status as StageStatus]),
    ) as Record<string, StageStatus>;

    return [
      { id: 'classify', title: 'Classify request', status: stageToTaskStatus(stageStatus.classify ?? 'pending') },
      { id: 'plan', title: 'Create implementation plan', status: stageToTaskStatus(stageStatus.plan ?? 'pending') },
      { id: 'execute', title: 'Generate Rust CLI code', status: stageToTaskStatus(stageStatus.execute ?? 'pending') },
      { id: 'gate', title: 'Run validation gates', status: stageToTaskStatus(stageStatus.gate ?? 'pending') },
    ];
  }, [progress.tasks, stages]);

  const gates = useMemo(() => [
    { name: 'compile', status: gateState('compile', progress.gates) },
    { name: 'clippy', status: gateState('clippy', progress.gates) },
    { name: 'test', status: gateState('test', progress.gates) },
  ], [progress.gates]);

  const metrics = useMemo(() => {
    const eventMetrics = metricFromEvents(metricEvents);
    return [
      { label: 'Cost', value: formatCost(costs.totalCost) },
      { label: 'Tokens', value: formatTokens(costs.totalTokens) },
      { label: 'Time', value: formatTime(eventMetrics.durationMs) },
      { label: 'Model', value: eventMetrics.model },
    ];
  }, [costs.totalCost, costs.totalTokens, metricEvents]);

  const panelState = progress.completed
    ? progress.success === false ? 'failed' : 'complete'
    : progress.started ? 'running' : 'pending';

  return (
    <section className="pipeline-stages-panel" aria-label="Pipeline stages">
      <header className="pipeline-stages-header">
        <div>
          <span className="pipeline-eyebrow">PIPELINE</span>
          <h3>Idea to Code</h3>
        </div>
        <div className={`pipeline-live-state pipeline-live-state--${panelState}`}>
          <span className="pipeline-live-dot" />
          <span>{panelState === 'pending' ? (connected ? 'ARMED' : 'OFFLINE') : panelState.toUpperCase()}</span>
        </div>
      </header>

      <div className="pipeline-command" title={`roko do "${prompt}"`}>
        <span>$</span>
        <code>roko do "{prompt}"</code>
      </div>

      <div className="pipeline-stage-track" aria-label={PIPELINE_EVENT_TYPES.join(', ')}>
        {stages.map((stage) => (
          <div
            key={stage.id}
            className={`pipeline-stage pipeline-stage--${stage.status}`}
          >
            <span className="pipeline-stage-light" />
            <span className="pipeline-stage-label">{stage.label}</span>
            <span className="pipeline-stage-status">{statusLabel(stage.status as StageStatus)}</span>
          </div>
        ))}
      </div>

      <div className="pipeline-section">
        <div className="pipeline-section-title">Tasks</div>
        <div className="pipeline-task-list">
          {tasks.map((task) => (
            <div key={task.id} className={`pipeline-task pipeline-task--${task.status}`}>
              <span className="pipeline-task-status">{statusLabel(task.status)}</span>
              <span className="pipeline-task-title">{task.title}</span>
            </div>
          ))}
        </div>
      </div>

      <div className="pipeline-section">
        <div className="pipeline-section-title">Gates</div>
        <div className="pipeline-gates">
          {gates.map((gate) => (
            <div key={gate.name} className={`pipeline-gate pipeline-gate--${gate.status}`}>
              <span>{gate.name}</span>
              <b>{statusLabel(gate.status)}</b>
            </div>
          ))}
        </div>
      </div>

      <div className="pipeline-metrics">
        {metrics.map((metric) => (
          <div key={metric.label} className="pipeline-metric">
            <span>{metric.label}</span>
            <b>{metric.value}</b>
          </div>
        ))}
      </div>

      {opId && (
        <footer className="pipeline-op-id">
          <span>op</span>
          <code>{opId}</code>
        </footer>
      )}
    </section>
  );
}
