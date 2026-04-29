import type {
  PipelineDemoState,
  PipelineEvent,
  PipelineExampleSummary,
  PipelinePhase,
  PipelinePlan,
  PipelineRouteTier,
  PipelineScenarioExample,
  PipelineTask,
  PipelineTaskStatus,
} from '../lib/prd-pipeline-types';
import GateBar from './GateBar';
import Mosaic, { MosaicCell } from './Mosaic';
import WorkflowConstellation from './WorkflowConstellation';
import './PrdPipelinePanel.css';

type ServerHealth = 'connected' | 'checking' | 'disconnected';

const WORKFLOW_STAGES: Array<{
  id: string;
  label: string;
  detail: string;
  phase: PipelinePhase;
}> = [
  { id: 'job', label: 'Job', detail: 'Plain-language request', phase: 'idea' },
  { id: 'prd', label: 'PRD', detail: 'Scope and acceptance', phase: 'draft' },
  { id: 'plan', label: 'Plan', detail: 'Implementation strategy', phase: 'planning' },
  { id: 'tasks', label: 'Tasks', detail: 'tasks.toml graph', phase: 'tasks' },
  { id: 'verify', label: 'Verify', detail: 'Gates and agents', phase: 'implementing' },
  { id: 'done', label: 'Code', detail: 'Runnable output', phase: 'complete' },
];

const PHASE_STAGE_INDEX: Record<PipelinePhase, number> = {
  idle: 0,
  setup: 0,
  idea: 0,
  draft: 1,
  published: 1,
  planning: 2,
  tasks: 3,
  implementing: 4,
  complete: 5,
  failed: 4,
};

const TASK_GROUPS: Array<{
  id: PipelineTaskStatus;
  label: string;
  empty: string;
}> = [
  { id: 'active', label: 'Working', empty: 'No task is active yet.' },
  { id: 'pending', label: 'Queued', empty: 'Nothing queued.' },
  { id: 'done', label: 'Done', empty: 'No completed tasks yet.' },
  { id: 'blocked', label: 'Blocked', empty: 'No blockers.' },
  { id: 'failed', label: 'Failed', empty: 'No failures.' },
];

function displayTaskStatus(status: PipelineTaskStatus): string {
  if (status === 'active') return 'working';
  if (status === 'done') return 'done';
  if (status === 'failed') return 'failed';
  if (status === 'blocked') return 'blocked';
  return 'queued';
}

function allTasks(plans: PipelinePlan[]): PipelineTask[] {
  return plans.flatMap((plan) => plan.tasks);
}

function totalProgress(plans: PipelinePlan[]): { done: number; total: number; active: number; failed: number; blocked: number } {
  return allTasks(plans).reduce(
    (acc, task) => ({
      done: acc.done + (task.status === 'done' ? 1 : 0),
      total: acc.total + 1,
      active: acc.active + (task.status === 'active' ? 1 : 0),
      failed: acc.failed + (task.status === 'failed' ? 1 : 0),
      blocked: acc.blocked + (task.status === 'blocked' ? 1 : 0),
    }),
    { done: 0, total: 0, active: 0, failed: 0, blocked: 0 },
  );
}

function planProgress(plan: PipelinePlan): { done: number; total: number; pct: number } {
  const total = plan.tasks.length;
  const done = plan.tasks.filter((task) => task.status === 'done').length;
  return { done, total, pct: total === 0 ? 0 : Math.round((done / total) * 100) };
}

function shortList(items: string[], max = 2): string {
  const filtered = items.filter(Boolean);
  if (filtered.length === 0) return 'none';
  if (filtered.length <= max) return filtered.join(', ');
  return `${filtered.slice(0, max).join(', ')} +${filtered.length - max}`;
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
  const counts = new Map<string, { count: number; command?: string; passed: number; failed: number }>();
  for (const task of allTasks(plans)) {
    for (const verify of task.verify) {
      const phase = verify.phase || 'verify';
      const existing = counts.get(phase) ?? { count: 0, command: verify.command, passed: 0, failed: 0 };
      existing.count += 1;
      existing.command = existing.command ?? verify.command;
      existing.passed += verify.status === 'passed' ? 1 : 0;
      existing.failed += verify.status === 'failed' ? 1 : 0;
      counts.set(phase, existing);
    }
  }
  return Array.from(counts.entries())
    .map(([phase, data]) => ({ phase, ...data }))
    .sort((a, b) => b.count - a.count || a.phase.localeCompare(b.phase));
}

