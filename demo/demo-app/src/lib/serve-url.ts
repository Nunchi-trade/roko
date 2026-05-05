/** Resolve browser-facing roko serve URLs. */
function viteEnv(name: string): string | undefined {
  if (typeof import.meta === 'undefined') return undefined;
  const value = (import.meta as any).env?.[name];
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

function hostWithPort(hostname: string, port: string): string {
  return hostname.includes(':') ? `[${hostname}]:${port}` : `${hostname}:${port}`;
}

function isLocalDevOrigin(hostname: string, port: string): boolean {
  const local =
    hostname === 'localhost' ||
    hostname === '127.0.0.1' ||
    hostname === '::1';
  return local && port !== '' && port !== '6677';
}

function resolveServeUrl(preferSameOrigin = true): string {
  const env = viteEnv('VITE_ROKO_SERVE_URL');
  if (env) return env.replace(/\/$/, '');

  if (typeof window === 'undefined') return 'http://localhost:6677';

  const { protocol, hostname, port, origin } = window.location;
  if (protocol === 'file:') return 'http://localhost:6677';

  // Vite dev server (for example :5173) talks to local roko serve directly.
  if (isLocalDevOrigin(hostname, port)) {
    return `http://${hostWithPort(hostname, '6677')}`;
  }

  // In Docker/Railway, the API and frontend are served by the same origin.
  return preferSameOrigin ? '' : origin;
}

function resolveWsBase(): string {
  const env = viteEnv('VITE_ROKO_WS_BASE');
  if (env) return env.replace(/\/$/, '');

  const httpBase = resolveServeUrl(false);
  return httpBase.replace(/^http:/, 'ws:').replace(/^https:/, 'wss:');
}

export const SERVE_URL = resolveServeUrl();
export const ABSOLUTE_SERVE_URL = resolveServeUrl(false);
export const WS_BASE = resolveWsBase();

/**
 * Direct Mirage websocket endpoint for local development.
 *
 * Railway exposes only roko serve publicly; mirage-rs stays loopback-only in
 * the same container. On HTTPS origins this intentionally returns null so the
 * UI uses roko-serve HTTP projections instead of attempting mixed-content
 * browser connections to :8545.
 */
function resolveMirageWs(): string | null {
  const env = viteEnv('VITE_MIRAGE_WS_URL');
  if (env) return env;
  if (typeof window === 'undefined') return 'ws://localhost:8545';

  const { protocol, hostname } = window.location;
  if (protocol === 'https:') {
    // On HTTPS origins, proxy through roko-serve instead of direct loopback.
    return `${resolveWsBase()}/api/rpc`;
  }
  return `ws://${hostWithPort(hostname, '8545')}`;
}

export const MIRAGE_WS_URL = resolveMirageWs();

function resolveMirageEventsWs(): string | null {
  if (MIRAGE_WS_URL === null) return null;

  if (typeof window !== 'undefined' && window.location.protocol === 'https:') {
    return `${resolveWsBase()}/api/rpc/events?insights=true&pheromones=true&agents=true`;
  }

  return `${MIRAGE_WS_URL.replace(/\/$/, '')}/api/ws?insights=true&pheromones=true&agents=true`;
}

export const MIRAGE_EVENTS_WS_URL = resolveMirageEventsWs();

/** Canonical timeout values (ms) used across all hooks and session logic. */
export const TIMEOUTS = {
  health:      parseInt(viteEnv('VITE_TIMEOUT_HEALTH')    ?? '5000',  10),
  workspace:   parseInt(viteEnv('VITE_TIMEOUT_WORKSPACE') ?? '10000', 10),
  command:     parseInt(viteEnv('VITE_TIMEOUT_COMMAND')    ?? '180000',10),
  terminal:    parseInt(viteEnv('VITE_TIMEOUT_TERMINAL')   ?? '8000',  10),
  shellPrompt: parseInt(viteEnv('VITE_TIMEOUT_PROMPT')     ?? '8000',  10),
} as const;

/** WebSocket reconnect backoff parameters used by useTerminal and WS transports. */
export const RECONNECT_BACKOFF = {
  baseMs:      parseInt(viteEnv('VITE_RECONNECT_BASE_MS')      ?? '500',   10),
  maxMs:       parseInt(viteEnv('VITE_RECONNECT_MAX_MS')       ?? '30000', 10),
  factor:      parseFloat(viteEnv('VITE_RECONNECT_FACTOR')     ?? '2'),
  maxAttempts: parseInt(viteEnv('VITE_RECONNECT_MAX_ATTEMPTS') ?? '20',    10),
} as const;
