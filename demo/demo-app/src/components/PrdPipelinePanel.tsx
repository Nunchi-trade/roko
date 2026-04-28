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
import './PrdPipelinePanel.css';

const PHASES: { id: PipelinePhase; label: string }[] = [
  { id: 'idea', label: 'Idea' },
  { id: 'draft', label: 'PRD' },
  { id: 'planning', label: 'Plan' },
  { id: 'tasks', label: 'Tasks' },
  { id: 'implementing', label: 'Run' },
  { id: 'complete', label: 'Done' },
];

const PHASE_ORDER: PipelinePhase[] = [
  'idle',
  'setup',
  'idea',
  'draft',
  'published',
  'planning',
  'tasks',
  'implementing',
  'complete',
  'failed',
];

function phaseIndex(phase: PipelinePhase): number {
  return PHASE_ORDER.indexOf(phase);
}

function displayTaskStatus(status: PipelineTaskStatus): string {
  if (status === 'active') return 'working';
  if (status === 'done') return 'done';
  if (status === 'failed') return 'failed';
  if (status === 'blocked') return 'blocked';
  return 'pending';
}

function statusClass(status: PipelineTaskStatus): string {
  return `pipeline-task-${status}`;
}

function planProgress(plan: PipelinePlan): { done: number; total: number; active: number } {
  const total = plan.tasks.length;
  const done = plan.tasks.filter((task) => task.status === 'done').length;
  const active = plan.tasks.filter((task) => task.status === 'active').length;
  return { done, total, active };
}

function totalProgress(plans: PipelinePlan[]): { done: number; total: number; active: number } {
  return plans.reduce(
    (acc, plan) => {
      const p = planProgress(plan);
      return {
        done: acc.done + p.done,
        total: acc.total + p.total,
        active: acc.active + p.active,
      };
    },
    { done: 0, total: 0, active: 0 },
  );
}

function allTasks(plans: PipelinePlan[]): PipelineTask[] {
  return plans.flatMap((plan) => plan.tasks);
}

function shortList(items: string[], max = 3): string {
  if (items.length === 0) return 'none';
  if (items.length <= max) return items.join(', ');
  return `${items.slice(0, max).join(', ')} +${items.length - max}`;
}

function routeSummary(plans: PipelinePlan[]) {
  const summary: Record<PipelineRouteTier, { count: number; active: number; models: Set<string> }> = {
    T1: { count: 0, active: 0, models: new Set<string>() },
    T2: { count: 0, active: 0, models: new Set<string>() },
    T3: { count: 0, active: 0, models: new Set<string>() },
  };
  for (const task of allTasks(plans)) {
    const tier = task.routeTier ?? 'T1';
    summary[tier].count += 1;
    if (task.status === 'active') summary[tier].active += 1;
    if (task.modelHint) summary[tier].models.add(task.modelHint);
  }
  return summary;
}

function gateSummary(plans: PipelinePlan[]) {
  const counts = new Map<string, { count: number; command?: string }>();
  for (const task of allTasks(plans)) {
    for (const verify of task.verify) {
      const phase = verify.phase || 'verify';
      const existing = counts.get(phase) ?? { count: 0, command: verify.command };
      existing.count += 1;
      existing.command = existing.command ?? verify.command;
      counts.set(phase, existing);
    }
  }
  return Array.from(counts.entries())
    .map(([phase, data]) => ({ phase, ...data }))
    .sort((a, b) => b.count - a.count || a.phase.localeCompare(b.phase));
}

function routeLabel(tier: PipelineRouteTier): string {
  if (tier === 'T1') return 'T1 fast';
  if (tier === 'T2') return 'T2 build';
  return 'T3 risk';
}

function EmptyState() {
  return (
    <div className="pipeline-empty">
      <div className="pipeline-empty-kicker">Live pipeline</div>
      <div className="pipeline-empty-title">PRD, plan, and task artifacts will appear here.</div>
      <div className="pipeline-empty-sub">
        The scenario reads the files generated under .roko and renders the task graph as it runs.
      </div>
    </div>
  );
}

