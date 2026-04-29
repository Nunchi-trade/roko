/* ===================================================================
   Motion Tokens
   Animation constants and preset variants for the motion system.
   Pure TypeScript — no component, no dependencies.
   =================================================================== */

/** Spring configs (stiffness + damping pairs) */
export const springs = {
  gentle: { stiffness: 120, damping: 14 },
  snappy: { stiffness: 300, damping: 30 },
  bouncy: { stiffness: 400, damping: 28 },
} as const;

/** Duration constants (ms) — match rosedust.css timing tokens */
export const durations = {
  instant: 80,
  fast: 150,
  normal: 220,
  slow: 350,
} as const;

/** Stagger delay per item (ms) */
export const STAGGER_MS = 40;

/** CSS easing functions — match rosedust.css --ease variants */
export const easings = {
  snappy: 'cubic-bezier(0.2, 0.8, 0.2, 1)',
  expo: 'cubic-bezier(0.16, 1, 0.3, 1)',
  out: 'cubic-bezier(0, 0, 0.2, 1)',
} as const;

/** Preset animation variants (initial / animate / exit states) */
export const variants = {
  fadeUp: {
    initial: { opacity: 0, y: 12 },
    animate: { opacity: 1, y: 0 },
    exit: { opacity: 0, y: -8 },
  },
  scaleIn: {
    initial: { opacity: 0, scale: 0.9 },
    animate: { opacity: 1, scale: 1 },
    exit: { opacity: 0, scale: 0.95 },
  },
  slideRight: {
    initial: { opacity: 0, x: -20 },
    animate: { opacity: 1, x: 0 },
    exit: { opacity: 0, x: 20 },
  },
} as const;
