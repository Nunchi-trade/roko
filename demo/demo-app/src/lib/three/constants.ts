/**
 * Shared Three.js constants — ROSEDUST palette as hex numbers + mapping helpers.
 */

export const TAU = Math.PI * 2;

/* ── ROSEDUST palette (Three.js hex) ─────────────────────────── */
export const COL = {
  roseGlow:    0xdca5bd,
  roseBright:  0xcc90a8,
  roseDim:     0x7a5060,
  rose:        0xaa7088,
  roseDeep:    0x3a2030,
  bone:        0xc8b890,
  boneBright:  0xd8c8a0,
  core:        0xe8b5ce,
  dim:         0x443844,
  faint:       0x2a1e28,
  dream:       0x7a7a98,
  dreamBright: 0x9494b4,
  success:     0x7a8a78,
  warning:     0xc89a68,
  teal:        0x56b6c2,
  tealBright:  0x2dd4bf,
  voidBg:      0x060608,
} as const;

/* ── ISFR class → color ─────────────────────────────────────── */
export const CLASS_HEX: Record<string, number> = {
  lending:    COL.teal,
  structured: COL.dreamBright,
  staking:    COL.boneBright,
  funding:    COL.roseBright,
};

/* ── Health status → color ──────────────────────────────────── */
export const HEALTH_HEX: Record<string, number> = {
  live:    COL.success,
  stale:   COL.warning,
  offline: COL.roseBright,
};

/* ── Feed kind → color ──────────────────────────────────────── */
export const FEED_KIND_HEX: Record<string, number> = {
  raw:       COL.teal,
  derived:   COL.roseBright,
  composite: COL.bone,
  meta:      COL.dream,
};
