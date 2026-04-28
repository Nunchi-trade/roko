import { useState, useCallback, useRef, useEffect, useMemo } from 'react';
import { SCENARIOS, type ScenarioContext } from '../lib/scenarios';
import { PlaybackController, TimelineStepper, type TimelineStepState } from '../lib/playback-controller';
import { useTerminal, type TerminalHandle } from '../hooks/useTerminal';
import { useServerHealth } from '../hooks/useServerHealth';
import { lookupCmdDesc } from '../lib/cmd-descriptions';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import Timeline from '../components/Timeline';
import CommandLog from '../components/CommandLog';
import GateBar from '../components/GateBar';
import ConnectScreen from '../components/ConnectScreen';
import '@xterm/xterm/css/xterm.css';
import '../components/Terminal/TerminalPane.css';
import './Demo.css';

// Singleton controllers for the page lifetime
const playback = new PlaybackController();
const timeline = new TimelineStepper();

const SPEEDS = [0.5, 1, 2, 4];

export default function Demo() {
  const [activeIdx, setActiveIdx] = useState(0);
  const [showIntro, setShowIntro] = useState(true);
  const [isRunning, setIsRunning] = useState(false);
  const [isPaused, setIsPaused] = useState(false);
  const [speedIdx, setSpeedIdx] = useState(1);
  const [playbackMode, setPlaybackMode] = useState<'auto' | 'step'>('auto');
  const scenario = SCENARIOS[activeIdx];
  const serverHealth = useServerHealth();

  // Sidebar state
  const [stats, setStats] = useState({ model: '--', cost: '--', tokens: '--', time: '--' });
  const [gates, setGates] = useState<{ name: string; status: 'pass' | 'fail' | 'pending' }[]>([]);
  const [logEntries, setLogEntries] = useState<{ ts: string; text: string; type?: 'info' | 'success' }[]>([]);
  const [timelineSteps, setTimelineSteps] = useState<TimelineStepState[]>([]);
  const [progressText, setProgressText] = useState('press Play to begin');
  const [progressLabel, setProgressLabel] = useState('--');

  const pausedRef = useRef(false);
  const runningRef = useRef(false);
  const handleRefs = useRef<(React.RefObject<TerminalHandle | null>)[]>([]);

  // Wire change listeners
  useEffect(() => {
    timeline.onChange((steps) => setTimelineSteps(steps));
    playback.onProgress((step, total, cmd) => {
      setProgressLabel(`Step ${step}/${total}`);
      setProgressText(cmd);
    });
  }, []);

  // Build scenario context matching ScenarioContext from scenarios.ts
  const buildContext = useCallback((): ScenarioContext => {
    const entries = handleRefs.current
      .map((ref) => ref.current)
      .filter((h): h is TerminalHandle => h !== null);

    return {
      entries,
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
      paused: pausedRef,
      running: runningRef,
    };
  }, []);

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
  }, []);

  const handlePlay = useCallback(async () => {
    if (runningRef.current) return;
    if (serverHealth !== 'connected') return;

    runningRef.current = true;
    pausedRef.current = false;
    setIsRunning(true);
    setIsPaused(false);
    setShowIntro(false);
    setLogEntries([]);
    setStats({ model: '--', cost: '--', tokens: '--', time: '--' });
    setGates([]);

    const ctx = buildContext();
    try {
      await scenario.run(ctx);
    } catch (err) {
      console.error('Scenario error:', err);
    }

    runningRef.current = false;
    setIsRunning(false);
    setIsPaused(false);
  }, [scenario, serverHealth, buildContext]);

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

  // ── Autoplay ────────────────────────────────────────────────

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    if (params.get('autoplay') === '1' && serverHealth === 'connected') {
      const t = setTimeout(() => handlePlay(), 1000);
      return () => clearTimeout(t);
    }
  }, [serverHealth]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Session IDs (regenerated on scenario switch) ────────────

  const sessionIds = useMemo(
    () => scenario.labels.map((_, i) => `demo-${scenario.id}-${i}-${Date.now()}`),
    [activeIdx], // eslint-disable-line react-hooks/exhaustive-deps
  );

  useEffect(() => {
    handleRefs.current = Array.from(
      { length: scenario.panes },
      () => ({ current: null }) as React.RefObject<TerminalHandle | null>,
    );
  }, [scenario.panes, activeIdx]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Timeline display ────────────────────────────────────────

  const timelineDisplay = timelineSteps.length > 0
    ? timelineSteps.map((s) => ({
        label: s.label,
        detail: s.sublabel,
        status: s.status === 'completed' ? ('done' as const) : s.status,
      }))
    : scenario.steps.map((s) => ({
        label: s.label,
        detail: s.sublabel,
        status: 'pending' as const,
      }));

  // Grid class for 4-pane scenarios (2x2)
  const gridCols = scenario.panes === 4 ? 2 : scenario.panes;

  return (
    <div className="demo-page">
      {/* ── Connect screen overlay ── */}
      {serverHealth !== 'connected' && <ConnectScreen />}

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
          <button className="demo-speed" onClick={cycleSpeed}>
            {SPEEDS[speedIdx]}x
          </button>
          {isRunning ? (
            <button className="demo-ctrl-btn" onClick={handlePauseResume}>
              {isPaused ? '\u25B6' : '\u275A\u275A'}
            </button>
          ) : (
            <button className="demo-ctrl-btn play" onClick={handlePlay}>
              {'\u25B6'}
            </button>
          )}
        </div>
      </div>

      {/* ── Main content ── */}
      <div className="demo-main">
        {/* Terminal zone */}
        <div className="demo-terminals">
          {showIntro && (
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
              />
            ))}
          </div>
        </div>

        {/* Sidebar */}
        {scenario.panel && (
          <div className="demo-sidebar">
            <Pane title="TIMELINE" flat>
              <Timeline steps={timelineDisplay} />
            </Pane>

            <div className="demo-stats-mosaic">
              <Mosaic columns={2}>
                <MosaicCell label="MODEL" value={stats.model} mono color="rose" />
                <MosaicCell label="COST" value={stats.cost} mono color="bone" />
                <MosaicCell label="TOKENS" value={stats.tokens} mono color="dream" />
                <MosaicCell label="TIME" value={stats.time} mono color="warning" />
              </Mosaic>
            </div>

            {gates.length > 0 && (
              <Pane title="GATES" flat>
                <div style={{ padding: '12px 16px' }}>
                  <GateBar gates={gates} />
                </div>
              </Pane>
            )}

            <Pane title="LOG" flat>
              <CommandLog entries={logEntries} maxHeight="240px" />
            </Pane>
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
            <button className="demo-pb-btn primary" onClick={handlePlay} title="Play (Space)">
              {'\u25B6'}
            </button>
          )}
          <button
            className={`demo-pb-btn${playbackMode === 'step' ? ' primary' : ''}`}
            onClick={handleStep}
            title="Next step (N)"
            disabled={playbackMode !== 'step'}
          >
            {'\u25B6\u2759'}
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
}: {
  sessionId: string;
  label: string;
  handleRef: React.RefObject<TerminalHandle | null> | undefined;
}) {
  const { attach, status, handle } = useTerminal(sessionId);

  useEffect(() => {
    if (handleRef && 'current' in handleRef) {
      (handleRef as React.MutableRefObject<TerminalHandle | null>).current = handle.current;
    }
  });

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
