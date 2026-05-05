import { useEffect, useRef, useState } from 'react';
import './AnimatedNumber.css';

interface AnimatedNumberProps {
  value: number;
  format?: (n: number) => string;
  duration?: number;
  className?: string;
}

/** Cubic ease-out: fast start, slow finish */
function easeOut(t: number): number {
  return 1 - Math.pow(1 - t, 3);
}

export function AnimatedNumber({
  value,
  format = String,
  duration = 500,
  className,
}: AnimatedNumberProps) {
  const prevRef = useRef(value);
  const rafRef = useRef(0);
  const [display, setDisplay] = useState(value);
  const [flash, setFlash] = useState(false);
  const flashTimer = useRef(0);

  useEffect(() => {
    const from = prevRef.current;
    const to = value;

    // No animation needed if value unchanged
    if (from === to) return;

    // Flash effect
    setFlash(true);
    window.clearTimeout(flashTimer.current);
    flashTimer.current = window.setTimeout(() => setFlash(false), 300);

    const start = performance.now();

    function tick(now: number) {
      const elapsed = now - start;
      const progress = Math.min(elapsed / duration, 1);
      const eased = easeOut(progress);
      const current = from + (to - from) * eased;

      setDisplay(current);

      if (progress < 1) {
        rafRef.current = requestAnimationFrame(tick);
      } else {
        setDisplay(to);
        prevRef.current = to;
      }
    }

    cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(rafRef.current);
      window.clearTimeout(flashTimer.current);
    };
  }, [value, duration]);

  const cls = ['animated-number', flash && 'flash', className]
    .filter(Boolean)
    .join(' ');

  return <span className={cls}>{format(display)}</span>;
}
