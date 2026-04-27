import { useState, useCallback } from 'react';
import TerminalGrid from '../components/Terminal/TerminalGrid';
import './Terminal.css';

export default function Terminal() {
  const [sessions, setSessions] = useState([{ id: `t${Date.now()}`, label: 'shell' }]);
  const [columns, setColumns] = useState<1 | 2 | 3>(1);

  const addTerminal = useCallback(() => {
    setSessions((prev) => [...prev, { id: `t${Date.now()}`, label: `shell ${prev.length + 1}` }]);
  }, []);

  const clearAll = useCallback(() => {
    setSessions([]);
  }, []);

  return (
    <div className="terminal-page">
      <div className="terminal-toolbar">
        <span className="terminal-page-title">terminal</span>
        <span className="terminal-info">multi-pane browser terminal with real PTY sessions</span>
        <div className="terminal-controls">
          <button className="btn-primary" onClick={addTerminal}>+ Terminal</button>
          <button className="btn-secondary" onClick={() => setColumns(1)}>1</button>
          <button className="btn-secondary" onClick={() => setColumns(2)}>2</button>
          <button className="btn-secondary" onClick={() => setColumns(3)}>3</button>
          <button className="btn-danger" onClick={clearAll}>Clear All</button>
        </div>
      </div>
      <div className="terminal-body">
        {sessions.length > 0 ? (
          <TerminalGrid sessions={sessions} columns={columns} />
        ) : (
          <div className="terminal-empty">
            <p>No terminal sessions</p>
            <button className="btn-primary" onClick={addTerminal}>+ Add Terminal</button>
          </div>
        )}
      </div>
      <div className="terminal-status-bar">
        <span>{sessions.length} session{sessions.length !== 1 ? 's' : ''}</span>
      </div>
    </div>
  );
}
