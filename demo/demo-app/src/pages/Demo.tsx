import { useState, useCallback, useRef, useEffect, useMemo } from 'react';
import { SCENARIOS, type ScenarioContext } from '../lib/scenarios';
import { PlaybackController, TimelineStepper, type TimelineStepState } from '../lib/playback-controller';
import { useTerminal, type TerminalHandle } from '../hooks/useTerminal';
import { setSpeedMultiplier } from '../lib/terminal-session';
import { useServerHealth } from '../hooks/useServerHealth';
import { useRokoConfig } from '../hooks/useRokoConfig';
import { useWorkspace } from '../hooks/useWorkspace';
import { lookupCmdDesc } from '../lib/cmd-descriptions';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import Timeline from '../components/Timeline';
import CommandLog from '../components/CommandLog';
import GateBar from '../components/GateBar';
import PrdPipelinePanel from '../components/PrdPipelinePanel';
import KnowledgeFlowPanel, { type InsightEvent, type AgentInfo } from '../components/KnowledgeFlowPanel';
import EfficiencyBar, { type EfficiencyMetric } from '../components/EfficiencyBar';
import ChainIntelPanel from '../components/ChainIntelPanel';
import RevealWhen from '../components/RevealWhen';
import { useChainWs, type InsightEvent as ChainInsightEvent } from '../hooks/useChain';
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
import '@xterm/xterm/css/xterm.css';
import '../components/Terminal/TerminalPane.css';
import './Demo.css';

// Singleton controllers for the page lifetime
const playback = new PlaybackController();
const timeline = new TimelineStepper();

const SPEEDS = [0.5, 1, 2, 4];

