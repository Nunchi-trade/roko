interface LiveIndicatorProps {
  active?: boolean;
}

export default function LiveIndicator({ active = true }: LiveIndicatorProps) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
      <span style={{
        width: 6, height: 6, borderRadius: '50%',
        background: active ? 'var(--success, #7d9e8c)' : 'var(--text-dim)',
        boxShadow: active ? '0 0 8px rgba(125,158,140,.5)' : 'none',
        animation: active ? 'livepulse 2s ease-in-out infinite' : 'none',
      }} />
      <span style={{
        fontFamily: 'var(--mono)', fontSize: 9,
        letterSpacing: '.22em', textTransform: 'uppercase',
        color: active ? 'var(--success, #7d9e8c)' : 'var(--text-dim)',
      }}>
        {active ? 'LIVE' : 'OFFLINE'}
      </span>
    </span>
  );
}