function gateOutcomes(plans: PipelinePlan[]) {
  const steps = allTasks(plans).flatMap((task) => task.verify);
  return {
    total: steps.length,
    passed: steps.filter((step) => step.status === 'passed').length,
    failed: steps.filter((step) => step.status === 'failed').length,
  };
}

function routeLabel(tier: PipelineRouteTier): string {
  if (tier === 'T1') return 'T1 Fast';
  if (tier === 'T2') return 'T2 Build';
  return 'T3 Risk';
}

function routeMeaning(tier: PipelineRouteTier): string {
  if (tier === 'T1') return 'Small mechanical changes and cheap verification.';
  if (tier === 'T2') return 'Feature work that needs context and code synthesis.';
  return 'Integration risk, security, marketplace, or unknowns.';
}

function phaseTitle(phase: PipelinePhase): string {
  if (phase === 'setup') return 'Preparing a real repository';
  if (phase === 'idea') return 'Capturing the job';
  if (phase === 'draft') return 'Writing the PRD';
  if (phase === 'published') return 'PRD scope locked';
  if (phase === 'planning') return 'Generating the implementation plan';
  if (phase === 'tasks') return 'Task graph ready';
  if (phase === 'implementing') return 'Agents are implementing';
  if (phase === 'complete') return 'Verified workflow complete';
  if (phase === 'failed') return 'Run needs attention';
  return 'Ready to generate';
}

function phaseDetail(phase: PipelinePhase, progress: ReturnType<typeof totalProgress>): string {
  if (phase === 'failed') return 'The artifacts stay visible so the failed route, gate, or provider decision is inspectable.';
  if (phase === 'complete') return 'The PRD, plan, task graph, model routing, and verification evidence all converged.';
  if (phase === 'implementing') {
    if (progress.active > 0) return `${progress.active} task${progress.active === 1 ? '' : 's'} currently running against the generated plan.`;
    return 'The runner is executing the generated plan and streaming task state changes.';
  }
  if (phase === 'tasks') return `${progress.total || 0} task${progress.total === 1 ? '' : 's'} are ready with route tiers, dependencies, and gates.`;
  if (phase === 'planning') return 'The planner is producing plan.md and tasks.toml from the PRD.';
  if (phase === 'draft') return 'The PRD writer is turning the job into requirements and acceptance criteria.';
  if (phase === 'idea') return 'The raw request is captured as durable product intent.';
  return 'Start with one plain-language job and watch Roko create the executable workflow.';
}

function gateBarItems(gates: ReturnType<typeof gateSummary>) {
  return gates.slice(0, 6).map((gate) => ({
    name: `${gate.phase} ${gate.count}`,
    status: gate.failed > 0 ? ('fail' as const) : gate.passed === gate.count && gate.count > 0 ? ('pass' as const) : ('pending' as const),
  }));
}

function healthLabel(health?: ServerHealth): string {
  if (health === 'connected') return 'Start live run';
  if (health === 'checking') return 'Checking serve';
  if (health === 'disconnected') return 'Serve offline';
  return 'Start live run';
}

