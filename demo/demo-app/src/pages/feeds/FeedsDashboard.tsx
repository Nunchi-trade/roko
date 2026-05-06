/**
 * Feeds Dashboard — flat panel layout (no Three.js / WebGL).
 *
 * Shows feed agents, their published feeds, live values, and activity log.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useDataHub } from '../../app/DataHub';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import type { RelayFeed } from '../../app/DataHub';
import Oscilloscope from '../../components/canvas/Oscilloscope';
import './FeedsDashboard.css';

// ── Helpers ──────────────────────────────────────────────────

const KIND_COLOR: Record<string, string> = {
  raw: 'var(--status-active)',
  derived: 'var(--rose-bright)',
  composite: 'var(--bone-bright)',
  meta: 'var(--dream-bright)',
};

function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString('en-GB', { hour12: false });
}

function timeAgo(ms: number | null): string {
  if (!ms) return '--';
  const diff = Date.now() - ms;
  if (diff < 1000) return 'just now';
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  return `${Math.floor(diff / 3_600_000)}h ago`;
}

function formatFeedValue(feed: RelayFeed): string {
  if (feed.lastValue === null) return 'awaiting...';
  const v = feed.lastValue as Record<string, unknown>;
  if (typeof v.compositeBps === 'number') return `${(v.compositeBps / 100).toFixed(2)}%`;
  if (typeof v.emaGwei === 'number') return `${v.emaGwei.toFixed(1)} gwei`;
  if (typeof v.rateBps === 'number') return `${(v.rateBps / 100).toFixed(2)}%`;
  if (typeof v.number === 'number') return `#${v.number}`;
  if (typeof v.blockNumber === 'number') return `#${v.blockNumber}`;
  if (typeof v.spreadBps === 'number') return `${v.spreadBps} bps`;
  if (typeof v.confidencePct === 'number') return `${v.confidencePct}%`;
  if (typeof v.relayAgentCount === 'number') return `${v.relayAgentCount} agents`;
  if (typeof v.totalFeeds === 'number') return `${v.totalFeeds} feeds`;
  const str = JSON.stringify(v);
  return str.length > 40 ? str.slice(0, 37) + '...' : str;
}

function statusDotClass(status: 'live' | 'stale' | 'offline'): string {
  if (status === 'live') return 'feeds__dot feeds__dot--live';
  if (status === 'stale') return 'feeds__dot feeds__dot--stale';
  return 'feeds__dot feeds__dot--offline';
}

// ── Main component ───────────────────────────────────────────

export default function FeedsDashboard() {
  const fetchFeedCatalog = useDataHub((s) => s.fetchFeedCatalog);
  const relayFeeds = useDataHub((s) => s.relayFeeds);
  const relayAgents = useDataHub((s) => s.relayAgents);
  const feedLog = useDataHub((s) => s.feedLog);

  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [kindFilter, setKindFilter] = useState<string>('all');
  const [, setTick] = useState(0);

  // Refresh time-ago labels every 5s.
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 5_000);
    return () => clearInterval(id);
  }, []);

  // Initial fetch.
  useEffect(() => { fetchFeedCatalog(); }, [fetchFeedCatalog]);

  // SSE subscriptions.
  const debouncedRefetch = useDebouncedRefetch(fetchFeedCatalog, 3000);
  useContextEventSubscription(
    ['feed_agent_online', 'feed_agent_offline'],
    useCallback(() => { debouncedRefetch(); }, [debouncedRefetch]),
  );

  // Derived stats.
  const liveFeeds = useMemo(() => relayFeeds.filter((f) => f.status === 'live').length, [relayFeeds]);
  const totalMsgs = useMemo(() => relayFeeds.reduce((sum, f) => sum + f.messageCount, 0), [relayFeeds]);
  const onlineAgents = useMemo(() => relayAgents.filter((a) => a.online).length, [relayAgents]);

  // Kind counts for tab badges.
  const kindCounts = useMemo(() => {
    const counts: Record<string, number> = { all: relayFeeds.length };
    for (const f of relayFeeds) {
      counts[f.kind] = (counts[f.kind] ?? 0) + 1;
    }
    return counts;
  }, [relayFeeds]);

  // Filtered feeds by kind.
  const filteredFeeds = useMemo(
    () => kindFilter === 'all' ? relayFeeds : relayFeeds.filter((f) => f.kind === kindFilter),
    [relayFeeds, kindFilter],
  );

  // Group filtered feeds by kind for the all-feeds grid.
  const feedsByKind = useMemo(() => {
    const groups: Record<string, RelayFeed[]> = {};
    for (const f of filteredFeeds) {
      (groups[f.kind] ??= []).push(f);
    }
    return Object.entries(groups).sort(([a], [b]) => a.localeCompare(b));
  }, [filteredFeeds]);

  // Selected agent's feeds.
  const agentFeeds = useMemo(
    () => selectedAgentId ? relayFeeds.filter((f) => f.agentId === selectedAgentId) : [],
    [relayFeeds, selectedAgentId],
  );
  const selectedAgent = useMemo(
    () => relayAgents.find((a) => a.agentId === selectedAgentId),
    [relayAgents, selectedAgentId],
  );

  // Agent online lookup for log coloring.
  const agentOnlineMap = useMemo(() => {
    const map = new Map<string, boolean>();
    for (const a of relayAgents) map.set(a.agentId, a.online);
    return map;
  }, [relayAgents]);

  return (
    <div className="feeds">
      {/* ── Stat bar ──────────────────────────────── */}
      <div className="feeds__stats">
        <div className="feeds__stat">
          <span className="feeds__stat-value feeds__stat-value--live">{liveFeeds}</span>
          <span className="feeds__stat-label">live feeds</span>
        </div>
        <div className="feeds__stat">
          <span className="feeds__stat-value">{totalMsgs.toLocaleString()}</span>
          <span className="feeds__stat-label">messages</span>
        </div>
        <div className="feeds__stat">
          <span className="feeds__stat-value">{onlineAgents}/{relayAgents.length}</span>
          <span className="feeds__stat-label">agents online</span>
        </div>
        <div className="feeds__stat">
          <span className="feeds__stat-value">{relayFeeds.length}</span>
          <span className="feeds__stat-label">total feeds</span>
        </div>
      </div>

      {/* ── Category tabs ────────────────────────────── */}
      <div className="feeds__tabs">
        {(['all', 'raw', 'derived', 'composite', 'meta'] as const).map((kind) => (
          <button
            key={kind}
            className={`feeds__tab${kindFilter === kind ? ' feeds__tab--active' : ''}`}
            style={kindFilter === kind && kind !== 'all'
              ? { '--tab-accent': KIND_COLOR[kind] } as React.CSSProperties
              : undefined}
            onClick={() => setKindFilter(kind)}
          >
            {kind.toUpperCase()}
            <span className="feeds__tab-count">{kindCounts[kind] ?? 0}</span>
          </button>
        ))}
      </div>

      {/* ── Main body ─────────────────────────────── */}
      <div className="feeds__body">
        {/* Left: agent list */}
        <div className="feeds__agents">
          <div className="feeds__section-hdr">
            <span className="feeds__section-title">AGENTS</span>
            <span className="feeds__section-count">{relayAgents.length}</span>
          </div>
          <div className="feeds__agent-scroll">
            {relayAgents.length === 0 && (
              <div className="feeds__empty">Loading agents...</div>
            )}
            {relayAgents.map((agent) => (
              <button
                key={agent.agentId}
                className={`feeds__agent-card${selectedAgentId === agent.agentId ? ' feeds__agent-card--selected' : ''}`}
                onClick={() => setSelectedAgentId(
                  selectedAgentId === agent.agentId ? null : agent.agentId,
                )}
              >
                <div className="feeds__agent-row">
                  <span className={agent.online ? 'feeds__dot feeds__dot--live' : 'feeds__dot feeds__dot--offline'} />
                  <span className="feeds__agent-name">{agent.name}</span>
                  <span className="feeds__agent-count">{agent.feedCount}</span>
                </div>
                {agent.capabilities.length > 0 && (
                  <div className="feeds__agent-caps">
                    {agent.capabilities.slice(0, 3).map((cap) => (
                      <span key={cap} className="feeds__cap-badge">{cap}</span>
                    ))}
                  </div>
                )}
              </button>
            ))}
          </div>
        </div>

        {/* Right: detail panel */}
        <div className="feeds__detail">
          {!selectedAgentId ? (
            <div className="feeds__all-feeds">
              {feedsByKind.length === 0 ? (
                <div className="feeds__detail-empty">
                  <span className="feeds__detail-empty-icon">&#x25CB;</span>
                  <span>No feeds available</span>
                </div>
              ) : (
                feedsByKind.map(([kind, feeds]) => (
                  <div key={kind} className="feeds__kind-group">
                    <div className="feeds__kind-header">
                      <span className="feeds__kind-dot" style={{ background: KIND_COLOR[kind] ?? 'var(--text-ghost)' }} />
                      <span>{kind.toUpperCase()}</span>
                      <span className="feeds__section-count">{feeds.length}</span>
                    </div>
                    <div className="feeds__kind-grid">
                      {feeds.map((feed, i) => (
                        <FeedCard key={feed.feedId} feed={feed} index={i} />
                      ))}
                    </div>
                  </div>
                ))
              )}
            </div>
          ) : (
            <>
              <div className="feeds__section-hdr">
                <span className="feeds__section-title">
                  {selectedAgent?.name ?? selectedAgentId}
                </span>
                <button
                  className="feeds__close-btn"
                  onClick={() => setSelectedAgentId(null)}
                >
                  &times;
                </button>
              </div>
              {selectedAgent && (
                <div className="feeds__agent-meta">
                  <span className={selectedAgent.online ? 'feeds__dot feeds__dot--live' : 'feeds__dot feeds__dot--offline'} />
                  <span>{selectedAgent.online ? 'Online' : 'Offline'}</span>
                  <span className="feeds__meta-sep">|</span>
                  <span>{selectedAgent.feedCount} feeds</span>
                </div>
              )}
              <div className="feeds__feed-grid">
                {agentFeeds.map((feed, i) => (
                  <FeedCard key={feed.feedId} feed={feed} index={i} />
                ))}
                {agentFeeds.length === 0 && (
                  <div className="feeds__empty">No feeds for this agent</div>
                )}
              </div>
            </>
          )}
        </div>
      </div>

      {/* ── Activity log ──────────────────────────── */}
      <div className="feeds__log">
        <div className="feeds__section-hdr">
          <span className="feeds__section-title">ACTIVITY</span>
          <span className="feeds__section-count">{feedLog.length}</span>
        </div>
        <div className="feeds__log-scroll">
          {feedLog.length === 0 ? (
            <div className="feeds__empty">Waiting for feed events...</div>
          ) : (
            feedLog.slice(0, 50).map((entry, i) => (
              <div key={`${entry.ts}-${i}`} className="feeds__log-row">
                <span className="feeds__log-time">{fmtTime(entry.ts)}</span>
                <span className="feeds__log-ago">{timeAgo(entry.ts)}</span>
                <span className={`feeds__log-agent${agentOnlineMap.get(entry.agentId) === false ? ' feeds__log-agent--offline' : ''}`}>
                  {entry.agentId}
                </span>
                <span className="feeds__log-preview">{entry.preview}</span>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

// ── Feed card sub-component ──────────────────────────────────

function FeedCard({ feed, index = 0 }: { feed: RelayFeed; index?: number }) {
  const accent = KIND_COLOR[feed.kind] ?? 'var(--status-active)';
  return (
    <div
      className="feeds__feed-card"
      style={{ '--i': index, '--feed-accent': accent } as React.CSSProperties}
    >
      <div className="feeds__feed-header">
        <span className={statusDotClass(feed.status)} />
        <span className="feeds__feed-name">{feed.name}</span>
        <span
          className="feeds__feed-kind"
          style={{ background: accent }}
        >
          {feed.kind}
        </span>
      </div>
      <div className="feeds__feed-value">{formatFeedValue(feed)}</div>
      {feed.sparkline.length > 1 && (
        <div className="feeds__feed-sparkline">
          <Oscilloscope data={feed.sparkline} height={28} color={accent} />
        </div>
      )}
      <div className="feeds__feed-footer">
        <span>{feed.messageCount} msgs</span>
        <span>{timeAgo(feed.lastUpdateMs)}</span>
        <span className="feeds__feed-rate">{feed.rate}</span>
      </div>
      {feed.description && (
        <div className="feeds__feed-desc">{feed.description}</div>
      )}
    </div>
  );
}
