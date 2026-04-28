import { useState, useCallback, useRef, useEffect, useMemo } from 'react';
import { SCENARIOS } from '../lib/demo-scenarios';
import { useDemoPlayback } from '../hooks/useDemoPlayback';
import { useTerminal, type TerminalHandle } from '../hooks/useTerminal';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import Timeline from '../components/Timeline';
import CommandLog from '../components/CommandLog';
import GateBar from '../components/GateBar';
import '@xterm/xterm/css/xterm.css';
import '../components/Terminal/TerminalPane.css';
import './Demo.css';

const GATE_NAMES = ['compile', 'test', 'clippy', 'diff', 'coverage'];

export default function Demo() {
  const [activeIdx, setActiveIdx] = useState(0);
  const [showIntro, setShowIntro] = useState(true);
  const scenario = SCENARIOS[activeIdx];

  const playback = useDemoPlayback();
  const { currentStep, isPlaying, isPaused, speed } = playback;

  // Terminal handles -- up to 2 panes
  const handle0 = useRef<TerminalHandle | null>(null);
  const handle1 = useRef<TerminalHandle | null>(null);

  // Wire terminals + steps into playback whenever scenario changes
  useEffect(() => {
    const handles = scenario.panes === 2 ? [handle0, handle1] : [handle0];
    playback.setTerminals(handles);
    playback.setSteps(scenario.steps);
  }, [activeIdx, scenario]);

  // Simulated stats
  const [stats, setStats] = useState({ model: 'claude-sonnet-4', cost: 0.12, tokens: 1840, time: 3 });

  // Command log entries
  const [logEntries, setLogEntries] = useState<{ ts: string; text: string; type?: 'info' | 'success' }[]>([]);

  // Update stats + log when step changes
  useEffect(() => {
    if (currentStep < 0) return;
    const step = scenario.steps[currentStep];
    if (!step) return;

    const now = new Date();
    const ts = `${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}:${now.getSeconds().toString().padStart(2, '0')}`;

    setLogEntries((prev) => [
      ...prev,
      { ts, text: `$ ${step.command}`, type: 'info' as const },
      { ts, text: step.description, type: 'success' as const },
    ]);

    setStats((s) => ({
      model: 'claude-sonnet-4',
      cost: +(s.cost + 0.02 + Math.random() * 0.08).toFixed(2),
      tokens: s.tokens + Math.floor(200 + Math.random() * 800),
      time: s.time + Math.round((step.wait_after_ms + step.delay_before_ms) / 1000),
    }));
  }, [currentStep, scenario.steps]);

  // Gate statuses derived from step progress
  const gates = useMemo(() => {
    const total = scenario.steps.length;
    const progress = currentStep < 0 ? 0 : (currentStep + 1) / Math.max(total, 1);
    return GATE_NAMES.map((name, i) => {
      const threshold = (i + 1) / GATE_NAMES.length;
      const status: 'pass' | 'pending' = progress >= threshold ? 'pass' : 'pending';
      return { name, status };
    });
  }, [currentStep, scenario.steps.length]);

  const selectScenario = useCallback((idx: number) => {
    playback.reset();
    setActiveIdx(idx);
    setShowIntro(true);
    setStats({ model: 'claude-sonnet-4', cost: 0.12, tokens: 1840, time: 3 });
    setLogEntries([]);
  }, [playback]);

  const handlePlay = useCallback(() => {
    setShowIntro(false);
    setLogEntries([]);
    setStats({ model: 'claude-sonnet-4', cost: 0.12, tokens: 1840, time: 3 });
    playback.play();
  }, [playback]);

  const handlePauseResume = useCallback(() => {
    if (isPaused) playback.resume();
    else playback.pause();
  }, [isPaused, playback]);

  const cycleSpeed = useCallback(() => {
    playback.setSpeed(speed >= 4 ? 0.5 : speed * 2);
  }, [speed, playback]);

  // Autoplay support via ?autoplay=1
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    if (params.get('autoplay') === '1') {
      const t = setTimeout(() => handlePlay(), 1000);
      return () => clearTimeout(t);
    }
  }, []);

  // Timeline steps
  const timelineSteps = scenario.steps.map((step, i) => ({
    label: step.description,
    detail: step.command.length > 50 ? step.command.slice(0, 47) + '...' : step.command,
    status: (i < currentStep ? 'done' : i === currentStep ? 'active' : 'pending') as 'done' | 'active' | 'pending',
  }));

  // Session IDs for terminals
  const sessionIds = scenario.labels.map((_, i) => `demo-${scenario.id}-${i}`);

  return (
    <div className="demo-page">
      {/* ── Top bar: tabs + controls ── */}
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
            {speed}x
          </button>
          {isPlaying ? (
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

      {/* ── Main content: terminals + sidebar ── */}
      <div className="demo-main">
        {/* Terminal area (left 60%) */}
        <div className="demo-terminals">
          {/* Scanline overlay applied via CSS ::before */}

          {/* Intro overlay */}
          {showIntro && (
            <div className="demo-intro-overlay" onClick={handlePlay}>
              <div className="demo-intro-title">{scenario.title}</div>
              <div className="demo-intro-sub">{scenario.subtitle}</div>
              <button className="demo-play-btn" onClick={(e) => { e.stopPropagation(); handlePlay(); }}>
                {'\u25B6'}
              </button>
            </div>
          )}

          <div className={`demo-terminal-grid demo-cols-${scenario.panes}`}>
            <TerminalPaneWithHandle
              key={`${scenario.id}-0`}
              sessionId={sessionIds[0]}
              label={scenario.labels[0]}
              handleRef={handle0}
            />
            {scenario.panes === 2 && (
              <TerminalPaneWithHandle
                key={`${scenario.id}-1`}
                sessionId={sessionIds[1]}
                label={scenario.labels[1]}
                handleRef={handle1}
              />
            )}
          </div>
        </div>

        {/* Sidebar (right 40%) */}
        <div className="demo-sidebar">
          <Pane title="TIMELINE" flat>
            <Timeline steps={timelineSteps} />
          </Pane>

          <div className="demo-stats-mosaic">
            <Mosaic columns={2}>
              <MosaicCell label="MODEL" value={stats.model} mono color="rose" />
              <MosaicCell label="COST" value={`$${stats.cost.toFixed(2)}`} mono color="bone" />
              <MosaicCell label="TOKENS" value={stats.tokens.toLocaleString()} mono color="dream" />
              <MosaicCell label="TIME" value={`${stats.time}s`} mono color="warning" />
            </Mosaic>
          </div>

          <Pane title="GATES" flat>
            <div style={{ padding: '12px 16px' }}>
              <GateBar gates={gates} />
            </div>
          </Pane>

          <Pane title="LOG" flat>
            <CommandLog entries={logEntries} maxHeight="240px" />
          </Pane>
        </div>
      </div>
    </div>
  );
}

/**
 * TerminalPane wrapper that exposes the TerminalHandle ref for playback.
 */
function TerminalPaneWithHandle({
  sessionId,
  label,
  handleRef,
}: {
  sessionId: string;
  label: string;
  handleRef: React.MutableRefObject<TerminalHandle | null>;
}) {
  const { attach, status, handle } = useTerminal(sessionId);

  // Sync the internal handle ref to the parent ref
  useEffect(() => {
    handleRef.current = handle.current;
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
