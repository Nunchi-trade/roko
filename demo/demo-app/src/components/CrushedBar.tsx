interface CrushedBarProps {
  naiveLabel: string;
  naiveValue: number;
  actualLabel: string;
  actualValue: number;
  format?: (n: number) => string;
}

export default function CrushedBar({ naiveLabel, naiveValue, actualLabel, actualValue, format }: CrushedBarProps) {
  const max = Math.max(naiveValue, actualValue, 1);
  const fmt = format ?? ((n: number) => `$${n.toFixed(2)}`);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8, width: '100%' }}>
      <div>
        <div style={{ display: 'flex', justifyContent: 'space-between', fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--text-soft)', marginBottom: 4 }}>
          <span>{naiveLabel}</span>
          <span style={{ color: 'var(--bone-bright)' }}>{fmt(naiveValue)}</span>
        </div>
        <div style={{ height: 6, background: 'rgba(255,255,255,.04)', borderRadius: 3, overflow: 'hidden' }}>
          <div style={{
            height: '100%', width: `${(naiveValue / max) * 100}%`,
            background: 'var(--bone, #d4c89c)',
            transition: 'width 1s ease',
            boxShadow: '0 0 8px rgba(228,216,176,.3)',
          }} />
        </div>
      </div>
      <div>
        <div style={{ display: 'flex', justifyContent: 'space-between', fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--text-soft)', marginBottom: 4 }}>
          <span>{actualLabel}</span>
          <span style={{ color: 'var(--rose-glow)' }}>{fmt(actualValue)}</span>
        </div>
        <div style={{ height: 6, background: 'rgba(255,255,255,.04)', borderRadius: 3, overflow: 'hidden' }}>
          <div style={{
            height: '100%', width: `${(actualValue / max) * 100}%`,
            background: 'var(--rose-glow, #e8b5ce)',
            transition: 'width 1s ease',
            boxShadow: '0 0 8px rgba(232,181,206,.3)',
          }} />
        </div>
      </div>
    </div>
  );
}
