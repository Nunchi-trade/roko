import { useState } from 'react';
import { useDataHub } from '../../app/DataHub';
import type { RelayFeed } from '../../app/DataHub';
import Oscilloscope from '../../components/canvas/Oscilloscope';

const KIND_COLOR: Record<string, string> = {
  raw: 'var(--status-active)', derived: 'var(--rose-bright)',
  composite: 'var(--bone-bright)', meta: 'var(--dream-bright)',
};

function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString('en-GB', { hour12: false });
}

function formatFeedValue(feed: RelayFeed): string {
  if (feed.lastValue === null) return '--';
  const v = feed.lastValue as unknown as Record<string, unknown>;
  if (typeof v.compositeBps === 'number') return `${(v.compositeBps as number / 100).toFixed(2)}%`;
  if (typeof v.emaGwei === 'number') return `${(v.emaGwei as number).toFixed(1)} gwei`;
  if (typeof v.number === 'number') return `#${v.number}`;
  return JSON.stringify(v).slice(0, 30);
}

interface Props {
  selectedAgentId: string | null;
  onClose: () => void;
}

export default function FeedDetailDrawer({ selectedAgentId, onClose }: Props) {
  const relayFeeds = useDataHub((s) => s.relayFeeds);
  const [collapsed, setCollapsed] = useState(false);

  if (!selectedAgentId) return null;

  const agentFeeds = relayFeeds.filter(
    (f) => f.agentId === selectedAgentId,
  );

  if (!agentFeeds.length) return null;

  return (
    <div className="feeds-drawer" data-open={!collapsed}>
      <div className="feeds-drawer__header">
        <button className="feeds-drawer__toggle" onClick={() => setCollapsed(!collapsed)}>
          {collapsed ? '\u25B2' : '\u25BC'}
        </button>
        <span style={{ fontFamily: 'var(--mono)', fontSize: 'var(--text-xs)', letterSpacing: '0.06em',
          textTransform: 'uppercase', color: 'var(--text-primary)' }}>
          FEEDS FOR {selectedAgentId}
        </span>
        <button className="feeds-drawer__close" onClick={onClose}>&times;</button>
      </div>
      {!collapsed && (
        <div className="feeds-drawer__body">
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(220px, 1fr))', gap: 8 }}>
            {agentFeeds.map((feed) => (
              <div key={feed.feedId} className="feeds-drawer__card">
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 4 }}>
                  <span style={{ fontFamily: 'var(--mono)', fontSize: 11, fontWeight: 600,
                    color: 'var(--text-primary)', flex: 1, overflow: 'hidden',
                    textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {feed.name}
                  </span>
                  <span style={{ fontFamily: 'var(--mono)', fontSize: 9, padding: '1px 5px',
                    borderRadius: 'var(--radius-sm)',
                    background: KIND_COLOR[feed.kind] ?? 'var(--status-active)',
                    color: 'var(--bg-void)', fontWeight: 600, textTransform: 'uppercase' }}>
                    {feed.kind}
                  </span>
                </div>
                <div style={{ fontFamily: 'var(--mono)', fontSize: 16, fontWeight: 700,
                  color: 'var(--rose-bright)', marginBottom: 4 }}>
                  {formatFeedValue(feed)}
                </div>
                <div style={{ height: 32, marginBottom: 4 }}>
                  <Oscilloscope data={feed.sparkline} height={32} color={KIND_COLOR[feed.kind]} />
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between',
                  fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--text-ghost)' }}>
                  <span>{feed.messageCount} msgs</span>
                  <span>{feed.lastUpdateMs ? fmtTime(feed.lastUpdateMs) : '--'}</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
