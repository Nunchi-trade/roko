import TerminalPane from './TerminalPane';
import './TerminalGrid.css';

interface TerminalGridProps {
  sessions: { id: string; label?: string }[];
  columns?: 1 | 2 | 3;
}

export default function TerminalGrid({ sessions, columns }: TerminalGridProps) {
  const cols = columns ?? (sessions.length <= 1 ? 1 : sessions.length <= 4 ? 2 : 3);

  return (
    <div className={`terminal-grid cols-${cols}`}>
      {sessions.map((s) => (
        <TerminalPane key={s.id} sessionId={s.id} label={s.label} />
      ))}
    </div>
  );
}
