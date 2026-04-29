/** Abbreviate a model ID to its first two dash-segments. */
export function shortModel(id: string): string {
  return id.split('-').slice(0, 2).join('-');
}

/** Format a duration in seconds as a compact uptime string (`3h 12m`, `45m`, `12s`). */
export function fmtUptime(secs: number): string {
  if (secs < 60) return `${Math.floor(secs)}s`;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h === 0) return `${m}m`;
  return `${h}h ${m}m`;
}

/** Format a Unix-ms timestamp as a relative age string (`12s ago`, `5m ago`, `3h ago`). */
export function relativeTime(ms: number): string {
  const delta = Math.max(0, Date.now() - ms);
  if (delta < 60_000) return `${Math.floor(delta / 1000)}s ago`;
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
  return new Date(ms).toLocaleDateString();
}
