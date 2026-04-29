interface ConnectScreenProps {
  onRetry?: () => void;
}

export default function ConnectScreen({ onRetry }: ConnectScreenProps) {
  return (
    <div style={{
      position: 'fixed', inset: 0, zIndex: 9999,
      display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
      background: 'var(--bg-void, #060608)',
    }}>
      <div style={{
        width: 16, height: 16, borderRadius: '50%',
        background: 'var(--rose-dim, #6b4a5e)',
        boxShadow: '0 0 30px rgba(220,165,189,.3)',
        animation: 'connectPulse 1.8s ease-in-out infinite',
      }} />
      <div style={{
        marginTop: 20,
        fontFamily: 'var(--mono)', fontSize: 14,
        letterSpacing: '.22em', textTransform: 'uppercase',
        color: 'var(--text-dim)',
      }}>
        connecting to roko serve...
      </div>
      {onRetry && (
        <button
          onClick={onRetry}
          style={{
            marginTop: 24, padding: '8px 20px',
            fontFamily: 'var(--mono)', fontSize: 13,
            letterSpacing: '.2em', textTransform: 'uppercase',
            color: 'var(--rose-glow)', background: 'transparent',
            border: '1px solid var(--rose-dim)', cursor: 'pointer',
          }}
        >
          Retry
        </button>
      )}
    </div>
  );
}
