import { useRef, useEffect } from 'react';
import { useDataHub } from '../../../app/DataHub';

function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString('en-GB', { hour12: false });
}

export default function FeedLogOverlay() {
  const feedLog = useDataHub((s) => s.feedLog);
  const logRef = useRef<HTMLDivElement>(null);
  useEffect(() => { logRef.current && (logRef.current.scrollTop = logRef.current.scrollHeight); }, [feedLog.length]);

  const entries = feedLog.slice(-30);
  return (
    <div style={{ minWidth: 380, maxWidth: 520 }}>
      <div className="gp-label">FEED LOG</div>
      <div ref={logRef} style={{ maxHeight: 140, overflowY: 'auto', marginTop: 4,
        display: 'flex', flexDirection: 'column', gap: 1 }}>
        {!entries.length && <div className="gp-sub" style={{ fontStyle: 'italic', padding: '8px 0' }}>
          Waiting for feed events...
        </div>}
        {entries.map((entry, i) => (
          <div key={`${entry.ts}-${i}`} style={{
            display: 'flex', gap: 8, fontFamily: 'var(--mono)', fontSize: 11,
            padding: '1px 0', whiteSpace: 'nowrap', overflow: 'hidden',
          }}>
            <span style={{ color: 'var(--text-ghost)', flexShrink: 0 }}>{fmtTime(entry.ts)}</span>
            <span style={{ color: 'var(--status-active)', flexShrink: 0 }}>[{entry.agentId}]</span>
            <span style={{ color: 'var(--text-soft)', overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {entry.preview}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
