import { useEffect, useRef, useState } from 'react';
import './ModelSlot.css';

interface ModelSlotProps {
  model: string;
  tier: 'T0' | 'T1' | 'T2';
  size?: 'sm' | 'md' | 'lg';
  /** Whether inference is actively running. */
  active?: boolean;
  /** Token count (animates counting up). */
  tokens?: number;
  /** Cost string e.g. "$0.03". */
  cost?: string;
  /** Latency in ms. */
  latencyMs?: number;
  className?: string;
}

/** Characters cycled through during the spin animation. */
const SPIN_CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';

function randomChar(): string {
  return SPIN_CHARS[Math.floor(Math.random() * SPIN_CHARS.length)];
}

/** Per-character delay in ms for staggered animation. */
const CHAR_STAGGER_MS = 40;

/** Latency color thresholds. */
function latencyColor(ms: number): string {
  if (ms < 500) return 'var(--status-success)';
  if (ms < 2000) return 'var(--status-warning)';
  return 'var(--status-error)';
}

/** Hook: animated integer counter. */
function useCountUp(target: number, durationMs: number = 600): number {
  const [value, setValue] = useState(target);
  const rafRef = useRef<number>(0);

  useEffect(() => {
    const from = value;
    if (from === target) return;
    const start = performance.now();

    function tick(now: number) {
      const elapsed = now - start;
      const progress = Math.min(elapsed / durationMs, 1);
      // easeOutCubic
      const eased = 1 - Math.pow(1 - progress, 3);
      setValue(Math.round(from + (target - from) * eased));
      if (progress < 1) {
        rafRef.current = requestAnimationFrame(tick);
      }
    }

    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, durationMs]);

  return value;
}

export default function ModelSlot({
  model,
  tier,
  size = 'md',
  active = false,
  tokens,
  cost,
  latencyMs,
  className,
}: ModelSlotProps) {
  const prevModelRef = useRef<string | null>(null);
  const [spinning, setSpinning] = useState<boolean[]>([]);
  const [displayChars, setDisplayChars] = useState<string[]>(() => model.split(''));
  const [revealed, setRevealed] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const timersRef = useRef<number[]>([]);

  const animatedTokens = useCountUp(tokens ?? 0, 800);

  useEffect(() => {
    // First render -- typewriter reveal.
    if (prevModelRef.current === null) {
      prevModelRef.current = model;
      // Typewriter: reveal chars one at a time
      const chars = model.split('');
      setDisplayChars([]);
      setRevealed(false);

      const revealTimers: number[] = [];
      for (let i = 0; i < chars.length; i++) {
        const timer = window.setTimeout(() => {
          setDisplayChars(prev => [...prev, chars[i]]);
          if (i === chars.length - 1) {
            setRevealed(true);
          }
        }, 30 * i);
        revealTimers.push(timer);
      }
      timersRef.current = revealTimers;
      return () => {
        revealTimers.forEach(t => window.clearTimeout(t));
      };
    }

    // Same model -- nothing to do.
    if (prevModelRef.current === model) return;

    const prev = prevModelRef.current;
    prevModelRef.current = model;
    setRevealed(false);

    const newChars = model.split('');
    const maxLen = Math.max(prev.length, newChars.length);

    // Clear any pending timers from a previous transition.
    timersRef.current.forEach(t => window.clearTimeout(t));
    timersRef.current = [];

    // Start all slots spinning with random placeholder chars.
    const spinState = Array.from({ length: maxLen }, () => true);
    const placeholders = Array.from({ length: maxLen }, () => randomChar());
    setSpinning(spinState);
    setDisplayChars(placeholders);

    // Stagger each character settling to its final value.
    const nextTimers: number[] = [];
    for (let i = 0; i < maxLen; i++) {
      const delay = CHAR_STAGGER_MS * i;
      const timer = window.setTimeout(() => {
        setDisplayChars(prev => {
          const next = [...prev];
          next[i] = newChars[i] ?? '';
          return next;
        });
        setSpinning(prev => {
          const next = [...prev];
          next[i] = false;
          return next;
        });
        if (i === maxLen - 1) {
          setRevealed(true);
        }
      }, 300 + delay);
      nextTimers.push(timer);
    }

    timersRef.current = nextTimers;

    return () => {
      timersRef.current.forEach(t => window.clearTimeout(t));
    };
  }, [model]);

  const tierKey = tier.toLowerCase() as 't0' | 't1' | 't2';
  const hasMeta = tokens !== undefined || cost !== undefined || latencyMs !== undefined;

  return (
    <span
      className={[
        'model-slot',
        `model-slot--${size}`,
        `model-slot--${tierKey}`,
        active && 'model-slot--active',
        expanded && 'model-slot--expanded',
        className,
      ]
        .filter(Boolean)
        .join(' ')}
      onMouseEnter={() => hasMeta && setExpanded(true)}
      onMouseLeave={() => setExpanded(false)}
    >
      {/* Status indicator */}
      <span className={`model-slot__status${active ? ' model-slot__status--spinning' : ''}${revealed && !active ? ' model-slot__status--done' : ''}`}>
        {active ? (
          <svg width="10" height="10" viewBox="0 0 10 10" className="model-slot__spinner">
            <circle cx="5" cy="5" r="3.5" fill="none" stroke="currentColor" strokeWidth="1.5"
              strokeDasharray="16" strokeDashoffset="4" strokeLinecap="round" />
          </svg>
        ) : revealed ? (
          <svg width="10" height="10" viewBox="0 0 10 10" className="model-slot__check">
            <path d="M2.5 5.5 L4 7 L7.5 3" fill="none" stroke="currentColor" strokeWidth="1.5"
              strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        ) : null}
      </span>

      {/* Character reels */}
      <span className="model-slot__reels">
        {displayChars.map((ch, i) => (
          <span key={i} className={`slot-reel slot-reel--${size}`}>
            <span
              className={`slot-reel__inner${spinning[i] ? ' slot-reel__inner--spinning' : ''}`}
              style={spinning[i] ? { animationDelay: `${i * CHAR_STAGGER_MS}ms` } : undefined}
            >
              <span aria-hidden="true">{randomChar()}</span>
              <span>{ch}</span>
            </span>
          </span>
        ))}
      </span>

      {/* Expandable meta badges */}
      {hasMeta && (
        <span className="model-slot__meta">
          {tokens !== undefined && (
            <span className="model-slot__badge model-slot__badge--tokens">
              {animatedTokens.toLocaleString()} tok
            </span>
          )}
          {cost !== undefined && (
            <span className="model-slot__badge model-slot__badge--cost">
              {cost}
            </span>
          )}
          {latencyMs !== undefined && (
            <span
              className="model-slot__badge model-slot__badge--latency"
              style={{ color: latencyColor(latencyMs) }}
            >
              {latencyMs < 1000 ? `${latencyMs}ms` : `${(latencyMs / 1000).toFixed(1)}s`}
            </span>
          )}
        </span>
      )}
    </span>
  );
}
