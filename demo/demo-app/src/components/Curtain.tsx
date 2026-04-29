import { useEffect, useState } from 'react';

const PHASES = ['INITIALIZING', 'RESOLVING SUBSTRATE', 'ENTERING COORDINATION PLANE'];
const PHASE_MS = 350;
const TOTAL_MS = 1200;

export default function Curtain() {
  const [phase, setPhase] = useState(0);
  const [exiting, setExiting] = useState(false);
  const [gone, setGone] = useState(false);

  useEffect(() => {
    // Cycle through text phases
    const phaseTimer = setInterval(() => {
      setPhase((p) => Math.min(p + 1, PHASES.length - 1));
    }, PHASE_MS);

    // Start exit animation
    const exitTimer = setTimeout(() => setExiting(true), TOTAL_MS);

    // Remove from DOM after exit animation completes
    const goneTimer = setTimeout(() => setGone(true), TOTAL_MS + 600);

    return () => {
      clearInterval(phaseTimer);
      clearTimeout(exitTimer);
      clearTimeout(goneTimer);
    };
  }, []);

  if (gone) return null;

  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        background: 'var(--bg-void)',
        zIndex: 10000,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        clipPath: exiting ? 'inset(0 0 100% 0)' : 'inset(0 0 0 0)',
        transition: 'clip-path 0.6s cubic-bezier(0.22, 1, 0.36, 1)',
        pointerEvents: exiting ? 'none' : 'auto',
      }}
    >
      {/* Pulsing rose orb */}
      <div
        style={{
          width: 24,
          height: 24,
          borderRadius: '50%',
          background: 'var(--rose-glow)',
          boxShadow: '0 0 40px var(--rose-glow), 0 0 80px rgba(220,165,189,.3)',
          animation: 'curtainPulse 1.2s ease-in-out infinite',
        }}
      />

      {/* Cycling text */}
      <div
        style={{
          marginTop: 24,
          fontFamily: 'var(--mono)',
          fontSize: 14,
          letterSpacing: '.22em',
          textTransform: 'uppercase',
          color: 'var(--text-dim)',
          minHeight: 18,
          transition: 'opacity 0.15s ease',
        }}
      >
        {PHASES[phase]}
      </div>

      <style>{`
        @keyframes curtainPulse {
          0%, 100% { transform: scale(1); opacity: .8; }
          50% { transform: scale(1.3); opacity: 1; }
        }
      `}</style>
    </div>
  );
}
