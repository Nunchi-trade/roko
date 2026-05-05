/**
 * FeedsDashboard — live feed agent grid with sparklines and feed log.
 *
 * Subscribes to feed_tick / feed_agent_online / feed_agent_offline SSE events
 * and shows 15 feed cards, an agent sidebar, and a scrolling message log.
 */
import { useCallback, useEffect, useRef } from 'react';
import { useDataHub } from '../../app/DataHub';
import type { RelayFeed } from '../../app/DataHub';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import Oscilloscope from '../../components/canvas/Oscilloscope';
import './FeedsDashboard.css';

// ── Helpers ─────────────────────────────────────────────────

function formatFeedValue(feed: RelayFeed): string {
  if (feed.lastValue === null) return '--';
  const v = feed.lastValue as Record<string, unknown>;

  // Try common numeric fields (camelCase — snakeToCamelObj converts payload keys).
  if (typeof v.compositeBps === 'number') return `${(v.compositeBps as number / 100).toFixed(2)}%`;
  if (typeof v.emaGwei === 'number') return `${(v.emaGwei as number).toFixed(1)} gwei`;
  if (typeof v.number === 'number') return `#${v.number}`;
  if (typeof v.currentEpoch === 'number') return `epoch ${v.currentEpoch}`;
  if (typeof v.epoch === 'number') return `epoch ${v.epoch}`;
  if (typeof v.confidencePct === 'number') return `${(v.confidencePct as number).toFixed(0)}%`;
  if (typeof v.maxSpreadBps === 'number') return `${v.maxSpreadBps} bps`;
  if (typeof v.stddevBps === 'number') return `${(v.stddevBps as number).toFixed(1)} bps`;
  if (typeof v.roc1m === 'number') return `${v.roc1m > 0 ? '+' : ''}${(v.roc1m as number * 10000).toFixed(1)} bps/1m`;
  if (typeof v.lastRateBps === 'number') return `${(v.lastRateBps as number / 100).toFixed(2)}%`;
  if (typeof v.feedAgentCount === 'number') return `${v.feedAgentCount} agents`;
  if (typeof v.totalFeeds === 'number') return `${v.totalFeeds} feeds`;
  if (typeof v.relayAgentCount === 'number') return `${v.relayAgentCount} agents`;

  return JSON.stringify(v).slice(0, 30);
}

function formatTime(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleTimeString('en-GB', { hour12: false });
}

function feedDotClass(status: RelayFeed['status']): string {
  switch (status) {
    case 'live': return 'feeds-dot feeds-dot--live';
    case 'stale': return 'feeds-dot feeds-dot--stale';
    case 'offline': return 'feeds-dot feeds-dot--offline';
    default: return 'feeds-dot';
  }
}

// ── Component ───────────────────────────────────────────────

export default function FeedsDashboard() {
  const relayFeeds = useDataHub((s) => s.relayFeeds);
  const relayAgents = useDataHub((s) => s.relayAgents);
  const feedLog = useDataHub((s) => s.feedLog);
  const fetchFeedCatalog = useDataHub((s) => s.fetchFeedCatalog);
  const logRef = useRef<HTMLDivElement>(null);

  // Initial load.
  useEffect(() => {
    fetchFeedCatalog();
  }, [fetchFeedCatalog]);

  // SSE subscription — refetch catalog on agent online/offline.
  const debouncedRefetch = useDebouncedRefetch(fetchFeedCatalog, 3000);
  useContextEventSubscription(
    ['feed_agent_online', 'feed_agent_offline'],
    useCallback(() => {
      debouncedRefetch();
    }, [debouncedRefetch]),
  );

  // Also subscribe to feed_tick so DataHub processes them.
  useContextEventSubscription(['feed_tick'], useCallback(() => {}, []));

  const liveCount = relayFeeds.filter((f) => f.status === 'live').length;
  const totalMsgCount = relayFeeds.reduce((sum, f) => sum + f.messageCount, 0);

  if (relayFeeds.length === 0 && relayAgents.length === 0) {
    return (
      <div className="feeds-dash">
        <div className="feeds-dash__empty">
          No feed agents connected. Enable feed_agents in roko.toml.
        </div>
      </div>
    );
  }

  return (
    <div className="feeds-dash">
      {/* ── Header bar ──────────────────────────────────────── */}
      <div className="feeds-dash__header">
        <span className="feeds-dash__header-title">FEEDS</span>
        <span className="feeds-dash__header-stat">
          <span className="stat-value">{liveCount}</span> live
        </span>
        <span className="feeds-dash__header-stat">
          <span className="stat-value">{totalMsgCount}</span> msgs
        </span>
        <span className="feeds-dash__header-stat">
          <span className="stat-value">{relayAgents.length}</span> agents
        </span>
      </div>

      {/* ── Body (sidebar + grid) ──────────────────────────── */}
      <div className="feeds-dash__body">
        {/* Agent sidebar */}
        <div className="feeds-dash__sidebar">
          <div className="feeds-dash__sidebar-title">AGENTS</div>
          {relayAgents.map((agent) => (
            <div key={agent.agentId} className="feeds-dash__agent-row">
              <span className={agent.online ? 'feeds-dot feeds-dot--live' : 'feeds-dot feeds-dot--offline'} />
              <span title={agent.agentId}>{agent.name}</span>
            </div>
          ))}
        </div>

        {/* Feed card grid */}
        <div className="feeds-dash__grid">
          {relayFeeds.map((feed, i) => (
            <div
              key={feed.feedId}
              className="feeds-card"
              style={{ animationDelay: `${Math.min(i, 10) * 40}ms` }}
            >
              <div className="feeds-card__header">
                <span className="feeds-card__name">{feed.name}</span>
                <div className="feeds-card__meta">
                  <span className={feedDotClass(feed.status)} />
                  <span>{feed.rate}</span>
                </div>
              </div>

              <div className="feeds-card__value">
                {formatFeedValue(feed)}
              </div>

              <div className="feeds-card__sparkline">
                <Oscilloscope data={feed.sparkline} height={40} />
              </div>

              <div className="feeds-card__footer">
                <span>{feed.kind}</span>
                <span>{feed.messageCount} msgs</span>
                <span>{feed.lastUpdateMs ? formatTime(feed.lastUpdateMs) : '--'}</span>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* ── Live feed log ──────────────────────────────────── */}
      <div className="feeds-dash__log" ref={logRef}>
        <div className="feeds-dash__log-title">LIVE FEED LOG</div>
        {feedLog.slice(0, 50).map((entry, i) => (
          <div key={`${entry.ts}-${i}`} className="feeds-log-entry">
            <span className="feeds-log-entry__time">
              {formatTime(entry.ts)}
            </span>
            <span className="feeds-log-entry__agent">
              [{entry.agentId}]
            </span>
            <span className="feeds-log-entry__preview">
              {entry.preview}
            </span>
          </div>
        ))}
        {feedLog.length === 0 && (
          <div style={{ color: 'var(--text-tertiary)', fontStyle: 'italic' }}>
            Waiting for feed events...
          </div>
        )}
      </div>
    </div>
  );
}
