import { useEffect, useRef, useCallback } from 'react';
import { stripAnsi } from '../../lib/strip-ansi';
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

/** Icon per event type for visual scanning. */
const TYPE_ICONS: Record<AgentEvent['type'], string> = {
  inference: '\u2726',  // four-pointed star
  tool_call: '\u2192',  // right arrow
  gate:      '\u2713',  // checkmark
  error:     '\u2717',  // X mark
  status:    '\u00B7',  // middle dot
};

/** Refine gate type: a gate event whose summary mentions "fail" is an error. */
function resolveGateType(ev: AgentEvent): { type: AgentEvent['type']; icon: string } {
  if (ev.type === 'gate') {
    const text = ev.summary.toLowerCase();
    if (/fail/i.test(text)) {
      return { type: 'error', icon: '\u2717' };
    }
    return { type: 'gate', icon: '\u2713' };
  }
  return { type: ev.type, icon: TYPE_ICONS[ev.type] };
}

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
          events.map((ev) => {
            const { type: resolvedType, icon } = resolveGateType(ev);
            const cleanSummary = stripAnsi(ev.summary);
            const cleanDetail = ev.detail ? stripAnsi(ev.detail) : undefined;

            return (
              <div key={ev.id} className={`agent-feed__event agent-feed__event--${resolvedType}`}>
                <span className={`agent-feed__icon agent-feed__icon--${resolvedType}`}>
                  {icon}
                </span>
                <span className="agent-feed__ts">{formatTs(ev.timestamp)}</span>
                <span className={`agent-feed__type agent-feed__type--${resolvedType}`}>
                  {TYPE_LABELS[ev.type]}
                </span>
                <span className="agent-feed__summary">{cleanSummary}</span>
                {cleanDetail && (
                  <div className="agent-feed__detail">{cleanDetail}</div>
                )}
              </div>
            );
          })
        )}
      </div>
      <div className="agent-feed__fade" />
    </div>
  );
}
