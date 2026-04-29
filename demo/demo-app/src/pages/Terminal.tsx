import { useState, useCallback, useRef, useEffect, useMemo } from 'react';
import { useTerminal } from '../hooks/useTerminal';
import { useWorkspace } from '../hooks/useWorkspace';
import { enterWorkspace } from '../lib/terminal-session';
import './Terminal.css';

/* ── Typewriter text (workspace badge) ── */
function TypewriterText({ text, speed = 40 }: { text: string; speed?: number }) {
  const [displayed, setDisplayed] = useState('');
  useEffect(() => {
    setDisplayed('');
    let i = 0;
    const iv = setInterval(() => {
      i++;
      setDisplayed(text.slice(0, i));
      if (i >= text.length) clearInterval(iv);
    }, speed);
    return () => clearInterval(iv);
  }, [text, speed]);
  return <>{displayed}<span className="tw-cursor">|</span></>;
}

/* ── Individual terminal pane ── */
function TerminalPaneReal({
  sessionId,
  label,
  onClose,
  workspacePath,
  isFocused,
  onFocus,
}: {
  sessionId: string;
  label: string;
  onClose?: () => void;
  workspacePath?: string | null;
  isFocused: boolean;
  onFocus: () => void;
}) {
  const { attach, status, handle } = useTerminal(sessionId);
  const [prevStatus, setPrevStatus] = useState<string>(status);
  const [showConnFlash, setShowConnFlash] = useState(false);
  const paneRef = useRef<HTMLDivElement>(null);

  /* Connection flash when status transitions to connected */
  useEffect(() => {
    if (status === 'connected' && prevStatus !== 'connected') {
      setShowConnFlash(true);
      const t = setTimeout(() => setShowConnFlash(false), 600);
      return () => clearTimeout(t);
    }
    setPrevStatus(status);
  }, [status, prevStatus]);

  const handleCdWorkspace = useCallback(() => {
    const h = handle.current;
    if (!h || !workspacePath) return;
    enterWorkspace(h, workspacePath);
  }, [handle, workspacePath]);

  const badgeText = workspacePath
    ? workspacePath.split('/').filter(Boolean).slice(-2).join('/')
    : '';

  return (
    <div
      ref={paneRef}
      className={[
        'term-pane-real',
        isFocused ? 'term-pane-focused gradient-border' : '',
        showConnFlash && 'term-pane-connected-flash',
      ].filter(Boolean).join(' ')}
      onClick={onFocus}
    >
      {/* Connection progress line along top border */}
      <div className={`term-conn-progress ${status}`} />

      <div className="term-pane-header">
        <span className={`term-conn-dot ${status}`} />
        <span className="term-pane-label">{label}</span>
        {workspacePath && (
          <span className="term-ws-badge" title={workspacePath}>
            <TypewriterText text={badgeText} speed={35} />
          </span>
        )}
        <span className="term-pane-status">{status}</span>
        {workspacePath && (
          <button
            className="term-init-btn term-header-btn btn-interactive"
            onClick={handleCdWorkspace}
            title={`cd ${workspacePath}`}
          >
            cd ws
          </button>
        )}
        {onClose && (
          <button
            className="term-close-btn term-header-btn btn-interactive"
            onClick={(e) => {
              e.stopPropagation();
              onClose();
            }}
            aria-label={`Close ${label}`}
          >
            &times;
          </button>
        )}
      </div>
      <div className="term-pane-body" ref={attach} />
    </div>
  );
}

interface TermEntry {
  id: string;
  label: string;
  /** Timestamp when created, for spawn animation */
  createdAt: number;
  /** Whether this pane is closing (for exit animation) */
  closing?: boolean;
}

