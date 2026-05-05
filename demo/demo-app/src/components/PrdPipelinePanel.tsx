import { useState, useMemo, useCallback } from 'react';
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
import { ConfidenceMeter } from './inference';
import './PrdPipelinePanel.css';

/* ── Constants ── */

const PHASES: { id: PipelinePhase; label: string; description: string }[] = [
  { id: 'idea', label: 'Idea', description: 'Capture a work item from a one-line prompt' },
  { id: 'draft', label: 'PRD', description: 'Agent drafts a product requirements document' },
  { id: 'planning', label: 'Plan', description: 'Agent generates implementation plan from PRD' },
  { id: 'tasks', label: 'Tasks', description: 'Plan is decomposed into routed, gated tasks' },
  { id: 'implementing', label: 'Run', description: 'Agents execute tasks with verification gates' },
  { id: 'complete', label: 'Done', description: 'All tasks passed gates; pipeline complete' },
];

const EXAMPLE_PREVIEWS: Record<string, string[]> = {
  'simple-status': [
    'Add `status` and `status --json` subcommands',
    'Single file, no network dependencies',
    'Cargo test verification gate',
  ],
  'release-watch': [
    'HTTP client for GitHub releases API',
    'Offline fixture tests, version comparison logic',
    'Mix of T1 verification and T2 implementation',
  ],
  'funding-alert': [
    'Hyperliquid API client for BTC funding rates',
    'Email notifier with dry-run mode',
    'T1/T2/T3 routing across 4+ task domains',
  ],
};

const COMPLEXITY_COLORS: Record<string, string> = {
  'Super simple': 'green',
  'Slightly more complex': 'amber',
  'Stage job': 'rose',
};

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
  return 'Start live run';
}

