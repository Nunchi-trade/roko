import { useRef, useEffect } from 'react';
import { TraceAnnotation } from './inference';
import './CommandLog.css';

interface LogEntry {
  ts: string;
  text: string;
  type?: 'info' | 'success' | 'error' | 'dim';
  /** T7.60: Optional trace metadata for inference-related log lines. */
  trace?: {
    agentName?: string;
    tier?: 'T0' | 'T1' | 'T2';
    model?: string;
    confidence?: number;
    cost?: number;
  };
}

interface CommandLogProps {
  entries: LogEntry[];
  maxHeight?: string;
}

/**
 * Parse inference metadata from command text.
 * Matches patterns like: [T1] claude-sonnet-4-20250514 1234tok ($0.02)
 * or: model=X tier=T1
 */
function extractTrace(text: string): LogEntry['trace'] | undefined {
  // Bracket pattern: [T0] model-name
  const bracket = text.match(/\[(T[012])\]\s+(\S+)/);
  if (bracket) {
    const costMatch = text.match(/\$([0-9.]+)/);
    return {
      tier: bracket[1] as 'T0' | 'T1' | 'T2',
      model: bracket[2],
      cost: costMatch ? parseFloat(costMatch[1]) : undefined,
    };
  }

  // KV pattern: model=X tier=Y
  const modelMatch = text.match(/model=(\S+)/);
  const tierMatch = text.match(/tier=(T[012])/);
  if (modelMatch && tierMatch) {
    const costMatch = text.match(/cost=([0-9.]+)/);
    return {
      tier: tierMatch[1] as 'T0' | 'T1' | 'T2',
      model: modelMatch[1],
      cost: costMatch ? parseFloat(costMatch[1]) : undefined,
    };
  }

  return undefined;
}

export default function CommandLog({ entries, maxHeight = '300px' }: CommandLogProps) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [entries.length]);

  return (
    <div className="command-log" style={{ maxHeight }}>
      {entries.map((entry, i) => {
        const trace = entry.trace ?? extractTrace(entry.text);

        return (
          <div key={i} className={`log-entry log-${entry.type ?? 'info'}`}>
            <span className="log-ts">{entry.ts}</span>
            <span className="log-text">{entry.text}</span>
            {/* T7.60: TraceAnnotation strip for inference-related log lines */}
            {trace && trace.tier && trace.model && (
              <TraceAnnotation
                tier={trace.tier}
                model={trace.model}
                agentName={trace.agentName}
                confidence={trace.confidence}
                cost={trace.cost}
                compact
              />
            )}
          </div>
        );
      })}
      <div ref={bottomRef} />
    </div>
  );
}
