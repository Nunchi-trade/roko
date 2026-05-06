import { useRef, useEffect } from 'react';
import { useDataHub } from '../../../app/DataHub';

const TYPE_ICON: Record<string, string> = { rate: '\u2248', source: '\u2022', keeper: '\u25C6' };
const TYPE_COLOR: Record<string, string> = {
  rate: 'var(--success)', source: 'var(--dream-bright)', keeper: 'var(--warning)',
};

function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString('en-GB', { hour12: false });
}

export default function IsfrEventLogOverlay() {
  const eventLog = useDataHub((s) => s.isfrEventLog);
  const logRef = useRef<HTMLDivElement>(null);
  useEffect(() => { logRef.current && (logRef.current.scrollTop = logRef.current.scrollHeight); }, [eventLog.length]);

  const events = eventLog.slice(-20);
  return (
    <div style={{ minWidth: 400, maxWidth: 520 }}>
      <div className="gp-label">EVENT LOG</div>
      <div ref={logRef} style={{ maxHeight: 160, overflowY: 'auto', marginTop: 4,
        display: 'flex', flexDirection: 'column', gap: 1 }}>
        {!events.length && <div className="gp-sub" style={{ fontStyle: 'italic', padding: '8px 0' }}>Waiting for events...</div>}
        {events.map((ev, i) => {
          const t = ev.type ?? 'source';
          return (
            <div key={`${ev.ts}-${i}`} style={{
              display: 'flex', alignItems: 'baseline', gap: 6,
              fontFamily: 'var(--mono)', fontSize: 11, padding: '1px 0',
              borderLeft: `2px solid color-mix(in srgb, ${TYPE_COLOR[t] ?? 'var(--text-ghost)'} 30%, transparent)`,
              paddingLeft: 6 }}>
              <span style={{ color: TYPE_COLOR[t] ?? 'var(--text-ghost)', flexShrink: 0, width: 12 }}>
                {TYPE_ICON[t] ?? '\u00B7'}
              </span>
              <span style={{ color: 'var(--text-ghost)', flexShrink: 0, width: 65 }}>{fmtTime(ev.ts)}</span>
              <span style={{ color: 'var(--text-soft)', flex: 1, minWidth: 0,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {ev.message}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
