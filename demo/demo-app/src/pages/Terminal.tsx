import { useState, useCallback } from 'react';
import { useTerminal } from '../hooks/useTerminal';
import { useWorkspace } from '../hooks/useWorkspace';
import { enterWorkspace } from '../lib/terminal-session';
import './Terminal.css';

/** Individual terminal pane using useTerminal hook internally. */
function TerminalPaneReal({ sessionId, label, onClose, workspacePath }: {
  sessionId: string;
  label: string;
  onClose?: () => void;
  workspacePath?: string | null;
}) {
  const { attach, status, handle } = useTerminal(sessionId);

  const handleCdWorkspace = useCallback(() => {
    const h = handle.current;
    if (!h || !workspacePath) return;
    enterWorkspace(h, workspacePath);
  }, [handle, workspacePath]);

  return (
    <div className="term-pane-real">
      <div className="term-pane-header">
        <span className={`term-conn-dot ${status}`} />
        <span className="term-pane-label">{label}</span>
        {workspacePath && (
          <span className="term-ws-badge" title={workspacePath}>
            {workspacePath.split('/').filter(Boolean).slice(-2).join('/')}
          </span>
        )}
        <span className="term-pane-status">{status}</span>
        {workspacePath && (
          <button className="term-init-btn" onClick={handleCdWorkspace} title={`cd ${workspacePath}`}>cd ws</button>
        )}
        {onClose && (
          <button className="term-close-btn" onClick={onClose} aria-label={`Close ${label}`}>&times;</button>
        )}
      </div>
      <div className="term-pane-body" ref={attach} />
    </div>
  );
}

interface TermEntry {
  id: string;
  label: string;
}

export default function Terminal() {
  const [terminals, setTerminals] = useState<TermEntry[]>(() => [
    { id: `t-${Date.now()}`, label: 'shell 1' },
  ]);
  const [columns, setColumns] = useState<1 | 2 | 4>(1);
  const [initDone, setInitDone] = useState(false);
  const { ensureWorkspace } = useWorkspace();

  const addTerminal = useCallback(() => {
    const n = terminals.length + 1;
    setTerminals(prev => [...prev, { id: `t-${Date.now()}-${n}`, label: `shell ${n}` }]);
  }, [terminals.length]);

  const removeTerminal = useCallback((id: string) => {
    setTerminals(prev => prev.filter(t => t.id !== id));
  }, []);

  const clearAll = useCallback(() => {
    setTerminals([]);
  }, []);

  const [workspacePath, setWorkspacePath] = useState<string | null>(null);

  const handleInitWorkspace = useCallback(async () => {
    if (initDone) return;
    setInitDone(true);
    const ws = await ensureWorkspace('roko-terminal');
    setWorkspacePath(ws.path);
  }, [initDone, ensureWorkspace]);

  const COL_OPTIONS: (1 | 2 | 4)[] = [1, 2, 4];

  return (
    <div className="terminal-page">
      <div className="terminal-toolbar">
        <span className="terminal-page-title">Terminal</span>
        <div className="terminal-controls">
          <button
            className={`term-btn${initDone ? ' active' : ''}`}
            onClick={handleInitWorkspace}
            disabled={initDone}
            title="Create a roko workspace"
          >
            {initDone ? 'ws ready' : 'Init workspace'}
          </button>
          <button className="term-btn-add" onClick={addTerminal}>+</button>
          {COL_OPTIONS.map(c => (
            <button
              key={c}
              className={`term-btn${columns === c ? ' active' : ''}`}
              onClick={() => setColumns(c)}
              aria-label={`${c} column${c > 1 ? 's' : ''}`}
            >
              {c}
            </button>
          ))}
          <button className="term-btn-clear" onClick={clearAll}>Clear</button>
        </div>
      </div>

      <div className="terminal-body">
        {terminals.length > 0 ? (
          <div className={`term-grid cols-${columns}`}>
            {terminals.map(t => (
              <div key={t.id} className="term-cell">
                <TerminalPaneReal sessionId={t.id} label={t.label} onClose={() => removeTerminal(t.id)} workspacePath={workspacePath} />
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
