import { useMemo } from 'react';
import type {
  PipelineDemoState,
  PipelineEvent,
  PipelinePhase,
  PipelinePlan,
  PipelineRouteTier,
  PipelineScenarioExample,
  PipelineTask,
  PipelineTaskStatus,
} from '../lib/prd-pipeline-types';
import type { ServerStatus } from '../hooks/useServerHealth';
import './PrdPipelinePanel.css';

/* ── Constants ── */

const PHASES: { id: PipelinePhase; label: string }[] = [
  { id: 'idea', label: 'Idea' },
  { id: 'draft', label: 'PRD' },
  { id: 'planning', label: 'Plan' },
  { id: 'tasks', label: 'Tasks' },
  { id: 'implementing', label: 'Run' },
  { id: 'complete', label: 'Done' },
];

const PHASE_ORDER: PipelinePhase[] = [
  'idle', 'setup', 'idea', 'draft', 'published',
  'planning', 'tasks', 'implementing', 'complete', 'failed',
];

/* ── Helpers ── */

function phaseIndex(phase: PipelinePhase): number {
  return PHASE_ORDER.indexOf(phase);
}

function statusLabel(status: PipelineTaskStatus): string {
  if (status === 'active') return 'working';
  if (status === 'done') return 'done';
  if (status === 'failed') return 'failed';
  if (status === 'blocked') return 'blocked';
  return 'pending';
}

function tierLabel(tier: PipelineRouteTier): string {
  if (tier === 'T1') return 'T1 fast';
  if (tier === 'T2') return 'T2 build';
  return 'T3 risk';
}

function planProgress(plan: PipelinePlan) {
  const total = plan.tasks.length;
  const done = plan.tasks.filter((t) => t.status === 'done').length;
  const active = plan.tasks.filter((t) => t.status === 'active').length;
  return { done, total, active };
}

function totalProgress(plans: PipelinePlan[]) {
  return plans.reduce(
    (acc, p) => {
      const s = planProgress(p);
      return { done: acc.done + s.done, total: acc.total + s.total, active: acc.active + s.active };
    },
    { done: 0, total: 0, active: 0 },
  );
}

function allTasks(plans: PipelinePlan[]): PipelineTask[] {
  return plans.flatMap((p) => p.tasks);
}

function routeSummary(plans: PipelinePlan[]) {
  const out: Record<PipelineRouteTier, { count: number; active: number; models: Set<string> }> = {
    T1: { count: 0, active: 0, models: new Set() },
    T2: { count: 0, active: 0, models: new Set() },
    T3: { count: 0, active: 0, models: new Set() },
  };
  for (const t of allTasks(plans)) {
    const tier = t.routeTier ?? 'T1';
    out[tier].count += 1;
    if (t.status === 'active') out[tier].active += 1;
    if (t.modelHint) out[tier].models.add(t.modelHint);
  }
  return out;
}

function gateSummary(plans: PipelinePlan[]) {
  const counts = new Map<string, { count: number; command?: string }>();
  for (const t of allTasks(plans)) {
    for (const v of t.verify) {
      const phase = v.phase || 'verify';
      const prev = counts.get(phase) ?? { count: 0, command: v.command };
      prev.count += 1;
      prev.command = prev.command ?? v.command;
      counts.set(phase, prev);
    }
  }
  return Array.from(counts.entries())
    .map(([phase, d]) => ({ phase, ...d }))
    .sort((a, b) => b.count - a.count || a.phase.localeCompare(b.phase));
}

function connectionDot(status?: string): string {
  return `pp-conn pp-conn-${status ?? 'idle'}`;
}

function runBtnLabel(status: ServerStatus, running: boolean): string {
  if (running) return 'Running';
  if (status === 'checking') return 'Checking';
  if (status === 'disconnected') return 'Offline';
  return 'Start live run';
}

/* ── Main component ── */

