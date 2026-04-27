/** Resolve the roko serve base URL and WebSocket base. */
function resolveServeUrl(): string {
  if (typeof window === 'undefined') return 'http://localhost:6677';
  const { protocol, port, origin } = window.location;
  if (protocol === 'file:') return 'http://localhost:6677';
  if (port === '6677' || port === '5173') return origin.replace(`:${port}`, ':6677');
  return origin;
}

export const SERVE_URL = resolveServeUrl();
export const WS_BASE = SERVE_URL.replace(/^http/, 'ws');
