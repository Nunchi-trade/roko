import { useMemo } from 'react';
import { useDataHub } from '../../../app/DataHub';
import Oscilloscope from '../../../components/canvas/Oscilloscope';
import { formatBps } from '../../../lib/isfr-api';

interface Props { onSelect: (name: string) => void; }

const HEALTH_DOT: Record<string, string> = {
  live: 'gp-dot gp-dot--live', stale: 'gp-dot gp-dot--stale', offline: 'gp-dot gp-dot--offline',
};
const CLASS_CLR: Record<string, string> = {
  lending: 'var(--status-active)', structured: 'var(--dream-bright)',
  staking: 'var(--bone-bright)', funding: 'var(--rose-bright)',
};

export default function IsfrSourceListOverlay({ onSelect }: Props) {
  const sources = useDataHub((s) => s.isfrSources);
  const sourceHistory = useDataHub((s) => s.isfrSourceHistory);
  const sorted = useMemo(() => [...sources].sort((a, b) => b.weight - a.weight), [sources]);

  if (!sorted.length) return (
    <div style={{ minWidth: 200 }}>
      <div className="gp-label">SOURCES</div>
      <div className="gp-sub">No sources connected</div>
    </div>
  );

  return (
    <div style={{ minWidth: 240, maxHeight: 280, overflowY: 'auto' }}>
      <div className="gp-label">SOURCES ({sorted.length})</div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 2, marginTop: 4 }}>
        {sorted.map((src) => {
          const hist = sourceHistory[src.name]?.map((s) => s.bps) ?? [];
          return (
            <button key={src.name} onClick={() => onSelect(src.name)}
              style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '3px 4px',
                borderRadius: 'var(--radius-sm)', border: 'none', background: 'transparent',
                cursor: 'pointer', width: '100%', textAlign: 'left' }}
              onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.background = 'var(--glass-bg)'; }}
              onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.background = 'transparent'; }}>
              <span className={HEALTH_DOT[src.health] ?? 'gp-dot'} />
              <span style={{ fontFamily: 'var(--mono)', fontSize: 11, fontWeight: 600,
                color: 'var(--text-primary)', flex: 1, minWidth: 0,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {src.name}
              </span>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 9, padding: '1px 4px',
                borderRadius: 'var(--radius-sm)',
                background: CLASS_CLR[src.class] ?? 'var(--rose-dim)',
                color: 'var(--bg-void)', fontWeight: 600,
                letterSpacing: '0.04em', textTransform: 'uppercase' }}>
                {src.class.slice(0, 4)}
              </span>
              <span style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--rose-bright)',
                fontWeight: 600, width: 50, textAlign: 'right', flexShrink: 0 }}>
                {src.lastRateBps != null ? formatBps(src.lastRateBps) : '--'}
              </span>
              {hist.length >= 2 && (
                <div style={{ width: 40, height: 16, flexShrink: 0 }}>
                  <Oscilloscope data={hist} height={16} color={CLASS_CLR[src.class]} />
                </div>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}
