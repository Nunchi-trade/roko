// ROSEDUST motion tokens — typed animation presets for Motion

// ── Spring configs ────────────────────────────────────────────────────────────

export const springs = {
  snappy: { stiffness: 400, damping: 30, mass: 0.8 },
  smooth: { stiffness: 200, damping: 28, mass: 1 },
  bouncy: { stiffness: 300, damping: 18, mass: 0.9 },
  stiff:  { stiffness: 600, damping: 40, mass: 0.7 },
} as const;

// ── Duration constants ────────────────────────────────────────────────────────

export const durations = {
  instant: 0.08,
  fast:    0.15,
  normal:  0.22,
  slow:    0.35,
  reveal:  0.6,
} as const;

// ── Easing curves ─────────────────────────────────────────────────────────────

export const easings = {
  ease: [0.22, 1, 0.36, 1],
  expo: [0.16, 1, 0.3,  1],
  out:  [0,    0, 0.2,  1],
} as const satisfies Record<string, [number, number, number, number]>;

// ── Stagger delays ────────────────────────────────────────────────────────────

export const staggers = {
  tight:   0.03,
  normal:  0.05,
  relaxed: 0.08,
} as const;

// ── Preset variants ───────────────────────────────────────────────────────────

export const fadeUp = {
  initial: { opacity: 0, y: 12 },
  animate: { opacity: 1, y: 0,    transition: { type: 'spring', ...springs.smooth } },
  exit:    { opacity: 0, y: 12,   transition: { duration: durations.fast } },
} as const;

export const fadeIn = {
  initial: { opacity: 0 },
  animate: { opacity: 1, transition: { duration: durations.normal } },
  exit:    { opacity: 0, transition: { duration: durations.fast } },
} as const;

export const scaleIn = {
  initial: { opacity: 0, scale: 0.95 },
  animate: { opacity: 1, scale: 1,    transition: { type: 'spring', ...springs.snappy } },
  exit:    { opacity: 0, scale: 0.95, transition: { duration: durations.fast } },
} as const;

export const slideRight = {
  initial: { opacity: 0, x: -20 },
  animate: { opacity: 1, x: 0,   transition: { type: 'spring', ...springs.smooth } },
  exit:    { opacity: 0, x: -20, transition: { duration: durations.fast } },
} as const;

export const slideDown = {
  initial: { opacity: 0, y: -12 },
  animate: { opacity: 1, y: 0,   transition: { type: 'spring', ...springs.snappy } },
  exit:    { opacity: 0, y: -12, transition: { duration: durations.fast } },
} as const;
