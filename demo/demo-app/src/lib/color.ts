/** Convert hex color to rgba string. */
export function hexToRgba(hex: string, alpha: number): string {
  const trimmed = hex.trim();
  if (!trimmed.startsWith('#')) return trimmed;

  let normalized = trimmed.slice(1);

  // Expand shorthand (#abc -> aabbcc)
  if (normalized.length === 3) {
    normalized = normalized
      .split('')
      .map((c) => `${c}${c}`)
      .join('');
  }

  if (!/^[0-9a-fA-F]{6}$/.test(normalized)) return hex;

  const r = Number.parseInt(normalized.slice(0, 2), 16);
  const g = Number.parseInt(normalized.slice(2, 4), 16);
  const b = Number.parseInt(normalized.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Read a CSS custom property value from :root. */
export function getCssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}
