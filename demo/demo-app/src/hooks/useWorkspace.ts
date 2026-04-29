/**
 * Server-side workspace management hook.
 *
 * Instead of creating workspaces via slow PTY commands (mkdir, roko init, curl config),
 * this hook calls `POST /api/workspaces` which creates them server-side instantly.
 */
import { createContext, useContext, useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { createElement } from 'react';
import { SERVE_URL } from '../lib/serve-url';

// ── Types ────────────────────────────────────────────────────

export interface WorkspaceInfo {
  id: string;
  path: string;
  ready: boolean;
}

interface WorkspaceContextValue {
  /** The server's default workdir (fetched from GET /api/workspaces/default). */
  serverWorkdir: string | null;
  /** Ensure a workspace exists for the given prefix. Cached per session. */
  ensureWorkspace: (prefix: string, opts?: { gitInit?: boolean }) => Promise<WorkspaceInfo>;
  /** Create a new workspace (always creates, no caching). */
  createWorkspace: (prefix: string, opts?: { gitInit?: boolean }) => Promise<WorkspaceInfo>;
  /** Destroy a previously created workspace by id. */
  destroyWorkspace: (id: string) => Promise<void>;
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null);

// ── Provider ─────────────────────────────────────────────────

export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const [serverWorkdir, setServerWorkdir] = useState<string | null>(null);
  const cacheRef = useRef<Map<string, WorkspaceInfo>>(new Map());

  // Fetch server's default workdir on mount
  useEffect(() => {
    let cancelled = false;
    fetch(`${SERVE_URL}/api/workspaces/default`)
      .then((res) => (res.ok ? res.json() : null))
      .then((data) => {
        if (!cancelled && data?.path) setServerWorkdir(data.path);
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, []);

  const createWorkspace = useCallback(
    async (prefix: string, opts?: { gitInit?: boolean }): Promise<WorkspaceInfo> => {
      const res = await fetch(`${SERVE_URL}/api/workspaces`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ prefix, git_init: opts?.gitInit ?? true }),
      });
      if (!res.ok) {
        const text = await res.text().catch(() => 'unknown error');
        throw new Error(`Failed to create workspace: ${res.status} ${text}`);
      }
      return (await res.json()) as WorkspaceInfo;
    },
    [],
  );

  const ensureWorkspace = useCallback(
    async (prefix: string, opts?: { gitInit?: boolean }): Promise<WorkspaceInfo> => {
      const cached = cacheRef.current.get(prefix);
      if (cached) return cached;
      const ws = await createWorkspace(prefix, opts);
      cacheRef.current.set(prefix, ws);
      return ws;
    },
    [createWorkspace],
  );

  // TODO(T5.9): Server-side workspace GC — the roko-serve backend should run a
  // background TTL sweep (e.g. 1-hour TTL, 5-min interval) to remove stale
  // /tmp/roko-ws-* directories. This is out of scope for the frontend; the
  // destroyWorkspace call below handles explicit user-initiated cleanup.
  // See: crates/roko-serve/src/state.rs for the server startup location.

  const destroyWorkspace = useCallback(async (id: string) => {
    await fetch(`${SERVE_URL}/api/workspaces/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    });
    // Remove from cache if present
    for (const [key, ws] of cacheRef.current.entries()) {
      if (ws.id === id) {
        cacheRef.current.delete(key);
        break;
      }
    }
  }, []);

  const value: WorkspaceContextValue = {
    serverWorkdir,
    ensureWorkspace,
    createWorkspace,
    destroyWorkspace,
  };

  return createElement(WorkspaceContext.Provider, { value }, children);
}

// ── Hook ─────────────────────────────────────────────────────

export function useWorkspace(): WorkspaceContextValue {
  const ctx = useContext(WorkspaceContext);
  if (!ctx) {
    throw new Error('useWorkspace must be used within a WorkspaceProvider');
  }
  return ctx;
}