type TerminalPaneState = {
  status: TerminalHandle['status'];
  connected: boolean;
};

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export default function Demo() {
  const [activeIdx, setActiveIdx] = useState(0);
  const [showIntro, setShowIntro] = useState(true);
  const [isRunning, setIsRunning] = useState(false);
  const [isPaused, setIsPaused] = useState(false);
  const [speedIdx, setSpeedIdx] = useState(1);
  const [playbackMode, setPlaybackMode] = useState<'auto' | 'step'>('auto');
  const scenario = SCENARIOS[activeIdx];
  const serverHealth = useServerHealth();
  const { defaultModel } = useRokoConfig();
  const { ensureWorkspace, createWorkspace: createWs } = useWorkspace();

  // Sidebar state
  const [stats, setStats] = useState({ model: '--', cost: '--', tokens: '--', time: '--' });
  const [gates, setGates] = useState<{ name: string; status: 'pass' | 'fail' | 'pending' }[]>([]);
  const [logEntries, setLogEntries] = useState<{ ts: string; text: string; type?: 'info' | 'success' | 'error' }[]>([]);
  const [timelineSteps, setTimelineSteps] = useState<TimelineStepState[]>([]);
  const [progressText, setProgressText] = useState('press Play to begin');
  const [progressLabel, setProgressLabel] = useState('--');
  const [waitingForStep, setWaitingForStep] = useState(false);
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
    });
    playback.onWaitingChange(setWaitingForStep);
  }, []);

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
    if (runningRef.current) {
      runningRef.current = false;
      setIsRunning(false);
    }
    playback.reset();
    timeline.reset();
    setActiveIdx(idx);
    setShowIntro(true);
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
  }, [selectedPipelineExample]);

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
    setIsRunning(true);
    setIsPaused(false);
    setShowIntro(false);
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

    const entries = await waitForTerminalReadiness();
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
      const ws = await ensureWorkspace(`roko-${scenario.id}`);
      workspaceDirRef.current = ws.path;
      const ctx = buildContext(ws.path, entries);
      await scenario.run(ctx);
    } catch (err) {
      console.error('Scenario error:', err);
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

  const handleReset = useCallback(() => {
    runningRef.current = false;
    pausedRef.current = false;
    setIsRunning(false);
    setIsPaused(false);
    playback.reset();
    timeline.reset();
    selectScenario(activeIdx);
  }, [activeIdx, selectScenario]);

  const cycleSpeed = useCallback(() => {
    setSpeedIdx((prev) => (prev + 1) % SPEEDS.length);
  }, []);

  useEffect(() => {
    setSpeedMultiplier(SPEEDS[speedIdx]);
  }, [speedIdx]);

  const toggleMode = useCallback((mode: 'auto' | 'step') => {
    setPlaybackMode(mode);
    playback.setMode(mode);
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

  // Grid class for 4-pane scenarios (2x2)
  const gridCols = scenario.panes === 4 ? 2 : scenario.panes;

  return (
    <div className="demo-page">
      {/* ── Top bar ── */}
      <div className="demo-tabs-bar">
        <div className="demo-tab-list">
          {SCENARIOS.map((s, i) => (
            <button
              key={s.id}
              className={`demo-tab${activeIdx === i ? ' active' : ''}`}
              onClick={() => selectScenario(i)}
            >
              <span className="demo-tab-num">{i + 1}</span>
              {s.title}
            </button>
          ))}
        </div>
        <div className="demo-controls">
          <div className={`demo-serve-status demo-serve-${serverHealth}`}>
            <span />
            {serverHealth === 'connected' ? 'serve live' : serverHealth === 'checking' ? 'checking serve' : 'serve offline'}
          </div>
          <button className="demo-speed" onClick={cycleSpeed}>
            {SPEEDS[speedIdx]}x
          </button>
          {isRunning ? (
            <button className="demo-ctrl-btn" onClick={handlePauseResume}>
              {isPaused ? '\u25B6' : '\u275A\u275A'}
            </button>
          ) : (
            <button
              className="demo-ctrl-btn play"
              onClick={handlePlay}
              title={`Play (${readyTerminalCount}/${scenario.panes} terminals ready)`}
            >
              {'\u25B6'}
            </button>
          )}
        </div>
      </div>

      {/* ── Main content ── */}
      <div className={`demo-main${scenario.id === 'prd-pipeline' ? ' demo-main-pipeline' : ''}`}>
        {/* Terminal zone */}
        <div className="demo-terminals">
          {showIntro && scenario.id !== 'prd-pipeline' && (
            <div className="demo-intro-overlay" onClick={handlePlay}>
              <div className="demo-intro-title">{scenario.title}</div>
              <div className="demo-intro-sub">{scenario.subtitle}</div>
              <button
                className="demo-play-btn"
                onClick={(e) => { e.stopPropagation(); handlePlay(); }}
              >
                {'\u25B6'}
              </button>
            </div>
          )}

          <div
            className={`demo-terminal-grid demo-cols-${gridCols}`}
            style={scenario.panes === 4 ? { gridTemplateRows: '1fr 1fr' } : undefined}
          >
            {Array.from({ length: scenario.panes }).map((_, i) => (
              <TerminalPaneWithHandle
                key={`${scenario.id}-${i}-${sessionIds[i]}`}
                sessionId={sessionIds[i]}
                label={scenario.labels[i] || `pane ${i + 1}`}
                handleRef={handleRefs.current[i]}
                paneIndex={i}
                onStatusChange={updateTerminalState}
              />
            ))}
          </div>
        </div>

        {/* Sidebar */}
        {scenario.panel && (
          <div className="demo-sidebar">
            {scenario.id === 'prd-pipeline' ? (
              <PrdPipelinePanel
                state={pipeline}
                examples={PIPELINE_EXAMPLES}
                selectedExampleId={pipelineExampleId}
                onSelectExample={handlePipelineExampleSelect}
                selectorDisabled={isRunning}
                onRun={handlePlay}
                isRunning={isRunning}
                serverHealth={serverHealth}
              />
            ) : scenario.id === 'knowledge-transfer' ? (
              <>
                <RevealWhen visible={timelineDisplay.length > 0}>
                  <Pane title="TIMELINE" flat>
                    <Timeline steps={timelineDisplay} />
                  </Pane>
                </RevealWhen>

                <RevealWhen visible={kfInsights.length > 0}>
                  <KnowledgeFlowPanel
                    leftAgent={kfLeftAgent}
                    rightAgent={kfRightAgent}
                    insights={kfInsights}
                    mode="local"
                  />
                </RevealWhen>

                <RevealWhen visible={hasKfMetrics}>
                  <EfficiencyBar metrics={kfMetrics} />
                </RevealWhen>

                <RevealWhen visible={gates.length > 0}>
                  <Pane title="GATES" flat>
                    <div style={{ padding: '12px 16px' }}>
                      <GateBar gates={gates} />
                    </div>
                  </Pane>
                </RevealWhen>

                <RevealWhen visible={logEntries.length > 0}>
                  <Pane title="LOG" flat>
                    <CommandLog entries={logEntries} maxHeight="180px" />
                  </Pane>
                </RevealWhen>
              </>
            ) : scenario.id === 'chain-intelligence' ? (
              <>
                <RevealWhen visible={timelineDisplay.length > 0}>
                  <Pane title="TIMELINE" flat>
                    <Timeline steps={timelineDisplay} />
                  </Pane>
                </RevealWhen>

                <RevealWhen visible={ciInsights.length > 0 || chainWs.connected}>
                  <ChainIntelPanel
                    leftAgent={ciLeftAgent}
                    rightAgent={ciRightAgent}
                    insights={ciInsights}
                    blocks={ciBlocks}
                    positions={ciPositions}
                    metrics={ciMetrics}
                    mirageConnected={chainWs.connected}
                  />
                </RevealWhen>

                <RevealWhen visible={logEntries.length > 0}>
                  <Pane title="LOG" flat>
                    <CommandLog entries={logEntries} maxHeight="140px" />
                  </Pane>
                </RevealWhen>
              </>
            ) : (
              <>
                <RevealWhen visible={timelineDisplay.length > 0}>
                  <Pane title="TIMELINE" flat>
                    <Timeline steps={timelineDisplay} />
                  </Pane>
                </RevealWhen>

                <RevealWhen visible={hasStats}>
                  <div className="demo-stats-mosaic">
                    <Mosaic columns={2}>
                      <MosaicCell label="MODEL" value={stats.model} mono color="rose" />
                      <MosaicCell label="COST" value={stats.cost} mono color="bone" />
                      <MosaicCell label="TOKENS" value={stats.tokens} mono color="dream" />
                      <MosaicCell label="TIME" value={stats.time} mono color="warning" />
                    </Mosaic>
                  </div>
                </RevealWhen>

                <RevealWhen visible={gates.length > 0}>
                  <Pane title="GATES" flat>
                    <div style={{ padding: '12px 16px' }}>
                      <GateBar gates={gates} />
                    </div>
                  </Pane>
                </RevealWhen>

                <RevealWhen visible={logEntries.length > 0}>
                  <Pane title="LOG" flat>
                    <CommandLog entries={logEntries} maxHeight="240px" />
                  </Pane>
                </RevealWhen>
              </>
            )}
          </div>
        )}
      </div>

      {/* ── Playback bar ── */}
      <div className="demo-playback-bar">
        <div className="demo-pb-controls">
          {isRunning ? (
            <button className="demo-pb-btn" onClick={handlePauseResume} title="Pause (Space)">
              {isPaused ? '\u25B6' : '\u275A\u275A'}
            </button>
          ) : (
            <button className="demo-pb-btn primary" onClick={handlePlay} title={`Play (Space) - ${readyTerminalCount}/${scenario.panes} terminals ready`}>
              {'\u25B6'}
            </button>
          )}
          <button
            className={`demo-pb-btn${playbackMode === 'step' ? ' primary' : ''}${waitingForStep ? ' waiting' : ''}`}
            onClick={handleStep}
            title="Next step (N)"
            disabled={playbackMode !== 'step' && !waitingForStep}
          >
            {waitingForStep ? 'NEXT' : '\u25B6\u2759'}
          </button>
          <button className="demo-pb-btn" onClick={handleReset} title="Reset (R)">
            {'\u21BA'}
          </button>
        </div>

        <div className="demo-mode-toggle">
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

        <div className="demo-pb-progress">
          <span className="demo-pb-step-label">{progressLabel}</span>
        </div>
        <div className="demo-pb-cmd-preview">{progressText}</div>
      </div>
    </div>
  );
}

function TerminalPaneWithHandle({
  sessionId,
  label,
  handleRef,
  paneIndex,
  onStatusChange,
}: {
  sessionId: string;
  label: string;
  handleRef: React.RefObject<TerminalHandle | null> | undefined;
  paneIndex: number;
  onStatusChange?: (index: number, state: TerminalPaneState) => void;
}) {
  const { attach, status, handle } = useTerminal(sessionId);

  useEffect(() => {
    if (handleRef && 'current' in handleRef) {
      (handleRef as React.MutableRefObject<TerminalHandle | null>).current = handle.current;
    }
    onStatusChange?.(paneIndex, {
      status,
      connected: status === 'connected' && handle.current?.ws?.readyState === WebSocket.OPEN,
    });
  }, [handleRef, handle, onStatusChange, paneIndex, status]);

  return (
    <div className="demo-term-pane">
      <div className="demo-term-head">
        <span className={`demo-term-dot ${status}`} />
        <span className="demo-term-label">{label}</span>
        <span className="demo-term-status">{status}</span>
      </div>
      <div className="demo-term-body" ref={attach} />
    </div>
  );
}