export default function PrdPipelinePanel({
  state,
  examples = [],
  selectedExampleId,
  onSelectExample,
  selectorDisabled,
  onRun,
  isRunning = false,
  serverHealth = 'checking',
}: {
  state: PipelineDemoState;
  examples?: PipelineScenarioExample[];
  selectedExampleId?: string;
  onSelectExample?: (id: string) => void;
  selectorDisabled?: boolean;
  onRun?: () => void;
  isRunning?: boolean;
  serverHealth?: ServerStatus;
}) {
  const progress = useMemo(() => totalProgress(state.plans), [state.plans]);
  const currentPhase = phaseIndex(state.phase);
  const tasks = useMemo(() => allTasks(state.plans), [state.plans]);
  const routes = useMemo(() => routeSummary(state.plans), [state.plans]);
  const gates = useMemo(() => gateSummary(state.plans), [state.plans]);

  const hasPrd = !!state.prd;
  const hasPlans = state.plans.length > 0;
  const hasTasks = tasks.length > 0;
  const hasEvents = state.events.length > 0;
  const pctDone = progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0;

  return (
    <div className="pp">
      {/* ── Top: two columns ── */}
      <div className="pp-top">
        {/* Left: job brief */}
        <div className="pp-brief">
          <div className="pp-kicker">
            {state.source === 'live' ? 'live' : 'ready'}
            {state.example ? ` · ${state.example.complexity}` : ''}
          </div>
          <h2 className="pp-title">{state.headline}</h2>

          {state.example?.stageQuote && (
            <blockquote className="pp-quote">{state.example.stageQuote}</blockquote>
          )}
          {state.example && <p className="pp-idea">{state.example.idea}</p>}

        </div>

        {/* Right: controls + progress */}
        <div className="pp-sidebar">
          {onRun && (
            <button
              className="pp-run-btn"
              onClick={onRun}
              disabled={isRunning || serverHealth !== 'connected'}
            >
              {runBtnLabel(serverHealth, isRunning)}
            </button>
          )}

          <div className="pp-score">
            {progress.total > 0 && (
              <div className="pp-ring">
                <svg viewBox="0 0 36 36">
                  <circle className="pp-ring-bg" cx="18" cy="18" r="15.5" />
                  <circle
                    className="pp-ring-fg"
                    cx="18" cy="18" r="15.5"
                    strokeDasharray={`${pctDone} ${100 - pctDone}`}
                    strokeDashoffset="25"
                  />
                </svg>
                <span className="pp-ring-label">{pctDone}%</span>
              </div>
            )}
            <div className="pp-score-text">
              <span className="pp-score-num">{progress.done}</span>
              <span className="pp-score-sep">/ {progress.total || '--'}</span>
              <span className="pp-score-sub">
                {progress.active > 0 ? `${progress.active} active` : 'tasks'}
              </span>
            </div>
          </div>

          {state.stream && (
            <div className="pp-stream">
              <span className={connectionDot(state.stream.sse)}>SSE</span>
              <span className={connectionDot(state.stream.ws)}>WS</span>
            </div>
          )}
        </div>
      </div>

      {/* ── Example selector ── */}
      {examples.length > 0 && onSelectExample && (
        <div className="pp-examples">
          {examples.map((ex) => (
            <button
              key={ex.id}
              className={selectedExampleId === ex.id ? 'active' : ''}
              onClick={() => onSelectExample(ex.id)}
              disabled={selectorDisabled}
            >
              <b>{ex.label}</b>
              <span>{ex.complexity}</span>
            </button>
          ))}
        </div>
      )}

      {/* ── Phase rail ── */}
      <div className="pp-rail">
        {PHASES.map((phase) => {
          const idx = phaseIndex(phase.id);
          const s = state.phase === 'failed'
            ? idx < currentPhase ? 'done' : 'pending'
            : idx < currentPhase ? 'done' : idx === currentPhase ? 'active' : 'pending';
          return (
            <div key={phase.id} className={`pp-phase pp-phase-${s}`}>
              <span />
              <b>{phase.label}</b>
            </div>
          );
        })}
      </div>

      {/* ── Current command ── */}
      {state.currentCommand && (
        <div className="pp-cmd">{state.currentCommand}</div>
      )}

      {/* ── PRD card (appears when generated) ── */}
      {hasPrd && state.prd && (
        <section className="pp-section pp-reveal">
          <div className="pp-section-head">
            <span>PRD</span>
            <b>{state.prd.status}</b>
          </div>
          <h3>{state.prd.title}</h3>
          <p className="pp-excerpt">{state.prd.excerpt}</p>
          <div className="pp-prd-stats">
            <span>{state.prd.requirements.length} requirements</span>
            <span>{state.prd.acceptance.length} acceptance</span>
            <span className="mono">{state.prd.slug}</span>
          </div>
        </section>
      )}

      {/* ── Plans + routing two-column grid ── */}
      {hasPlans && (
        <div className="pp-grid pp-reveal">
          <section className="pp-section">
            <div className="pp-section-head">
              <span>Plans</span>
              <b>{state.plans.length}</b>
            </div>
            <div className="pp-plan-list">
              {state.plans.map((plan) => (
                <PlanCard key={plan.id} plan={plan} />
              ))}
            </div>
          </section>

          {hasTasks && (
            <section className="pp-section">
              <div className="pp-section-head">
                <span>Routing</span>
                <b>{tasks.length} tasks</b>
              </div>
              <div className="pp-route-grid">
                {(['T1', 'T2', 'T3'] as PipelineRouteTier[]).map((tier) => {
                  const d = routes[tier];
                  if (d.count === 0) return null;
                  return (
                    <div key={tier} className={`pp-route-card pp-tier-${tier.toLowerCase()}`}>
                      <b>{d.count}</b>
                      <span>{tierLabel(tier)}</span>
                      {d.active > 0 && <em>{d.active} active</em>}
                    </div>
                  );
                })}
              </div>
              {gates.length > 0 && (
                <div className="pp-gates">
                  {gates.slice(0, 6).map((g) => (
                    <span key={g.phase} title={g.command}>
                      <b>{g.phase}</b> {g.count}
                    </span>
                  ))}
                </div>
              )}
            </section>
          )}
        </div>
      )}

      {/* ── Task board ── */}
      {hasTasks && (
        <section className="pp-section pp-reveal">
          <div className="pp-section-head">
            <span>Tasks</span>
            <b>{progress.done}/{progress.total}</b>
          </div>
          <div className="pp-task-list">
            {state.plans.map((plan) =>
              plan.tasks.map((task) => (
                <TaskRow key={`${plan.id}:${task.id}`} planId={plan.id} task={task} />
              )),
            )}
          </div>
        </section>
      )}

      {/* ── Event log ── */}
      {hasEvents && (
        <section className="pp-section pp-section-log pp-reveal">
          <div className="pp-section-head">
            <span>Log</span>
            <b>{state.lastUpdated ?? ''}</b>
          </div>
          <div className="pp-event-list">
            {state.events.slice(-5).map((ev) => (
              <EventRow key={ev.id} event={ev} />
            ))}
          </div>
        </section>
      )}
    </div>
  );
}

