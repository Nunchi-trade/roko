import { useState, useCallback, useEffect } from 'react';
import { useEventStreamContext } from '../contexts/EventStreamContext';

// ── Types ────────────────────────────────────────────────────

export interface InferenceCall {
  model: string;
  tier: 'T0' | 'T1' | 'T2';
  cost: number;
  inputTokens: number;
  outputTokens: number;
  latencyMs: number;
  timestamp: number;
}

export interface InferenceTraceTotals {
  cost: number;
  tokens: number;
  calls: number;
  avgLatencyMs: number;
}

export interface InferenceTraceState {
  calls: InferenceCall[];
  totals: InferenceTraceTotals;
  costSeries: number[];
  reset: () => void;
}

// ── Constants ────────────────────────────────────────────────

const MAX_CALLS = 50;
const MAX_SPARKLINE_POINTS = 20;

// ── Helpers ──────────────────────────────────────────────────

function isRecord(v: unknown): v is Record<string, unknown> {
  return v !== null && typeof v === 'object' && !Array.isArray(v);
}

function readNum(obj: Record<string, unknown>, keys: string[]): number {
  for (const k of keys) {
    const v = obj[k];
    if (typeof v === 'number' && Number.isFinite(v)) return v;
  }
  // Check nested .data / .event
  for (const nest of ['data', 'event'] as const) {
    const sub = obj[nest];
    if (!isRecord(sub)) continue;
    for (const k of keys) {
      const v = sub[k];
      if (typeof v === 'number' && Number.isFinite(v)) return v;
    }
  }
  return 0;
}

function readStr(obj: Record<string, unknown>, keys: string[]): string {
  for (const k of keys) {
    const v = obj[k];
    if (typeof v === 'string' && v.length > 0) return v;
  }
  for (const nest of ['data', 'event'] as const) {
    const sub = obj[nest];
    if (!isRecord(sub)) continue;
    for (const k of keys) {
      const v = sub[k];
      if (typeof v === 'string' && v.length > 0) return v;
    }
  }
  return '';
}

function inferTier(model: string): 'T0' | 'T1' | 'T2' {
  const m = model.toLowerCase();
  if (m.includes('opus') || m.includes('gpt-4') || m.includes('o1') || m.includes('o3')) return 'T0';
  if (m.includes('sonnet') || m.includes('gpt-3.5') || m.includes('gemini')) return 'T1';
  return 'T2';
}

function eventType(event: unknown): string | null {
  if (!isRecord(event)) return null;
  if (typeof event.type === 'string') return event.type;
  if (typeof event.kind === 'string') return event.kind;
  if (isRecord(event.data) && typeof event.data.type === 'string') return event.data.type;
  return null;
}

// ── Hook ─────────────────────────────────────────────────────

export function useInferenceTrace(): InferenceTraceState {
  const [calls, setCalls] = useState<InferenceCall[]>([]);
  const [costSeries, setCostSeries] = useState<number[]>([]);

  const { subscribe } = useEventStreamContext();

  const reset = useCallback(() => {
    setCalls([]);
    setCostSeries([]);
  }, []);

  useEffect(() => {
    return subscribe(['*'], (event: unknown) => {
      const type = eventType(event);
      if (type !== 'inference_completed') return;
      if (!isRecord(event)) return;

      const model = readStr(event, ['model', 'model_id', 'modelId']);
      const inputTokens = readNum(event, ['input_tokens', 'inputTokens', 'tokens_in', 'tokensIn']);
      const outputTokens = readNum(event, ['output_tokens', 'outputTokens', 'tokens_out', 'tokensOut']);
      const fallbackTokens = readNum(event, ['total_tokens', 'totalTokens', 'tokens', 'tokens_used']);
      const cost = readNum(event, ['cost_usd', 'costUsd', 'cost']);
      const latencyMs = readNum(event, ['latency_ms', 'latencyMs', 'latency', 'duration_ms', 'durationMs']);
      const tier = readStr(event, ['tier']) as 'T0' | 'T1' | 'T2' || inferTier(model);

      const call: InferenceCall = {
        model: model || 'unknown',
        tier,
        cost,
        inputTokens,
        outputTokens: outputTokens || (fallbackTokens - inputTokens) || 0,
        latencyMs,
        timestamp: Date.now(),
      };

      setCalls((prev) => [...prev.slice(-(MAX_CALLS - 1)), call]);
      setCostSeries((prev) => [...prev.slice(-(MAX_SPARKLINE_POINTS - 1)), cost]);
    });
  }, [subscribe]);

  // Derive totals from the calls buffer (not a separate accumulator)
  const totals: InferenceTraceTotals = {
    cost: calls.reduce((s, c) => s + c.cost, 0),
    tokens: calls.reduce((s, c) => s + c.inputTokens + c.outputTokens, 0),
    calls: calls.length,
    avgLatencyMs: calls.length > 0
      ? calls.reduce((s, c) => s + c.latencyMs, 0) / calls.length
      : 0,
  };

  return { calls, totals, costSeries, reset };
}