export default function PrdPipelinePanel({
  state,
  examples = [],
  selectedExampleId,
  onSelectExample,
  selectorDisabled,
  onRun,
  isRunning = false,
  serverHealth,
}: {
  state: PipelineDemoState;
  examples?: PipelineScenarioExample[];
  selectedExampleId?: string;
  onSelectExample?: (id: string) => void;
  selectorDisabled?: boolean;
  onRun?: () => void;
  isRunning?: boolean;
  serverHealth?: ServerHealth;
}) {
  const progress = totalProgress(state.plans);
  const tasks = allTasks(state.plans);
  const routes = routeSummary(state.plans);
  const gates = gateSummary(state.plans);
  const gateStats = gateOutcomes(state.plans);
  const activeStage = PHASE_STAGE_INDEX[state.phase] ?? 0;
  const started = state.source !== 'empty' || state.phase !== 'idle';
  const progressPct = progress.total === 0 ? 0 : Math.round((progress.done / progress.total) * 100);
  const primaryStatement = state.example?.stageQuote ?? state.example?.idea ?? 'Pick an example and run the workflow.';

  return (
    <div className={`pipeline-panel pipeline-phase-${state.phase}`}>
      <div className="pipeline-topline">
        <div>
          <span className="pipeline-overline">Roko workflow demo</span>
          <b>Job to PRD to plan to verified tasks</b>
        </div>
        <StreamStatus stream={state.stream} source={state.source} />
      </div>

      {examples.length > 0 && onSelectExample && (
        <ExampleSwitcher
          examples={examples}
          selectedExampleId={selectedExampleId}
          onSelectExample={onSelectExample}
          disabled={selectorDisabled}
        />
      )}

      <div className="pipeline-hero">
        <div className="pipeline-hero-copy">
          <div className="pipeline-hero-kicker">
            <span>{state.example?.complexity ?? 'Live demo'}</span>
            <span>{state.source === 'live' ? 'real artifacts' : state.source === 'sample' ? 'sample state' : 'ready'}</span>
          </div>
          <h2>{phaseTitle(state.phase)}</h2>
          <p className="pipeline-hero-statement">{primaryStatement}</p>
          <p className="pipeline-hero-detail">{phaseDetail(state.phase, progress)}</p>
          <div className="pipeline-hero-actions">
            {onRun && (
              <button
                className="pipeline-primary-action"
                onClick={onRun}
                disabled={isRunning || serverHealth !== 'connected'}
              >
                {isRunning ? 'Running live' : healthLabel(serverHealth)}
              </button>
            )}
            {state.currentCommand && <code className="pipeline-command">{state.currentCommand}</code>}
          </div>
        </div>
        <div className="pipeline-hero-visual">
          <WorkflowConstellation
            phase={state.phase}
            plans={state.plans}
            gateTotal={gateStats.total}
            gatePassed={gateStats.passed}
          />
          <div className="pipeline-visual-score">
            <span>{progress.total ? `${progress.done}/${progress.total}` : '0'}</span>
            <b>tasks</b>
          </div>
        </div>
      </div>

      <div className="pipeline-scoreboard">
        <Mosaic columns={4} className="pipeline-score-mosaic">
          <MosaicCell
            label="PRD"
            value={state.prd ? state.prd.status : 'waiting'}
            sub={state.prd?.title ?? 'No PRD artifact yet'}
            color={state.prd ? 'success' : 'rose'}
            mono
          />
          <MosaicCell
            label="Plans"
            value={String(state.plans.length)}
            sub={state.plans[0]?.title ?? 'Waiting for planner'}
            color={state.plans.length > 0 ? 'bone' : 'rose'}
            mono
          />
          <MosaicCell
            label="Tasks"
            value={progress.total ? `${progress.done}/${progress.total}` : '0'}
            sub={progress.active ? `${progress.active} working` : progress.failed ? `${progress.failed} failed` : progress.total ? `${progress.total - progress.done} queued` : 'Waiting for tasks.toml'}
            color={progress.failed ? 'warning' : progress.active ? 'rose' : progress.total ? 'success' : 'dream'}
            mono
          />
          <MosaicCell
            label="Gates"
            value={gateStats.total ? `${gateStats.passed}/${gateStats.total}` : '0'}
            sub={gateStats.failed ? `${gateStats.failed} failed` : gateStats.total ? 'Declared verify checks' : 'Waiting for gates'}
            color={gateStats.failed ? 'warning' : gateStats.passed ? 'success' : 'dream'}
            mono
          />
        </Mosaic>
      </div>

      <WorkflowRail activeStage={activeStage} failed={state.phase === 'failed'} />

      {state.example && <ScenarioBrief example={state.example} />}

      <div className="pipeline-artifact-grid">
        <PrdArtifact prd={state.prd} started={started} />
        <PlanArtifact plans={state.plans} started={started} progressPct={progressPct} />
        <RoutingArtifact routes={routes} gates={gates} gateStats={gateStats} tasks={tasks} />
      </div>

      <TaskBoard plans={state.plans} tasks={tasks} progress={progress} />

      <EventTape events={state.events} lastUpdated={state.lastUpdated} />
    </div>
  );
}

function ExampleSwitcher({
  examples,
  selectedExampleId,
  onSelectExample,
  disabled,
}: {
  examples: PipelineScenarioExample[];
  selectedExampleId?: string;
  onSelectExample: (id: string) => void;
  disabled?: boolean;
}) {
  return (
    <div className="pipeline-example-switcher" aria-label="Pipeline examples">
      {examples.map((example) => (
        <button
          key={example.id}
          className={selectedExampleId === example.id ? 'active' : ''}
          onClick={() => onSelectExample(example.id)}
          disabled={disabled}
        >
          <span>{example.complexity}</span>
          <b>{example.label}</b>
          <em>{example.why[0]}</em>
        </button>
      ))}
    </div>
  );
}

function WorkflowRail({ activeStage, failed }: { activeStage: number; failed: boolean }) {
  return (
    <div className="pipeline-workflow-rail">
      {WORKFLOW_STAGES.map((stage, index) => {
        const state = failed && index === activeStage ? 'failed' : index < activeStage ? 'done' : index === activeStage ? 'active' : 'pending';
        return (
          <div key={stage.id} className={`pipeline-stage-card pipeline-stage-${state}`}>
            <span>{String(index + 1).padStart(2, '0')}</span>
            <b>{stage.label}</b>
            <em>{stage.detail}</em>
          </div>
        );
      })}
    </div>
  );
}

