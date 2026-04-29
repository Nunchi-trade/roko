import { useEffect, useRef, useState } from 'react';
import './ModelSlot.css';

interface ModelSlotProps {
  model: string;
  tier: 'T0' | 'T1' | 'T2';
  size?: 'sm' | 'md' | 'lg';
  className?: string;
}

/** Characters cycled through during the spin animation. */
const SPIN_CHARS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';

function randomChar(): string {
  return SPIN_CHARS[Math.floor(Math.random() * SPIN_CHARS.length)];
}

/** Per-character delay in ms for staggered animation. */
const CHAR_STAGGER_MS = 40;

export default function ModelSlot({
  model,
  tier,
  size = 'md',
  className,
}: ModelSlotProps) {
  const prevModelRef = useRef<string | null>(null);
  const [spinning, setSpinning] = useState<boolean[]>([]);
  const [displayChars, setDisplayChars] = useState<string[]>(() => model.split(''));
  const timersRef = useRef<number[]>([]);

  useEffect(() => {
    // First render -- just display, no animation.
    if (prevModelRef.current === null) {
      prevModelRef.current = model;
      setDisplayChars(model.split(''));
      return;
    }

    // Same model -- nothing to do.
    if (prevModelRef.current === model) return;

    const prev = prevModelRef.current;
    prevModelRef.current = model;

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
      }, 300 + delay); // base animation duration + stagger
      nextTimers.push(timer);
    }

    timersRef.current = nextTimers;

    return () => {
      timersRef.current.forEach(t => window.clearTimeout(t));
    };
  }, [model]);

  const tierKey = tier.toLowerCase() as 't0' | 't1' | 't2';

  return (
    <span
      className={[
        'model-slot',
        `model-slot--${size}`,
        `model-slot--${tierKey}`,
        className,
      ]
        .filter(Boolean)
        .join(' ')}
    >
      {displayChars.map((ch, i) => (
        <span key={i} className={`slot-reel slot-reel--${size}`}>
          <span
            className={`slot-reel__inner${spinning[i] ? ' slot-reel__inner--spinning' : ''}`}
            style={spinning[i] ? { animationDelay: `${i * CHAR_STAGGER_MS}ms` } : undefined}
          >
            {/* Old char (scrolls up out of view) */}
            <span aria-hidden="true">{randomChar()}</span>
            {/* New char (settles into view) */}
            <span>{ch}</span>
          </span>
        </span>
      ))}
    </span>
  );
}
