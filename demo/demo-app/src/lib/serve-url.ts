/** Resolve the roko serve base URL and WebSocket base. */
function resolveServeUrl(preferSameOrigin = true): string {
  if (typeof window === 'undefined') return 'http://localhost:6677';
  const { protocol, hostname, port } = window.location;
  if (protocol === 'file:') return 'http://localhost:6677';
  // When served from roko-serve itself (:6677), API is same-origin.
  if (port === '6677' && preferSameOrigin) return '';
  // Vite dev server or other dev port → talk directly to roko-serve.
  return `http://${hostname}:6677`;
}

export const SERVE_URL = resolveServeUrl();
export const ABSOLUTE_SERVE_URL = resolveServeUrl(false);
export const WS_BASE = resolveServeUrl(false).replace(/^http/, 'ws');

/** Mirage-rs WS endpoint for eth_subscribe. Configurable via VITE_MIRAGE_WS_URL. */
function resolveMirageWs(): string {
  const env = typeof import.meta !== 'undefined' ? (import.meta as any).env?.VITE_MIRAGE_WS_URL : undefined;
  if (env) return env;
  if (typeof window === 'undefined') return 'ws://localhost:8545';
  const { hostname } = window.location;
  return `ws://${hostname}:8545`;
}

export const MIRAGE_WS_URL = resolveMirageWs();
