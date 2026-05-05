import {
  useState,
  useCallback,
  useRef,
  useEffect,
  useMemo,
  useImperativeHandle,
  forwardRef,
} from 'react';
import type { Scenario, ClickableScenario, ScenarioContext } from '../../lib/scenarios';
import { isClickableScenario } from '../../lib/scenarios';
import { CommandList } from '../../components/CommandList';
import { useCommandList } from '../../hooks/useCommandList';
import { PlaybackController, TimelineStepper, type TimelineStepState } from '../../lib/playback-controller';
import { enterWorkspace, resetRokoResolution } from '../../lib/terminal-session';
import type { TerminalHandle } from '../../hooks/useTerminal';
import { markStart, markEnd, measure, clearMarks } from '../../lib/perf-markers';
import { lookupCmdDesc } from '../../lib/cmd-descriptions';
import type { GateEntry } from '../../components/GateVerdictCard';
import type { InsightEvent, AgentInfo } from '../../components/KnowledgeFlowPanel';
import type { EfficiencyMetric } from '../../components/EfficiencyBar';
import { useChainWs, type InsightEvent as ChainInsightEvent } from '../../hooks/useChain';
import type { BlockData } from '../../components/ChainActivityPanel';
import type { AgentPosition } from '../../components/LivePositionsPanel';
import {
  EMPTY_PIPELINE_STATE,
  type PipelineDemoState,
  type PipelineEvent,
  type PipelineStreamState,
  type PipelineTask,
} from '../../lib/prd-pipeline-types';
import {
  createPipelineIntroState,
  DEFAULT_PIPELINE_EXAMPLE_ID,
  getPipelineExample,
  PIPELINE_EXAMPLES,
} from '../../lib/prd-pipeline-sample';
import { ConfettiBurst, SuccessRing } from '../../components/Celebration';
import ScenarioPreview from '../../components/ScenarioPreview';
import SidebarRenderer from '../../components/SidebarRenderer';
import DemoStatusBar from '../../components/DemoStatusBar';
import type { ToastOptions } from '../../components/Toast';
// DemoCompletionOverlay removed — too intrusive for demo flow
import TerminalPaneWithHandle, { type TerminalPaneState } from './TerminalPaneWithHandle';

// ── Helpers ────────────────────────────────────────────────

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function now_ts(): string {
  const d = new Date();
  return `${d.getHours().toString().padStart(2, '0')}:${d.getMinutes().toString().padStart(2, '0')}:${d.getSeconds().toString().padStart(2, '0')}`;
}

// ── Default state factories (single source of truth for reset) ──

const INITIAL_STATS = { model: '--', cost: '--', tokens: '--', time: '--' };

const INITIAL_KF_LEFT: AgentInfo = { name: 'Alpha', color: 'var(--rose-bright)', posts: 0, confirms: 0 };
const INITIAL_KF_RIGHT: AgentInfo = { name: 'Beta', color: 'var(--dream-bright)', posts: 0, confirms: 0 };

function initialKfMetrics(): EfficiencyMetric[] {
  return [
    { label: 'ALPHA COST', value: 0, format: (n) => `$${n.toFixed(2)}`, color: 'rose' },
    { label: 'BETA COST', value: 0, format: (n) => `$${n.toFixed(2)}`, color: 'dream' },
    { label: 'SAVINGS', value: 0, format: (n) => `${n.toFixed(0)}%`, color: 'bone' },
  ];
}

function initialPipeline(scenarioId: string, example: ReturnType<typeof getPipelineExample>): PipelineDemoState {
  return scenarioId === 'prd-pipeline' ? createPipelineIntroState(example) : EMPTY_PIPELINE_STATE;
}

const INITIAL_CI_POSITIONS: AgentPosition[] = [
  {
    name: 'Yield Scout',
    address: '0x70997970C51812dc3A010C7d01b50e0d17dc79C8',
    color: 'rose',
    balances: [
      { token: 'ETH', amount: 10, decimals: 4 },
      { token: 'USDC', amount: 500000, decimals: 2 },
    ],
    keyMetric: { label: 'APR', value: '--' },
  },
  {
    name: 'Risk Hedger',
    address: '0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC',
    color: 'sage',
    balances: [
      { token: 'ETH', amount: 110, decimals: 4 },
      { token: 'USDC', amount: 0, decimals: 2 },
    ],
    keyMetric: { label: 'HF', value: '--' },
  },
];

// ── Public interfaces ──────────────────────────────────────

export interface SlotStateReport {
  isRunning: boolean;
  isPaused: boolean;
  scenarioComplete: boolean;
  waitingForStep: boolean;
  progressLabel: string;
  progressText: string;
  progressStep: number;
  progressTotal: number;
  elapsedMs: number;
  readyTerminalCount: number;
  workspaceDir: string;
}

export interface ScenarioSlotHandle {
  play(): void;
  pauseResume(): void;
  step(): void;
  reset(): void;
}

