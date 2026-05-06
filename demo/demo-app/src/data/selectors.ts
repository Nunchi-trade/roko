/**
 * DataHub selectors — thin hooks that read individual slices from the
 * centralised Zustand store.
 *
 * These coexist with the legacy context-based hooks (useServerHealth,
 * useLiveApi, useRokoConfig, useWorkspace). Consumers should migrate to
 * these selectors; the old hooks are marked @deprecated.
 *
 * Implementation task: A1.6
 */

import { useDataHub } from '../app/DataHub';
import { useShallow } from 'zustand/react/shallow';

// ── Connection / health ─────────────────────────────────────────────

/** Server reachability status: 'connected' | 'checking' | 'disconnected'. */
export const useServerConnected = () =>
  useDataHub((s) => s.serverStatus === 'connected');

/** Raw server status enum. */
export const useServerStatus = () => useDataHub((s) => s.serverStatus);

/** SSE transport status. */
export const useSseStatus = () => useDataHub((s) => s.sseStatus);

/** WebSocket transport status. */
export const useWsStatus = () => useDataHub((s) => s.wsStatus);

/** All three transport statuses in one selector. */
export const useTransportStatus = () =>
  useDataHub(
    useShallow((s) => ({
      serverStatus: s.serverStatus,
      sseStatus: s.sseStatus,
      wsStatus: s.wsStatus,
    })),
  );

// ── Config ──────────────────────────────────────────────────────────

/** Full config blob (null until first fetch completes). */
export const useFullConfig = () => useDataHub((s) => s.config);

/** Default model string from config. */
export const useDefaultModel = () => useDataHub((s) => s.defaultModel);

/** Default backend string from config. */
export const useDefaultBackend = () => useDataHub((s) => s.defaultBackend);

/** Config slice: config + model + backend + actions. */
export const useConfigSlice = () =>
  useDataHub(
    useShallow((s) => ({
      config: s.config,
      defaultModel: s.defaultModel,
      defaultBackend: s.defaultBackend,
      fetchConfig: s.fetchConfig,
      updateConfig: s.updateConfig,
    })),
  );

// ── Workspace ───────────────────────────────────────────────────────

/** Server working directory (null until fetched). */
export const useServerWorkdir = () => useDataHub((s) => s.serverWorkdir);

/** Current workspace info (null until ensured). */
export const useWorkspaceInfo = () => useDataHub((s) => s.workspace);

/** Workspace actions: ensure, destroy, workdir. */
export const useWorkspaceSlice = () =>
  useDataHub(
    useShallow((s) => ({
      serverWorkdir: s.serverWorkdir,
      workspace: s.workspace,
      ensureWorkspace: s.ensureWorkspace,
      destroyWorkspace: s.destroyWorkspace,
      fetchServerWorkdir: s.fetchServerWorkdir,
    })),
  );

// ── Plan execution ──────────────────────────────────────────────────

/** Active plan ID (null when idle). */
export const useActivePlanId = () => useDataHub((s) => s.activePlanId);

/** Current phase label. */
export const useActivePhase = () => useDataHub((s) => s.activePhase);

/** Whether the active plan has completed. */
export const usePlanCompleted = () => useDataHub((s) => s.planCompleted);

/** Full plan execution slice. */
export const usePlanSlice = () =>
  useDataHub(
    useShallow((s) => ({
      activePlanId: s.activePlanId,
      activePhase: s.activePhase,
      planCompleted: s.planCompleted,
    })),
  );

// ── Agents ──────────────────────────────────────────────────────────

/** Agent list. */
export const useAgents = () => useDataHub((s) => s.agents);

/** Fetch agents action. */
export const useFetchAgents = () => useDataHub((s) => s.fetchAgents);

// ── Episodes / metrics ──────────────────────────────────────────────

/** Episode ring buffer. */
export const useEpisodes = () => useDataHub((s) => s.episodes);

/** Total cost across all inferences. */
export const useTotalCost = () => useDataHub((s) => s.totalCost);

/** Total tokens across all inferences. */
export const useTotalTokens = () => useDataHub((s) => s.totalTokens);

/** Recent inferences ring buffer. */
export const useRecentInferences = () =>
  useDataHub((s) => s.recentInferences);

/** Cost + token metrics bundle. */
export const useCostSlice = () =>
  useDataHub(
    useShallow((s) => ({
      totalCost: s.totalCost,
      totalTokens: s.totalTokens,
      recentInferences: s.recentInferences,
    })),
  );

// ── Bench ───────────────────────────────────────────────────────────

/** Bench run history. */
export const useBenchRuns = () => useDataHub((s) => s.benchRuns);

/** Bench suites. */
export const useBenchSuites = () => useDataHub((s) => s.benchSuites);

/** Bench models. */
export const useBenchModels = () => useDataHub((s) => s.benchModels);

/** Full bench slice with fetch actions. */
export const useBenchSlice = () =>
  useDataHub(
    useShallow((s) => ({
      benchRuns: s.benchRuns,
      benchSuites: s.benchSuites,
      benchModels: s.benchModels,
      fetchBenchRuns: s.fetchBenchRuns,
      fetchBenchSuites: s.fetchBenchSuites,
      fetchBenchModels: s.fetchBenchModels,
    })),
  );

// ── ISFR ────────────────────────────────────────────────────────────

/** Current ISFR composite rate (null until first computation). */
export const useIsfrCurrentRate = () => useDataHub((s) => s.isfrCurrentRate);

/** ISFR rate history ring buffer. */
export const useIsfrHistory = () => useDataHub((s) => s.isfrHistory);

/** ISFR source health list. */
export const useIsfrSources = () => useDataHub((s) => s.isfrSources);

/** ISFR keeper running/stopped/unknown. */
export const useIsfrKeeperStatus = () => useDataHub((s) => s.isfrKeeperStatus);

/** Full ISFR slice for dashboard tiles. */
export const useIsfrSlice = () =>
  useDataHub(
    useShallow((s) => ({
      currentRate: s.isfrCurrentRate,
      history: s.isfrHistory,
      sources: s.isfrSources,
      keeperStatus: s.isfrKeeperStatus,
      fetchIsfrStatus: s.fetchIsfrStatus,
      fetchIsfrCurrent: s.fetchIsfrCurrent,
      fetchIsfrHistory: s.fetchIsfrHistory,
      fetchIsfrSources: s.fetchIsfrSources,
    })),
  );

/** Derived: composite rate as percentage (bps / 100). */
export const useIsfrCompositePercent = () =>
  useDataHub((s) =>
    s.isfrCurrentRate ? s.isfrCurrentRate.compositeBps / 100 : null,
  );

/** Derived: number of live (healthy) sources. */
export const useIsfrHealthySourceCount = () =>
  useDataHub(
    (s) => s.isfrSources.filter((src) => src.health === 'live').length,
  );
