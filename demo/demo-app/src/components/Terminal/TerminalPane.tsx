import { useRef, useEffect, useCallback } from 'react';
import { useTerminal } from '../../hooks/useTerminal';
import '@xterm/xterm/css/xterm.css';
import './TerminalPane.css';

interface TerminalPaneProps {
  sessionId: string;
  label?: string;
}

export default function TerminalPane({ sessionId, label }: TerminalPaneProps) {
  const { attach, status } = useTerminal(sessionId);
  const paneRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const prevStatusRef = useRef(status);

  // Combine refs for body: pass to both useTerminal attach and our bodyRef
  const bodyCallbackRef = useCallback(
    (node: HTMLDivElement | null) => {
      (bodyRef as React.MutableRefObject<HTMLDivElement | null>).current = node;
      attach(node);
    },
    [attach],
  );

  // Connection / disconnect animation classes
  useEffect(() => {
    const pane = paneRef.current;
    if (!pane) return;
    const prev = prevStatusRef.current;
    prevStatusRef.current = status;

    if (prev !== 'connected' && status === 'connected') {
      pane.classList.add('just-connected');
      const t = setTimeout(() => pane.classList.remove('just-connected'), 600);
      return () => clearTimeout(t);
    }
    if (prev === 'connected' && status === 'disconnected') {
      pane.classList.add('just-disconnected');
      const t = setTimeout(() => pane.classList.remove('just-disconnected'), 200);
      return () => clearTimeout(t);
    }
  }, [status]);

  // NOTE: Output flash animation was removed — the MutationObserver fired on
  // every PTY write, causing visible opacity jitter during streaming output.
  // The connection pulse (just-connected) is sufficient visual feedback.

  return (
    <div className="terminal-pane" ref={paneRef}>
      <div className="pane-header">
        <span className={`pane-dot ${status}`} />
        <span className="pane-label">{label ?? sessionId}</span>
        <span className="pane-status">{status}</span>
      </div>
      <div className="pane-body" ref={bodyCallbackRef} />
    </div>
  );
}
