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
  name: string;
  class: string;
  weight: number;
  lastRateBps: number | null;
  health: 'live' | 'stale' | 'offline';
  lastPollMs: number | null;
}

export type IsfrKeeperStatus = 'unknown' | 'running' | 'stopped';

export interface IsfrFieldHistory {
  composite: number[];
  lending: number[];
  structured: number[];
  funding: number[];
  staking: number[];
  confidence: number[];
}

export interface IsfrSourceSnapshot {
  bps: number;
  ts: number;
}

export interface IsfrEventEntry {
  ts: number;
  type: 'rate' | 'source' | 'keeper';
  message: string;
}

// ── Chain types ──────────────────────────────────────────────────

export interface ChainBlockEntry {
  number: number;
  hash: string;
  parentHash: string;
  timestamp: number;
  gasUsed: number;
  gasLimit: number;
  txCount: number;
  baseFeePerGas: number | null;
}

export interface ChainTxEntry {
  blockNumber: number;
  txHash: string;
  from: string;
  to: string | null;
  valueWei: string;
  gasUsed: number;
  methodSig: string | null;
  success: boolean;
}

export interface ChainEventEntry {
  blockNumber: number;
  txHash: string;
  logIndex: number;
  contract: string;
  eventName: string;
  decoded: Record<string, unknown>;
}

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
  isfrFieldHistory: IsfrFieldHistory;
  isfrSourceHistory: Record<string, IsfrSourceSnapshot[]>;
  isfrEventLog: IsfrEventEntry[];
  isfrReadingsCache: Record<string, Record<string, unknown>>;

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

  // -- Chain slice ------------------------------------------------
  chainBlocks: ChainBlockEntry[];
  chainTxs: ChainTxEntry[];
  chainEvents: ChainEventEntry[];
  chainLatestBlock: ChainBlockEntry | null;
  chainWatcherRunning: boolean;
  chainGasHistory: number[];

  // -- Actions: Chain REST fetches --------------------------------
  fetchChainBlocks: () => Promise<void>;
  fetchChainTxs: () => Promise<void>;
  fetchChainEvents: () => Promise<void>;
  fetchChainStatus: () => Promise<void>;
}

// ── Ring-buffer limits ──────────────────────────────────────────

