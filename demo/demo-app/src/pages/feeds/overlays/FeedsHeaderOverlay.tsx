import { useMemo } from 'react';
import { useDataHub } from '../../../app/DataHub';
import { useCountUp, fmtCount } from '../../../hooks/useCountUp';

export default function FeedsHeaderOverlay() {
  const relayFeeds = useDataHub((s) => s.relayFeeds);
  const relayAgents = useDataHub((s) => s.relayAgents);
  const feedThroughput = useDataHub((s) => s.feedThroughput);

  const liveCount = useMemo(() => relayFeeds.filter((f) => f.status === 'live').length, [relayFeeds]);
  const totalMsgs = useMemo(() => relayFeeds.reduce((sum, f) => sum + f.messageCount, 0), [relayFeeds]);
  const msgsPerSec = feedThroughput.length > 0 ? feedThroughput[feedThroughput.length - 1] : 0;

  const animLive = useCountUp(liveCount, 600);
  const animMsgs = useCountUp(totalMsgs, 900);
  const animAgents = useCountUp(relayAgents.length, 600);
  const animRate = useCountUp(msgsPerSec, 800);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 200 }}>
      <div className="gp-label">FEEDS NETWORK</div>
      <div style={{ display: 'flex', gap: 16 }}>
        <StatCell label="LIVE" value={fmtCount(Math.round(animLive))} color="var(--success)" />
        <StatCell label="MSGS" value={fmtCount(Math.round(animMsgs))} color="var(--rose-bright)" />
        <StatCell label="AGENTS" value={fmtCount(Math.round(animAgents))} color="var(--status-active)" />
      </div>
      <div style={{ fontFamily: 'var(--mono)', fontSize: 'var(--text-xs)', color: 'var(--text-dim)' }}>
        {fmtCount(animRate, 1)} msgs/sec
      </div>
    </div>
  );
}

function StatCell({ label, value, color }: { label: string; value: string; color: string }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
      <span style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.1em',
        color: 'var(--text-ghost)' }}>{label}</span>
      <span style={{ fontFamily: 'var(--mono)', fontSize: 'var(--text-lg)', fontWeight: 700,
        color, textShadow: `0 0 8px color-mix(in srgb, ${color} 30%, transparent)` }}>{value}</span>
    </div>
  );
}
