import { useState, useCallback, useRef, useEffect, createRef } from 'react';
import { SCENARIOS } from '../../lib/scenarios';
import { setSpeedMultiplier } from '../../lib/terminal-session';
import { useServerHealth } from '../../hooks/useServerHealth';
import { useRokoConfig } from '../../hooks/useRokoConfig';
import { useWorkspace } from '../../hooks/useWorkspace';
import { useToast } from '../../components/Toast';
import Tooltip from '../../components/Tooltip';
import { useLearningStats } from '../../hooks/useLearningStats';
import { useAgentHandoffs } from '../../hooks/useAgentHandoffs';
import { PulseIcon, SpinnerIcon, CrossIcon } from '../../components/icons/AnimatedIcons';
import type { TerminalHandle } from '../../hooks/useTerminal';
import ConfigWidget from '../../components/ConfigWidget';
import BlockTicker from '../../components/BlockTicker';
import ScenarioSlot, { type ScenarioSlotHandle, type SlotStateReport } from './ScenarioSlot';
import BottomTerminalPane from './BottomTerminalPane';
import '@xterm/xterm/css/xterm.css';
import '../../components/Terminal/TerminalPane.css';
import './Demo.css';

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
  'isfr-agents': 'chain',
};

/** Color values per category (used for the sliding indicator) */
const CAT_COLORS: Record<string, string> = {
  pipeline: 'var(--rose-bright)',
  comparison: 'var(--status-active)',
  exploration: 'var(--dream-bright)',
  learning: 'var(--status-success)',
  chain: 'var(--warning)',
};