export default function PrdPipelinePanel({
  state,
  examples = [],
  selectedExampleId,
  onSelectExample,
  selectorDisabled,
}: {
  state: PipelineDemoState;
  examples?: PipelineScenarioExample[];
  selectedExampleId?: string;
  onSelectExample?: (id: string) => void;
  selectorDisabled?: boolean;
}) {
  const progress = totalProgress(state.plans);
  const currentPhase = phaseIndex(state.phase);
  const tasks = allTasks(state.plans);
  const routes = routeSummary(state.plans);
  const gates = gateSummary(state.plans);

  return (
    <div className="pipeline-panel">
      {examples.length > 0 && onSelectExample && (
        <div className="pipeline-example-switcher">
          {examples.map((example) => (
            <button
              key={example.id}
              className={selectedExampleId === example.id ? 'active' : ''}
              onClick={() => onSelectExample(example.id)}
              disabled={selectorDisabled}
            >
              <span>{example.complexity}</span>
              <b>{example.label}</b>
            </button>
          ))}
        </div>
      )}

      <div className="pipeline-hero">
        <div>
          <div className="pipeline-eyebrow">
            {state.source === 'live' ? 'live artifacts' : state.source === 'sample' ? 'sample fallback' : 'awaiting run'}
            {state.example ? ` / ${state.example.complexity}` : ''}
          </div>
          <h2>{state.headline}</h2>
          {state.currentCommand && <div className="pipeline-command">{state.currentCommand}</div>}
        </div>
        <div className="pipeline-score">
          <span>{progress.done}</span>
          <b>/ {progress.total || '--'}</b>
          <em>{progress.active > 0 ? `${progress.active} working` : 'tasks'}</em>
        </div>
      </div>

      <div className="pipeline-phase-rail">
        {PHASES.map((phase) => {
          const phaseStep = phaseIndex(phase.id);
          const status = state.phase === 'failed'
            ? phaseStep < currentPhase ? 'done' : 'pending'
            : phaseStep < currentPhase ? 'done' : phaseStep === currentPhase ? 'active' : 'pending';
          return (
            <div key={phase.id} className={`pipeline-phase pipeline-phase-${status}`}>
              <span />
              <b>{phase.label}</b>
            </div>
          );
        })}
      </div>

      {state.example && (
        <section className="pipeline-example-brief">
          {state.example.stageQuote && <blockquote>{state.example.stageQuote}</blockquote>}
          <p>{state.example.idea}</p>
          <div className="pipeline-why-list">
            {state.example.why.map((item) => (
              <span key={item}>{item}</span>
            ))}
          </div>
        </section>
      )}

      {state.prd || state.plans.length > 0 ? (
        <>
          {state.prd && (
            <section className="pipeline-prd">
              <div className="pipeline-section-head">
                <span>Generated PRD</span>
                <b>{state.prd.status}</b>
              </div>
              <h3>{state.prd.title}</h3>
              <p>{state.prd.excerpt}</p>
              <div className="pipeline-prd-grid">
                <Metric label="requirements" value={String(state.prd.requirements.length)} />
                <Metric label="acceptance" value={String(state.prd.acceptance.length)} />
                <Metric label="slug" value={state.prd.slug} mono />
              </div>
            </section>
          )}

          <section className="pipeline-plans">
            <div className="pipeline-section-head">
              <span>Generated Plans</span>
              <b>{state.plans.length}</b>
            </div>
            {state.plans.length === 0 ? (
              <div className="pipeline-muted">Waiting for plan generation.</div>
            ) : (
              <div className="pipeline-plan-list">
                {state.plans.map((plan) => (
                  <PlanCard key={plan.id} plan={plan} />
                ))}
              </div>
            )}
          </section>

          {tasks.length > 0 && (
            <section className="pipeline-insights">
              <div className="pipeline-section-head">
                <span>Routing and Gates</span>
                <b>{tasks.length} tasks</b>
              </div>
              <div className="pipeline-route-grid">
                {(['T1', 'T2', 'T3'] as PipelineRouteTier[]).map((tier) => {
                  const data = routes[tier];
                  return (
                    <div key={tier} className={`pipeline-route-card pipeline-route-${tier.toLowerCase()}`}>
                      <span>{routeLabel(tier)}</span>
                      <b>{data.count}</b>
                      <em>{data.active > 0 ? `${data.active} working` : shortList(Array.from(data.models), 1)}</em>
                    </div>
                  );
                })}
              </div>
              <div className="pipeline-gate-list">
                {gates.length === 0 ? (
                  <span className="pipeline-gate-empty">waiting for verify gates</span>
                ) : (
                  gates.slice(0, 7).map((gate) => (
                    <span key={gate.phase} title={gate.command}>
                      <b>{gate.phase}</b>
                      {gate.count}
                    </span>
                  ))
                )}
              </div>
            </section>
          )}

          <section className="pipeline-tasks">
            <div className="pipeline-section-head">
              <span>Task State Board</span>
              <b>{progress.done}/{progress.total}</b>
            </div>
            {state.plans.flatMap((plan) => plan.tasks).length === 0 ? (
              <div className="pipeline-muted">Waiting for tasks.toml.</div>
            ) : (
              <div className="pipeline-task-list">
                {state.plans.map((plan) =>
                  plan.tasks.map((task) => (
                    <TaskRow key={`${plan.id}:${task.id}`} planId={plan.id} task={task} />
                  )),
                )}
              </div>
            )}
          </section>
        </>
      ) : (
        <EmptyState />
      )}

      <section className="pipeline-events">
        <div className="pipeline-section-head">
          <span>Artifact Log</span>
          <b>{state.lastUpdated ?? '--'}</b>
        </div>
        <div className="pipeline-event-list">
          {state.events.slice(-7).map((event) => (
            <EventRow key={event.id} event={event} />
          ))}
          {state.events.length === 0 && <div className="pipeline-muted">No artifact events yet.</div>}
        </div>
      </section>
    </div>
  );
}