export default function Terminal() {
  const [terminals, setTerminals] = useState<TermEntry[]>(() => [
    { id: `t-${Date.now()}`, label: 'shell 1', createdAt: Date.now() },
  ]);
  const [columns, setColumns] = useState<1 | 2 | 4>(1);
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [activeTabIdx, setActiveTabIdx] = useState(0);
  const [initState, setInitState] = useState<'idle' | 'spinning' | 'done'>('idle');
  const { ensureWorkspace } = useWorkspace();
  const [workspacePath, setWorkspacePath] = useState<string | null>(null);
  const tabBarRef = useRef<HTMLDivElement>(null);

  /* Live terminals (not closing) */
  const liveTerminals = useMemo(
    () => terminals.filter((t) => !t.closing),
    [terminals],
  );

  const addTerminal = useCallback(() => {
    const n = terminals.length + 1;
    const entry: TermEntry = {
      id: `t-${Date.now()}-${n}`,
      label: `shell ${n}`,
      createdAt: Date.now(),
    };
    setTerminals((prev) => [...prev, entry]);
    setActiveTabIdx(terminals.length); // new tab becomes active
  }, [terminals.length]);

  const removeTerminal = useCallback((id: string) => {
    /* Mark as closing so the exit animation plays */
    setTerminals((prev) =>
      prev.map((t) => (t.id === id ? { ...t, closing: true } : t)),
    );
    /* After animation, actually remove */
    setTimeout(() => {
      setTerminals((prev) => prev.filter((t) => t.id !== id));
    }, 350);
  }, []);

  const clearAll = useCallback(() => {
    setTerminals((prev) => prev.map((t) => ({ ...t, closing: true })));
    setTimeout(() => setTerminals([]), 350);
  }, []);

  const handleInitWorkspace = useCallback(async () => {
    if (initState !== 'idle') return;
    setInitState('spinning');
    try {
      const ws = await ensureWorkspace('roko-terminal');
      setWorkspacePath(ws.path);
      setInitState('done');
    } catch {
      setInitState('idle');
    }
  }, [initState, ensureWorkspace]);

  const COL_OPTIONS: (1 | 2 | 4)[] = [1, 2, 4];

  return (
    <div className="terminal-page">
      <div className="terminal-toolbar">
        <span className="terminal-page-title">Terminal</span>

        {/* Tab bar */}
        {liveTerminals.length > 0 && (
          <div className="term-tab-bar" ref={tabBarRef}>
            {liveTerminals.map((t, idx) => (
              <button
                key={t.id}
                className={`term-tab${idx === activeTabIdx ? ' term-tab-active' : ''}`}
                onClick={() => {
                  setActiveTabIdx(idx);
                  setFocusedId(t.id);
                }}
              >
                <span className="term-tab-text">{t.label}</span>
                {idx === activeTabIdx && (
                  <span className="term-tab-indicator" />
                )}
              </button>
            ))}
          </div>
        )}

        <div className="terminal-controls">
          <button
            className={`term-btn term-init-workspace-btn btn-primary-glow ${initState}`}
            onClick={handleInitWorkspace}
            disabled={initState !== 'idle'}
            title="Create a roko workspace"
          >
            {initState === 'spinning' && <span className="term-spinner" />}
            {initState === 'done' && <span className="term-check">&#10003;</span>}
            <span className="term-init-label">
              {initState === 'idle'
                ? 'Init workspace'
                : initState === 'spinning'
                  ? 'Creating...'
                  : 'ws ready'}
            </span>
          </button>
          <button className="term-btn-add btn-interactive" onClick={addTerminal} title="Add terminal">
            +
          </button>
          {COL_OPTIONS.map((c) => (
            <button
              key={c}
              className={`term-btn btn-ghost-reveal${columns === c ? ' active' : ''}`}
              onClick={() => setColumns(c)}
              aria-label={`${c} column${c > 1 ? 's' : ''}`}
            >
              {c}
            </button>
          ))}
          <button className="term-btn-clear btn-interactive" onClick={clearAll}>
            Clear
          </button>
        </div>
      </div>

      <div className="terminal-body">
        {terminals.length > 0 ? (
          <div className={`term-grid cols-${columns}`}>
            {terminals.map((t) => (
              <div
                key={t.id}
                className={`term-cell${t.closing ? ' term-cell-closing' : ' term-cell-spawning'}`}
              >
                <TerminalPaneReal
                  sessionId={t.id}
                  label={t.label}
                  onClose={() => removeTerminal(t.id)}
                  workspacePath={workspacePath}
                  isFocused={focusedId === t.id}
                  onFocus={() => {
                    setFocusedId(t.id);
                    const idx = liveTerminals.findIndex((lt) => lt.id === t.id);
                    if (idx >= 0) setActiveTabIdx(idx);
                  }}
                />
              </div>
            ))}
          </div>
        ) : (
          <div className="terminal-empty">
            <span className="terminal-empty-title">No terminals open</span>
            <span className="terminal-empty-sub">Click + to add one</span>
          </div>
        )}
      </div>
    </div>
  );
}