const MAX_EPISODES = 500;
const MAX_INFERENCES = 200;
const MAX_ISFR_HISTORY = 256;
const MAX_FIELD_HISTORY = 30;
const MAX_SOURCE_HISTORY = 30;
const MAX_EVENT_LOG = 500;
const MAX_CHAIN_BLOCKS = 64;
const MAX_CHAIN_TXS = 128;
const MAX_CHAIN_EVENTS = 128;
const MAX_CHAIN_GAS_HISTORY = 64;

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
  isfrFieldHistory: { composite: [], lending: [], structured: [], funding: [], staking: [], confidence: [] },
  isfrSourceHistory: {},
  isfrEventLog: [],
  isfrReadingsCache: {},
  chainBlocks: [],
  chainTxs: [],
  chainEvents: [],
  chainLatestBlock: null,
  chainWatcherRunning: false,
  chainGasHistory: [],

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

      case 'isfr_rate_computed': {
        const capField = (arr: number[], v: number) =>
          [...arr.slice(-(MAX_FIELD_HISTORY - 1)), v];
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
          isfrFieldHistory: {
            composite: capField(s.isfrFieldHistory.composite, event.compositeBps),
            lending: capField(s.isfrFieldHistory.lending, event.lendingBps),
            structured: capField(s.isfrFieldHistory.structured, event.structuredBps),
            funding: capField(s.isfrFieldHistory.funding, event.fundingBps),
            staking: capField(s.isfrFieldHistory.staking, event.stakingBps),
            confidence: capField(s.isfrFieldHistory.confidence, event.confidenceBps),
          },
          isfrEventLog: [
            {
              ts: event.timestampMs,
              type: 'rate' as const,
              message: `Composite ${event.compositeBps} bps \u00b7 ${event.sourceCount} sources \u00b7 ${(event.confidenceBps / 100).toFixed(1)}% conf`,
            },
            ...s.isfrEventLog.slice(0, MAX_EVENT_LOG - 1),
          ],
        }));
        // Refetch current rate (with readings) to populate per-source history
        get().fetchIsfrCurrent();
        break;
      }

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
                {
                  id: event.sourceId,
                  name: event.sourceId,
                  class: 'unknown',
                  weight: 0,
                  health: event.health,
                  lastRateBps: event.lastRateBps,
                  lastPollMs: null,
                },
              ],
          isfrEventLog: [
            {
              ts: Date.now(),
              type: 'source' as const,
              message: `${event.sourceId} \u2192 ${event.health}${event.lastRateBps != null ? ` (${event.lastRateBps} bps)` : ''}`,
            },
            ...s.isfrEventLog.slice(0, MAX_EVENT_LOG - 1),
          ],
        }));
        // Also refetch full source list to get complete metadata
        get().fetchIsfrSources();
        break;

      case 'isfr_keeper_state_changed':
        set((s) => ({
          isfrKeeperStatus: event.running ? 'running' : 'stopped',
          isfrEventLog: [
            {
              ts: Date.now(),
              type: 'keeper' as const,
              message: event.running ? 'Keeper started' : 'Keeper stopped',
            },
            ...s.isfrEventLog.slice(0, MAX_EVENT_LOG - 1),
          ],
        }));
        break;

      case 'chain_block':
        set((s) => ({
          chainLatestBlock: {
            number: event.number,
            hash: event.hash,
            parentHash: event.parentHash,
            timestamp: event.timestamp,
            gasUsed: event.gasUsed,
            gasLimit: event.gasLimit,
            txCount: event.txCount,
            baseFeePerGas: event.baseFeePerGas,
          },
          chainBlocks: [
            {
              number: event.number,
              hash: event.hash,
              parentHash: event.parentHash,
              timestamp: event.timestamp,
              gasUsed: event.gasUsed,
              gasLimit: event.gasLimit,
              txCount: event.txCount,
              baseFeePerGas: event.baseFeePerGas,
            },
            ...s.chainBlocks.slice(0, MAX_CHAIN_BLOCKS - 1),
          ],
          chainGasHistory: [
            ...s.chainGasHistory.slice(-(MAX_CHAIN_GAS_HISTORY - 1)),
            event.gasUsed,
          ],
        }));
        break;

      case 'chain_tx':
        set((s) => ({
          chainTxs: [
            {
              blockNumber: event.blockNumber,
              txHash: event.txHash,
              from: event.from,
              to: event.to,
              valueWei: event.valueWei,
              gasUsed: event.gasUsed,
              methodSig: event.methodSig,
              success: event.success,
            },
            ...s.chainTxs.slice(0, MAX_CHAIN_TXS - 1),
          ],
        }));
        break;

      case 'chain_contract_event':
        set((s) => ({
          chainEvents: [
            {
              blockNumber: event.blockNumber,
              txHash: event.txHash,
              logIndex: event.logIndex,
              contract: event.contract,
              eventName: event.eventName,
              decoded: event.decoded,
            },
            ...s.chainEvents.slice(0, MAX_CHAIN_EVENTS - 1),
          ],
        }));
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
        readings?: Array<{
          source_id: string; source_name: string; rate_bps: number;
          weight: number; class: string; metadata: Record<string, unknown> | null;
        }>;
      }>('/api/isfr/current');
      if (res.ok && 'composite_bps' in res.data) {
        const d = res.data;
        const capSrc = (arr: IsfrSourceSnapshot[] | undefined, v: IsfrSourceSnapshot) =>
          [...(arr ?? []).slice(-(MAX_SOURCE_HISTORY - 1)), v];
        set((s) => {
          const nextSourceHistory = { ...s.isfrSourceHistory };
          const nextReadingsCache = { ...s.isfrReadingsCache };
          if (d.readings) {
            for (const r of d.readings) {
              nextSourceHistory[r.source_name] = capSrc(
                nextSourceHistory[r.source_name],
                { bps: r.rate_bps, ts: d.timestamp_ms },
              );
              if (r.metadata) {
                nextReadingsCache[r.source_name] = r.metadata;
              }
            }
          }
          return {
            isfrCurrentRate: {
              compositeBps: d.composite_bps,
              lendingBps: d.lending_bps,
              structuredBps: d.structured_bps,
              fundingBps: d.funding_bps,
              stakingBps: d.staking_bps,
              confidenceBps: d.confidence_bps,
              sourceCount: d.source_count,
              timestampMs: d.timestamp_ms,
            },
            isfrSourceHistory: nextSourceHistory,
            isfrReadingsCache: nextReadingsCache,
          };
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
        | Array<{ id: string; name: string; class: string; weight: number;
                  health: string; last_rate_bps: number | null; last_poll_ms: number | null }>
        | { sources: Array<{ id: string; name: string; class: string; weight: number;
                             health: string; last_rate_bps: number | null; last_poll_ms: number | null }> }
      >('/api/isfr/sources');
      if (res.ok) {
        const data = res.data;
        const arr = Array.isArray(data) ? data : (data?.sources ?? []);
        set({
          isfrSources: arr.map((s) => ({
            id: s.id,
            name: s.name,
            class: s.class,
            weight: s.weight,
            health: s.health as IsfrSource['health'],
            lastRateBps: s.last_rate_bps,
            lastPollMs: s.last_poll_ms,
          })),
        });
      }
    } catch (err) {
      console.warn('[DataHub] fetchIsfrSources failed:', err);
    }
  },

  // -- Chain fetch actions -----------------------------------------

  fetchChainBlocks: async () => {
    try {
      const res = await api.get<{ blocks: Array<{
        number: number; hash: string; parent_hash: string; timestamp: number;
        gas_used: number; gas_limit: number; tx_count: number; base_fee_per_gas: number | null;
      }> }>('/api/chain/blocks?limit=64');
      if (res.ok) {
        set({
          chainBlocks: res.data.blocks.map((b) => ({
            number: b.number,
            hash: b.hash,
            parentHash: b.parent_hash,
            timestamp: b.timestamp,
            gasUsed: b.gas_used,
            gasLimit: b.gas_limit,
            txCount: b.tx_count,
            baseFeePerGas: b.base_fee_per_gas,
          })),
        });
      }
    } catch (err) {
      console.warn('[DataHub] fetchChainBlocks failed:', err);
    }
  },

  fetchChainTxs: async () => {
    try {
      const res = await api.get<{ transactions: Array<{
        block_number: number; tx_hash: string; from: string; to: string | null;
        value_wei: string; gas_used: number; method_sig: string | null; success: boolean;
      }> }>('/api/chain/transactions?limit=128');
      if (res.ok) {
        set({
          chainTxs: res.data.transactions.map((t) => ({
            blockNumber: t.block_number,
            txHash: t.tx_hash,
            from: t.from,
            to: t.to,
            valueWei: t.value_wei,
            gasUsed: t.gas_used,
            methodSig: t.method_sig,
            success: t.success,
          })),
        });
      }
    } catch (err) {
      console.warn('[DataHub] fetchChainTxs failed:', err);
    }
  },

  fetchChainEvents: async () => {
    try {
      const res = await api.get<{ events: Array<{
        block_number: number; tx_hash: string; log_index: number;
        contract: string; event_name: string; decoded: Record<string, unknown>;
      }> }>('/api/chain/events?limit=128');
      if (res.ok) {
        set({
          chainEvents: res.data.events.map((e) => ({
            blockNumber: e.block_number,
            txHash: e.tx_hash,
            logIndex: e.log_index,
            contract: e.contract,
            eventName: e.event_name,
            decoded: e.decoded,
          })),
        });
      }
    } catch (err) {
      console.warn('[DataHub] fetchChainEvents failed:', err);
    }
  },

  fetchChainStatus: async () => {
    try {
      const res = await api.get<{ watcher_running: boolean; latest_block: number | null }>('/api/chain/watcher');
      if (res.ok) {
        set({ chainWatcherRunning: res.data.watcher_running });
      }
    } catch (err) {
      console.warn('[DataHub] fetchChainStatus failed:', err);
    }
  },
}));
