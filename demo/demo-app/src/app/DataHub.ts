/**
 * DataHub — centralised Zustand store for the demo-app.
 *
 * Replaces:
 *   - contexts/EventStreamContext.tsx (event dispatch)
 *   - hooks/useRokoConfig.ts (config state + polling)
 *   - hooks/useLiveApi.ts (health tracking)
 *   - hooks/useServerHealth.ts (status polling)
 *   - hooks/useApiWithFallback.ts (offline detection)
 *   - hooks/useWorkspace.ts (workspace context)
 *
 * Implementation tasks: T1.9 (core store) + T1.10 (workspace slice).
 */

import { create } from 'zustand';
import type { ServerEvent } from '../transport/types';
import { api } from '../transport/api';
import type { BenchRun, BenchSuite, BenchModel } from '../lib/bench-types';

// ── Public types ────────────────────────────────────────────────

export type ServerStatus = 'connected' | 'checking' | 'disconnected';
export type StreamStatus =
  | 'idle'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'failed';

export interface WorkspaceInfo {
  id: string;
  path: string;
  ready: boolean;
}

export interface AgentInfo {
  agentId: string;
  role: string;
  model: string;
  status: 'running' | 'stopped';
}

export interface EpisodeInfo {
  planId: string;
  taskId: string;
  passed: boolean;
  timestamp: number;
}

export interface InferenceRecord {
  requestId: string;
  model: string;
  agentId: string;
  inputTokens: number;
  outputTokens: number;
  costUsd: number;
  durationMs: number;
}

// ── ISFR types ──────────────────────────────────────────────────

export interface IsfrRate {
  compositeBps: number;
  lendingBps: number;
  structuredBps: number;
  fundingBps: number;
  stakingBps: number;
  confidenceBps: number;
  sourceCount: number;
  timestampMs: number;
}

export interface IsfrSource {
  id: string;
  health: 'live' | 'stale' | 'offline';
  lastRateBps: number | null;
}

export type IsfrKeeperStatus = 'unknown' | 'running' | 'stopped';

// ── Store interface ─────────────────────────────────────────────

export interface DataHub {
  // -- Connection status -------------------------------------------
  serverStatus: ServerStatus;
  sseStatus: StreamStatus;
  wsStatus: StreamStatus;

  // -- Config slice ------------------------------------------------
  config: Record<string, unknown> | null;
  defaultModel: string;
  defaultBackend: string;

  // -- Workspace slice ---------------------------------------------
  serverWorkdir: string | null;
  workspace: WorkspaceInfo | null;
  workspaceCache: Map<string, WorkspaceInfo>;

  // -- Plan execution slice ----------------------------------------
  activePlanId: string | null;
  activePhase: string | null;
  planCompleted: boolean;

  // -- Agent slice -------------------------------------------------
  agents: AgentInfo[];

  // -- Episode / metrics slice -------------------------------------
  episodes: EpisodeInfo[];
  totalCost: number;
  totalTokens: number;
  recentInferences: InferenceRecord[]; // ring buffer, max 200

  // -- Bench slice -------------------------------------------------
  benchRuns: BenchRun[];
  benchSuites: BenchSuite[];
  benchModels: BenchModel[];

  // -- ISFR slice -------------------------------------------------
  isfrCurrentRate: IsfrRate | null;
  isfrHistory: IsfrRate[];       // ring buffer, max 256
  isfrSources: IsfrSource[];
  isfrKeeperStatus: IsfrKeeperStatus;

  // -- Actions: event handling -------------------------------------
  handleServerEvent: (event: ServerEvent) => void;
  setServerStatus: (status: ServerStatus) => void;
  setSseStatus: (status: StreamStatus) => void;
  setWsStatus: (status: StreamStatus) => void;