export default function Demo() {
  // ── Global state (shared across all slots) ─────────────────
  const [activeIdx, setActiveIdx] = useState(0);
  const [activated, setActivated] = useState<Set<number>>(() => new Set([0]));
  const [speedIdx, setSpeedIdx] = useState(1);
  const [playbackMode, setPlaybackMode] = useState<'auto' | 'step'>('auto');
  const [completedScenarios, setCompletedScenarios] = useState<Set<number>>(() => new Set());
  const [slotStates, setSlotStates] = useState<Map<number, SlotStateReport>>(() => new Map());

  // Slot refs for imperative control
  const slotRefs = useRef<Map<number, React.RefObject<ScenarioSlotHandle | null>>>(new Map());
  const getSlotRef = useCallback((idx: number) => {
    if (!slotRefs.current.has(idx)) {
      slotRefs.current.set(idx, createRef<ScenarioSlotHandle>());
    }
    return slotRefs.current.get(idx)!;
  }, []);

  // Bottom terminal (global, not per-scenario)
  const [bottomTermOpen, setBottomTermOpen] = useState(false);
  const bottomTermSessionId = useRef(`bottom-${Date.now().toString(36)}`);
  const bottomTermHandleRef = useRef<TerminalHandle | null>(null);

  // Tab bar refs
  const tabListRef = useRef<HTMLDivElement>(null);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const indicatorRef = useRef<HTMLDivElement>(null);
  const [tabScrollState, setTabScrollState] = useState({ left: false, right: false });

  // Shared hooks (called once, passed as props)
  const { status: serverHealth, checkNow: checkServeHealth } = useServerHealth();
  const { defaultModel } = useRokoConfig();
  const { ensureWorkspace, createWorkspace } = useWorkspace();
  const learningStats = useLearningStats();
  const { handoffs, activeHandoff } = useAgentHandoffs();
  const { toast } = useToast();

  // Current active slot state
  const activeState = slotStates.get(activeIdx);
  const activeIsRunning = activeState?.isRunning ?? false;
  const activeIsPaused = activeState?.isPaused ?? false;
  const activeWaitingForStep = activeState?.waitingForStep ?? false;
  const activeProgressLabel = activeState?.progressLabel ?? '--';
  const activeProgressText = activeState?.progressText ?? 'press Play to begin';
  const activeProgressStep = activeState?.progressStep ?? 0;
  const activeProgressTotal = activeState?.progressTotal ?? 0;
  const activeReadyTerminalCount = activeState?.readyTerminalCount ?? 0;
  const activeWorkspaceDir = activeState?.workspaceDir ?? '';

  const scenario = SCENARIOS[activeIdx];

  // ── Slot callbacks ─────────────────────────────────────────
  const handleSlotStateChange = useCallback((idx: number, state: SlotStateReport) => {
    setSlotStates((prev) => {
      const next = new Map(prev);
      next.set(idx, state);
      return next;
    });
  }, []);

  const handleSlotComplete = useCallback((idx: number) => {
    setCompletedScenarios((prev) => new Set(prev).add(idx));
  }, []);

  const handleNextScenario = useCallback((fromIdx: number) => {
    const nextIdx = (fromIdx + 1) % SCENARIOS.length;
    setActiveIdx(nextIdx);
    setActivated((prev) => prev.has(nextIdx) ? prev : new Set(prev).add(nextIdx));
  }, []);

  // ── Tab selection (trivial — just set active + lazy mount) ─
  const selectScenario = useCallback((idx: number) => {
    if (idx === activeIdx) return;
    setActiveIdx(idx);
    setActivated((prev) => prev.has(idx) ? prev : new Set(prev).add(idx));
  }, [activeIdx]);

  // ── Forwarded controls ─────────────────────────────────────
  const handlePlay = useCallback(() => {
    getSlotRef(activeIdx).current?.play();
  }, [activeIdx, getSlotRef]);

  const handlePauseResume = useCallback(() => {
    getSlotRef(activeIdx).current?.pauseResume();
  }, [activeIdx, getSlotRef]);

  const handleStep = useCallback(() => {
    getSlotRef(activeIdx).current?.step();
  }, [activeIdx, getSlotRef]);

  const handleReset = useCallback(() => {
    getSlotRef(activeIdx).current?.reset();
  }, [activeIdx, getSlotRef]);

  const toggleMode = useCallback((mode: 'auto' | 'step') => {
    setPlaybackMode(mode);
  }, []);

  // ── Speed ──────────────────────────────────────────────────
  useEffect(() => {
    setSpeedMultiplier(SPEEDS[speedIdx]);
  }, [speedIdx]);

  // ── Sliding tab indicator ──────────────────────────────────
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

  // ── Keyboard shortcuts ─────────────────────────────────────
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Never intercept keys when a terminal or input has focus
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
      if (e.target instanceof HTMLTextAreaElement) return; // xterm.js uses a hidden textarea
      if ((e.target as HTMLElement)?.closest?.('.demo-term-body')) return; // inside terminal pane
      if (e.code === 'Space') {
        e.preventDefault();
        if (activeIsRunning) handlePauseResume();
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
  }, [activeIsRunning, handlePauseResume, handlePlay, handleStep, handleReset, selectScenario]);

  // ── URL hygiene ────────────────────────────────────────────
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    if (params.has('autoplay')) {
      params.delete('autoplay');
      const query = params.toString();
      const next = `${window.location.pathname}${query ? `?${query}` : ''}${window.location.hash}`;
      window.history.replaceState(null, '', next);
    }
  }, []);

  // ── Render ─────────────────────────────────────────────────
  return (
    <div className="demo-page">
      {/* ── Top bar (tabs + merged playback) ── */}
      <div className={`demo-tabs-bar${activeIsRunning ? ' demo-tabs-bar--running' : ''}`}>
        {activeIsRunning && activeProgressTotal > 0 && (
          <div
            className="demo-topbar-fill"
            style={{ width: `${(activeProgressStep / activeProgressTotal) * 100}%` }}
          />
        )}

        <div className={`demo-tab-list-wrapper${tabScrollState.left ? ' scroll-left' : ''}${tabScrollState.right ? ' scroll-right' : ''}`}>
          <div className="demo-tab-list" ref={tabListRef}>
            {SCENARIOS.map((s, i) => {
              const cat = TAB_CATEGORY[s.id] ?? 'pipeline';
              const slotRunning = slotStates.get(i)?.isRunning ?? false;
              return (
                <Tooltip content={s.subtitle} placement="bottom" key={s.id}>
                  <button
                    ref={(el) => { tabRefs.current[i] = el; }}
                    className={[
                      'demo-tab btn-ghost-reveal',
                      activeIdx === i ? 'active' : '',
                      `cat-${cat}`,
                      slotRunning && activeIdx !== i ? 'running' : '',
                    ].filter(Boolean).join(' ')}
                    onClick={() => selectScenario(i)}
                  >
                    <span className={`demo-tab-accent cat-${cat}`} />
                    {slotRunning && activeIdx !== i && (
                      <span className="demo-tab-running-dot" />
                    )}
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

          {activeIsRunning ? (
            <button className="demo-ctrl-btn btn-interactive" onClick={handlePauseResume} title="Pause (Space)">
              {activeIsPaused ? '\u25B6' : '\u275A\u275A'}
            </button>
          ) : (
            <button
              className="demo-ctrl-btn play btn-primary-glow"
              onClick={handlePlay}
              title={`Play (Space) — ${activeReadyTerminalCount}/${scenario.panes} terminals ready`}
            >
              {'\u25B6'}
            </button>
          )}
          <button
            className={`demo-ctrl-btn btn-interactive${playbackMode === 'step' ? ' play' : ''}${activeWaitingForStep ? ' waiting' : ''}`}
            onClick={handleStep}
            title="Next step (N)"
            disabled={playbackMode !== 'step' && !activeWaitingForStep}
          >
            {activeWaitingForStep ? 'N' : '\u25B6\u2759'}
          </button>
          <button className="demo-ctrl-btn btn-interactive" onClick={handleReset} title="Reset (R)">
            {'\u21BA'}
          </button>

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

          {activeIsRunning && (
            <div className="demo-topbar-playback">
              <div className="demo-topbar-progress">
                <span className="demo-pb-step-label">{activeProgressLabel}</span>
              </div>
              <div className="demo-topbar-cmd">{activeProgressText}</div>
            </div>
          )}

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

      {/* ── Block ticker (live chain blocks) ── */}
      <BlockTicker enabled={!!SCENARIOS[activeIdx]?.mirageBar} />

      {/* ── Scenario slots (lazy-mounted, never unmounted) ── */}
      {SCENARIOS.map((s, i) => {
        if (!activated.has(i)) return null;
        return (
          <ScenarioSlot
            key={s.id}
            ref={getSlotRef(i)}
            scenario={s}
            scenarioIdx={i}
            active={activeIdx === i}
            playbackMode={playbackMode}
            serverHealth={serverHealth}
            checkServeHealth={checkServeHealth}
            defaultModel={defaultModel}
            learningStats={learningStats}
            handoffs={handoffs}
            activeHandoff={activeHandoff}
            ensureWorkspace={ensureWorkspace}
            createWorkspace={createWorkspace}
            toast={toast}
            onStateChange={handleSlotStateChange}
            onComplete={handleSlotComplete}
            onNextScenario={handleNextScenario}
          />
        );
      })}

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
            workspaceDir={activeWorkspaceDir}
          />
        )}
      </div>

      {/* ── Config widget (bottom-right pill) ── */}
      <ConfigWidget />
    </div>
  );
}
