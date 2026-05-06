import { useMemo } from 'react';
import { useDataHub } from '../../../app/DataHub';
import { useCountUp, fmtCount } from '../../../hooks/useCountUp';
import { formatBps } from '../../../lib/isfr-api';

export default function IsfrTickerOverlay() {
  const currentRate = useDataHub((s) => s.isfrCurrentRate);
  const keeper = useDataHub((s) => s.isfrKeeperStatus);
  const history = useDataHub((s) => s.isfrHistory);

  const compositeBps = currentRate?.compositeBps ?? 0;
  const confidenceBps = currentRate?.confidenceBps ?? 0;
  const animComposite = useCountUp(compositeBps, 900);
  const animConfidence = useCountUp(confidenceBps / 100, 800);

  const delta = useMemo(() => {
    if (history.length < 2) return 0;
    return history[history.length - 1].compositeBps - history[history.length - 2].compositeBps;
  }, [history]);

  const arcR = 22;
  const arcC = 2 * Math.PI * arcR;
  const arcPct = Math.min(animConfidence / 100, 1);
  const gaugeColor = confidenceBps >= 7000 ? 'var(--success)' :
                     confidenceBps >= 4000 ? 'var(--warning)' : 'var(--rose-bright)';

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 200 }}>
      <div className="gp-label">COMPOSITE RATE</div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <span className="gp-value" style={{ fontSize: 28 }}>
          {formatBps(Math.round(animComposite))}
        </span>
        {delta !== 0 && (
          <span style={{
            fontFamily: 'var(--mono)', fontSize: 'var(--text-sm)', fontWeight: 600,
            color: delta > 0 ? 'var(--success)' : 'var(--rose-bright)',
          }}>
            {delta > 0 ? '\u25B2' : '\u25BC'} {fmtCount(Math.abs(delta), 0)} bps
          </span>
        )}
      </div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginTop: 4 }}>
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center' }}>
          <svg width={56} height={56} viewBox="0 0 56 56">
            <circle cx={28} cy={28} r={arcR} fill="none" stroke="var(--glass-border)" strokeWidth={4} />
            <circle cx={28} cy={28} r={arcR} fill="none" stroke={gaugeColor} strokeWidth={4}
              strokeLinecap="round" strokeDasharray={arcC} strokeDashoffset={arcC * (1 - arcPct)}
              style={{ transform: 'rotate(-90deg)', transformOrigin: 'center',
                transition: 'stroke-dashoffset 0.8s ease, stroke 0.4s ease',
                filter: `drop-shadow(0 0 4px ${gaugeColor})` }} />
            <text x={28} y={28} textAnchor="middle" dominantBaseline="central"
              style={{ fontFamily: 'var(--mono)', fontSize: 11, fontWeight: 700, fill: 'var(--text-primary)' }}>
              {Math.round(animConfidence)}%
            </text>
          </svg>
          <span className="gp-sub" style={{ marginTop: 2 }}>CONFIDENCE</span>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <div className="gp-row">
            <span className={`gp-dot ${keeper === 'running' ? 'gp-dot--live' : 'gp-dot--offline'}`} />
            <span className="gp-sub">KEEPER {keeper === 'running' ? 'ACTIVE' : 'IDLE'}</span>
          </div>
          <div className="gp-row">
            <span className="gp-sub" style={{
              padding: '1px 6px', borderRadius: 'var(--radius-sm)',
              background: 'var(--glass-bg)', border: '1px solid var(--glass-border)',
              color: 'var(--bone-bright)',
            }}>{currentRate?.sourceCount ?? 0} SOURCES</span>
          </div>
        </div>
      </div>
    </div>
  );
}
