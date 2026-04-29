/* ═══════════════════════════════════════════════════════════
   motion.ts — Shared animation utilities
   Springs, staggers, easing, keyframe generators, React hooks
   ═══════════════════════════════════════════════════════════ */

import { useCallback, useEffect, useRef, useState } from 'react';

// ── Spring physics helpers ─────────────────────────────────

/** Attempt to approximate a spring with a CSS cubic-bezier. Returns a CSS timing string. */
export function springEase(stiffness: number, damping: number): string {
  // Map spring params to cubic-bezier control points.
  // High stiffness → fast attack; low damping → more overshoot.
  const ratio = damping / (2 * Math.sqrt(stiffness));
  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

  if (ratio >= 1) {
    // Overdamped — smooth ease-out
    const speed = clamp(1 - ratio * 0.3, 0.1, 0.5);
    return `cubic-bezier(${speed.toFixed(3)}, 1, 0.36, 1)`;
  }

  // Underdamped — bouncy
  const overshoot = clamp(1.0 + (1 - ratio) * 0.4, 1.0, 1.3);
  const x1 = clamp(0.5 - ratio * 0.3, 0.05, 0.45);
  return `cubic-bezier(${x1.toFixed(3)}, ${overshoot.toFixed(3)}, 0.36, 1)`;
}

/**
 * JS spring easing function factory.
 * Returns `(t: number) => number` where t ∈ [0, 1].
 */
export function springEaseFn(
  stiffness: number,
  damping: number,
): (t: number) => number {
  const omega = Math.sqrt(stiffness);
  const zeta = damping / (2 * omega);

  return (t: number) => {
    if (zeta >= 1) {
      // Overdamped
      return 1 - Math.exp(-omega * t) * (1 + omega * t * (zeta - 1));
    }
    // Underdamped
    const omegaD = omega * Math.sqrt(1 - zeta * zeta);
    return (
      1 -
      Math.exp(-zeta * omega * t) *
        (Math.cos(omegaD * t) + (zeta * omega / omegaD) * Math.sin(omegaD * t))
    );
  };
}

// ── Stagger calculator ─────────────────────────────────────

/** Calculate stagger delay for item `index` out of `total`. */
export function stagger(index: number, total: number, baseDelay = 60): number {
  if (total <= 1) return 0;
  return index * baseDelay;
}

/** Calculate stagger delay with easing so later items are closer together. */
export function staggerEased(index: number, total: number, baseDelay = 80): number {
  if (total <= 1) return 0;
  const progress = index / (total - 1);
  // Ease-out curve: items bunch up at the end
  const eased = 1 - Math.pow(1 - progress, 2);
  return eased * (total - 1) * baseDelay;
}

// ── Keyframe generators ────────────────────────────────────

type Direction = 'up' | 'down' | 'left' | 'right';

/**
 * Generate a CSS animation string for a fade+slide entrance.
 * Returns a value suitable for `style={{ animation: ... }}`.
 */
export function fadeSlideIn(
  direction: Direction = 'up',
  _distance = 24,
  duration = 600,
  delay = 0,
): string {
  const nameMap: Record<Direction, string> = {
    up: 'fadeSlideUp',
    down: 'fadeSlideDown',
    left: 'fadeSlideLeft',
    right: 'fadeSlideRight',
  };
  const name = nameMap[direction];
  return `${name} ${duration}ms cubic-bezier(0.22, 1, 0.36, 1) ${delay}ms both`;
}

/**
 * Generate a CSS animation string for a scale reveal.
 * Uses the scaleIn keyframe defined in motion.css.
 */
export function scaleReveal(
  _from = 0.92,
  _to = 1,
  duration = 500,
  delay = 0,
): string {
  // We use CSS custom properties for the scale values
  return `motionScaleIn ${duration}ms cubic-bezier(0.22, 1, 0.36, 1) ${delay}ms both`;
}

/**
 * Generate a CSS animation string for a glow pulse.
 * Uses the glowPulse keyframe defined in motion.css.
 */
export function glowPulse(
  _color = 'var(--rose-glow)',
  _intensity = 1,
  duration = 3000,
): string {
  return `motionGlowPulse ${duration}ms ease-in-out infinite`;
}

// ── Motion presets ─────────────────────────────────────────

export interface SpringConfig {
  stiffness: number;
  damping: number;
}

export const MOTION_PRESETS = {
  snappy: { stiffness: 400, damping: 30 } as SpringConfig,
  gentle: { stiffness: 120, damping: 20 } as SpringConfig,
  bouncy: { stiffness: 300, damping: 12 } as SpringConfig,
  dramatic: { stiffness: 200, damping: 8 } as SpringConfig,
} as const;

// ── useSpring hook ─────────────────────────────────────────

/**
 * Animate a numeric value with spring physics using requestAnimationFrame.
 * Returns the current animated value.
 */
export function useSpring(
  target: number,
  config: SpringConfig = MOTION_PRESETS.snappy,
): number {
  const [current, setCurrent] = useState(target);
  const velocityRef = useRef(0);
  const valueRef = useRef(target);
  const targetRef = useRef(target);
  const rafRef = useRef(0);
  const lastTimeRef = useRef(0);

  targetRef.current = target;

  const tick = useCallback(() => {
    const now = performance.now();
    const dt = lastTimeRef.current ? Math.min((now - lastTimeRef.current) / 1000, 0.064) : 0.016;
    lastTimeRef.current = now;

    const { stiffness, damping } = config;
    const displacement = valueRef.current - targetRef.current;
    const springForce = -stiffness * displacement;
    const dampingForce = -damping * velocityRef.current;
    const acceleration = springForce + dampingForce;

    velocityRef.current += acceleration * dt;
    valueRef.current += velocityRef.current * dt;

    // Settle check
    if (
      Math.abs(velocityRef.current) < 0.01 &&
      Math.abs(valueRef.current - targetRef.current) < 0.01
    ) {
      valueRef.current = targetRef.current;
      velocityRef.current = 0;
      setCurrent(targetRef.current);
      rafRef.current = 0;
      return;
    }

    setCurrent(valueRef.current);
    rafRef.current = requestAnimationFrame(tick);
  }, [config]);

  useEffect(() => {
    if (valueRef.current === target && velocityRef.current === 0) {
      // Already settled at target
      return;
    }
    lastTimeRef.current = 0;
    if (!rafRef.current) {
      rafRef.current = requestAnimationFrame(tick);
    }

    return () => {
      if (rafRef.current) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = 0;
      }
    };
  }, [target, tick]);

  return current;
}

// ── useStaggeredReveal hook ────────────────────────────────

/**
 * Returns an array of booleans indicating which items should be visible,
 * with each item revealing after a staggered delay from mount.
 */
export function useStaggeredReveal(count: number, delay = 60): boolean[] {
  const [revealed, setRevealed] = useState<boolean[]>(() =>
    new Array(count).fill(false),
  );
  const timersRef = useRef<ReturnType<typeof setTimeout>[]>([]);

  useEffect(() => {
    // Reset on count change
    setRevealed(new Array(count).fill(false));

    // Clear old timers
    for (const t of timersRef.current) clearTimeout(t);
    timersRef.current = [];

    for (let i = 0; i < count; i++) {
      const t = setTimeout(() => {
        setRevealed(prev => {
          const next = [...prev];
          next[i] = true;
          return next;
        });
      }, i * delay);
      timersRef.current.push(t);
    }

    return () => {
      for (const t of timersRef.current) clearTimeout(t);
      timersRef.current = [];
    };
  }, [count, delay]);

  return revealed;
}
