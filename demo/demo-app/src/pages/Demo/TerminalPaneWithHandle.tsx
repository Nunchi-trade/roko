import { useState, useCallback, useRef, useEffect } from 'react';
import { useTerminal, type TerminalHandle } from '../../hooks/useTerminal';
import Tooltip from '../../components/Tooltip';
import { PulseIcon, SpinnerIcon, CrossIcon, WaveformIcon } from '../../components/icons/AnimatedIcons';

export type TerminalPaneState = {
  status: TerminalHandle['status'];
  connected: boolean;
};

/** Map scenario + label to a CSS color for scenario-aware label styling */
function getLabelColor(scenarioId: string, label: string, category: string): string | undefined {
  const lower = label.toLowerCase();
  if (scenarioId === 'cost') {
    if (lower.includes('naive')) return 'var(--warning)';
    if (lower.includes('cascade')) return '#6bb8a8';
  }
  if (scenarioId === 'isfr') {
    if (lower.includes('lending')) return 'var(--rose-bright)';
    if (lower.includes('staking')) return '#6bb87a';
    if (lower.includes('aggregat')) return '#68a8d8';
    if (lower.includes('validat')) return 'var(--warning)';
  }
  if (scenarioId === 'oracle') {
    if (lower.includes('data')) return 'var(--dream-bright)';
    if (lower.includes('strategy')) return '#b888d8';
  }
  if (category === 'comparison') return '#6bb8a8';
  if (category === 'learning') return 'var(--dream-bright)';
  if (category === 'chain') return 'var(--warning)';
  if (category === 'pipeline') return 'var(--rose-bright)';
  return undefined;
}

interface TerminalPaneWithHandleProps {
  sessionId: string;
  label: string;
  handleRef: React.RefObject<TerminalHandle | null> | undefined;
  paneIndex: number;
  onStatusChange?: (index: number, state: TerminalPaneState) => void;
  termReveal?: boolean;
  scenarioId: string;
  scenarioCategory: string;
  isRunning: boolean;
}

export default function TerminalPaneWithHandle({
  sessionId,
  label,
  handleRef,
  paneIndex,
  onStatusChange,
  termReveal,
  scenarioId,
  scenarioCategory,
  isRunning,
}: TerminalPaneWithHandleProps) {
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

  // Detect output activity by polling the terminal handle's outputBuffer
  // (lightweight alternative to MutationObserver on xterm's DOM subtree)
  useEffect(() => {
    const h = handle.current;
    if (!h) return;
    let lastLen = 0;
    let activityTimeout: ReturnType<typeof setTimeout> | null = null;
    const timer = setInterval(() => {
      const curLen = h.outputBuffer.length;
      if (curLen !== lastLen) {
        setHasOutput(true);
        lastLen = curLen;
        if (activityTimeout) clearTimeout(activityTimeout);
        activityTimeout = setTimeout(() => setHasOutput(false), 800);
      }
    }, 300);
    return () => {
      clearInterval(timer);
      if (activityTimeout) clearTimeout(activityTimeout);
    };
  }, [handle]);

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
