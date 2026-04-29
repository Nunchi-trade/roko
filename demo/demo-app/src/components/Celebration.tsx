/**
 * Celebration effects system — reusable confetti, success ring, sparkle, and glow flash.
 * All CSS-based, auto-cleans after animation. Respects prefers-reduced-motion.
 */
import { useState, useCallback, useRef, useEffect, useMemo } from 'react';
import './Celebration.css';

// ── Shared reduced-motion check ──

function usePrefersReducedMotion(): boolean {
  return useMemo(
    () => typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches,
    [],
  );
}

// ── ConfettiBurst ──

const CONFETTI_COLORS = [
  'var(--rose-bright)',   // rose
  'var(--rose-glow)',
  'var(--bone-bright)',   // amber/cream
  'var(--bone)',
  'var(--status-active)', // teal
  'var(--success)',
  'var(--dream-bright)',
];

interface ConfettiBurstProps {
  active: boolean;
  count?: number;
  duration?: number;
  onDone?: () => void;
}

export function ConfettiBurst({ active, count = 40, duration = 1200, onDone }: ConfettiBurstProps) {
  const reduced = usePrefersReducedMotion();
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => {
    if (active && onDone) {
      timerRef.current = setTimeout(onDone, duration);
      return () => clearTimeout(timerRef.current);
    }
  }, [active, duration, onDone]);

  if (!active || reduced) return null;

  const particles = Array.from({ length: count }, (_, i) => {
    const angle = (360 / count) * i + (Math.random() - 0.5) * 30;
    const dist = 60 + Math.random() * 120;
    const size = 3 + Math.random() * 5;
    const aspectRatio = 0.4 + Math.random() * 0.6;
    const color = CONFETTI_COLORS[i % CONFETTI_COLORS.length];
    const delay = Math.random() * 150;
    const spin = (Math.random() - 0.5) * 720;

    return (
      <span
        key={i}
        className="cele-confetti-particle"
        style={{
          '--angle': `${angle}deg`,
          '--dist': `${dist}px`,
          '--spin': `${spin}deg`,
          '--delay': `${delay}ms`,
          '--duration': `${duration}ms`,
          width: size,
          height: size * aspectRatio,
          background: color,
        } as React.CSSProperties}
      />
    );
  });

  return <div className="cele-confetti-burst">{particles}</div>;
}

// ── SuccessRing ──

interface SuccessRingProps {
  active: boolean;
  color?: string;
  duration?: number;
  onDone?: () => void;
}

export function SuccessRing({ active, color = 'var(--status-success)', duration = 800, onDone }: SuccessRingProps) {
  const reduced = usePrefersReducedMotion();
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => {
    if (active && onDone) {
      timerRef.current = setTimeout(onDone, duration);
      return () => clearTimeout(timerRef.current);
    }
  }, [active, duration, onDone]);

  if (!active || reduced) return null;

  return (
    <div className="cele-success-ring-wrap">
      <span
        className="cele-success-ring"
        style={{
          '--ring-color': color,
          '--ring-duration': `${duration}ms`,
        } as React.CSSProperties}
      />
    </div>
  );
}

// ── Sparkle ──

interface SparkleProps {
  active: boolean;
  count?: number;
  duration?: number;
  onDone?: () => void;
}

export function Sparkle({ active, count = 7, duration = 1000, onDone }: SparkleProps) {
  const reduced = usePrefersReducedMotion();
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => {
    if (active && onDone) {
      timerRef.current = setTimeout(onDone, duration);
      return () => clearTimeout(timerRef.current);
    }
  }, [active, duration, onDone]);

  if (!active || reduced) return null;

  const stars = Array.from({ length: count }, (_, i) => {
    const top = 10 + Math.random() * 80;
    const left = 10 + Math.random() * 80;
    const size = 6 + Math.random() * 10;
    const delay = Math.random() * 400;
    const twinkleDuration = 300 + Math.random() * 400;

    return (
      <span
        key={i}
        className="cele-sparkle-star"
        style={{
          top: `${top}%`,
          left: `${left}%`,
          '--sparkle-size': `${size}px`,
          '--sparkle-delay': `${delay}ms`,
          '--sparkle-duration': `${twinkleDuration}ms`,
        } as React.CSSProperties}
      />
    );
  });

  return <div className="cele-sparkle-wrap">{stars}</div>;
}

// ── GlowFlash ──

interface GlowFlashProps {
  active: boolean;
  color?: string;
  duration?: number;
  onDone?: () => void;
}

export function GlowFlash({ active, color = 'var(--status-success)', duration = 500, onDone }: GlowFlashProps) {
  const reduced = usePrefersReducedMotion();
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => {
    if (active && onDone) {
      timerRef.current = setTimeout(onDone, duration);
      return () => clearTimeout(timerRef.current);
    }
  }, [active, duration, onDone]);

  if (!active || reduced) return null;

  return (
    <span
      className="cele-glow-flash"
      style={{
        '--flash-color': color,
        '--flash-duration': `${duration}ms`,
      } as React.CSSProperties}
    />
  );
}

// ── useCelebration hook ──

interface CelebrationState {
  confetti: boolean;
  success: boolean;
  sparkle: boolean;
  glow: boolean;
  glowColor: string;
}

export function useCelebration() {
  const [state, setState] = useState<CelebrationState>({
    confetti: false,
    success: false,
    sparkle: false,
    glow: false,
    glowColor: 'var(--status-success)',
  });

  const triggerConfetti = useCallback(() => {
    setState((prev) => ({ ...prev, confetti: true }));
  }, []);

  const triggerSuccess = useCallback(() => {
    setState((prev) => ({ ...prev, success: true }));
  }, []);

  const triggerSparkle = useCallback(() => {
    setState((prev) => ({ ...prev, sparkle: true }));
  }, []);

  const triggerGlow = useCallback((color?: string) => {
    setState((prev) => ({ ...prev, glow: true, glowColor: color ?? 'var(--status-success)' }));
  }, []);

  const clearConfetti = useCallback(() => setState((prev) => ({ ...prev, confetti: false })), []);
  const clearSuccess = useCallback(() => setState((prev) => ({ ...prev, success: false })), []);
  const clearSparkle = useCallback(() => setState((prev) => ({ ...prev, sparkle: false })), []);
  const clearGlow = useCallback(() => setState((prev) => ({ ...prev, glow: false })), []);

  return {
    state,
    triggerConfetti,
    triggerSuccess,
    triggerSparkle,
    triggerGlow,
    clearConfetti,
    clearSuccess,
    clearSparkle,
    clearGlow,
  };
}

// ── CelebrationLayer — drop-in wrapper that renders all effects ──

interface CelebrationLayerProps {
  confetti: boolean;
  success: boolean;
  sparkle: boolean;
  glow: boolean;
  glowColor?: string;
  onConfettiDone?: () => void;
  onSuccessDone?: () => void;
  onSparkleDone?: () => void;
  onGlowDone?: () => void;
}

export function CelebrationLayer({
  confetti, success, sparkle, glow,
  glowColor = 'var(--status-success)',
  onConfettiDone, onSuccessDone, onSparkleDone, onGlowDone,
}: CelebrationLayerProps) {
  return (
    <div className="cele-layer">
      <ConfettiBurst active={confetti} onDone={onConfettiDone} />
      <SuccessRing active={success} onDone={onSuccessDone} />
      <Sparkle active={sparkle} onDone={onSparkleDone} />
      <GlowFlash active={glow} color={glowColor} onDone={onGlowDone} />
    </div>
  );
}
