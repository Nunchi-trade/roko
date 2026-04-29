import { useEffect, useRef, useState } from 'react';
import { PulseIcon, SpinnerIcon, CrossIcon } from './icons/AnimatedIcons';
import './DemoStatusBar.css';

export interface DemoStatusBarProps {
  serverHealth: { ok: boolean; latency?: number };
  terminalStates: { label: string; status: 'connected' | 'connecting' | 'disconnected' }[];
  scenarioId: string;
  isRunning: boolean;
  elapsedMs?: number;
  speed?: number;
}

function formatElapsed(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  return `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
}

function abbreviateLabel(label: string): string {
  // Take first 3 characters of each word, or first 4 of single word
  const words = label.trim().split(/\s+/);
  if (words.length === 1) return words[0].slice(0, 4).toUpperCase();
  return words.map((w) => w.slice(0, 3)).join('').toUpperCase();
}

export default function DemoStatusBar({
  serverHealth,
  terminalStates,
  scenarioId,
  isRunning,
  elapsedMs,
  speed,
}: DemoStatusBarProps) {
  const [fadeIn, setFadeIn] = useState(false);
  const prevScenario = useRef(scenarioId);

  // Trigger fade-in animation on scenario change
  useEffect(() => {
    if (scenarioId !== prevScenario.current) {
      setFadeIn(true);
      prevScenario.current = scenarioId;
      const t = setTimeout(() => setFadeIn(false), 350);
      return () => clearTimeout(t);
    }
  }, [scenarioId]);

  const serverClass = serverHealth.ok ? 'dsb-connected' : 'dsb-disconnected';

  return (
    <div className={`demo-status-bar${fadeIn ? ' fade-enter' : ''}`}>
      {/* Server status */}
      <div className={`dsb-server ${serverClass}`}>
        {serverHealth.ok
          ? <PulseIcon size={10} color="var(--success)" />
          : <CrossIcon size={10} color="var(--rose-bright)" />}
        <span className="dsb-server-label">SERVE</span>
        {serverHealth.ok && serverHealth.latency != null && (
          <span className="dsb-latency">{serverHealth.latency}ms</span>
        )}
      </div>

      <span className="dsb-sep" />

      {/* Terminal status dots */}
      <div className="dsb-terminals">
        {terminalStates.map((t, i) => (
          <div className="dsb-term" key={i}>
            <span className={`dsb-term-dot ${t.status}`} />
            <span className="dsb-term-label">{abbreviateLabel(t.label)}</span>
          </div>
        ))}
      </div>

      <span className="dsb-sep" />

      {/* Scenario indicator */}
      <div className={`dsb-scenario${isRunning ? ' dsb-running' : ''}`}>
        {isRunning && <SpinnerIcon size={10} />}
        <span className="dsb-scenario-name">{scenarioId}</span>
      </div>

      {/* Elapsed timer */}
      {isRunning && elapsedMs != null && (
        <>
          <span className="dsb-sep" />
          <div className="dsb-elapsed">
            <span className="dsb-elapsed-value">{formatElapsed(elapsedMs)}</span>
          </div>
        </>
      )}

      {/* Speed badge */}
      {speed != null && (
        <span className="dsb-speed">{speed}x</span>
      )}
    </div>
  );
}