  // -- Actions: REST fetches ---------------------------------------
  fetchConfig: () => Promise<void>;
  updateConfig: (partial: Record<string, unknown>) => Promise<boolean>;
  fetchBenchRuns: () => Promise<void>;
  fetchBenchSuites: () => Promise<void>;
  fetchBenchModels: () => Promise<void>;
  fetchAgents: () => Promise<void>;
  fetchServerWorkdir: () => Promise<void>;
  ensureWorkspace: (
    prefix: string,
    opts?: { gitInit?: boolean },
  ) => Promise<WorkspaceInfo>;
  destroyWorkspace: (id: string) => Promise<void>;

  // -- Actions: ISFR REST fetches ---------------------------------
  fetchIsfrStatus: () => Promise<void>;
  fetchIsfrCurrent: () => Promise<void>;
  fetchIsfrHistory: (limit?: number) => Promise<void>;
  fetchIsfrSources: () => Promise<void>;
}

// ── Ring-buffer limits ──────────────────────────────────────────

const MAX_EPISODES = 500;
const MAX_INFERENCES = 200;
const MAX_ISFR_HISTORY = 256;

// ── Store implementation ────────────────────────────────────────

export const useDataHub = create<DataHub>()((set, get) => ({
  // -- Initial state -----------------------------------------------
  serverStatus: 'checking',
  sseStatus: 'idle',
  wsStatus: 'idle',
  config: null,
  defaultModel: '',
  defaultBackend: '',
  serverWorkdir: null,
  workspace: null,
  workspaceCache: new Map(),
  activePlanId: null,
  activePhase: null,
  planCompleted: false,
  agents: [],
  episodes: [],
  totalCost: 0,
  totalTokens: 0,
  recentInferences: [],
  benchRuns: [],
  benchSuites: [],
  benchModels: [],
  isfrCurrentRate: null,
  isfrHistory: [],
  isfrSources: [],
  isfrKeeperStatus: 'unknown',

  // -- Event handling -----------------------------------------------

  handleServerEvent: (event: ServerEvent) => {
    switch (event.type) {
      case 'plan_started':
        set({
          activePlanId: event.planId,
          activePhase: 'started',
          planCompleted: false,
        });
        break;

      case 'plan_completed':
        set({ planCompleted: true, activePhase: 'completed' });
        break;

      case 'phase_transition':
        set({ activePhase: event.to });
        break;

      case 'agent_spawned':
        set((s) => ({
          agents: [
            ...s.agents,
            {
              agentId: event.agentId,
              role: event.role,
              model: event.model,
              status: 'running' as const,
            },
          ],
        }));
        break;

      case 'agent_stopped':
        set((s) => ({
          agents: s.agents.map((a) =>
            a.agentId === event.agentId
              ? { ...a, status: 'stopped' as const }
              : a,
          ),
        }));
        break;

      case 'episode':
        set((s) => ({
          episodes: [
            ...s.episodes.slice(-(MAX_EPISODES - 1)),
            {
              planId: event.planId,
              taskId: event.taskId,
              passed: event.passed,
              timestamp: Date.now(),
            },
          ],
        }));
        break;

      case 'inference_completed':
        set((s) => ({
          totalCost: s.totalCost + event.costUsd,
          totalTokens:
            s.totalTokens + event.inputTokens + event.outputTokens,
          recentInferences: [
            ...s.recentInferences.slice(-(MAX_INFERENCES - 1)),
            {
              requestId: event.requestId,
              model: event.model,
              agentId: event.agentId,
              inputTokens: event.inputTokens,
              outputTokens: event.outputTokens,
              costUsd: event.costUsd,
              durationMs: event.durationMs,
            },
          ],
        }));
        break;

      case 'gate_result':
        // Consumed by components via raw event subscriptions; no store update.
        break;

      case 'config_reloaded':
        get().fetchConfig();
        break;

      case 'BenchRunCompleted':
        get().fetchBenchRuns();
        break;

      case 'isfr_rate_computed':
        set((s) => ({
          isfrCurrentRate: {
            compositeBps: event.compositeBps,
            lendingBps: event.lendingBps,
            structuredBps: event.structuredBps,
            fundingBps: event.fundingBps,
            stakingBps: event.stakingBps,
            confidenceBps: event.confidenceBps,
            sourceCount: event.sourceCount,
            timestampMs: event.timestampMs,
          },
          isfrHistory: [
            ...s.isfrHistory.slice(-(MAX_ISFR_HISTORY - 1)),
            {
              compositeBps: event.compositeBps,
              lendingBps: event.lendingBps,
              structuredBps: event.structuredBps,
              fundingBps: event.fundingBps,
              stakingBps: event.stakingBps,
              confidenceBps: event.confidenceBps,
              sourceCount: event.sourceCount,
              timestampMs: event.timestampMs,
            },
          ],
        }));
        break;

      case 'isfr_source_health_changed':
        set((s) => ({
          isfrSources: s.isfrSources.some((src) => src.id === event.sourceId)
            ? s.isfrSources.map((src) =>
                src.id === event.sourceId
                  ? { ...src, health: event.health, lastRateBps: event.lastRateBps }
                  : src,
              )
            : [
                ...s.isfrSources,
                { id: event.sourceId, health: event.health, lastRateBps: event.lastRateBps },
              ],
        }));
        break;

      case 'isfr_keeper_state_changed':
        set({ isfrKeeperStatus: event.running ? 'running' : 'stopped' });
        break;

      case 'server_shutdown':
        set({ serverStatus: 'disconnected' });
        break;

      case 'error':
        console.warn('[DataHub] server error:', event.message);
        break;

      default:
        // Unknown events silently ignored.
        break;
    }
  },

  // -- Status setters -----------------------------------------------

  setServerStatus: (status) => set({ serverStatus: status }),
  setSseStatus: (status) => set({ sseStatus: status }),
  setWsStatus: (status) => set({ wsStatus: status }),

  // -- REST fetch actions -------------------------------------------

  fetchConfig: async () => {
    const res = await api.get<Record<string, unknown>>('/api/config');
    if (res.ok) {
      const cfg = res.data;
      const agent = cfg?.agent as Record<string, string> | undefined;
      const defaultModel = agent?.default_model ?? '';
      const defaultBackend = agent?.default_backend ?? '';
      set({ config: cfg, defaultModel, defaultBackend });
    }
  },

  updateConfig: async (partial) => {
    const res = await api.put<Record<string, unknown>>(
      '/api/config',
      partial,
    );
    if (res.ok) {
      set({ config: res.data });
      return true;
    }
    return false;
  },

  fetchBenchRuns: async () => {
    const res = await api.get<BenchRun[]>('/api/bench/runs');
    if (res.ok) set({ benchRuns: res.data });
  },

  fetchBenchSuites: async () => {
    const res = await api.get<BenchSuite[]>('/api/bench/suites');
    if (res.ok) set({ benchSuites: res.data });
  },

  fetchBenchModels: async () => {
    const res = await api.get<BenchModel[]>('/api/bench/models');
    if (res.ok) set({ benchModels: res.data });
  },

  fetchAgents: async () => {
    const res = await api.get<AgentInfo[]>('/api/managed-agents');
    if (res.ok) set({ agents: res.data });
  },

  // -- Workspace actions (T1.10) -----------------------------------

  fetchServerWorkdir: async () => {
    const res = await api.get<{ path: string }>(
      '/api/workspaces/default',
    );
    if (res.ok) set({ serverWorkdir: res.data.path });
  },

  ensureWorkspace: async (prefix, opts) => {
    const cached = get().workspaceCache.get(prefix);
    if (cached) return cached;

    const res = await api.post<WorkspaceInfo>('/api/workspaces', {
      prefix,
      git_init: opts?.gitInit ?? true,
    });
    if (!res.ok) {
      throw new Error(
        `Failed to create workspace: ${res.error.status} ${res.error.body ?? res.error.statusText}`,
      );
    }
    const ws = res.data;
    set((s) => {
      const next = new Map(s.workspaceCache);
      next.set(prefix, ws);
      return { workspace: ws, workspaceCache: next };
    });
    return ws;
  },

  destroyWorkspace: async (id) => {
    await api.delete(`/api/workspaces/${encodeURIComponent(id)}`);
    set((s) => {
      const next = new Map(s.workspaceCache);
      for (const [key, ws] of next.entries()) {
        if (ws.id === id) {
          next.delete(key);
          break;
        }
      }
      return {
        workspace: s.workspace?.id === id ? null : s.workspace,
        workspaceCache: next,
      };
    });
  },

  // -- ISFR fetch actions -----------------------------------------

  fetchIsfrStatus: async () => {
    try {
      const res = await api.get<{ keeper_running: boolean; sources_count: number;
        current_rate_bps: number | null }>('/api/isfr/status');
      if (res.ok) {
        set({
          isfrKeeperStatus: res.data.keeper_running ? 'running' : 'stopped',
        });
      }
    } catch (err) {
      console.warn('[DataHub] fetchIsfrStatus failed:', err);
    }
  },

  fetchIsfrCurrent: async () => {
    try {
      const res = await api.get<{
        composite_bps: number; lending_bps: number; structured_bps: number;
        funding_bps: number; staking_bps: number; confidence_bps: number;
        source_count: number; timestamp_ms: number;
      }>('/api/isfr/current');
      if (res.ok && 'composite_bps' in res.data) {
        set({
          isfrCurrentRate: {
            compositeBps: res.data.composite_bps,
            lendingBps: res.data.lending_bps,
            structuredBps: res.data.structured_bps,
            fundingBps: res.data.funding_bps,
            stakingBps: res.data.staking_bps,
            confidenceBps: res.data.confidence_bps,
            sourceCount: res.data.source_count,
            timestampMs: res.data.timestamp_ms,
          },
        });
      }
    } catch (err) {
      console.warn('[DataHub] fetchIsfrCurrent failed:', err);
    }
  },

  fetchIsfrHistory: async (limit = 50) => {
    try {
      const res = await api.get<
        | Array<{
            composite_bps: number; lending_bps: number; structured_bps: number;
            funding_bps: number; staking_bps: number; confidence_bps: number;
            source_count: number; timestamp_ms: number;
          }>
        | { rates: Array<{
            composite_bps: number; lending_bps: number; structured_bps: number;
            funding_bps: number; staking_bps: number; confidence_bps: number;
            source_count: number; timestamp_ms: number;
          }>; total: number }
      >(`/api/isfr/history?limit=${limit}`);
      if (res.ok) {
        const data = res.data;
        const arr = Array.isArray(data) ? data : (data?.rates ?? []);
        set({
          isfrHistory: arr.map((r) => ({
            compositeBps: r.composite_bps,
            lendingBps: r.lending_bps,
            structuredBps: r.structured_bps,
            fundingBps: r.funding_bps,
            stakingBps: r.staking_bps,
            confidenceBps: r.confidence_bps,
            sourceCount: r.source_count,
            timestampMs: r.timestamp_ms,
          })),
        });
      }
    } catch (err) {
      console.warn('[DataHub] fetchIsfrHistory failed:', err);
    }
  },

  fetchIsfrSources: async () => {
    try {
      const res = await api.get<
        | Array<{ id: string; health: string; last_rate_bps: number | null }>
        | { sources: Array<{ id: string; health: string; last_rate_bps: number | null }> }
      >('/api/isfr/sources');
      if (res.ok) {
        const data = res.data;
        const arr = Array.isArray(data) ? data : (data?.sources ?? []);
        set({
          isfrSources: arr.map((s) => ({
            id: s.id,
            health: s.health as IsfrSource['health'],
            lastRateBps: s.last_rate_bps,
          })),
        });
      }
    } catch (err) {
      console.warn('[DataHub] fetchIsfrSources failed:', err);
    }
  },
}));
