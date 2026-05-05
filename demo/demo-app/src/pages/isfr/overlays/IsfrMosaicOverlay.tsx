import { useDataHub } from '../../../app/DataHub';
import Oscilloscope from '../../../components/canvas/Oscilloscope';
import { formatBps } from '../../../lib/isfr-api';
import { useCountUp } from '../../../hooks/useCountUp';

const FIELDS = [
  { key: 'lendingBps',    label: 'LENDING',    field: 'lending',    color: 'var(--status-active)' },
  { key: 'structuredBps', label: 'STRUCTURED', field: 'structured', color: 'var(--dream-bright)' },
  { key: 'stakingBps',    label: 'STAKING',    field: 'staking',    color: 'var(--bone-bright)' },
  { key: 'fundingBps',    label: 'FUNDING',    field: 'funding',    color: 'var(--rose-bright)' },
] as const;

export default function IsfrMosaicOverlay() {
  const currentRate = useDataHub((s) => s.isfrCurrentRate);
  const fieldHistory = useDataHub((s) => s.isfrFieldHistory);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6, minWidth: 260 }}>
      <div className="gp-label">FIELD BREAKDOWN</div>
      {FIELDS.map((f) => (
        <FieldRow key={f.key} label={f.label}
          bps={currentRate?.[f.key] ?? 0} color={f.color}
          sparkline={fieldHistory[f.field] ?? []} />
      ))}
    </div>
  );
}

function FieldRow({ label, bps, color, sparkline }: {
  label: string; bps: number; color: string; sparkline: number[];
}) {
  const anim = useCountUp(bps, 800);
  return (
    <div style={{
      display: 'flex', alignItems: 'center', gap: 8, padding: '3px 0',
      borderBottom: '1px solid color-mix(in srgb, var(--glass-border) 30%, transparent)',
    }}>
      <span style={{ fontFamily: 'var(--mono)', fontSize: 9, letterSpacing: '0.08em',
        color, width: 80, flexShrink: 0 }}>{label}</span>
      <span style={{ fontFamily: 'var(--mono)', fontSize: 'var(--text-sm)', fontWeight: 600,
        color: 'var(--text-primary)', width: 70, textAlign: 'right' }}>
        {formatBps(Math.round(anim))}
      </span>
      <div style={{ flex: 1, height: 24, minWidth: 60 }}>
        <Oscilloscope data={sparkline} color={color} height={24} />
      </div>
    </div>
  );
}
