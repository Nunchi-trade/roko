import { useState, useEffect, useRef } from 'react';

/**
 * Animates a numeric value from its previous value to a new target
 * using requestAnimationFrame with easeOutExpo easing.
 *
 * @param target  The target number to animate towards.
 * @param duration  Animation duration in ms (default 900).
 * @param enabled  When false, immediately returns target without animating (default true).
 * @returns The current animated value.
 */
export function useCountUp(target: number, duration = 900, enabled = true): number {
  const [val, setVal] = useState(0);
  const prevTarget = useRef<number | null>(null);
  const valRef = useRef(0);

  useEffect(() => {
    if (!enabled) {
      valRef.current = target;
      setVal(target);
      prevTarget.current = target;
      return;
    }
    if (prevTarget.current === target) return;
    prevTarget.current = target;
    const start = performance.now();
    const from = valRef.current;
    let frame = 0;

    const tick = (now: number) => {
      const t = Math.min((now - start) / duration, 1);
      // easeOutExpo: fast start, slow end
      const eased = t === 1 ? 1 : 1 - Math.pow(2, -10 * t);
      const next = from + (target - from) * eased;
      valRef.current = next;
      setVal(next);
      if (t < 1) {
        frame = requestAnimationFrame(tick);
      } else {
        valRef.current = target;
        setVal(target);
      }
    };

    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [target, duration, enabled]);

  return val;
}

/** Format an animated number with commas for thousands. */
export function fmtCount(n: number, decimals = 0): string {
  const fixed = n.toFixed(decimals);
  const [whole, frac] = fixed.split('.');
  const withCommas = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return frac !== undefined ? `${withCommas}.${frac}` : withCommas;
}

/** Format an animated cost value (2 decimal places, dollar prefix). */
export function fmtCost(n: number): string {
  return `$${fmtCount(n, 2)}`;
}