interface ScenarioSlotProps {
  scenario: Scenario | ClickableScenario;
  scenarioIdx: number;
  active: boolean;
  playbackMode: 'auto' | 'step';
  serverHealth: 'connected' | 'checking' | 'disconnected';
  checkServeHealth: () => Promise<void>;
  defaultModel: string | null;
  learningStats: ReturnType<typeof import('../../hooks/useLearningStats').useLearningStats>;
  handoffs: ReturnType<typeof import('../../hooks/useAgentHandoffs').useAgentHandoffs>['handoffs'];
  activeHandoff: ReturnType<typeof import('../../hooks/useAgentHandoffs').useAgentHandoffs>['activeHandoff'];
  ensureWorkspace: (prefix: string) => Promise<{ path: string }>;
  createWorkspace: (prefix: string) => Promise<{ path: string }>;
  toast: (msg: string, opts?: ToastOptions) => void;
  onStateChange: (idx: number, state: SlotStateReport) => void;
  onComplete: (idx: number) => void;
  onNextScenario: (idx: number) => void;
}

const ScenarioSlot = forwardRef<ScenarioSlotHandle, ScenarioSlotProps>(function ScenarioSlot(
  {
    scenario,
    scenarioIdx,
    active,
    playbackMode,
    serverHealth,
    checkServeHealth,
    defaultModel,
    learningStats,
    handoffs,
    activeHandoff,
    ensureWorkspace: _ensureWorkspace,
    createWorkspace: createWs,
    toast,
    onStateChange,
    onComplete,
    onNextScenario,
  },
  ref,
) {
  // ── Playback controllers (per-slot, stable for lifetime) ───
  const playbackRef = useRef<PlaybackController | null>(null);
  if (!playbackRef.current) playbackRef.current = new PlaybackController();
  const playback = playbackRef.current;

  const timelineRef = useRef<TimelineStepper | null>(null);
  if (!timelineRef.current) timelineRef.current = new TimelineStepper();
  const timeline = timelineRef.current;

  // ── Per-slot state ─────────────────────────────────────────
  const [showIntro, setShowIntro] = useState(true);
  const [isRunning, setIsRunning] = useState(false);
  const [isPaused, setIsPaused] = useState(false);

  // Cinematic animation states
  const [introDismissing, setIntroDismissing] = useState(false);
  const [termReveal, setTermReveal] = useState(false);
  const [phaseFlash, setPhaseFlash] = useState(false);
  const [scenarioComplete, setScenarioComplete] = useState(false);
  const [showBurst, setShowBurst] = useState(false);
  // Completion overlay disabled — kept as no-op to avoid breaking timer cleanup
  const setShowCompletionOverlay = (_v: boolean) => {}; // eslint-disable-line @typescript-eslint/no-unused-vars
  const completionOverlayTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const completionAutoDismissTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Countdown + fullscreen states
  const [countdownNum, setCountdownNum] = useState<number | null>(null);
  const [isFullscreen, setIsFullscreen] = useState(true);
  const [termBlackout, setTermBlackout] = useState(false);

  const [elapsedMs, setElapsedMs] = useState(0);
  const elapsedRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const runStartRef = useRef<number>(0);

  // SSE-driven inference state
  const [inferenceModel, setInferenceModel] = useState('--');
  const [inferenceTier, setInferenceTier] = useState<'T0' | 'T1' | 'T2'>('T1');
  const [allGatesPass, setAllGatesPass] = useState(false);
  const allGatesPassTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [showGateRing, setShowGateRing] = useState(false);

  // Sidebar state
  const [stats, setStats] = useState(INITIAL_STATS);
  const [gates, setGates] = useState<{ name: string; status: 'pass' | 'fail' | 'pending' }[]>([]);
  const [logEntries, setLogEntries] = useState<{ ts: string; text: string; type?: 'info' | 'success' | 'error' }[]>([]);
  const [timelineSteps, setTimelineSteps] = useState<TimelineStepState[]>([]);
  const [progressText, setProgressText] = useState('press Play to begin');
  const [progressLabel, setProgressLabel] = useState('--');
  const [waitingForStep, setWaitingForStep] = useState(false);
  const [progressStep, setProgressStep] = useState(0);
  const [progressTotal, setProgressTotal] = useState(0);
  const [pipelineExampleId, setPipelineExampleId] = useState(DEFAULT_PIPELINE_EXAMPLE_ID);
  const selectedPipelineExample = getPipelineExample(pipelineExampleId);
  const [pipeline, setPipeline] = useState<PipelineDemoState>(
    () => initialPipeline(scenario.id, selectedPipelineExample),
  );

  // Knowledge Transfer panel state
  const [kfInsights, setKfInsights] = useState<InsightEvent[]>([]);
  const [kfLeftAgent, setKfLeftAgent] = useState<AgentInfo>(INITIAL_KF_LEFT);
  const [kfRightAgent, setKfRightAgent] = useState<AgentInfo>(INITIAL_KF_RIGHT);
  const [kfMetrics, setKfMetrics] = useState<EfficiencyMetric[]>(initialKfMetrics);

  // Chain Intelligence panel state
  const chainWs = useChainWs(scenario.category === 'chain');
  const [ciBlocks] = useState<BlockData[]>([]);
  const [ciPositions] = useState<AgentPosition[]>(INITIAL_CI_POSITIONS);

  const ciInsights: InsightEvent[] = useMemo(
    () =>
      chainWs.insights.map((ev: ChainInsightEvent) => ({
        id: ev.id,
        type: ev.type === 'stateTransition' ? 'posted' as const : ev.type,
        agent: ev.author ?? ev.by ?? 'unknown',
        kind: (ev.kind ?? 'heuristic') as InsightEvent['kind'],
        content: ev.content ?? `${ev.from} -> ${ev.to}`,
        timestamp: ev.createdAt ?? ev.at ?? Date.now(),
      })),
    [chainWs.insights],
  );

  const ciMetrics: EfficiencyMetric[] = useMemo(
    () => [
      { label: 'INSIGHTS', value: chainWs.stats.insights, color: 'bone' as const },
      { label: 'CONFIRMS', value: chainWs.stats.confirms, color: 'success' as const },
      {
        label: 'REUSE',
        value: chainWs.stats.insights > 0
          ? Math.round((chainWs.stats.confirms / chainWs.stats.insights) * 100)
          : 0,
        format: (n: number) => `${n}%`,
        color: 'dream' as const,
      },
      {
        label: 'CALLS SAVED',
        value: chainWs.stats.confirms * 3,
        color: 'rose' as const,
      },
    ],
    [chainWs.stats],
  );

  const ciLeftAgent: AgentInfo = useMemo(
    () => ({
      name: 'Alpha',
      color: 'var(--rose-bright)',
      posts: ciInsights.filter((i) => i.agent === 'yield-scout' || i.agent === 'agent-alpha').length,
      confirms: ciInsights.filter(
        (i) => i.type === 'confirmed' && (i.agent === 'yield-scout' || i.agent === 'agent-alpha'),
      ).length,
    }),
    [ciInsights],
  );

  const ciRightAgent: AgentInfo = useMemo(
    () => ({
      name: 'Beta',
      color: 'var(--dream-bright)',
      posts: ciInsights.filter((i) => i.agent === 'risk-hedger' || i.agent === 'agent-beta').length,
      confirms: ciInsights.filter(
        (i) => i.type === 'confirmed' && (i.agent === 'risk-hedger' || i.agent === 'agent-beta'),
      ).length,
    }),
    [ciInsights],
  );

  // ── Refs ────────────────────────────────────────────────────
  const pausedRef = useRef(false);
  const runningRef = useRef(false);
  const abortRef = useRef<AbortController | null>(null);
  const workspaceDirRef = useRef<string>('');
  const workspaceEnteredRef = useRef(false);
  const handleRefsRef = useRef<(React.RefObject<TerminalHandle | null>)[]>([]);
  const [terminalStates, setTerminalStates] = useState<TerminalPaneState[]>([]);

  const readyTerminalCount = useMemo(
    () => terminalStates.slice(0, scenario.panes).filter((state) => state.connected).length,
    [scenario.panes, terminalStates],
  );

  // ── Stable session IDs (never regenerated) ─────────────────
  const [sessionIds] = useState(() =>
    scenario.labels.map((_, i) => `demo-${scenario.id}-${i}-${Date.now()}`),
  );

  // Stable terminal handle refs (created once, never recreated)
  const stableRefsRef = useRef<React.RefObject<TerminalHandle | null>[] | null>(null);
  if (!stableRefsRef.current) {
    stableRefsRef.current = Array.from(
      { length: scenario.panes },
      () => ({ current: null }) as React.RefObject<TerminalHandle | null>,
    );
  }
  handleRefsRef.current = stableRefsRef.current;

  useEffect(() => {
    setTerminalStates(Array.from(
      { length: scenario.panes },
      () => ({ status: 'connecting' as const, connected: false }),
    ));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // once on mount

  const updateTerminalState = useCallback((
    index: number,
    state: TerminalPaneState,
  ) => {
    setTerminalStates((prev) => {
      if (prev[index]?.status === state.status && prev[index]?.connected === state.connected) {
        return prev;
      }
      const next = prev.slice();
      next[index] = state;
      return next;
    });
  }, []);

  // ── Wire change listeners ──────────────────────────────────
  useEffect(() => {
    timeline.onChange((steps) => setTimelineSteps(steps));
    playback.onProgress((step, total, cmd) => {
      setProgressLabel(step <= 0 ? 'Preparing' : `Step ${step}/${total}`);
      setProgressText(cmd);
      setProgressStep(Math.max(0, step));
      setProgressTotal(total);
    });
    playback.onWaitingChange(setWaitingForStep);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Sync playback mode from parent
  useEffect(() => {
    playback.setMode(playbackMode);
  }, [playbackMode, playback]);

  // ── Elapsed timer ──────────────────────────────────────────
  useEffect(() => {
    if (isRunning && !isPaused) {
      runStartRef.current = Date.now() - elapsedMs;
      elapsedRef.current = setInterval(() => {
        setElapsedMs(Date.now() - runStartRef.current);
      }, 250);
    } else if (elapsedRef.current) {
      clearInterval(elapsedRef.current);
      elapsedRef.current = null;
    }
    if (!isRunning) {
      setElapsedMs(0);
    }
    return () => {
      if (elapsedRef.current) clearInterval(elapsedRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isRunning, isPaused]);

  // ── Phase transition ripple ────────────────────────────────
  const prevLabelRef = useRef(progressLabel);
  useEffect(() => {
    if (isRunning && progressLabel !== prevLabelRef.current && prevLabelRef.current !== '--') {
      setPhaseFlash(true);
      const timer = setTimeout(() => setPhaseFlash(false), 650);
      prevLabelRef.current = progressLabel;
      return () => clearTimeout(timer);
    }
    prevLabelRef.current = progressLabel;
  }, [progressLabel, isRunning]);

  // ── All gates pass effect ──────────────────────────────────
  useEffect(() => {
    if (gates.length > 0 && gates.every((g) => g.status === 'pass')) {
      setAllGatesPass(true);
      if (allGatesPassTimer.current) clearTimeout(allGatesPassTimer.current);
      allGatesPassTimer.current = setTimeout(() => setAllGatesPass(false), 3000);
    } else {
      setAllGatesPass(false);
    }
  }, [gates]);

  // ── Inference model tier ───────────────────────────────────
  useEffect(() => {
    if (stats.model !== '--') {
      setInferenceModel(stats.model);
      const m = stats.model.toLowerCase();
      if (m.includes('opus') || m.includes('gpt-4') || m.includes('o1') || m.includes('o3')) {
        setInferenceTier('T0');
      } else if (m.includes('sonnet') || m.includes('gpt-3.5') || m.includes('gemini')) {
        setInferenceTier('T1');
      } else {
        setInferenceTier('T2');
      }
    }
  }, [stats.model]);

  // ── Push state reports to parent ───────────────────────────
  // Only push when active (avoids 250ms timer churn from inactive slots)
  useEffect(() => {
    if (!active) return;
    onStateChange(scenarioIdx, {
      isRunning,
      isPaused,
      scenarioComplete,
      waitingForStep,
      progressLabel,
      progressText,
      progressStep,
      progressTotal,
      elapsedMs,
      readyTerminalCount,
      workspaceDir: workspaceDirRef.current,
    });
  }, [
    active, scenarioIdx, isRunning, isPaused, scenarioComplete, waitingForStep,
    progressLabel, progressText, progressStep, progressTotal,
    elapsedMs, readyTerminalCount, onStateChange,
  ]);

  // Also push meaningful state changes even when inactive (running/complete transitions)
  const prevRunning = useRef(false);
  const prevComplete = useRef(false);
  useEffect(() => {
    if (active) return; // active slot already pushes everything above
    if (isRunning !== prevRunning.current || scenarioComplete !== prevComplete.current) {
      prevRunning.current = isRunning;
      prevComplete.current = scenarioComplete;
      onStateChange(scenarioIdx, {
        isRunning,
        isPaused,
        scenarioComplete,
        waitingForStep,
        progressLabel,
        progressText,
        progressStep,
        progressTotal,
        elapsedMs,
        readyTerminalCount,
        workspaceDir: workspaceDirRef.current,
      });
    }
  }, [
    active, scenarioIdx, isRunning, isPaused, scenarioComplete, waitingForStep,
    progressLabel, progressText, progressStep, progressTotal,
    elapsedMs, readyTerminalCount, onStateChange,
  ]);

  // ── Shared reset logic ─────────────────────────────────────
  const clearCompletionTimers = useCallback(() => {
    if (completionOverlayTimer.current) clearTimeout(completionOverlayTimer.current);
    if (completionAutoDismissTimer.current) clearTimeout(completionAutoDismissTimer.current);
  }, []);

  const resetSidebarState = useCallback(() => {
    setStats(INITIAL_STATS);
    setGates([]);
    setLogEntries([]);
    setTimelineSteps([]);
    setProgressText('press Play to begin');
    setProgressLabel('--');
    setPipeline(initialPipeline(scenario.id, selectedPipelineExample));
    setKfInsights([]);
    setKfLeftAgent(INITIAL_KF_LEFT);
    setKfRightAgent(INITIAL_KF_RIGHT);
    setKfMetrics(initialKfMetrics());
  }, [scenario.id, selectedPipelineExample]);

  // ── Build scenario context ─────────────────────────────────
  const patchPipeline = useCallback((patch: Partial<PipelineDemoState>) => {
    setPipeline((prev) => ({ ...prev, ...patch }));
  }, []);

  const patchPipelineStream = useCallback((patch: Partial<PipelineStreamState>) => {
    setPipeline((prev) => ({
      ...prev,
      stream: {
        sse: prev.stream?.sse ?? 'idle',
        ws: prev.stream?.ws ?? 'idle',
        ...prev.stream,
        ...patch,
      },
    }));
  }, []);

  const updatePipelineTask = useCallback((
    planId: string,
    taskId: string,
    patch: Partial<PipelineTask>,
  ) => {
    setPipeline((prev) => ({
      ...prev,
      plans: prev.plans.map((plan) => {
        if (plan.id !== planId) return plan;
        return {
          ...plan,
          tasks: plan.tasks.map((task) =>
            task.id === taskId ? { ...task, ...patch } : task,
          ),
        };
      }),
    }));
  }, []);

  const appendPipelineEvent = useCallback((event: PipelineEvent) => {
    setPipeline((prev) => ({
      ...prev,
      events: [...prev.events, event].slice(-30),
    }));
  }, []);

  const buildContext = useCallback((
    workspaceDir: string,
    scenarioEntries?: TerminalHandle[],
  ): ScenarioContext => {
    const entries = scenarioEntries ?? handleRefsRef.current
      .map((r) => r.current)
      .filter((h): h is TerminalHandle =>
        h !== null && h.status === 'connected' && h.ws?.readyState === WebSocket.OPEN,
      );

    const pipelineCtx = {
      setPipeline,
      patchPipeline,
      patchPipelineStream,
      updatePipelineTask,
      appendPipelineEvent,
      example: selectedPipelineExample,
    };

    return {
      entries,
      workspaceDir,
      createWorkspace: async (prefix: string) => {
        const ws = await createWs(prefix);
        return ws.path;
      },
      playback,
      timeline,
      signal: abortRef.current!.signal,
      pipeline: pipelineCtx,
      setMetric: (key: string, value: string) => {
        setStats((prev) => {
          const k = key.replace('m-', '') as keyof typeof prev;
          if (k in prev) return { ...prev, [k]: value };
          return prev;
        });
      },
      setGate: (name: string, status: 'pass' | 'fail' | 'pending') => {
        if (status === 'pass') {
          setShowGateRing(true);
        }
        setGates((prev) => {
          const existing = prev.findIndex((g) => g.name === name);
          if (existing >= 0) {
            const next = [...prev];
            next[existing] = { name, status };
            return next;
          }
          return [...prev, { name, status }];
        });
      },
      logCommand: (cmd: string, desc: string) => {
        const ts = now_ts();
        setLogEntries((prev) => [
          ...prev,
          { ts, text: `$ ${cmd}`, type: 'info' as const },
          { ts, text: desc || lookupCmdDesc(cmd) || 'Executing...', type: 'info' as const },
        ]);
      },
      logCommandComplete: (cmd: string, ok: boolean) => {
        setLogEntries((prev) => {
          const copy = [...prev];
          for (let i = copy.length - 1; i >= 0; i--) {
            if (copy[i].text === `$ ${cmd}`) {
              if (i + 1 < copy.length) {
                copy[i + 1] = { ...copy[i + 1], type: ok ? 'success' : 'error' };
              }
              break;
            }
          }
          return copy;
        });
      },
      setPipeline,
      patchPipeline,
      patchPipelineStream,
      updatePipelineTask,
      appendPipelineEvent,
      pipelineExample: selectedPipelineExample,
      activeModel: defaultModel || undefined,
      paused: pausedRef,
      running: runningRef,
    };
  }, [appendPipelineEvent, createWs, defaultModel, patchPipeline, patchPipelineStream, selectedPipelineExample, updatePipelineTask, playback, timeline]);

  // ── Terminal helpers ────────────────────────────────────────
  const getReadyTerminalEntries = useCallback((): TerminalHandle[] => (
    handleRefsRef.current
      .slice(0, scenario.panes)
      .map((r) => r.current)
      .filter((h): h is TerminalHandle =>
        h !== null && h.status === 'connected' && h.ws?.readyState === WebSocket.OPEN,
      )
  ), [scenario.panes]);

  const waitForTerminalReadiness = useCallback(async (): Promise<TerminalHandle[] | null> => {
    const timeoutMs = 10000;
    const startedAt = Date.now();

    while (Date.now() - startedAt < timeoutMs && runningRef.current) {
      const entries = getReadyTerminalEntries();
      if (entries.length >= scenario.panes) return entries;

      setProgressLabel('Terminals');
      setProgressText(`waiting for terminals (${entries.length}/${scenario.panes})...`);
      await sleep(50);
    }

    const entries = getReadyTerminalEntries();
    return entries.length >= scenario.panes ? entries : null;
  }, [getReadyTerminalEntries, scenario.panes]);

  // ── Pipeline example select ────────────────────────────────
  const handlePipelineExampleSelect = useCallback((id: string) => {
    if (runningRef.current) return;
    const example = getPipelineExample(id);
    setPipelineExampleId(example.id);
    setPipeline(createPipelineIntroState(example));
    setProgressText('press Play to begin');
    setProgressLabel('--');
    setGates([]);
    setLogEntries([]);
    setTimelineSteps([]);
    setShowIntro(true);
  }, []);

  // ── Lifecycle handlers ─────────────────────────────────────
  const handlePlay = useCallback(async () => {
    if (runningRef.current) return;
    // Immediate health re-check on play press — don't rely on stale poll
    await checkServeHealth();
    const serveOnline = serverHealth === 'connected';
    if (!serveOnline) {
      setLogEntries((prev) => [
        ...prev,
        { ts: now_ts(), text: `roko serve not reachable — running without live workflow projections.`, type: 'info' as const },
      ]);
    }

    abortRef.current = new AbortController();
    runningRef.current = true;
    pausedRef.current = false;
    setScenarioComplete(false);
    setShowBurst(false);
    setShowCompletionOverlay(false);
    clearCompletionTimers();

    setIntroDismissing(true);
    setTimeout(() => {
      setShowIntro(false);
      setIntroDismissing(false);
    }, 550);

    // Skip cinematic countdown for click-to-run scenarios
    if (!isClickable) {
      for (const n of [3, 2, 1]) {
        setCountdownNum(n);
        await sleep(800);
      }
      setCountdownNum(null);

      setTermBlackout(true);
      setIsFullscreen(false);
      await sleep(600);
      setTermBlackout(false);

      setTermReveal(true);
      setTimeout(() => setTermReveal(false), 600);
    } else {
      setIsFullscreen(false);
    }

    setIsRunning(true);
    setIsPaused(false);

    resetSidebarState();

    markStart('terminal-connect');
    const entries = await waitForTerminalReadiness();
    markEnd('terminal-connect');
    const termConnectMs = measure('terminal-connect');
    if (termConnectMs !== null) {
      console.debug(`[perf] terminal-connect: ${termConnectMs.toFixed(1)}ms`);
    }
    if (!entries) {
      const connected = getReadyTerminalEntries().length;
      console.error(
        `Timed out waiting for terminals: need ${scenario.panes} but only ${connected} connected`,
      );
      runningRef.current = false;
      setIsRunning(false);
      setProgressLabel('Terminals');
      setProgressText(`terminal connection timed out (${connected}/${scenario.panes})`);
      setLogEntries((prev) => [
        ...prev,
        { ts: now_ts(), text: `Timed out waiting for ${scenario.panes} terminal connection${scenario.panes === 1 ? '' : 's'}.`, type: 'error' as const },
      ]);
      return;
    }
    try {
      setProgressLabel('Workspace');
      setProgressText(`creating live workspace for ${scenario.title}`);
      markStart('workspace-create');
      let wsPath: string;
      if (serveOnline) {
        const ws = await createWs(`roko-${scenario.id}`);
        wsPath = ws.path;
      } else {
        // Fallback: create workspace via PTY mktemp when roko serve is unavailable
        const mkResult = await entries[0].execCmd(
          'DIR=$(mktemp -d /tmp/roko-ws-XXXXXX) && cd "$DIR" && git init -q && echo "WSDIR:$DIR"',
          8000,
        );
        const match = entries[0].outputBuffer.match(/WSDIR:(\S+)/);
        if (!mkResult.ok || !match) {
          throw new Error('Failed to create local workspace via mktemp');
        }
        wsPath = match[1];
        console.log('[ScenarioSlot] created local workspace:', wsPath);
      }
      markEnd('workspace-create');
      const wsMs = measure('workspace-create');
      if (wsMs !== null) {
        console.debug(`[perf] workspace-create: ${wsMs.toFixed(1)}ms`);
      }
      workspaceDirRef.current = wsPath;

      // For ClickableScenario: initialise ALL terminals (roko resolution + cd) but
      // don't run the scenario automatically. Users click individual commands.
      if (isClickable) {
        try {
          // Enter workspace on all panes (first pane resolves roko binary,
          // subsequent panes reuse the cached resolution)
          await enterWorkspace(entries[0], wsPath);
          if (entries.length > 1) {
            await Promise.all(entries.slice(1).map(e => enterWorkspace(e, wsPath)));
          }
          workspaceEnteredRef.current = true;
        } catch (err) {
          console.warn('[ScenarioSlot] ClickableScenario enterWorkspace failed:', err);
          // Non-fatal — roko binary will fall back to 'roko' on PATH
        }
        runningRef.current = false;
        setIsRunning(false);
        setIsPaused(false);
        return;
      }

      const ctx = buildContext(wsPath, entries);
      markStart('scenario-run');
      await (scenario as Scenario).run(ctx);
      markEnd('scenario-run');
      const scenarioMs = measure('scenario-run');
      if (scenarioMs !== null) {
        console.debug(`[perf] scenario-run: ${scenarioMs.toFixed(1)}ms`);
      }

      onComplete(scenarioIdx);
      setScenarioComplete(true);
      setShowBurst(true);
      toast(`Scenario complete: ${scenario.title}`, { type: 'success' });
      setTimeout(() => {
        setScenarioComplete(false);
        setShowBurst(false);
      }, 1400);

      clearCompletionTimers();
      completionOverlayTimer.current = setTimeout(() => {
        setShowCompletionOverlay(true);
        completionAutoDismissTimer.current = setTimeout(() => {
          setShowCompletionOverlay(false);
        }, 8000);
      }, 1000);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Scenario error:', err);
      toast(msg, { type: 'error', duration: 5000 });
      setProgressLabel('Error');
      setProgressText(msg);
      setLogEntries((prev) => [
        ...prev,
        { ts: now_ts(), text: `Workspace creation failed: ${msg}`, type: 'error' as const },
      ]);
    }

    runningRef.current = false;
    setIsRunning(false);
    setIsPaused(false);
  }, [scenario, serverHealth, checkServeHealth, buildContext, waitForTerminalReadiness, getReadyTerminalEntries, createWs, scenarioIdx, onComplete, toast, clearCompletionTimers, resetSidebarState]);

  const handlePauseResume = useCallback(() => {
    pausedRef.current = !isPaused;
    setIsPaused(!isPaused);
  }, [isPaused]);

  const handleStep = useCallback(() => {
    playback.advanceStep();
  }, [playback]);

  // ── ClickableScenario state (must precede handleReset which uses cmdReset) ──
  const isClickable = isClickableScenario(scenario);

  // Hooks must be called unconditionally.
  // For non-clickable scenarios these are unused but satisfy the Rules of Hooks.
  const clickableCommands = isClickable ? (scenario as ClickableScenario).commands : [];

  const {
    items: cmdItems,
    markRunning: cmdMarkRunning,
    markSuccess: cmdMarkSuccess,
    markFailure: cmdMarkFailure,
    reset: cmdReset,
  } = useCommandList(clickableCommands);

  // Completion overlay and related handlers removed
  void onNextScenario; // keep prop used

  const handleReset = useCallback(() => {
    abortRef.current?.abort();
    runningRef.current = false;
    pausedRef.current = false;
    setIsRunning(false);
    setIsPaused(false);
    setIsFullscreen(true);
    setCountdownNum(null);
    setScenarioComplete(false);
    setShowBurst(false);
    setShowCompletionOverlay(false);
    clearCompletionTimers();
    playback.reset();
    timeline.reset();
    clearMarks();
    setShowIntro(true);
    setIntroDismissing(false);
    setTermReveal(false);
    resetSidebarState();
    cmdReset();
    resetRokoResolution();
    workspaceEnteredRef.current = false;
  }, [playback, timeline, clearCompletionTimers, resetSidebarState, cmdReset]);

  // ── Imperative handle ──────────────────────────────────────
  useImperativeHandle(ref, () => ({
    play: handlePlay,
    pauseResume: handlePauseResume,
    step: handleStep,
    reset: handleReset,
  }), [handlePlay, handlePauseResume, handleStep, handleReset]);

  // ── Derived display values ─────────────────────────────────
  const timelineDisplay = timelineSteps.map((s) => ({
    label: s.label,
    detail: s.sublabel,
    status: s.status === 'completed' ? ('done' as const) : s.status,
  }));

  const hasStats = stats.model !== '--' || stats.cost !== '--' || stats.tokens !== '--' || stats.time !== '--';
  const hasKfMetrics = kfMetrics.some((m) => m.value > 0);

  const gateEntries: GateEntry[] = useMemo(
    () => gates.map((g) => ({
      name: g.name,
      status: g.status as GateEntry['status'],
    })),
    [gates],
  );

  const gridCols = scenario.panes;

  const handleClickableRun = useCallback(async (id: string) => {
    if (!isClickable) return;
    const clickable = scenario as ClickableScenario;

    // Build ctx with current entries (must wait for terminals)
    const entries = getReadyTerminalEntries();
    if (entries.length === 0) {
      toast('No terminal connected. Wait for the terminal to be ready.', { type: 'error' });
      return;
    }

    // Ensure workspace is available
    let wsPath = workspaceDirRef.current;
    if (!wsPath) {
      try {
        setProgressLabel('Workspace');
        setProgressText('creating workspace…');
        const ws = await createWs(`roko-${scenario.id}`);
        wsPath = ws.path;
        workspaceDirRef.current = wsPath;
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        toast(`Workspace creation failed: ${msg}`, { type: 'error' });
        return;
      }
    }

    // On first click, initialise ALL terminals (resolve roko binary, cd, clear)
    if (!workspaceEnteredRef.current) {
      try {
        await enterWorkspace(entries[0], wsPath);
        if (entries.length > 1) {
          await Promise.all(entries.slice(1).map(e => enterWorkspace(e, wsPath)));
        }
        workspaceEnteredRef.current = true;
      } catch (err) {
        console.warn('[ScenarioSlot] handleClickableRun enterWorkspace failed:', err);
        // Non-fatal — roko binary will fall back to 'roko' on PATH
      }
    }

    const ctx = buildContext(wsPath, entries);

    cmdMarkRunning(id);
    setIsRunning(true);
    try {
      const result = await clickable.runCommand(ctx, id);
      if (result.ok) {
        cmdMarkSuccess(id);
      } else {
        cmdMarkFailure(id, result.error ?? 'Command returned non-zero exit code');
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      cmdMarkFailure(id, msg);
    } finally {
      setIsRunning(false);
    }
  }, [isClickable, scenario, getReadyTerminalEntries, createWs, buildContext, cmdMarkRunning, cmdMarkSuccess, cmdMarkFailure, toast]);

  // ── Render ─────────────────────────────────────────────────
  return (
    <div style={{ display: active ? 'contents' : 'none' }}>
      {/* ── Status bar ── */}
      <DemoStatusBar
        serverHealth={{
          ok: serverHealth === 'connected',
        }}
        terminalStates={Array.from({ length: scenario.panes }, (_, i) => ({
          label: scenario.labels[i] || `pane ${i + 1}`,
          status: (terminalStates[i]?.status ?? 'disconnected') as 'connected' | 'connecting' | 'disconnected',
        }))}
        scenarioId={scenario.id}
        isRunning={isRunning}
        elapsedMs={isRunning ? elapsedMs : undefined}
        speed={1}
      />

      {/* ── Main content ── */}
      <div className={[
        'demo-main',
        isClickable ? 'demo-main-clickable' : (scenario.id === 'prd-pipeline' ? 'demo-main-pipeline' : ''),
        isFullscreen && !isClickable ? 'demo-main--fullscreen' : '',
      ].filter(Boolean).join(' ')}>
        {/* ── ClickableScenario: 2-column layout ── */}
        {isClickable ? (
          <div className="demo-clickable-layout">
            {/* Left 70%: terminal */}
            <div className={[
              'demo-clickable-terminal',
              isRunning ? 'gradient-border-active' : 'gradient-border-subtle',
              scenarioComplete ? 'scenario-complete' : '',
              termBlackout ? 'term-blackout' : '',
            ].filter(Boolean).join(' ')}>
              <ConfettiBurst
                active={showBurst}
                count={40}
                duration={1200}
                onDone={() => setShowBurst(false)}
              />
              <SuccessRing
                active={showGateRing}
                onDone={() => setShowGateRing(false)}
              />

              {(showIntro || introDismissing) && (
                <ScenarioPreview
                  scenario={scenario}
                  onPlay={handlePlay}
                  serverHealth={serverHealth}
                  isRunning={isRunning}
                  dismissing={introDismissing}
                />
              )}

              <div className={`demo-terminal-grid demo-cols-${scenario.panes}`}>
                {Array.from({ length: scenario.panes }).map((_, i) => (
                  <TerminalPaneWithHandle
                    key={sessionIds[i]}
                    sessionId={sessionIds[i]}
                    label={scenario.labels[i] || `pane ${i + 1}`}
                    handleRef={handleRefsRef.current[i]}
                    paneIndex={i}
                    onStatusChange={updateTerminalState}
                    termReveal={termReveal}
                    scenarioId={scenario.id}
                    scenarioCategory={scenario.category}
                    isRunning={isRunning}
                  />
                ))}
              </div>
            </div>

            {/* Right 30%: command list + optional context panel */}
            <div className="demo-clickable-sidebar">
              <div className="demo-clickable-commands">
                <CommandList
                  commands={cmdItems}
                  onRun={handleClickableRun}
                  onRetry={handleClickableRun}
                />
              </div>
            </div>
          </div>
        ) : (
          /* ── Standard scenario: existing layout ── */
          <>
            <div className={[
              'demo-terminals',
              isRunning ? 'gradient-border-active' : 'gradient-border-subtle',
              phaseFlash ? 'phase-flash' : '',
              scenarioComplete ? 'scenario-complete' : '',
              termBlackout ? 'term-blackout' : '',
            ].filter(Boolean).join(' ')}>
              <ConfettiBurst
                active={showBurst}
                count={40}
                duration={1200}
                onDone={() => setShowBurst(false)}
              />
              <SuccessRing
                active={showGateRing}
                onDone={() => setShowGateRing(false)}
              />

              {countdownNum !== null && (
                <div className="demo-countdown-overlay">
                  <span key={countdownNum} className="demo-countdown-num">{countdownNum}</span>
                  <span className="demo-countdown-label">launching {scenario.title}</span>
                </div>
              )}

              {/* Completion overlay removed — too intrusive for demo flow */}

              {(showIntro || introDismissing) && (
                <ScenarioPreview
                  scenario={scenario}
                  onPlay={handlePlay}
                  serverHealth={serverHealth}
                  isRunning={isRunning}
                  dismissing={introDismissing}
                />
              )}

              <div className={`demo-terminal-grid demo-cols-${gridCols}`}>
                {Array.from({ length: scenario.panes }).map((_, i) => (
                  <TerminalPaneWithHandle
                    key={sessionIds[i]}
                    sessionId={sessionIds[i]}
                    label={scenario.labels[i] || `pane ${i + 1}`}
                    handleRef={handleRefsRef.current[i]}
                    paneIndex={i}
                    onStatusChange={updateTerminalState}
                    termReveal={termReveal}
                    scenarioId={scenario.id}
                    scenarioCategory={scenario.category}
                    isRunning={isRunning}
                  />
                ))}
              </div>
            </div>

            {scenario.panel && (
              <div className="demo-sidebar">
                <SidebarRenderer
                  scenarioId={scenario.id}
                  isRunning={isRunning}
                  scenarioComplete={scenarioComplete}
                  timelineSteps={timelineDisplay}
                  stats={stats}
                  hasStats={hasStats}
                  inferenceModel={inferenceModel}
                  inferenceTier={inferenceTier}
                  gates={gates}
                  gateEntries={gateEntries}
                  allGatesPass={allGatesPass}
                  logEntries={logEntries}
                  pipeline={pipeline}
                  pipelineExamples={PIPELINE_EXAMPLES}
                  pipelineExampleId={pipelineExampleId}
                  onSelectExample={handlePipelineExampleSelect}
                  onRun={handlePlay}
                  serverHealth={serverHealth}
                  learningStats={learningStats}
                  handoffs={handoffs}
                  activeHandoff={activeHandoff}
                  kfInsights={kfInsights}
                  kfLeftAgent={kfLeftAgent}
                  kfRightAgent={kfRightAgent}
                  kfMetrics={kfMetrics}
                  hasKfMetrics={hasKfMetrics}
                  ciInsights={ciInsights}
                  ciBlocks={ciBlocks}
                  ciPositions={ciPositions}
                  ciMetrics={ciMetrics}
                  ciLeftAgent={ciLeftAgent}
                  ciRightAgent={ciRightAgent}
                  chainConnected={chainWs.connected}
                />
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
});

export default ScenarioSlot;
