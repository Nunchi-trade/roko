import { useState, useEffect, useRef, useCallback } from 'react';
import { useEventStreamContext } from '../contexts/EventStreamContext';
import { useApi } from './useApi';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface LearningStats {
  /** Cascade router confidence 0-1. */
  routerConfidence: number;
  /** Direction trend based on delta between recent values. */
  confidenceTrend: 'improving' | 'stable' | 'declining';
  /** Total routing decisions observed. */
  totalDecisions: number;
  /** Adaptive gate thresholds from /api/learn/gate-thresholds. */
  gateThresholds: { name: string; ema: number; count: number }[];
  /** Active experiments from /api/learn/experiments. */
  experiments: {
    id: string;
    variantA: string;
    variantB: string;
    aWins: number;
    bWins: number;
    status: string;
  }[];
  /** True while initial fetch is in progress. */
  loading: boolean;
}

// ---------------------------------------------------------------------------
// SSE event shapes
// ---------------------------------------------------------------------------

interface InferenceCompletedEvent {
  type: 'InferenceCompleted';
  model: string;
  tier: string;
  tokens_in: number;
  tokens_out: number;
  cost: number;
  router_confidence?: number;
  duration_ms: number;
}

// ---------------------------------------------------------------------------
// API response shapes
// ---------------------------------------------------------------------------

interface RouterResponse {
  model_stats: Record<string, { selections: number; successes: number; avg_confidence: number }>;
  total_decisions: number;
}

interface GateThresholdsResponse {
  thresholds: Record<string, { ema: number; count: number; last_updated: string }>;
}

interface ExperimentsResponse {
  experiments: {
    id: string;
    variant_a: string;
    variant_b: string;
    a_wins: number;
    b_wins: number;
    status: string;
  }[];
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TREND_DELTA = 0.02; // confidence change threshold for trend
const POLL_INTERVAL_MS = 10_000;

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Fetches and streams learning feedback loop data:
 * - Cascade router confidence (from SSE InferenceCompleted + initial fetch)
 * - Gate thresholds (polled every 10s)
 * - Experiments (polled every 10s)
 */
export function useLearningStats(): LearningStats {
  const { manager } = useEventStreamContext();
  const { get } = useApi();

  const [routerConfidence, setRouterConfidence] = useState(0);
  const [confidenceTrend, setConfidenceTrend] = useState<LearningStats['confidenceTrend']>('stable');
  const [totalDecisions, setTotalDecisions] = useState(0);
  const [gateThresholds, setGateThresholds] = useState<LearningStats['gateThresholds']>([]);
  const [experiments, setExperiments] = useState<LearningStats['experiments']>([]);
  const [loading, setLoading] = useState(true);

  const prevConfidence = useRef(0);

  // -- Seed from REST endpoints on mount --
  const fetchRouter = useCallback(async () => {
    try {
      const data = await get<RouterResponse>('/api/learn/router');
      if (data.total_decisions > 0) {
        // Compute average confidence across all models
        const entries = Object.values(data.model_stats);
        const avgConf =
          entries.length > 0
            ? entries.reduce((sum, m) => sum + m.avg_confidence, 0) / entries.length
            : 0;
        setRouterConfidence(avgConf);
        setTotalDecisions(data.total_decisions);
        prevConfidence.current = avgConf;
      }
    } catch {
      // Server may be offline; ignore
    }
  }, [get]);

  const fetchThresholds = useCallback(async () => {
    try {
      const data = await get<GateThresholdsResponse>('/api/learn/gate-thresholds');
      const items = Object.entries(data.thresholds).map(([name, t]) => ({
        name,
        ema: t.ema,
        count: t.count,
      }));
      setGateThresholds(items);
    } catch {
      // ignore
    }
  }, [get]);

  const fetchExperiments = useCallback(async () => {
    try {
      const data = await get<ExperimentsResponse>('/api/learn/experiments');
      setExperiments(
        (data.experiments ?? []).map((e) => ({
          id: e.id,
          variantA: e.variant_a,
          variantB: e.variant_b,
          aWins: e.a_wins,
          bWins: e.b_wins,
          status: e.status,
        })),
      );
    } catch {
      // ignore
    }
  }, [get]);

  // Initial fetch
  useEffect(() => {
    let cancelled = false;
    (async () => {
      await Promise.all([fetchRouter(), fetchThresholds(), fetchExperiments()]);
      if (!cancelled) setLoading(false);
    })();
    return () => {
      cancelled = true;
    };
  }, [fetchRouter, fetchThresholds, fetchExperiments]);

  // Poll thresholds and experiments every 10s
  useEffect(() => {
    const id = setInterval(() => {
      void fetchThresholds();
      void fetchExperiments();
    }, POLL_INTERVAL_MS);
    return () => clearInterval(id);
  }, [fetchThresholds, fetchExperiments]);

  // -- Subscribe to SSE InferenceCompleted events --
  useEffect(() => {
    if (!manager) return;

    const unsub = manager.subscribe(['InferenceCompleted'], (event: unknown) => {
      const e = event as InferenceCompletedEvent;
      if (e.router_confidence != null) {
        const newConf = e.router_confidence;
        const delta = newConf - prevConfidence.current;

        setRouterConfidence(newConf);
        setConfidenceTrend(
          delta > TREND_DELTA
            ? 'improving'
            : delta < -TREND_DELTA
              ? 'declining'
              : 'stable',
        );
        prevConfidence.current = newConf;
      }
      setTotalDecisions((prev) => prev + 1);
    });

    return unsub;
  }, [manager]);

  return {
    routerConfidence,
    confidenceTrend,
    totalDecisions,
    gateThresholds,
    experiments,
    loading,
  };
}
