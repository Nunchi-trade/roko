/** Deterministic color from agent name. */
const PALETTE = ['#d89ab2', '#a4a4c8', '#8a9c86', '#d8a878', '#d4c89c', '#b87a94'];

export function agentColor(name: string): string {
  let hash = 0;
  for (let i = 0; i < name.length; i++) hash = (hash * 31 + name.charCodeAt(i)) | 0;
  return PALETTE[Math.abs(hash) % PALETTE.length];
}
