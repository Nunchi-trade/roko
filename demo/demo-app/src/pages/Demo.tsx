import { useState, useCallback, useRef, useEffect, useMemo } from 'react';
import { SCENARIOS, type ScenarioContext } from '../lib/scenarios';
import { PlaybackController, TimelineStepper, type TimelineStepState } from '../lib/playback-controller';
import { useTerminal, type TerminalHandle } from '../hooks/useTerminal';
import { setSpeedMultiplier } from '../lib/terminal-session';
import { markStart, markEnd, measure, clearMarks } from '../lib/perf-markers';
import { useServerHealth } from '../hooks/useServerHealth';
import { useRokoConfig } from '../hooks/useRokoConfig';
import { useWorkspace } from '../hooks/useWorkspace';
import { useToast } from '../components/Toast';
import { lookupCmdDesc } from '../lib/cmd-descriptions';
import Tooltip from '../components/Tooltip';
import type { GateEntry } from '../components/GateVerdictCard';
import type { InsightEvent, AgentInfo } from '../components/KnowledgeFlowPanel';
import type { EfficiencyMetric } from '../components/EfficiencyBar';
import { useChainWs, type InsightEvent as ChainInsightEvent } from '../hooks/useChain';
import { useLearningStats } from '../hooks/useLearningStats';
import { useAgentHandoffs } from '../hooks/useAgentHandoffs';
import type { BlockData } from '../components/ChainActivityPanel';
import type { AgentPosition } from '../components/LivePositionsPanel';
import {
  EMPTY_PIPELINE_STATE,
  type PipelineDemoState,
  type PipelineEvent,
  type PipelineStreamState,
  type PipelineTask,
} from '../lib/prd-pipeline-types';
import {
  createPipelineIntroState,
  DEFAULT_PIPELINE_EXAMPLE_ID,
  getPipelineExample,
  PIPELINE_EXAMPLES,
} from '../lib/prd-pipeline-sample';
import { SERVE_URL } from '../lib/serve-url';
import { ConfettiBurst, SuccessRing } from '../components/Celebration';
import ScenarioPreview from '../components/ScenarioPreview';
import SidebarRenderer from '../components/SidebarRenderer';
import DemoStatusBar from '../components/DemoStatusBar';
import { PulseIcon, SpinnerIcon, CrossIcon, CheckmarkIcon, WaveformIcon } from '../components/icons/AnimatedIcons';
import '@xterm/xterm/css/xterm.css';
import '../components/Terminal/TerminalPane.css';
import './Demo.css';

// Moved into component as useRef (see playbackRef / timelineRef)

const SPEEDS = [0.5, 1, 2, 4];

/** Category color mapping for tab bar accent dots and active styling */
const TAB_CATEGORY: Record<string, string> = {
  'prd-pipeline': 'pipeline',
  'prd-research-loop': 'pipeline',
  'race': 'comparison',
  'gate-retry': 'comparison',
  'providers': 'comparison',
  'provider-race': 'comparison',
  'explore': 'exploration',
  'knowledge-accumulation': 'learning',
  'dream-consolidation': 'learning',
  'chat': 'learning',
  'knowledge-transfer': 'learning',
  'chain-intelligence': 'chain',
  'mirage': 'chain',
};

/** Color values per category (used for the sliding indicator) */
const CAT_COLORS: Record<string, string> = {
  pipeline: 'var(--rose-bright)',
  comparison: 'var(--status-active)',
  exploration: 'var(--dream-bright)',
  learning: 'var(--status-success)',
  chain: 'var(--warning)',
};


