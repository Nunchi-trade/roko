/**
 * Feeds Dashboard — premium data-feed monitoring UI.
 *
 * Shows feed agents, their published feeds, live values, sparklines, and activity.
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

/** Resolve CSS var to a raw fallback for canvas/non-CSS contexts. */
const KIND_RAW: Record<string, string> = {
  raw: '#98c379',
  derived: '#cc90a8',
  composite: '#d8c8a0',
  meta: '#61afef',
};

const KIND_LABEL: Record<string, string> = {
  raw: 'Raw Signal',
  derived: 'Derived',
  composite: 'Composite',
  meta: 'Meta',
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

function truncId(id: string): string {
  if (id.length <= 16) return id;
  return `${id.slice(0, 8)}…${id.slice(-4)}`;
}

interface FeedValue {
  primary: string;
  secondary?: string;
  /** True when the value is a status/label rather than a data point. */
  dim?: boolean;
}

/**
 * Smart value formatter — extracts the most meaningful number from any feed
 * payload shape and formats it as a concise, human-readable string.
 */
function formatFeedValue(feed: RelayFeed): FeedValue {
  if (feed.lastValue === null || feed.lastValue === undefined) {
    return { primary: 'Awaiting', secondary: 'no data yet', dim: true };
  }

  const v = feed.lastValue as Record<string, unknown>;

  // ── Specific known shapes (most common first) ──
  if (typeof v.compositeBps === 'number') {
    return { primary: `${(v.compositeBps / 100).toFixed(2)}%`, secondary: 'composite rate' };
  }
  if (typeof v.rateBps === 'number') {
    return { primary: `${(v.rateBps / 100).toFixed(2)}%`, secondary: 'rate' };
  }
  if (typeof v.emaGwei === 'number') {
    return { primary: `${v.emaGwei.toFixed(1)}`, secondary: 'gwei EMA' };
  }
  if (typeof v.spreadBps === 'number') {
    return { primary: `${v.spreadBps}`, secondary: 'bps spread' };
  }
  if (typeof v.confidencePct === 'number') {
    return { primary: `${v.confidencePct}%`, secondary: 'confidence' };
  }
  if (typeof v.number === 'number') {
    return { primary: `#${v.number.toLocaleString()}`, secondary: 'block' };
  }
  if (typeof v.blockNumber === 'number') {
    return { primary: `#${v.blockNumber.toLocaleString()}`, secondary: 'block' };
  }

  // ── Aggregation-type feeds ──
  if (typeof v.relayAgentCount === 'number') {
    return { primary: `${v.relayAgentCount}`, secondary: 'agents' };
  }
  if (typeof v.totalFeeds === 'number') {
    return { primary: `${v.totalFeeds}`, secondary: 'feeds' };
  }

  // ── Health/status objects — show rate if present, otherwise show status nicely ──
  if (typeof v.health === 'string') {
    const cls = typeof v.class === 'string' && v.class !== 'null' ? v.class : null;
    const rate = typeof v.lastRateBps === 'number' ? `${(v.lastRateBps / 100).toFixed(2)}%` : null;
    if (rate) return { primary: rate, secondary: cls ? `${cls} · ${v.health}` : String(v.health) };
    // Map raw health statuses to polished labels
    const healthLabels: Record<string, string> = {
      unavailable: 'Pending',
      live: 'Live',
      stale: 'Stale',
      offline: 'Offline',
      unknown: 'Initializing',
    };
    const label = healthLabels[v.health] ?? String(v.health);
    return { primary: label, secondary: cls ?? 'source status', dim: v.health !== 'live' };
  }

  // ── Epoch/state objects — show epoch or count ──
  if (typeof v.currentEpoch === 'number') {
    const cap = typeof v.capacitiveIndex === 'number' ? `cap ${v.capacitiveIndex.toFixed(2)}` : undefined;
    return { primary: `Epoch ${v.currentEpoch}`, secondary: cap };
  }
  if (typeof v.sourceCount === 'number') {
    return { primary: `${v.sourceCount}`, secondary: 'sources' };
  }

  // ── Pick the first real numeric value ──
  for (const [key, val] of Object.entries(v)) {
    if (typeof val === 'number' && !Number.isNaN(val)) {
      const formatted = Number.isInteger(val) ? val.toLocaleString() : val.toFixed(2);
      return { primary: formatted, secondary: key.replace(/([A-Z])/g, ' $1').toLowerCase().trim() };
    }
  }

  // ── Pick first non-null string value ──
  for (const [key, val] of Object.entries(v)) {
    if (typeof val === 'string' && val !== 'null' && val !== '') {
      const short = val.length > 20 ? val.slice(0, 18) + '...' : val;
      return { primary: short, secondary: key.replace(/([A-Z])/g, ' $1').toLowerCase().trim(), dim: true };
    }
  }

  // ── Truly unknown ──
  const keys = Object.keys(v).filter((k) => v[k] !== null);
  if (keys.length === 0) return { primary: 'No Data', dim: true };
  return { primary: `${keys.length} fields`, secondary: keys.slice(0, 2).join(', '), dim: true };
}

/**
 * Clean up raw log preview text — strip null values and format for readability.
 */
function cleanPreview(raw: string): string {
  if (!raw) return '';
  // Remove key:null pairs
  let cleaned = raw.replace(/\w+:null\s*/g, '').trim();
  // Collapse multiple spaces
  cleaned = cleaned.replace(/\s+/g, ' ');
  // If it's all gone, show something useful
  if (!cleaned || cleaned === '{}') return 'update received';
  // Trim overly long previews
  if (cleaned.length > 80) return cleaned.slice(0, 77) + '...';
  return cleaned;
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

  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 5_000);
    return () => clearInterval(id);
  }, []);

  useEffect(() => { fetchFeedCatalog(); }, [fetchFeedCatalog]);

  const debouncedRefetch = useDebouncedRefetch(fetchFeedCatalog, 3000);
  useContextEventSubscription(
    ['feed_agent_online', 'feed_agent_offline'],
    useCallback(() => { debouncedRefetch(); }, [debouncedRefetch]),
  );

  // Derived stats.
  const liveFeeds = useMemo(() => relayFeeds.filter((f) => f.status === 'live').length, [relayFeeds]);
  const staleFeeds = useMemo(() => relayFeeds.filter((f) => f.status === 'stale').length, [relayFeeds]);
  const totalMsgs = useMemo(() => relayFeeds.reduce((sum, f) => sum + f.messageCount, 0), [relayFeeds]);
  const onlineAgents = useMemo(() => relayAgents.filter((a) => a.online).length, [relayAgents]);

  // Kind counts.
  const kindCounts = useMemo(() => {
    const counts: Record<string, number> = { all: relayFeeds.length };
    for (const f of relayFeeds) counts[f.kind] = (counts[f.kind] ?? 0) + 1;
    return counts;
  }, [relayFeeds]);

  // Filtered + grouped feeds.
  const filteredFeeds = useMemo(
    () => kindFilter === 'all' ? relayFeeds : relayFeeds.filter((f) => f.kind === kindFilter),
    [relayFeeds, kindFilter],
  );
  const feedsByKind = useMemo(() => {
    const groups: Record<string, RelayFeed[]> = {};
    for (const f of filteredFeeds) (groups[f.kind] ??= []).push(f);
    // Sort: composite first (most important), then derived, raw, meta
    const order = ['composite', 'derived', 'raw', 'meta'];
    return Object.entries(groups).sort(([a], [b]) => order.indexOf(a) - order.indexOf(b));
  }, [filteredFeeds]);

  // Selected agent.
  const agentFeeds = useMemo(
    () => selectedAgentId ? relayFeeds.filter((f) => f.agentId === selectedAgentId) : [],
    [relayFeeds, selectedAgentId],
  );
  const selectedAgent = useMemo(
    () => relayAgents.find((a) => a.agentId === selectedAgentId),
    [relayAgents, selectedAgentId],
  );

  // Agent name lookup for log.
  const agentNameMap = useMemo(() => {
    const map = new Map<string, { name: string; online: boolean }>();
    for (const a of relayAgents) map.set(a.agentId, { name: a.name, online: a.online });
    return map;
  }, [relayAgents]);

  return (
    <div className="feeds">
      {/* ── Hero stat mosaic ────────────────────────── */}
      <div className="feeds__stats">
        <div className="feeds__stat feeds__stat--hero">
          <span className="feeds__stat-value feeds__stat-value--live">{liveFeeds}</span>
          <span className="feeds__stat-label">live</span>
        </div>
        <div className="feeds__stat">
          <span className="feeds__stat-value feeds__stat-value--stale">{staleFeeds}</span>
          <span className="feeds__stat-label">stale</span>
        </div>
        <div className="feeds__stat">
          <span className="feeds__stat-value">{totalMsgs.toLocaleString()}</span>
          <span className="feeds__stat-label">messages</span>
        </div>
        <div className="feeds__stat">
          <span className="feeds__stat-value feeds__stat-value--agents">
            {onlineAgents}<span className="feeds__stat-dim">/{relayAgents.length}</span>
          </span>
          <span className="feeds__stat-label">agents</span>
        </div>
        <div className="feeds__stat">
          <span className="feeds__stat-value">{relayFeeds.length}</span>
          <span className="feeds__stat-label">total feeds</span>
        </div>
      </div>

      {/* ── Kind filter tabs ────────────────────────── */}
      <div className="feeds__tabs">
        {(['all', 'raw', 'derived', 'composite', 'meta'] as const).map((kind) => {
          const active = kindFilter === kind;
          const accent = kind !== 'all' ? KIND_COLOR[kind] : undefined;
          return (
            <button
              key={kind}
              className={`feeds__tab${active ? ' feeds__tab--active' : ''}`}
              style={active && accent ? { '--tab-accent': accent } as React.CSSProperties : undefined}
              onClick={() => setKindFilter(kind)}
            >
              {kind === 'all' ? 'ALL' : kind.toUpperCase()}
              <span className="feeds__tab-count">{kindCounts[kind] ?? 0}</span>
            </button>
          );
        })}
      </div>

      {/* ── Main body ─────────────────────────────── */}
      <div className="feeds__body">
        {/* Left: agent sidebar */}
        <div className="feeds__agents">
          <div className="feeds__section-hdr">
            <span className="feeds__section-title">AGENTS</span>
            <span className="feeds__section-count">{relayAgents.length}</span>
          </div>
          <div className="feeds__agent-scroll">
            {relayAgents.length === 0 && (
              <div className="feeds__empty">
                <span className="feeds__empty-icon">&#x25CB;</span>
                Waiting for agents...
              </div>
            )}
            {relayAgents.map((agent) => {
              const isSelected = selectedAgentId === agent.agentId;
              return (
                <button
                  key={agent.agentId}
                  className={`feeds__agent-card${isSelected ? ' feeds__agent-card--selected' : ''}`}
                  onClick={() => setSelectedAgentId(isSelected ? null : agent.agentId)}
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
              );
            })}
          </div>
        </div>

        {/* Right: feed grid or agent detail */}
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
                      <span className="feeds__kind-label">{KIND_LABEL[kind] ?? kind}</span>
                      <span className="feeds__section-count">{feeds.length}</span>
                      <span className="feeds__kind-line" style={{ background: KIND_COLOR[kind] ?? 'var(--glass-border)' }} />
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
              <div className="feeds__section-hdr feeds__section-hdr--detail">
                <span className="feeds__section-title">
                  {selectedAgent?.name ?? selectedAgentId}
                </span>
                <button className="feeds__close-btn" onClick={() => setSelectedAgentId(null)}>
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
            <div className="feeds__empty">
              <span className="feeds__empty-icon">&#x23DA;</span>
              Waiting for feed events...
            </div>
          ) : (
            feedLog.slice(0, 50).map((entry, i) => {
              const agent = agentNameMap.get(entry.agentId);
              return (
                <div key={`${entry.ts}-${i}`} className="feeds__log-row">
                  <span className="feeds__log-time">{fmtTime(entry.ts)}</span>
                  <span className="feeds__log-ago">{timeAgo(entry.ts)}</span>
                  <span className={`feeds__log-agent${agent?.online === false ? ' feeds__log-agent--offline' : ''}`}>
                    {agent?.name ?? truncId(entry.agentId)}
                  </span>
                  <span className="feeds__log-preview">{cleanPreview(entry.preview)}</span>
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}

// ── Feed card ────────────────────────────────────────────────

function FeedCard({ feed, index = 0 }: { feed: RelayFeed; index?: number }) {
  const accent = KIND_COLOR[feed.kind] ?? 'var(--status-active)';
  const rawColor = KIND_RAW[feed.kind] ?? '#98c379';
  const { primary, secondary, dim } = formatFeedValue(feed);

  return (
    <div
      className={`feeds__feed-card${dim ? ' feeds__feed-card--dim' : ''}`}
      style={{ '--i': index, '--feed-accent': accent } as React.CSSProperties}
    >
      {/* Top row: status + name + kind badge */}
      <div className="feeds__feed-header">
        <span className={statusDotClass(feed.status)} />
        <span className="feeds__feed-name">{feed.name}</span>
        <span className="feeds__feed-kind" style={{ background: accent }}>{feed.kind}</span>
      </div>

      {/* Topic */}
      {feed.topic && (
        <div className="feeds__feed-topic">{feed.topic}</div>
      )}

      {/* Value area — the hero of each card */}
      <div className="feeds__feed-value-area">
        <span className={`feeds__feed-value${dim ? ' feeds__feed-value--dim' : ''}`}>{primary}</span>
        {secondary && <span className="feeds__feed-secondary">{secondary}</span>}
      </div>

      {/* Sparkline */}
      {feed.sparkline.length > 1 && (
        <div className="feeds__feed-sparkline">
          <Oscilloscope data={feed.sparkline} height={36} color={rawColor} />
        </div>
      )}

      {/* Footer: message count + last update + rate */}
      <div className="feeds__feed-footer">
        <span className="feeds__feed-msgs">{feed.messageCount.toLocaleString()} msgs</span>
        <span className="feeds__feed-updated">{timeAgo(feed.lastUpdateMs)}</span>
        {feed.rate && <span className="feeds__feed-rate">{feed.rate}</span>}
      </div>

      {/* Description */}
      {feed.description && (
        <div className="feeds__feed-desc">{feed.description}</div>
      )}
    </div>
  );
}
