import { useRef, useEffect } from 'react';
import './CommandLog.css';

interface LogEntry {
  ts: string;
  text: string;
  type?: 'info' | 'success' | 'error' | 'dim';
}

interface CommandLogProps {
  entries: LogEntry[];
  maxHeight?: string;
}

export default function CommandLog({ entries, maxHeight = '300px' }: CommandLogProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [entries.length]);

  return (
    <div className="command-log" style={{ maxHeight }}>
      {entries.map((entry, i) => (
        <div key={i} className={`log-entry log-${entry.type ?? 'info'}`}>
          <span className="log-ts">{entry.ts}</span>
          <span className="log-text">{entry.text}</span>
        </div>
      ))}
      <div ref={bottomRef} />
    </div>
  );
}
