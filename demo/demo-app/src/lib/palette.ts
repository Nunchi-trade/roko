/** Domain color map used across knowledge, dream, and graph views. */
export const DOMAIN_COLORS: Record<string, string> = {
  gate:      '#CC90A8',
  agent:     '#C8B890',
  knowledge: '#9494B4',
  plan:      '#7A8A78',
  config:    '#C89A68',
};

/** Resolve a domain string to its palette color, with a neutral fallback. */
export function domainColor(domain?: string): string {
  return DOMAIN_COLORS[domain ?? ''] ?? '#706070';
}

/** Role color map used in fleet topology and cascade-router views. */
export const ROLE_COLORS: Record<string, string> = {
  implementer: '#C8B890',
  researcher:  '#9A8AB8',
  reviewer:    '#8A9C86',
  planner:     '#D8A878',
  auditor:     '#AA7088',
  executor:    '#CC90A8',
  composer:    '#7A8AA8',
};

/** Resolve a role string to its palette color, matching by substring. */
export function roleColor(role: string): string {
  const key = Object.keys(ROLE_COLORS).find((k) => role.toLowerCase().includes(k));
  return ROLE_COLORS[key ?? ''] ?? '#706070';
}

/** Model color map used in cost-race and bench views. */
export const MODEL_COLORS: Record<string, string> = {
  'claude-sonnet-4':  '#C8B890',
  'claude-haiku-3':   '#8A9C86',
  'claude-opus-4':    '#AA7088',
  'gpt-5.4':          '#D8A878',
  'gpt-5.4-mini':     '#D8C098',
  'gemini-2.5-pro':   '#9A8AB8',
};
