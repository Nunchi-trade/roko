import { useRef, useEffect, useState, useCallback, useMemo } from 'react';
import './InferenceFeed.css';

/* ── Types ── */

export interface InferenceEvent {
  id: string;
  timestamp: number;
  model: string;
  provider: string;
  inputTokens: number;
  outputTokens: number;
  latencyMs: number;
  status: 'success' | 'error' | 'streaming';
  cost?: number;
}

export interface InferenceFeedProps {
  events: InferenceEvent[];
  maxHeight?: number | string;
  autoScroll?: boolean;
  showCost?: boolean;
  className?: string;
}

/* ── Helpers ── */

/** Abbreviate model ID to first two dash-segments. */
function shortModel(id: string): string {
  return id.split('-').slice(0, 2).join('-');
}

function formatCost(cost: number): string {
  if (cost < 0.001) return `$${cost.toFixed(4)}`;
  if (cost < 1) return `$${cost.toFixed(3)}`;
  return `$${cost.toFixed(2)}`;
}

function formatLatency(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

const SCROLL_THRESHOLD = 50;

/* ── Component ── */

export default function InferenceFeed({
  events,
  maxHeight = '300px',
  autoScroll = true,
  showCost = true,
  className,
}: InferenceFeedProps) {
  const bodyRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const [pinned, setPinned] = useState(true);
  const prevLenRef = useRef(events.length);

  /* Running cost total */
  const totalCost = useMemo(
    () => events.reduce((sum, ev) => sum + (ev.cost ?? 0), 0),
    [events],
  );

  /* Track scroll position */
  const handleScroll = useCallback(() => {
    const el = bodyRef.current;
    if (!el) return;
    const nearBottom =
      el.scrollHeight - el.scrollTop - el.clientHeight < SCROLL_THRESHOLD;
    setPinned(nearBottom);
  }, []);

  /* Auto-scroll on new events */
  useEffect(() => {
    if (!autoScroll) return;
    const added = events.length - prevLenRef.current;
    prevLenRef.current = events.length;
    if (added > 0 && pinned) {
      bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [events.length, pinned, autoScroll]);

  return (
    <div
      className={`inference-feed${className ? ` ${className}` : ''}`}
      style={{ maxHeight }}
    >
      {showCost && (
        <div className="inference-feed__header">
          <span className="inference-feed__title">Inference</span>
          <span className="inference-feed__total-cost">
            {formatCost(totalCost)}
          </span>
        </div>
      )}

      <div
        ref={bodyRef}
        className="inference-feed__body"
        onScroll={handleScroll}
        style={{ maxHeight }}
      >
        {events.length === 0 ? (
          <div className="inference-feed__empty">No inference events</div>
        ) : (
          <div className="inference-feed__list">
            {events.map((ev, i) => {
              const cardCls = [
                'inference-feed__card',
                ev.status === 'streaming'
                  ? 'inference-feed__card--streaming'
                  : '',
                ev.status === 'error' ? 'inference-feed__card--error' : '',
              ]
                .filter(Boolean)
                .join(' ');

              return (
                <div
                  key={ev.id}
                  className={cardCls}
                  style={{ animationDelay: `${Math.min(i, 8) * 30}ms` }}
                >
                  <div className="inference-feed__top">
                    <span className="inference-feed__model">
                      {shortModel(ev.model)}
                    </span>
                    <span className="inference-feed__provider">
                      {ev.provider}
                    </span>
                    <span
                      className={`inference-feed__status-dot inference-feed__status-dot--${ev.status}`}
                    />
                  </div>
                  <div className="inference-feed__bottom">
                    <span className="inference-feed__tokens">
                      <span className="inference-feed__tokens-in">
                        {'\u2192'}{ev.inputTokens.toLocaleString()}
                      </span>
                      {' '}
                      <span className="inference-feed__tokens-out">
                        {'\u2190'}{ev.outputTokens.toLocaleString()}
                      </span>
                    </span>
                    <span className="inference-feed__latency">
                      {formatLatency(ev.latencyMs)}
                    </span>
                    {showCost && ev.cost != null && (
                      <span className="inference-feed__cost">
                        {formatCost(ev.cost)}
                      </span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}
