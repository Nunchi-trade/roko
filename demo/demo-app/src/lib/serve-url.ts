/** Resolve the roko serve base URL and WebSocket base. */
function resolveServeUrl(): string {
  if (typeof window === 'undefined') return 'http://localhost:6677';
  const { protocol, hostname, port } = window.location;
  if (protocol === 'file:') return 'http://localhost:6677';
  // When served from roko-serve itself (:6677), API is same-origin.
  if (port === '6677') return '';
  // Vite dev server or other dev port → talk directly to roko-serve.
  return `http://${hostname}:6677`;
}

export const SERVE_URL = resolveServeUrl();
export const WS_BASE = SERVE_URL.replace(/^http/, 'ws');
