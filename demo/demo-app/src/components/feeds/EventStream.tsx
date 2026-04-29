import { useRef, useEffect, useState, useCallback } from 'react';
import './EventStream.css';

/* ── Types ── */

export interface StreamEvent {
  id: string;
  timestamp: number;
  kind: string;
  summary: string;
  detail?: string;
  severity?: 'info' | 'success' | 'warning' | 'error';
}

export interface EventStreamProps {
  events: StreamEvent[];
  maxHeight?: number | string;
  autoScroll?: boolean;
  showTimestamp?: boolean;
  title?: string;
  className?: string;
}

/* ── Helpers ── */

function formatTimestamp(ms: number): string {
  const d = new Date(ms);
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  const ss = String(d.getSeconds()).padStart(2, '0');
  return `${hh}:${mm}:${ss}`;
}

const SCROLL_THRESHOLD = 50;

/* ── Component ── */

export default function EventStream({
  events,
  maxHeight = '320px',
  autoScroll = true,
  showTimestamp = true,
  title,
  className,
}: EventStreamProps) {
  const bodyRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const [pinned, setPinned] = useState(true);
  const [unseenCount, setUnseenCount] = useState(0);
  const prevLenRef = useRef(events.length);

  /* Track user scroll position */
  const handleScroll = useCallback(() => {
    const el = bodyRef.current;
    if (!el) return;
    const nearBottom =
      el.scrollHeight - el.scrollTop - el.clientHeight < SCROLL_THRESHOLD;
    setPinned(nearBottom);
    if (nearBottom) setUnseenCount(0);
  }, []);

  /* Auto-scroll when pinned and new events arrive */
  useEffect(() => {
    if (!autoScroll) return;
    const added = events.length - prevLenRef.current;
    prevLenRef.current = events.length;

    if (added <= 0) return;

    if (pinned) {
      bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
    } else {
      setUnseenCount(c => c + added);
    }
  }, [events.length, pinned, autoScroll]);

  /* Click indicator to jump to bottom */
  const jumpToBottom = useCallback(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
    setPinned(true);
    setUnseenCount(0);
  }, []);

  const showIndicator = !pinned && unseenCount > 0;

  return (
    <div
      className={`event-stream${className ? ` ${className}` : ''}`}
      style={{ maxHeight }}
    >
      {title && (
        <div className="event-stream__header">
          <span className="event-stream__title">{title}</span>
          <span className="event-stream__count">{events.length}</span>
        </div>
      )}

      <div
        ref={bodyRef}
        className="event-stream__body"
        onScroll={handleScroll}
        style={{ maxHeight }}
      >
        {events.length === 0 ? (
          <div className="event-stream__empty">No events</div>
        ) : (
          <div className="event-stream__list">
            {events.map((ev, i) => {
              const severity = ev.severity ?? 'info';
              return (
                <div
                  key={ev.id}
                  className="event-stream__row"
                  style={{ animationDelay: `${Math.min(i, 8) * 30}ms` }}
                  title={ev.detail}
                >
                  {showTimestamp && (
                    <span className="event-stream__ts">
                      {formatTimestamp(ev.timestamp)}
                    </span>
                  )}
                  <span
                    className={`event-stream__kind event-stream__kind--${severity}`}
                  >
                    {ev.kind}
                  </span>
                  <span className="event-stream__summary">{ev.summary}</span>
                </div>
              );
            })}
          </div>
        )}
        <div ref={bottomRef} />
      </div>

      {showIndicator && (
        <div className="event-stream__new-indicator" onClick={jumpToBottom}>
          New events \u2193
        </div>
      )}
    </div>
  );
}