function phaseStatus(
  phaseId: PipelinePhase,
  currentPhase: PipelinePhase,
  currentIndex: number,
): 'done' | 'active' | 'pending' | 'failed' {
  const idx = phaseIndex(phaseId);
  if (currentPhase === 'failed') {
    if (idx < currentIndex) return 'done';
    if (idx === currentIndex) return 'failed';
    return 'pending';
  }
  if (idx < currentIndex) return 'done';
  if (idx === currentIndex) return 'active';
  return 'pending';
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
  learningStats,
}: {
  state: PipelineDemoState;
  examples?: PipelineScenarioExample[];
  selectedExampleId?: string;
  onSelectExample?: (id: string) => void;
  selectorDisabled?: boolean;
  onRun?: () => void;
  isRunning?: boolean;
  serverHealth?: ServerStatus;
  learningStats?: { routerConfidence: number; confidenceTrend: 'improving' | 'stable' | 'declining'; totalDecisions: number };
}) {
  const progress = useMemo(() => totalProgress(state.plans), [state.plans]);
  const currentPhaseIdx = phaseIndex(state.phase);
  const tasks = useMemo(() => allTasks(state.plans), [state.plans]);
  const routes = useMemo(() => routeSummary(state.plans), [state.plans]);
  const gates = useMemo(() => gateSummary(state.plans), [state.plans]);

  const hasPrd = !!state.prd;
  const hasPlans = state.plans.length > 0;
  const hasTasks = tasks.length > 0;
  const hasEvents = state.events.length > 0;
  const pctDone = progress.total > 0 ? Math.round((progress.done / progress.total) * 100) : 0;
  const isActive = progress.active > 0;

  return (
    <div className="pp">
      {/* ── Overall progress bar ── */}
      {progress.total > 0 && (
        <div className={`pp-progress-bar${isActive ? ' pp-progress-active' : ''}`}>
          <div className="pp-progress-fill" style={{ width: `${pctDone}%` }} />
        </div>
      )}

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
            <div className="pp-run-group">
              <button
                className={`pp-run-btn${!isRunning ? ' pp-run-btn-ready' : ''}`}
                onClick={onRun}
                disabled={isRunning}
              >
                {runBtnLabel(serverHealth, isRunning)}
              </button>
              <span className={`pp-health-badge pp-health-${serverHealth}`}>
                <span className="pp-health-dot" />
                {serverHealth === 'connected' ? 'Server ready' : serverHealth === 'checking' ? 'Connecting\u2026' : 'Offline'}
              </span>
            </div>
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
          {examples.map((ex) => {
            const bullets = EXAMPLE_PREVIEWS[ex.id] ?? [];
            const badgeColor = COMPLEXITY_COLORS[ex.complexity] ?? 'amber';
            return (
              <button
                key={ex.id}
                className={selectedExampleId === ex.id ? 'active' : ''}
                onClick={() => onSelectExample(ex.id)}
                disabled={selectorDisabled}
              >
                <div className="pp-example-header">
                  <b>{ex.label}</b>
                  <span className={`pp-complexity-badge pp-complexity-${badgeColor}`}>{ex.complexity}</span>
                </div>
                {bullets.length > 0 && (
                  <ul className="pp-example-bullets">
                    {bullets.map((b, i) => <li key={i}>{b}</li>)}
                  </ul>
                )}
              </button>
            );
          })}
        </div>
      )}

      {/* ── Last run stats (idle) ── */}
      {learningStats && learningStats.totalDecisions > 0 && !hasPlans && (
        <div className="pp-last-run">
          <span className="pp-last-run-label">Last run</span>
          <div className="pp-last-run-stats">
            <span>{learningStats.totalDecisions} decisions</span>
            <span>confidence {Math.round(learningStats.routerConfidence * 100)}%</span>
            <span className={`pp-trend-${learningStats.confidenceTrend}`}>{learningStats.confidenceTrend}</span>
          </div>
        </div>
      )}

      {/* ── Phase rail ── */}
      <div className="pp-rail">
        {PHASES.map((phase, i) => {
          const s = phaseStatus(phase.id, state.phase, currentPhaseIdx);
          const isLast = i === PHASES.length - 1;
          const isIdle = state.phase === 'idle';
          return (
            <div
              key={phase.id}
              className={`pp-phase pp-phase-${s}${s === 'active' ? ' gradient-border-active' : ''}`}
              title={phase.description}
            >
              {s === 'done' ? (
                <PhaseCheckIcon />
              ) : s === 'failed' ? (
                <PhaseXIcon />
              ) : s === 'active' ? (
                <span />
              ) : (
                <span />
              )}
              <b>{phase.label}</b>
              {/* Connection line between phases — animated dashes in idle */}
              {!isLast && <div className={`pp-phase-connector${isIdle ? ' pp-phase-connector-idle' : ''}`} />}
              {/* Hover tooltip */}
              <div className="pp-phase-tooltip">{phase.description}</div>
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
              {learningStats && learningStats.totalDecisions > 0 && (
                <div className="pp-router-confidence">
                  <ConfidenceMeter
                    confidence={learningStats.routerConfidence}
                    trend={learningStats.confidenceTrend}
                    decisions={learningStats.totalDecisions}
                    label="ROUTER"
                    compact
                  />
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

/* ── SVG Icons ── */

function PhaseCheckIcon() {
  return (
    <svg className="pp-status-icon" viewBox="0 0 16 16">
      <path className="pp-check-path" d="M3 8.5 L6.5 12 L13 4" />
    </svg>
  );
}

function PhaseXIcon() {
  return (
    <svg className="pp-status-icon" viewBox="0 0 16 16">
      <path className="pp-x-path" d="M4 4 L12 12" />
      <path className="pp-x-path" d="M12 4 L4 12" style={{ animationDelay: '0.15s' }} />
    </svg>
  );
}

function TaskStatusIndicator({ status }: { status: PipelineTaskStatus }) {
  if (status === 'done') {
    return (
      <svg className="pp-status-icon" viewBox="0 0 16 16">
        <path className="pp-check-path" d="M3 8.5 L6.5 12 L13 4" />
      </svg>
    );
  }
  if (status === 'failed') {
    return (
      <svg className="pp-status-icon" viewBox="0 0 16 16">
        <path className="pp-x-path" d="M4 4 L12 12" />
        <path className="pp-x-path" d="M12 4 L4 12" style={{ animationDelay: '0.15s' }} />
      </svg>
    );
  }
  if (status === 'active') {
    return (
      <svg className="pp-spinner" viewBox="0 0 16 16">
        <circle cx="8" cy="8" r="6" />
      </svg>
    );
  }
  return <div className="pp-task-dot" />;
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
  const [expanded, setExpanded] = useState(false);
  const toggle = useCallback(() => setExpanded((v) => !v), []);

  const hasDetail = (task.files.length > 0) || task.role || task.modelHint || (task.dependsOn.length > 0);

  return (
    <article
      className={`pp-task pp-task-${task.status}${hasDetail ? ' pp-task-expandable' : ''}`}
      onClick={hasDetail ? toggle : undefined}
    >
      <TaskStatusIndicator status={task.status} />
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
        {/* Expandable detail panel */}
        {hasDetail && (
          <div className={`pp-task-detail${expanded ? ' pp-task-detail-open' : ''}`}>
            <div className="pp-task-detail-inner">
              {task.role && (
                <div className="pp-detail-row">
                  <span className="pp-detail-label">Role</span>
                  <span className="pp-detail-value">{task.role}</span>
                </div>
              )}
              {task.modelHint && (
                <div className="pp-detail-row">
                  <span className="pp-detail-label">Model</span>
                  <span className="pp-detail-value">{task.modelHint}</span>
                </div>
              )}
              {task.dependsOn.length > 0 && (
                <div className="pp-detail-row">
                  <span className="pp-detail-label">Deps</span>
                  <span className="pp-detail-value">{task.dependsOn.join(', ')}</span>
                </div>
              )}
              {task.files.length > 0 && (
                <div className="pp-detail-row">
                  <span className="pp-detail-label">Files</span>
                  <span className="pp-detail-value">{task.files.join(', ')}</span>
                </div>
              )}
            </div>
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
