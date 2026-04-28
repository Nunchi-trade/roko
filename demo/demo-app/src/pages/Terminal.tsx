import { useState, useCallback } from 'react';
import TerminalPane from '../components/Terminal/TerminalPane';
import './Terminal.css';

export default function Terminal() {
  const [terminals, setTerminals] = useState<{ id: string; label: string }[]>([
    { id: `t-${Date.now()}`, label: 'shell' },
  ]);
  const [columns, setColumns] = useState<1 | 2 | 3 | 4>(1);

  const addTerminal = useCallback(() => {
    const n = terminals.length + 1;
    setTerminals((prev) => [...prev, { id: `t-${Date.now()}-${n}`, label: `shell ${n}` }]);
  }, [terminals.length]);

  const clearAll = useCallback(() => {
    setTerminals([]);
  }, []);

  const COL_OPTIONS: (1 | 2 | 3 | 4)[] = [1, 2, 3, 4];

  return (
    <div className="terminal-page">
      <div className="terminal-toolbar">
        <span className="terminal-page-title">Terminal</span>
        <div className="terminal-controls">
          <button className="term-btn-add" onClick={addTerminal}>+</button>
          {COL_OPTIONS.map((c) => (
            <button
              key={c}
              className={`term-btn${columns === c ? ' active' : ''}`}
              onClick={() => setColumns(c)}
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
            {terminals.map((t) => (
              <div key={t.id} className="term-cell">
                <TerminalPane sessionId={t.id} label={t.label} />
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