/* ── Sub-components ── */

function PlanCard({ plan }: { plan: PipelinePlan }) {
  const { done, total } = planProgress(plan);
  const pct = total === 0 ? 0 : Math.round((done / total) * 100);

  return (
    <article className={`pp-plan pp-plan-${plan.status}`}>
      <div className="pp-plan-head">
        <div>
          <h4>{plan.title}</h4>
          <span className="pp-plan-id">{plan.id}</span>
        </div>
        <b className="pp-plan-pct">{pct}%</b>
      </div>
      <div className="pp-plan-bar">
        <span style={{ width: `${pct}%` }} />
      </div>
      {plan.excerpt && <p className="pp-excerpt">{plan.excerpt}</p>}
      <div className="pp-plan-meta">
        <span>{total} tasks</span>
        {plan.estimatedMinutes != null && <span>{plan.estimatedMinutes}m</span>}
      </div>
    </article>
  );
}

function TaskRow({ planId, task }: { planId: string; task: PipelineTask }) {
  const tier: PipelineRouteTier = task.routeTier ?? 'T1';
  return (
    <article className={`pp-task pp-task-${task.status}`}>
      <div className="pp-task-dot" />
      <div className="pp-task-body">
        <div className="pp-task-top">
          <h4>{task.title}</h4>
          <div className="pp-task-badges">
            <b className={`pp-tier-badge pp-tier-${tier.toLowerCase()}`}>{tier}</b>
            <span className="pp-task-status-label">{statusLabel(task.status)}</span>
          </div>
        </div>
        {task.description && <p>{task.description}</p>}
        <div className="pp-task-meta">
          <span>{planId}:{task.id}</span>
          {task.modelHint && <span>{task.modelHint}</span>}
          {task.role && <span>{task.role}</span>}
          {task.files.length > 0 && <span>{task.files.length} files</span>}
        </div>
        {task.verify.length > 0 && (
          <div className="pp-task-gates">
            {task.verify.slice(0, 3).map((v, i) => (
              <code key={`${task.id}-v-${i}`} className={`pp-gate-${v.status ?? 'pending'}`}>
                {v.phase}: {v.command}
              </code>
            ))}
            {task.verify.length > 3 && <em>+{task.verify.length - 3}</em>}
          </div>
        )}
      </div>
    </article>
  );
}

function EventRow({ event }: { event: PipelineEvent }) {
  return (
    <div className={`pp-event pp-event-${event.kind ?? 'info'}`}>
      <span className="pp-event-ts">{event.ts}</span>
      <b>{event.phase}</b>
      <p>{event.text}</p>
    </div>
  );
}