function ScenarioBrief({ example }: { example: PipelineExampleSummary }) {
  return (
    <div className="pipeline-scenario-brief">
      <div>
        <span className="pipeline-overline">Selected job</span>
        <h3>{example.stageQuote ?? example.prdTitle}</h3>
        <p>{example.idea}</p>
      </div>
      <div className="pipeline-why-list">
        {example.why.map((item) => (
          <span key={item}>{item}</span>
        ))}
      </div>
    </div>
  );
}

function PrdArtifact({ prd, started }: { prd: PipelineDemoState['prd']; started: boolean }) {
  const bullets = prd?.requirements.length ? prd.requirements : prd?.acceptance ?? [];
  return (
    <article className="pipeline-artifact pipeline-prd-artifact">
      <Header eyebrow="Artifact 1" title="Product spec" value={prd?.status ?? (started ? 'generating' : 'waiting')} />
      {prd ? (
        <>
          <h3>{prd.title}</h3>
          <p>{prd.excerpt || 'The generated PRD is available in the workflow projection.'}</p>
          <div className="pipeline-doc-metrics">
            <Metric label="requirements" value={String(prd.requirements.length)} />
            <Metric label="acceptance" value={String(prd.acceptance.length)} />
            <Metric label="slug" value={prd.slug} mono />
          </div>
          <ul className="pipeline-bullet-list">
            {bullets.slice(0, 3).map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </>
      ) : (
        <WaitingLines lines={['Waiting for .roko/prd draft markdown', 'The document appears here as soon as the PRD agent writes it.']} />
      )}
    </article>
  );
}

function PlanArtifact({
  plans,
  started,
  progressPct,
}: {
  plans: PipelinePlan[];
  started: boolean;
  progressPct: number;
}) {
  return (
    <article className="pipeline-artifact pipeline-plan-artifact">
      <Header eyebrow="Artifact 2" title="Implementation plan" value={plans.length ? `${plans.length} plan${plans.length === 1 ? '' : 's'}` : started ? 'generating' : 'waiting'} />
      {plans.length === 0 ? (
        <WaitingLines lines={['Waiting for plan.md and tasks.toml', 'Roko will split the PRD into ordered work with dependencies.']} />
      ) : (
        <div className="pipeline-plan-stack">
          {plans.map((plan) => {
            const progress = planProgress(plan);
            return (
              <div className={`pipeline-plan-row pipeline-plan-${plan.status}`} key={plan.id}>
                <div>
                  <h3>{plan.title}</h3>
                  <span>{plan.id}</span>
                </div>
                <b>{progress.pct}%</b>
                <div className="pipeline-plan-progress">
                  <i style={{ width: `${progress.pct}%` }} />
                </div>
                <p>{plan.excerpt || 'Plan details are available in the generated plan directory.'}</p>
                <em>{progress.done}/{progress.total} tasks</em>
              </div>
            );
          })}
          <div className="pipeline-progress-legend">
            <span style={{ width: `${progressPct}%` }} />
          </div>
        </div>
      )}
    </article>
  );
}

function RoutingArtifact({
  routes,
  gates,
  gateStats,
  tasks,
}: {
  routes: ReturnType<typeof routeSummary>;
  gates: ReturnType<typeof gateSummary>;
  gateStats: ReturnType<typeof gateOutcomes>;
  tasks: PipelineTask[];
}) {
  return (
    <article className="pipeline-artifact pipeline-routing-artifact">
      <Header eyebrow="Artifact 3" title="Routing and gates" value={tasks.length ? `${tasks.length} tasks` : 'waiting'} />
      <div className="pipeline-route-grid">
        {(['T1', 'T2', 'T3'] as PipelineRouteTier[]).map((tier) => {
          const data = routes[tier];
          return (
            <div key={tier} className={`pipeline-route-card pipeline-route-${tier.toLowerCase()}`}>
              <span>{routeLabel(tier)}</span>
              <b>{data.count}</b>
              <em>{data.active > 0 ? `${data.active} active` : shortList(Array.from(data.models), 1)}</em>
              <p>{routeMeaning(tier)}</p>
            </div>
          );
        })}
      </div>
      <div className="pipeline-gate-strip">
        {gates.length > 0 ? (
          <GateBar gates={gateBarItems(gates)} />
        ) : (
          <span>Verification gates appear after tasks.toml is generated.</span>
        )}
      </div>
      <div className="pipeline-gate-score">
        <b>{gateStats.total ? `${gateStats.passed}/${gateStats.total}` : '0/0'}</b>
        <span>{gateStats.failed ? `${gateStats.failed} failed gates` : 'verify checks declared'}</span>
      </div>
    </article>
  );
}

function TaskBoard({
  plans,
  tasks,
  progress,
}: {
  plans: PipelinePlan[];
  tasks: PipelineTask[];
  progress: ReturnType<typeof totalProgress>;
}) {
  const planByTask = new Map<string, string>();
  for (const plan of plans) {
    for (const task of plan.tasks) planByTask.set(task.id, plan.id);
  }

  return (
    <div className="pipeline-task-board">
      <Header
        eyebrow="Live task state"
        title="Task graph"
        value={progress.total ? `${progress.done}/${progress.total}` : 'waiting'}
      />
      {tasks.length === 0 ? (
        <WaitingLines lines={['Waiting for generated tasks.toml', 'Task cards will move from queued to working to done as the runner emits workflow state.']} />
      ) : (
        <div className="pipeline-task-columns">
          {TASK_GROUPS.map((group) => {
            const groupedTasks = tasks.filter((task) => task.status === group.id);
            return (
              <div key={group.id} className={`pipeline-task-column pipeline-task-column-${group.id}`}>
                <div className="pipeline-task-column-head">
                  <b>{group.label}</b>
                  <span>{groupedTasks.length}</span>
                </div>
                <div className="pipeline-task-card-list">
                  {groupedTasks.length === 0 ? (
                    <p>{group.empty}</p>
                  ) : (
                    groupedTasks.slice(0, 8).map((task) => (
                      <TaskCard key={`${planByTask.get(task.id) ?? 'plan'}:${task.id}`} task={task} planId={planByTask.get(task.id)} />
                    ))
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function TaskCard({ task, planId }: { task: PipelineTask; planId?: string }) {
  const routeTier: PipelineRouteTier = task.routeTier ?? 'T1';
  return (
    <article className={`pipeline-task-card pipeline-task-${task.status}`}>
      <div className="pipeline-task-top">
        <span>{planId ? `${planId}:` : ''}{task.id}</span>
        <b className={`pipeline-route-badge pipeline-route-badge-${routeTier.toLowerCase()}`}>{routeTier}</b>
      </div>
      <h4>{task.title}</h4>
      {task.description && <p>{task.description}</p>}
      <div className="pipeline-task-facts">
        <span>{displayTaskStatus(task.status)}</span>
        <span>{task.modelHint ?? 'model pending'}</span>
        <span>{task.verify.length} gates</span>
        {task.files.length > 0 && <span>{shortList(task.files)}</span>}
        {task.dependsOn.length > 0 && <span>after {shortList(task.dependsOn, 3)}</span>}
      </div>
    </article>
  );
}

function EventTape({ events, lastUpdated }: { events: PipelineEvent[]; lastUpdated?: string }) {
  return (
    <div className="pipeline-events">
      <Header eyebrow="Evidence" title="Live event feed" value={lastUpdated ?? '--'} />
      <div className="pipeline-event-list">
        {events.slice(-6).map((event) => (
          <EventRow key={event.id} event={event} />
        ))}
        {events.length === 0 && <p>No workflow events yet.</p>}
      </div>
    </div>
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

function Header({ eyebrow, title, value }: { eyebrow: string; title: string; value?: string }) {
  return (
    <div className="pipeline-section-head">
      <div>
        <span>{eyebrow}</span>
        <b>{title}</b>
      </div>
      {value && <em>{value}</em>}
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

function WaitingLines({ lines }: { lines: string[] }) {
  return (
    <div className="pipeline-waiting">
      {lines.map((line) => (
        <p key={line}>{line}</p>
      ))}
    </div>
  );
}

function StreamStatus({ stream, source }: { stream: PipelineDemoState['stream']; source: PipelineDemoState['source'] }) {
  const live = stream?.sse === 'live' || stream?.ws === 'live';
  const errored = stream?.sse === 'error' && stream?.ws === 'error';
  const status = live ? 'live' : errored ? 'error' : source === 'live' ? 'connecting' : 'idle';
  const label = live ? 'Live projection' : errored ? 'Projection offline' : source === 'live' ? 'Connecting projection' : 'Projection ready';

  return (
    <div className={`pipeline-stream-status pipeline-stream-${status}`} title={stream?.message}>
      <span />
      <b>{label}</b>
      {stream?.workflowId && <em>{stream.workflowId}</em>}
      {stream?.cursor != null && <code>#{stream.cursor}</code>}
    </div>
  );
}