type TerminalPaneState = {
  status: TerminalHandle['status'];
  connected: boolean;
};

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export default function Demo() {
  const playbackRef = useRef<PlaybackController | null>(null);
  if (!playbackRef.current) playbackRef.current = new PlaybackController();
  const playback = playbackRef.current;

  const timelineRef = useRef<TimelineStepper | null>(null);
  if (!timelineRef.current) timelineRef.current = new TimelineStepper();
  const timeline = timelineRef.current;

  const [activeIdx, setActiveIdx] = useState(0);
  const [showIntro, setShowIntro] = useState(true);
  const [isRunning, setIsRunning] = useState(false);
  const [isPaused, setIsPaused] = useState(false);

  // Cinematic animation states
  const [scenarioAnim, setScenarioAnim] = useState<'idle' | 'exit' | 'enter'>('idle');
  const [introDismissing, setIntroDismissing] = useState(false);
  const [termReveal, setTermReveal] = useState(false);
  const [phaseFlash, setPhaseFlash] = useState(false);
  const [scenarioComplete, setScenarioComplete] = useState(false);
  const [showBurst, setShowBurst] = useState(false);
  const [showCompletionOverlay, setShowCompletionOverlay] = useState(false);
  const completionOverlayTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const completionAutoDismissTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [, setLaunchingBtn] = useState(false);

  // Countdown + fullscreen states
  const [countdownNum, setCountdownNum] = useState<number | null>(null);
  const [isFullscreen, setIsFullscreen] = useState(true);
  const [termBlackout, setTermBlackout] = useState(false);

  // Collapsible bottom terminal
  const [bottomTermOpen, setBottomTermOpen] = useState(false);
  const bottomTermSessionId = useRef(`bottom-${Date.now().toString(36)}`);
  const bottomTermHandleRef = useRef<TerminalHandle | null>(null);
  const [speedIdx, setSpeedIdx] = useState(1);
  const [playbackMode, setPlaybackMode] = useState<'auto' | 'step'>('auto');
  const [elapsedMs, setElapsedMs] = useState(0);
  const elapsedRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const runStartRef = useRef<number>(0);
  const scenario = SCENARIOS[activeIdx];
  const serverHealth = useServerHealth();
  const { defaultModel } = useRokoConfig();
  const { ensureWorkspace, createWorkspace: createWs } = useWorkspace();
  const learningStats = useLearningStats();
  const { handoffs, activeHandoff } = useAgentHandoffs();
  const { toast } = useToast();

  // SSE-driven inference state for ModelSlot and CrystallizeTransition
  const [inferenceModel, setInferenceModel] = useState('--');
  const [inferenceTier, setInferenceTier] = useState<'T0' | 'T1' | 'T2'>('T1');
  const [allGatesPass, setAllGatesPass] = useState(false);
  const allGatesPassTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [showGateRing, setShowGateRing] = useState(false);

  // Tab bar state — completed scenarios and sliding indicator
  const [completedScenarios, setCompletedScenarios] = useState<Set<number>>(() => new Set());
  const tabListRef = useRef<HTMLDivElement>(null);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const indicatorRef = useRef<HTMLDivElement>(null);
  const [tabScrollState, setTabScrollState] = useState({ left: false, right: false });

  // Sidebar state
  const [stats, setStats] = useState({ model: '--', cost: '--', tokens: '--', time: '--' });
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
    createPipelineIntroState(selectedPipelineExample),
  );

  // Knowledge Transfer panel state
  const [kfInsights, setKfInsights] = useState<InsightEvent[]>([]);
  const [kfLeftAgent, setKfLeftAgent] = useState<AgentInfo>({ name: 'Alpha', color: 'var(--rose-bright)', posts: 0, confirms: 0 });
  const [kfRightAgent, setKfRightAgent] = useState<AgentInfo>({ name: 'Beta', color: 'var(--dream-bright)', posts: 0, confirms: 0 });
  const [kfMetrics, setKfMetrics] = useState<EfficiencyMetric[]>([
    { label: 'ALPHA COST', value: 0, format: (n) => `$${n.toFixed(2)}`, color: 'rose' },
    { label: 'BETA COST', value: 0, format: (n) => `$${n.toFixed(2)}`, color: 'dream' },
    { label: 'SAVINGS', value: 0, format: (n) => `${n.toFixed(0)}%`, color: 'bone' },
  ]);

  // Chain Intelligence panel state — only connect when the scenario needs it
  const chainWs = useChainWs(scenario.id === 'chain-intelligence');
  const [ciBlocks] = useState<BlockData[]>([]);
  const [ciPositions] = useState<AgentPosition[]>([
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
  ]);

  // Map chain WS insights to KnowledgeFlowPanel InsightEvent format
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

  const pausedRef = useRef(false);
  const runningRef = useRef(false);
  const workspaceDirRef = useRef<string>('');
  const handleRefs = useRef<(React.RefObject<TerminalHandle | null>)[]>([]);
  const [terminalStates, setTerminalStates] = useState<TerminalPaneState[]>([]);

  const readyTerminalCount = useMemo(
    () => terminalStates.slice(0, scenario.panes).filter((state) => state.connected).length,
    [scenario.panes, terminalStates],
  );

  // Wire change listeners
  useEffect(() => {
    timeline.onChange((steps) => setTimelineSteps(steps));
    playback.onProgress((step, total, cmd) => {
      setProgressLabel(step <= 0 ? 'Preparing' : `Step ${step}/${total}`);
      setProgressText(cmd);
      setProgressStep(Math.max(0, step));
      setProgressTotal(total);

    });
    playback.onWaitingChange(setWaitingForStep);
  }, []);

  // Elapsed timer for status bar
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

  // Phase transition ripple when progress label changes during a run
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

  // Build scenario context matching ScenarioContext from scenarios.ts
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
    const entries = scenarioEntries ?? handleRefs.current
      .map((ref) => ref.current)
      .filter((h): h is TerminalHandle =>
        h !== null && h.status === 'connected' && h.ws?.readyState === WebSocket.OPEN,
      );

    return {
      entries,
      workspaceDir,
      createWorkspace: async (prefix: string) => {
        const ws = await createWs(prefix);
        return ws.path;
      },
      playback,
      timeline,
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
        const now = new Date();
        const ts = `${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}:${now.getSeconds().toString().padStart(2, '0')}`;
        setLogEntries((prev) => [
          ...prev,
          { ts, text: `$ ${cmd}`, type: 'info' as const },
          { ts, text: desc || lookupCmdDesc(cmd) || 'Executing...', type: 'success' as const },
        ]);
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
  }, [appendPipelineEvent, createWs, defaultModel, patchPipeline, patchPipelineStream, selectedPipelineExample, updatePipelineTask]);

  const getReadyTerminalEntries = useCallback((): TerminalHandle[] => (
    handleRefs.current
      .slice(0, scenario.panes)
      .map((ref) => ref.current)
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

  // ── Scenario lifecycle ──────────────────────────────────────

  const selectScenario = useCallback((idx: number) => {
    if (idx === activeIdx) return;
    if (runningRef.current) {
      runningRef.current = false;
      setIsRunning(false);
    }

    // Cinematic exit -> enter transition
    setScenarioAnim('exit');
    setScenarioComplete(false);
    setShowBurst(false);
    setShowCompletionOverlay(false);
    if (completionOverlayTimer.current) clearTimeout(completionOverlayTimer.current);
    if (completionAutoDismissTimer.current) clearTimeout(completionAutoDismissTimer.current);

    setTimeout(() => {
      playback.reset();
      timeline.reset();
      setActiveIdx(idx);
      setShowIntro(true);
      setIntroDismissing(false);
      setTermReveal(false);
      setIsFullscreen(true);
      setCountdownNum(null);
      setStats({ model: '--', cost: '--', tokens: '--', time: '--' });
      setGates([]);
      setLogEntries([]);
      setTimelineSteps([]);
      setProgressText('press Play to begin');
      setProgressLabel('--');
      setPipeline(SCENARIOS[idx]?.id === 'prd-pipeline' ? createPipelineIntroState(selectedPipelineExample) : EMPTY_PIPELINE_STATE);
      setKfInsights([]);
      setKfLeftAgent({ name: 'Alpha', color: 'var(--rose-bright)', posts: 0, confirms: 0 });
      setKfRightAgent({ name: 'Beta', color: 'var(--dream-bright)', posts: 0, confirms: 0 });
      setKfMetrics([
        { label: 'ALPHA COST', value: 0, format: (n) => `$${n.toFixed(2)}`, color: 'rose' },
        { label: 'BETA COST', value: 0, format: (n) => `$${n.toFixed(2)}`, color: 'dream' },
        { label: 'SAVINGS', value: 0, format: (n) => `${n.toFixed(0)}%`, color: 'bone' },
      ]);

      setScenarioAnim('enter');
      setTimeout(() => setScenarioAnim('idle'), 450);
    }, 260);
  }, [activeIdx, selectedPipelineExample]);

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

  const handlePlay = useCallback(async () => {
    if (runningRef.current) return;
    if (serverHealth !== 'connected') {
      const now = new Date();
      const ts = `${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}:${now.getSeconds().toString().padStart(2, '0')}`;
      setProgressLabel('Serve');
      setProgressText(`roko serve is ${serverHealth}; waiting for ${SERVE_URL}/health`);
      setLogEntries((prev) => [
        ...prev,
        {
          ts,
          text: `Cannot start the demo until roko serve is reachable at ${SERVE_URL}.`,
          type: 'error' as const,
        },
      ]);
      return;
    }

    runningRef.current = true;
    pausedRef.current = false;
    setScenarioComplete(false);
    setShowBurst(false);
    setShowCompletionOverlay(false);
    if (completionOverlayTimer.current) clearTimeout(completionOverlayTimer.current);
    if (completionAutoDismissTimer.current) clearTimeout(completionAutoDismissTimer.current);

    // ── 3-2-1 Countdown ──
    setLaunchingBtn(true);
    setIntroDismissing(true);
    setTimeout(() => {
      setShowIntro(false);
      setIntroDismissing(false);
    }, 550);

    // Run countdown: 3, 2, 1
    for (const n of [3, 2, 1]) {
      setCountdownNum(n);
      await sleep(800);
    }
    setCountdownNum(null);

    // Transition from fullscreen to split layout
    setTermBlackout(true);
    setIsFullscreen(false);
    await sleep(600);
    setTermBlackout(false);
    setLaunchingBtn(false);

    // Now reveal terminals
    setTermReveal(true);
    setTimeout(() => setTermReveal(false), 600);
    setIsRunning(true);
    setIsPaused(false);

    setLogEntries([]);
    setStats({ model: '--', cost: '--', tokens: '--', time: '--' });
    setGates([]);
    setPipeline(scenario.id === 'prd-pipeline' ? createPipelineIntroState(selectedPipelineExample) : EMPTY_PIPELINE_STATE);
    setKfInsights([]);
    setKfLeftAgent({ name: 'Alpha', color: 'var(--rose-bright)', posts: 0, confirms: 0 });
    setKfRightAgent({ name: 'Beta', color: 'var(--dream-bright)', posts: 0, confirms: 0 });
    setKfMetrics([
      { label: 'ALPHA COST', value: 0, format: (n) => `$${n.toFixed(2)}`, color: 'rose' },
      { label: 'BETA COST', value: 0, format: (n) => `$${n.toFixed(2)}`, color: 'dream' },
      { label: 'SAVINGS', value: 0, format: (n) => `${n.toFixed(0)}%`, color: 'bone' },
    ]);

    markStart('terminal-connect');
    const entries = await waitForTerminalReadiness();
    markEnd('terminal-connect');
    const termConnectMs = measure('terminal-connect');
    if (termConnectMs !== null) {
      console.debug(`[perf] terminal-connect: ${termConnectMs.toFixed(1)}ms`);
    }
    if (!entries) {
      const connected = getReadyTerminalEntries().length;
      const now = new Date();
      const ts = `${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}:${now.getSeconds().toString().padStart(2, '0')}`;
      console.error(
        `Timed out waiting for terminals: need ${scenario.panes} but only ${connected} connected`,
      );
      runningRef.current = false;
      setIsRunning(false);
      setProgressLabel('Terminals');
      setProgressText(`terminal connection timed out (${connected}/${scenario.panes})`);
      setLogEntries((prev) => [
        ...prev,
        {
          ts,
          text: `Timed out waiting for ${scenario.panes} terminal connection${scenario.panes === 1 ? '' : 's'}.`,
          type: 'error' as const,
        },
      ]);
      return;
    }
    try {
      setProgressLabel('Workspace');
      setProgressText(`creating live workspace for ${scenario.title}`);
      markStart('workspace-create');
      const ws = await ensureWorkspace(`roko-${scenario.id}`);
      markEnd('workspace-create');
      const wsMs = measure('workspace-create');
      if (wsMs !== null) {
        console.debug(`[perf] workspace-create: ${wsMs.toFixed(1)}ms`);
      }
      workspaceDirRef.current = ws.path;
      const ctx = buildContext(ws.path, entries);
      markStart('scenario-run');
      await scenario.run(ctx);
      markEnd('scenario-run');
      const scenarioMs = measure('scenario-run');
      if (scenarioMs !== null) {
        console.debug(`[perf] scenario-run: ${scenarioMs.toFixed(1)}ms`);
      }

      // Completion celebration
      setCompletedScenarios(prev => new Set(prev).add(activeIdx));
      setScenarioComplete(true);
      setShowBurst(true);
      toast(`Scenario complete: ${scenario.title}`, { type: 'success' });
      setTimeout(() => {
        setScenarioComplete(false);
        setShowBurst(false);
      }, 1400);

      // Show completion overlay after confetti settles
      if (completionOverlayTimer.current) clearTimeout(completionOverlayTimer.current);
      if (completionAutoDismissTimer.current) clearTimeout(completionAutoDismissTimer.current);
      completionOverlayTimer.current = setTimeout(() => {
        setShowCompletionOverlay(true);
        // Auto-dismiss after 8s
        completionAutoDismissTimer.current = setTimeout(() => {
          setShowCompletionOverlay(false);
        }, 8000);
      }, 1000);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Scenario error:', err);
      toast(msg, { type: 'error', duration: 5000 });
      const now = new Date();
      const ts = `${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}:${now.getSeconds().toString().padStart(2, '0')}`;
      setProgressLabel('Error');
      setProgressText(msg);
      setLogEntries((prev) => [
        ...prev,
        { ts, text: `Workspace creation failed: ${msg}`, type: 'error' as const },
      ]);
    }

    runningRef.current = false;
    setIsRunning(false);
    setIsPaused(false);
  }, [scenario, serverHealth, buildContext, selectedPipelineExample, waitForTerminalReadiness, getReadyTerminalEntries, ensureWorkspace]);

  const handlePauseResume = useCallback(() => {
    pausedRef.current = !isPaused;
    setIsPaused(!isPaused);
  }, [isPaused]);

  const handleStep = useCallback(() => {
    playback.advanceStep();
  }, []);

  const dismissCompletionOverlay = useCallback(() => {
    setShowCompletionOverlay(false);
    if (completionAutoDismissTimer.current) clearTimeout(completionAutoDismissTimer.current);
  }, []);

  const handleNextScenario = useCallback(() => {
    dismissCompletionOverlay();
    const nextIdx = (activeIdx + 1) % SCENARIOS.length;
    selectScenario(nextIdx);
  }, [activeIdx, dismissCompletionOverlay, selectScenario]);

  const handleRunAgain = useCallback(() => {
    dismissCompletionOverlay();
    // Small delay so overlay dismisses visually before re-play
    setTimeout(() => handlePlay(), 150);
  }, [dismissCompletionOverlay, handlePlay]);

  const handleReset = useCallback(() => {
    runningRef.current = false;
    pausedRef.current = false;
    setIsRunning(false);
    setIsPaused(false);
    setIsFullscreen(true);
    setCountdownNum(null);
    playback.reset();
    timeline.reset();
    clearMarks();
    selectScenario(activeIdx);
  }, [activeIdx, selectScenario]);

  useEffect(() => {
    setSpeedMultiplier(SPEEDS[speedIdx]);
  }, [speedIdx]);

  const toggleMode = useCallback((mode: 'auto' | 'step') => {
    setPlaybackMode(mode);
    playback.setMode(mode);
  }, []);

  // ── Sliding tab indicator ────────────────────────────────────

  useEffect(() => {
    const tab = tabRefs.current[activeIdx];
    const indicator = indicatorRef.current;
    const list = tabListRef.current;
    if (!tab || !indicator || !list) return;
    const listRect = list.getBoundingClientRect();
    const tabRect = tab.getBoundingClientRect();
    const cat = TAB_CATEGORY[SCENARIOS[activeIdx]?.id ?? ''] ?? 'pipeline';
    indicator.style.left = `${tabRect.left - listRect.left + list.scrollLeft}px`;
    indicator.style.width = `${tabRect.width}px`;
    indicator.style.background = CAT_COLORS[cat] ?? 'var(--rose-bright)';
  }, [activeIdx]);

  // ── Tab scroll fade edges ──────────────────────────────────

  useEffect(() => {
    const list = tabListRef.current;
    if (!list) return;
    const check = () => {
      setTabScrollState({
        left: list.scrollLeft > 4,
        right: list.scrollLeft < list.scrollWidth - list.clientWidth - 4,
      });
    };
    check();
    list.addEventListener('scroll', check, { passive: true });
    const ro = new ResizeObserver(check);
    ro.observe(list);
    return () => { list.removeEventListener('scroll', check); ro.disconnect(); };
  }, []);

  // ── Keyboard shortcuts ──────────────────────────────────────

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
      if (e.code === 'Space') {
        e.preventDefault();
        if (isRunning) handlePauseResume();
        else handlePlay();
      }
      if (e.code === 'KeyN' && !e.metaKey && !e.ctrlKey) { e.preventDefault(); handleStep(); }
      if (e.code === 'KeyR' && !e.metaKey && !e.ctrlKey) { e.preventDefault(); handleReset(); }
      if (e.code === 'KeyT' && !e.metaKey && !e.ctrlKey) { e.preventDefault(); setBottomTermOpen((v) => !v); }
      const n = parseInt(e.key);
      if (n >= 1 && n <= SCENARIOS.length && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        selectScenario(n - 1);
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [isRunning, handlePauseResume, handlePlay, handleStep, handleReset, selectScenario]);

  // ── URL hygiene ─────────────────────────────────────────────

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    if (params.has('autoplay')) {
      params.delete('autoplay');
      const query = params.toString();
      const next = `${window.location.pathname}${query ? `?${query}` : ''}${window.location.hash}`;
      window.history.replaceState(null, '', next);
    }
  }, []);

  // ── Session IDs (regenerated on scenario switch) ────────────

  const sessionIds = useMemo(
    () => scenario.labels.map((_, i) => `demo-${scenario.id}-${i}-${Date.now()}`),
    [scenario.id, scenario.labels],
  );

  // Create refs synchronously during render (not in useEffect) so they're
  // available when child TerminalPaneWithHandle components mount and write
  // their handles. Using useEffect caused a race: children wrote to old refs
  // that were then replaced by the parent effect.
  const stableRefs = useMemo(
    () => Array.from(
      { length: scenario.panes },
      () => ({ current: null }) as React.RefObject<TerminalHandle | null>,
    ),
    [scenario.id, scenario.panes],
  );
  handleRefs.current = stableRefs;

  useEffect(() => {
    setTerminalStates(Array.from(
      { length: scenario.panes },
      () => ({ status: 'connecting' as const, connected: false }),
    ));
  }, [scenario.id, scenario.panes]);

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

  // ── Timeline display ────────────────────────────────────────

  const timelineDisplay = timelineSteps.map((s) => ({
    label: s.label,
    detail: s.sublabel,
    status: s.status === 'completed' ? ('done' as const) : s.status,
  }));

  const hasStats = stats.model !== '--' || stats.cost !== '--' || stats.tokens !== '--' || stats.time !== '--';
  const hasKfMetrics = kfMetrics.some((m) => m.value > 0);

  // T7.57: Detect when all gates pass for CrystallizeTransition
  useEffect(() => {
    if (gates.length > 0 && gates.every((g) => g.status === 'pass')) {
      setAllGatesPass(true);
      // Auto-dismiss after 3s
      if (allGatesPassTimer.current) clearTimeout(allGatesPassTimer.current);
      allGatesPassTimer.current = setTimeout(() => setAllGatesPass(false), 3000);
    } else {
      setAllGatesPass(false);
    }
  }, [gates]);

  // T7.59: Track model from stats for ModelSlot
  useEffect(() => {
    if (stats.model !== '--') {
      setInferenceModel(stats.model);
      // Infer tier from model name heuristic
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

  // Map gates to GateVerdictCard GateEntry[] format
  const gateEntries: GateEntry[] = useMemo(
    () => gates.map((g) => ({
      name: g.name,
      status: g.status as GateEntry['status'],
    })),
    [gates],
  );

  // Grid class for 4-pane scenarios (2x2)
  const gridCols = scenario.panes === 4 ? 2 : scenario.panes;

  return (
    <div className="demo-page">
      {/* ── Top bar (tabs + merged playback) ── */}
      <div className={`demo-tabs-bar${isRunning ? ' demo-tabs-bar--running' : ''}`}>
        {/* Progress fill bar */}
        {isRunning && progressTotal > 0 && (
          <div
            className="demo-topbar-fill"
            style={{ width: `${(progressStep / progressTotal) * 100}%` }}
          />
        )}

        <div className={`demo-tab-list-wrapper${tabScrollState.left ? ' scroll-left' : ''}${tabScrollState.right ? ' scroll-right' : ''}`}>
          <div className="demo-tab-list" ref={tabListRef}>
            {SCENARIOS.map((s, i) => {
              const cat = TAB_CATEGORY[s.id] ?? 'pipeline';
              return (
                <Tooltip content={s.subtitle} placement="bottom" key={s.id}>
                  <button
                    ref={(el) => { tabRefs.current[i] = el; }}
                    className={[
                      'demo-tab btn-ghost-reveal',
                      activeIdx === i ? 'active' : '',
                      `cat-${cat}`,
                    ].filter(Boolean).join(' ')}
                    onClick={() => selectScenario(i)}
                  >
                    <span className={`demo-tab-accent cat-${cat}`} />
                    <span className="demo-tab-num">{i + 1}</span>
                    {s.title}
                    {completedScenarios.has(i) && <span className="demo-tab-done">&#10003;</span>}
                  </button>
                </Tooltip>
              );
            })}
            <div className="demo-tab-indicator" ref={indicatorRef} />
          </div>
        </div>
        <div className="demo-controls">
          <div className={`demo-serve-status demo-serve-${serverHealth}`}>
            {serverHealth === 'connected'
              ? <PulseIcon size={10} color="var(--success)" />
              : serverHealth === 'checking'
                ? <SpinnerIcon size={10} />
                : <CrossIcon size={10} color="var(--rose-bright)" />}
            {serverHealth === 'connected' ? 'serve live' : serverHealth === 'checking' ? 'checking serve' : 'serve offline'}
          </div>

          {/* Playback controls — merged into top bar */}
          {isRunning ? (
            <button className="demo-ctrl-btn btn-interactive" onClick={handlePauseResume} title="Pause (Space)">
              {isPaused ? '\u25B6' : '\u275A\u275A'}
            </button>
          ) : (
            <button
              className="demo-ctrl-btn play btn-primary-glow"
              onClick={handlePlay}
              title={`Play (Space) — ${readyTerminalCount}/${scenario.panes} terminals ready`}
            >
              {'\u25B6'}
            </button>
          )}
          <button
            className={`demo-ctrl-btn btn-interactive${playbackMode === 'step' ? ' play' : ''}${waitingForStep ? ' waiting' : ''}`}
            onClick={handleStep}
            title="Next step (N)"
            disabled={playbackMode !== 'step' && !waitingForStep}
          >
            {waitingForStep ? 'N' : '\u25B6\u2759'}
          </button>
          <button className="demo-ctrl-btn btn-interactive" onClick={handleReset} title="Reset (R)">
            {'\u21BA'}
          </button>

          {/* Mode toggle */}
          <div className="demo-mode-toggle">
            <div className={`demo-mode-toggle-track${playbackMode === 'step' ? ' at-step' : ''}`} />
            <button
              className={`demo-mode-btn${playbackMode === 'auto' ? ' active' : ''}`}
              onClick={() => toggleMode('auto')}
            >
              Auto
            </button>
            <button
              className={`demo-mode-btn${playbackMode === 'step' ? ' active' : ''}`}
              onClick={() => toggleMode('step')}
            >
              Step
            </button>
          </div>

          {/* Speed pills */}
          <div className="demo-pb-speed-pills">
            {SPEEDS.map((s, i) => (
              <button
                key={s}
                className={`demo-pb-speed-pill${i === speedIdx ? ' active' : ''}`}
                onClick={() => setSpeedIdx(i)}
              >
                {s}x
              </button>
            ))}
          </div>

          {/* Progress + command preview (visible when running) */}
          {isRunning && (
            <div className="demo-topbar-playback">
              <div className="demo-topbar-progress">
                <span className="demo-pb-step-label">{progressLabel}</span>
              </div>
              <div className="demo-topbar-cmd">{progressText}</div>
            </div>
          )}

          {/* Bottom terminal toggle */}
          <Tooltip content={bottomTermOpen ? 'Hide shell' : 'Open shell'} placement="bottom">
            <button
              className={`demo-ctrl-btn btn-interactive${bottomTermOpen ? ' play' : ''}`}
              onClick={() => setBottomTermOpen((v) => !v)}
              title="Toggle shell (T)"
            >
              {'\u2318'}
            </button>
          </Tooltip>
        </div>
      </div>

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
        speed={SPEEDS[speedIdx]}
      />

      {/* ── Main content ── */}
      <div className={[
        'demo-main',
        scenario.id === 'prd-pipeline' ? 'demo-main-pipeline' : '',
        isFullscreen ? 'demo-main--fullscreen' : '',
      ].filter(Boolean).join(' ')}>
        {/* Terminal zone */}
        <div className={[
          'demo-terminals',
          isRunning ? 'gradient-border-active' : 'gradient-border-subtle',
          scenarioAnim === 'exit' ? 'scenario-exit' : '',
          scenarioAnim === 'enter' ? 'scenario-enter' : '',
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

          {/* ── Countdown overlay ── */}
          {countdownNum !== null && (
            <div className="demo-countdown-overlay">
              <span key={countdownNum} className="demo-countdown-num">{countdownNum}</span>
              <span className="demo-countdown-label">launching {scenario.title}</span>
            </div>
          )}

          {/* ── Completion summary overlay ── */}
          {showCompletionOverlay && (
            <DemoCompletionOverlay
              title={scenario.title}
              stats={stats}
              gates={gates}
              onDismiss={dismissCompletionOverlay}
              onRunAgain={handleRunAgain}
              onNextScenario={handleNextScenario}
              hasNext={SCENARIOS.length > 1}
            />
          )}

          {(showIntro || introDismissing) && scenario.id !== 'prd-pipeline' && (
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
                key={`${scenario.id}-${i}-${sessionIds[i]}`}
                sessionId={sessionIds[i]}
                label={scenario.labels[i] || `pane ${i + 1}`}
                handleRef={handleRefs.current[i]}
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

        {/* Sidebar */}
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
      </div>

      {/* ── Collapsible bottom terminal ── */}
      <div className={`demo-bottom-terminal-wrapper ${bottomTermOpen ? 'expanded' : 'collapsed'}`}>
        <div
          className="demo-bottom-handle"
          onClick={() => setBottomTermOpen((v) => !v)}
        >
          <span className="demo-bottom-handle-grip" />
          <span className="demo-bottom-handle-label">
            {bottomTermOpen ? 'shell' : 'open shell'}
          </span>
          <button className="demo-bottom-handle-toggle">
            {bottomTermOpen ? '\u25BC' : '\u25B2'}
          </button>
        </div>
        {bottomTermOpen && (
          <BottomTerminalPane
            sessionId={bottomTermSessionId.current}
            handleRef={bottomTermHandleRef}
            workspaceDir={workspaceDirRef.current}
          />
        )}
      </div>
    </div>
  );
}

/* ── Completion summary overlay ─────────────────────────────── */

function useCountUp(target: number, duration = 600): number {
  const [value, setValue] = useState(0);
  const frameRef = useRef<number>(0);
  useEffect(() => {
    const start = performance.now();
    const tick = (now: number) => {
      const progress = Math.min((now - start) / duration, 1);
      const eased = 1 - (1 - progress) * (1 - progress);
      setValue(Math.round(target * eased));
      if (progress < 1) frameRef.current = requestAnimationFrame(tick);
    };
    frameRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frameRef.current);
  }, [target, duration]);
  return value;
}

function DemoCompletionOverlay({
  title, stats, gates, onDismiss, onRunAgain, onNextScenario, hasNext,
}: {
  title: string;
  stats: { model: string; cost: string; tokens: string; time: string };
  gates: { name: string; status: 'pass' | 'fail' | 'pending' }[];
  onDismiss: () => void;
  onRunAgain: () => void;
  onNextScenario: () => void;
  hasNext: boolean;
}) {
  const [dismissing, setDismissing] = useState(false);
  const passCount = gates.filter((g) => g.status === 'pass').length;
  const failCount = gates.filter((g) => g.status === 'fail').length;
  const animatedPass = useCountUp(passCount, 500);
  const animatedFail = useCountUp(failCount, 500);
  const doDismiss = useCallback(() => { setDismissing(true); setTimeout(() => onDismiss(), 350); }, [onDismiss]);

  return (
    <div className={`demo-completion-overlay${dismissing ? ' dismissing' : ''}`} onClick={doDismiss}>
      <div className="demo-completion-card" onClick={(e) => e.stopPropagation()}>
        <div className="demo-completion-header">
          <span className="demo-completion-title">{title}</span>
          <span className="demo-completion-badge">COMPLETE</span>
        </div>
        <div className="demo-completion-stats">
          {stats.model !== '--' && (
            <div className="demo-completion-stat">
              <span className="demo-completion-stat-label">MODEL</span>
              <span className="demo-completion-stat-value mono">{stats.model}</span>
            </div>
          )}
          {stats.cost !== '--' && (
            <div className="demo-completion-stat">
              <span className="demo-completion-stat-label">COST</span>
              <span className="demo-completion-stat-value mono">{stats.cost}</span>
            </div>
          )}
          {stats.time !== '--' && (
            <div className="demo-completion-stat">
              <span className="demo-completion-stat-label">DURATION</span>
              <span className="demo-completion-stat-value mono">{stats.time}</span>
            </div>
          )}
          {gates.length > 0 && (
            <div className="demo-completion-stat">
              <span className="demo-completion-stat-label">GATES</span>
              <span className="demo-completion-stat-value demo-completion-gates">
                <span className="demo-completion-gate-pass">
                  <CheckmarkIcon size={12} color="var(--success)" />
                  {animatedPass}
                </span>
                {failCount > 0 && (
                  <span className="demo-completion-gate-fail">
                    <CrossIcon size={12} color="var(--rose-bright)" />
                    {animatedFail}
                  </span>
                )}
              </span>
            </div>
          )}
        </div>
        <div className="demo-completion-actions">
          <button className="demo-completion-btn demo-completion-btn-again" onClick={(e) => { e.stopPropagation(); setDismissing(true); setTimeout(() => onRunAgain(), 350); }}>
            Run Again
          </button>
          {hasNext && (
            <button className="demo-completion-btn demo-completion-btn-next" onClick={(e) => { e.stopPropagation(); setDismissing(true); setTimeout(() => onNextScenario(), 350); }}>
              Next Scenario {'\u2192'}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

/** Map scenario + label to a CSS color for scenario-aware label styling */
function getLabelColor(scenarioId: string, label: string, category: string): string | undefined {
  const lower = label.toLowerCase();
  if (scenarioId === 'race') {
    if (lower.includes('naive')) return 'var(--warning)';
    if (lower.includes('cascade')) return '#6bb8a8';
  }
  if (scenarioId === 'providers' || scenarioId === 'provider-race') {
    if (lower.includes('anthropic')) return 'var(--rose-bright)';
    if (lower.includes('openai')) return '#6bb87a';
    if (lower.includes('zhipu')) return '#68a8d8';
    if (lower.includes('gemini')) return '#68a8d8';
    if (lower.includes('moonshot')) return 'var(--warning)';
    return 'var(--dream-bright)';
  }
  if (scenarioId === 'explore') {
    if (lower.includes('workspace') || lower.includes('status')) return 'var(--success)';
    if (lower.includes('learn')) return 'var(--dream-bright)';
    if (lower.includes('config')) return 'var(--warning)';
    if (lower.includes('knowledge')) return '#b888d8';
    return 'var(--rose-glow)';
  }
  if (category === 'comparison') return '#6bb8a8';
  if (category === 'learning') return 'var(--dream-bright)';
  if (category === 'chain') return 'var(--warning)';
  if (category === 'exploration') return 'var(--success)';
  return undefined;
}

function TerminalPaneWithHandle({
  sessionId,
  label,
  handleRef,
  paneIndex,
  onStatusChange,
  termReveal,
  scenarioId,
  scenarioCategory,
  isRunning,
}: {
  sessionId: string;
  label: string;
  handleRef: React.RefObject<TerminalHandle | null> | undefined;
  paneIndex: number;
  onStatusChange?: (index: number, state: TerminalPaneState) => void;
  termReveal?: boolean;
  scenarioId: string;
  scenarioCategory: string;
  isRunning: boolean;
}) {
  const { attach, status, handle } = useTerminal(sessionId);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [hasOutput, setHasOutput] = useState(false);
  const [cmdEcho, setCmdEcho] = useState<string | null>(null);
  const cmdEchoTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [focused, setFocused] = useState(false);

  useEffect(() => {
    if (handleRef && 'current' in handleRef) {
      (handleRef as React.MutableRefObject<TerminalHandle | null>).current = handle.current;
    }
    onStatusChange?.(paneIndex, {
      status,
      connected: status === 'connected' && handle.current?.ws?.readyState === WebSocket.OPEN,
    });
  }, [handleRef, handle, onStatusChange, paneIndex, status]);

  // Detect output activity via DOM mutations on the terminal body
  useEffect(() => {
    const body = bodyRef.current;
    if (!body) return;
    let activityTimeout: ReturnType<typeof setTimeout> | null = null;
    const observer = new MutationObserver(() => {
      setHasOutput(true);
      if (activityTimeout) clearTimeout(activityTimeout);
      activityTimeout = setTimeout(() => setHasOutput(false), 800);
    });
    observer.observe(body, { childList: true, subtree: true, characterData: true });
    return () => {
      observer.disconnect();
      if (activityTimeout) clearTimeout(activityTimeout);
    };
  }, []);

  // Command echo: listen for typed commands via custom event
  useEffect(() => {
    function onCmdTyped(e: Event) {
      const detail = (e as CustomEvent<{ sessionId: string; cmd: string }>).detail;
      if (detail.sessionId !== sessionId) return;
      setCmdEcho(detail.cmd);
      if (cmdEchoTimer.current) clearTimeout(cmdEchoTimer.current);
      cmdEchoTimer.current = setTimeout(() => setCmdEcho(null), 2000);
    }
    window.addEventListener('roko-cmd-typed', onCmdTyped);
    return () => {
      window.removeEventListener('roko-cmd-typed', onCmdTyped);
      if (cmdEchoTimer.current) clearTimeout(cmdEchoTimer.current);
    };
  }, [sessionId]);

  const revealClass = termReveal
    ? `term-reveal ${paneIndex % 2 === 0 ? 'from-left' : 'from-right'}`
    : '';
  const revealDelay = termReveal ? { animationDelay: `${paneIndex * 80}ms` } : undefined;
  const labelColor = getLabelColor(scenarioId, label, scenarioCategory);
  const labelStyle = labelColor
    ? { color: labelColor, textShadow: `0 0 8px ${labelColor}44` } as const
    : undefined;
  const paneClasses = [
    'demo-term-pane',
    `demo-term-${status}`,
    revealClass,
    focused ? 'demo-term-focused' : '',
  ].filter(Boolean).join(' ');

  const bodyCallbackRef = useCallback(
    (node: HTMLDivElement | null) => {
      (bodyRef as React.MutableRefObject<HTMLDivElement | null>).current = node;
      attach(node);
    },
    [attach],
  );

  return (
    <div
      className={paneClasses}
      style={revealDelay}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      tabIndex={-1}
    >
      <div className="demo-term-head">
        <span className="demo-term-num">{paneIndex + 1}</span>
        <Tooltip content={status === 'connected' ? 'Terminal connected' : status === 'connecting' ? 'Connecting...' : 'Disconnected'} placement="right" variant="code">
          <span className={`demo-term-dot ${status}`}>
            {status === 'connected'
              ? <PulseIcon size={8} color="var(--success)" />
              : status === 'connecting'
                ? <SpinnerIcon size={8} />
                : <CrossIcon size={8} color="var(--rose-dim)" />}
          </span>
        </Tooltip>
        <span className="demo-term-label" style={labelStyle}>{'\u2308'} {label} {'\u230B'}</span>
        {hasOutput && isRunning && (
          <span className="demo-term-waveform">
            <WaveformIcon size={10} color={labelColor ?? 'var(--rose-dim)'} />
          </span>
        )}
        <span className="demo-term-status">{status}</span>
      </div>
      {cmdEcho && (
        <div className="demo-term-cmd-echo">{cmdEcho}</div>
      )}
      <div className="demo-term-body" ref={bodyCallbackRef} />
      <div className="demo-term-vignette" />
    </div>
  );
}

/* ── Collapsible bottom terminal ─────────────────────────────── */

function BottomTerminalPane({
  sessionId,
  handleRef,
  workspaceDir,
}: {
  sessionId: string;
  handleRef: React.RefObject<TerminalHandle | null>;
  workspaceDir: string;
}) {
  const { attach, status, handle } = useTerminal(sessionId);
  const bodyRef = useRef<HTMLDivElement>(null);
  const cdSent = useRef(false);

  useEffect(() => {
    if (handleRef && 'current' in handleRef) {
      (handleRef as React.MutableRefObject<TerminalHandle | null>).current = handle.current;
    }
  }, [handleRef, handle]);

  // Auto-cd into workspace when connected
  useEffect(() => {
    if (status === 'connected' && workspaceDir && !cdSent.current && handle.current?.ws?.readyState === WebSocket.OPEN) {
      cdSent.current = true;
      handle.current.sendRaw(`cd ${workspaceDir}\r`);
    }
  }, [status, workspaceDir, handle]);

  const bodyCallbackRef = useCallback(
    (node: HTMLDivElement | null) => {
      (bodyRef as React.MutableRefObject<HTMLDivElement | null>).current = node;
      attach(node);
    },
    [attach],
  );

  return (
    <div className="demo-bottom-term-body" ref={bodyCallbackRef}>
      {status === 'connecting' && (
        <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <SpinnerIcon size={16} />
        </div>
      )}
    </div>
  );
}
