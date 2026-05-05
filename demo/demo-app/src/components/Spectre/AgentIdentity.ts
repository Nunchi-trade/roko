/* ── Spectre Identity System ── */

export type SpectreArchetype =
  | 'planner'
  | 'executor'
  | 'researcher'
  | 'validator'
  | 'observer'
  | 'orchestrator'
  | 'specialist'
  | 'guardian';

export type AgentRole =
  | 'lead'
  | 'worker'
  | 'reviewer'
  | 'scout'
  | 'sentinel'
  | 'architect'
  | 'default';

export interface AgentIdentity {
  id: string;
  name: string;
  archetype: SpectreArchetype;
  role: AgentRole;
  /** Deterministic seed derived from id hash */
  seed: number;
}

/** 7 role palettes — each is [primary, secondary, accent] */
export const ROLE_PALETTES: Record<AgentRole, [string, string, string]> = {
  lead:     ['#CC90A8', '#E8B5CE', '#D89AB2'],  // rose spectrum
  worker:   ['#C8B890', '#E4D8B0', '#D4C89C'],  // bone spectrum
  reviewer: ['#8A9C86', '#A8B8A4', '#6A8468'],  // sage
  scout:    ['#9A8AB8', '#B4A8D0', '#7A6A9C'],  // dream spectrum
  sentinel: ['#D8A878', '#E8C098', '#B88A60'],  // warning/amber
  architect:['#7FA8A4', '#98C0BC', '#608880'],  // lane-sage
  default:  ['#9A8A98', '#B8A8B4', '#786878'],  // muted rose-grey
};

const ARCHETYPE_MAP: Record<string, SpectreArchetype> = {
  planner:      'planner',
  orchestrator: 'orchestrator',
  executor:     'executor',
  implementer:  'executor',
  researcher:   'researcher',
  validator:    'validator',
  reviewer:     'validator',
  observer:     'observer',
  auditor:      'guardian',
  guardian:     'guardian',
  specialist:   'specialist',
  composer:     'specialist',
};

const ROLE_MAP: Record<string, AgentRole> = {
  lead:      'lead',
  worker:    'worker',
  reviewer:  'reviewer',
  scout:     'scout',
  sentinel:  'sentinel',
  architect: 'architect',
};

/** Simple deterministic hash → unsigned 32-bit seed */
export function hashSeed(id: string): number {
  let h = 0x811c9dc5; // FNV offset basis
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193); // FNV prime
  }
  return h >>> 0;
}

/** Build an AgentIdentity from raw id/name/role strings */
export function identityFromAgent(
  id: string,
  name: string,
  role?: string,
): AgentIdentity {
  const key = (role ?? name).toLowerCase();
  const archetype: SpectreArchetype =
    ARCHETYPE_MAP[key] ??
    Object.keys(ARCHETYPE_MAP).find((k) => key.includes(k))
      ? ARCHETYPE_MAP[Object.keys(ARCHETYPE_MAP).find((k) => key.includes(k))!] ?? 'specialist'
      : 'specialist';
  const agentRole: AgentRole =
    ROLE_MAP[key] ??
    (Object.keys(ROLE_MAP).find((k) => key.includes(k))
      ? ROLE_MAP[Object.keys(ROLE_MAP).find((k) => key.includes(k))!]!
      : 'default');

  return { id, name, archetype, role: agentRole, seed: hashSeed(id) };
}

/** Seeded PRNG (mulberry32) */
export function mulberry32(seed: number): () => number {
  let s = seed;
  return () => {
    s |= 0;
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
