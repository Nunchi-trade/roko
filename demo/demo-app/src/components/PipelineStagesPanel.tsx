import { useMemo } from 'react';
import type { RunMetrics } from './CostComparisonPanel';
import './PipelineStagesPanel.css';

const PIPELINE_PROMPT = 'Build a Rust CLI that converts temperatures between Celsius and Fahrenheit';

export interface PipelineMetrics extends RunMetrics {
  model: string;
}

export const EMPTY_PIPELINE_METRICS: PipelineMetrics = { cost: 0, tokens: 0, calls: 0, elapsed: 0, model: '--' };

interface PipelineStagesPanelProps {
  metrics: PipelineMetrics;
  stages?: StageInfo[];
  gates?: GateInfo[];
  isRunning?: boolean;
}

interface StageInfo {
  id: string;
  label: string;
  status: 'pending' | 'active' | 'complete' | 'failed';
}

interface GateInfo {
  name: string;
  status: 'pending' | 'done' | 'failed';
}

function formatCost(value: number): string {
  if (value <= 0) return '--';
  return `$${value.toFixed(value < 0.01 ? 4 : 3)}`;
}

function formatTokens(value: number): string {
  if (value <= 0) return '--';
  return value >= 1000 ? `${(value / 1000).toFixed(1)}k` : String(value);
}

function formatTime(s: number): string {
  if (!s || s <= 0) return '--';
  return s < 1 ? `${Math.round(s * 1000)}ms` : `${s.toFixed(1)}s`;
}

function statusLabel(status: string): string {
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

const DEFAULT_STAGES: StageInfo[] = [
  { id: 'classify', label: 'Classify', status: 'pending' },
  { id: 'plan', label: 'Plan', status: 'pending' },
  { id: 'execute', label: 'Execute', status: 'pending' },
  { id: 'gate', label: 'Gate', status: 'pending' },
];

const DEFAULT_GATES: GateInfo[] = [
  { name: 'compile', status: 'pending' },
  { name: 'clippy', status: 'pending' },
  { name: 'test', status: 'pending' },
];

export default function PipelineStagesPanel({
  metrics,
  stages: stagesProp,
  gates: gatesProp,
  isRunning = false,
}: PipelineStagesPanelProps) {
  const stages = stagesProp ?? DEFAULT_STAGES;
  const gates = gatesProp ?? DEFAULT_GATES;

  const hasData = metrics.calls > 0 || metrics.cost > 0;
  const allComplete = stages.every((s) => s.status === 'complete');
  const anyFailed = stages.some((s) => s.status === 'failed') || gates.some((g) => g.status === 'failed');

  const panelState = anyFailed
    ? 'failed'
    : allComplete ? 'complete'
    : isRunning ? 'running'
    : 'pending';

  const tasks = useMemo(() => stages.map((stage) => ({
    id: stage.id,
    title: stage.id === 'classify' ? 'Classify request'
      : stage.id === 'plan' ? 'Create implementation plan'
      : stage.id === 'execute' ? 'Generate Rust CLI code'
      : stage.id === 'gate' ? 'Run validation gates'
      : stage.label,
    status: stage.status === 'complete' ? 'done' as const
      : stage.status === 'active' ? 'running' as const
      : stage.status as 'pending' | 'failed',
  })), [stages]);

  const metricRows = [
    { label: 'Cost', value: formatCost(metrics.cost) },
    { label: 'Tokens', value: formatTokens(metrics.tokens) },
    { label: 'Time', value: formatTime(metrics.elapsed) },
    { label: 'Model', value: metrics.model },
  ];

  return (
    <section className="pipeline-stages-panel" aria-label="Pipeline stages">
      <header className="pipeline-stages-header">
        <div>
          <span className="pipeline-eyebrow">PIPELINE</span>
          <h3>Idea to Code</h3>
        </div>
        <div className={`pipeline-live-state pipeline-live-state--${panelState}`}>
          <span className="pipeline-live-dot" />
          <span>{panelState === 'pending' ? (hasData ? 'READY' : 'ARMED') : panelState.toUpperCase()}</span>
        </div>
      </header>

      <div className="pipeline-description">
        A single command drives the full pipeline: classify the request, plan the implementation,
        generate Rust code, then validate with compile, clippy, and test gates — all automated.
      </div>

      <div className="pipeline-command" title={`roko do "${PIPELINE_PROMPT}"`}>
        <span>$</span>
        <code>roko do "{PIPELINE_PROMPT}"</code>
      </div>

      <div className="pipeline-stage-track">
        {stages.map((stage) => (
          <div
            key={stage.id}
            className={`pipeline-stage pipeline-stage--${stage.status}`}
          >
            <span className="pipeline-stage-light" />
            <span className="pipeline-stage-label">{stage.label}</span>
            <span className="pipeline-stage-status">{statusLabel(stage.status)}</span>
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
        {metricRows.map((metric) => (
          <div key={metric.label} className="pipeline-metric">
            <span>{metric.label}</span>
            <b>{metric.value}</b>
          </div>
        ))}
      </div>
    </section>
  );
}
