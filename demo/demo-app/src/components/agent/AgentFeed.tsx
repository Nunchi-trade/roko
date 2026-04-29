import { useEffect, useRef, useCallback } from 'react';
import './AgentFeed.css';

interface AgentEvent {
  id: string;
  timestamp: number;
  type: 'inference' | 'tool_call' | 'gate' | 'error' | 'status';
  summary: string;
  detail?: string;
}

interface AgentFeedProps {
  events: AgentEvent[];
  maxHeight?: number | string;
  autoScroll?: boolean;
  className?: string;
}

const TYPE_LABELS: Record<AgentEvent['type'], string> = {
  inference: 'infer',
  tool_call: 'tool',
  gate: 'gate',
  error: 'error',
  status: 'status',
};

function formatTs(ms: number): string {
  const d = new Date(ms);
  const h = String(d.getHours()).padStart(2, '0');
  const m = String(d.getMinutes()).padStart(2, '0');
  const s = String(d.getSeconds()).padStart(2, '0');
  return `${h}:${m}:${s}`;
}

export default function AgentFeed({
  events,
  maxHeight = '240px',
  autoScroll = true,
  className,
}: AgentFeedProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const userScrolledRef = useRef(false);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 16;
    userScrolledRef.current = !atBottom;
  }, []);

  useEffect(() => {
    if (!autoScroll || userScrolledRef.current) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [events.length, autoScroll]);

  const height = typeof maxHeight === 'number' ? `${maxHeight}px` : maxHeight;

  return (
    <div
      className={['agent-feed', className].filter(Boolean).join(' ')}
      style={{ maxHeight: height }}
    >
      <div
        ref={scrollRef}
        className="agent-feed__scroll"
        style={{ maxHeight: height }}
        onScroll={handleScroll}
      >
        {events.length === 0 ? (
          <div className="agent-feed__empty">No events</div>
        ) : (
          events.map((ev) => (
            <div key={ev.id} className="agent-feed__event">
              <span className="agent-feed__ts">{formatTs(ev.timestamp)}</span>
              <span className={`agent-feed__type agent-feed__type--${ev.type}`}>
                {TYPE_LABELS[ev.type]}
              </span>
              <span className="agent-feed__summary">{ev.summary}</span>
              {ev.detail && (
                <div className="agent-feed__detail">{ev.detail}</div>
              )}
            </div>
          ))
        )}
      </div>
      <div className="agent-feed__fade" />
    </div>
  );
}