function Metric({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="pipeline-metric">
      <span>{label}</span>
      <b className={mono ? 'mono' : undefined}>{value}</b>
    </div>
  );
}

function PlanCard({ plan }: { plan: PipelinePlan }) {
  const progress = planProgress(plan);
  const pct = progress.total === 0 ? 0 : Math.round((progress.done / progress.total) * 100);

  return (
    <article className={`pipeline-plan-card pipeline-plan-${plan.status}`}>
      <div className="pipeline-plan-top">
        <div>
          <h4>{plan.title}</h4>
          <span>{plan.id}</span>
        </div>
        <b>{pct}%</b>
      </div>
      <div className="pipeline-plan-bar">
        <span style={{ width: `${pct}%` }} />
      </div>
      <p>{plan.excerpt || 'Plan details will appear after generation.'}</p>
      <div className="pipeline-plan-meta">
        <span>{progress.total} tasks</span>
        {plan.estimatedMinutes != null && <span>{plan.estimatedMinutes}m est</span>}
      </div>
    </article>
  );
}

function TaskRow({ planId, task }: { planId: string; task: PipelineTask }) {
  const routeTier: PipelineRouteTier = task.routeTier ?? 'T1';
  return (
    <article className={`pipeline-task-row ${statusClass(task.status)}`}>
      <div className="pipeline-task-status">
        <span />
        <b>{displayTaskStatus(task.status)}</b>
      </div>
      <div className="pipeline-task-main">
        <div className="pipeline-task-title">
          <div>
            <span>{planId}:{task.id}</span>
            <h4>{task.title}</h4>
          </div>
          <b className={`pipeline-route-badge pipeline-route-badge-${routeTier.toLowerCase()}`}>
            {routeTier}
          </b>
        </div>
        {task.description && <p>{task.description}</p>}
        <div className="pipeline-task-meta">
          <span>{task.tier ?? routeLabel(routeTier)}</span>
          {task.role && <span>{task.role}</span>}
          <span>{task.modelHint ?? 'model pending'}</span>
          {task.maxLoc != null && <span>{task.maxLoc} max loc</span>}
          <span>{shortList(task.files)}</span>
          {task.dependsOn.length > 0 && <span>after {task.dependsOn.join(', ')}</span>}
        </div>
        {task.verify.length > 0 && (
          <div className="pipeline-task-verify">
            {task.verify.slice(0, 2).map((verify, i) => (
              <code key={`${task.id}-verify-${i}`}>
                <span>{verify.phase}</span>
                {verify.command}
              </code>
            ))}
            {task.verify.length > 2 && <em>+{task.verify.length - 2} more gates</em>}
          </div>
        )}
      </div>
    </article>
  );
}

function EventRow({ event }: { event: PipelineEvent }) {
  return (
    <div className={`pipeline-event pipeline-event-${event.kind ?? 'info'}`}>
      <span>{event.ts}</span>
      <b>{event.phase}</b>
      <p>{event.text}</p>
    </div>
  );
}
