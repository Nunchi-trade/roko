/**
 * ISFR API client — typed wrappers around roko-serve ISFR endpoints.
 *
 * Uses the singleton `api` from transport/api.ts (never throws, returns ApiResult).
 * Components should prefer DataHub fetch actions for state integration; use these
 * directly only for one-off queries or non-reactive contexts.
 */

import { api } from '../transport/api';
import type { ApiResult } from '../transport/api';

// ── Response types (snake_case, matching roko-serve JSON) ────────

export interface IsfrStatusResponse {
  enabled: boolean;
  keeper_running: boolean;
  sources_count: number;
  current_rate_bps: number | null;
  current_confidence: number | null;
  poll_interval_secs: number;
  epoch_duration_secs: number;
}

/** Flat composite rate snapshot — matches the backend's CompositeRate serialisation.
 *  Fields are snake_case as returned by roko-serve; DataHub maps these to camelCase. */
export interface IsfrRateResponse {
  composite_bps: number;
  lending_bps: number;
  structured_bps: number;
  funding_bps: number;
  staking_bps: number;
  confidence_bps: number;
  source_count: number;
  timestamp_ms: number;
}

export interface IsfrSourceResponse {
  id: string;
  name: string;
  class: string;
  weight: number;
  health: 'live' | 'stale' | 'offline';
  last_rate_bps: number | null;
  last_poll_ms: number | null;
}

/** Individual source reading within a CompositeRate response. */
export interface SourceReading {
  source_id: string;
  source_name: string;
  rate_bps: number;
  weight: number;
  class: string;
  metadata: Record<string, unknown> | null;
}

/** Extended composite rate with per-source readings (from /api/isfr/current). */
export interface IsfrRateWithReadings extends IsfrRateResponse {
  readings?: SourceReading[];
}

// ── Canonical aliases (used by IsfrDashboard) ────────────────────

/** @alias IsfrStatusResponse */
export type IsfrStatus = IsfrStatusResponse;
/** @alias IsfrRateResponse */
export type IsfrRate = IsfrRateResponse;
/** @alias IsfrSourceResponse */
export type IsfrSource = IsfrSourceResponse;

// ── API functions ────────────────────────────────────────────────

/** GET /api/isfr/status — keeper status and config overview. */
export function fetchIsfrStatus(
  signal?: AbortSignal,
): Promise<ApiResult<IsfrStatusResponse>> {
  return api.get<IsfrStatusResponse>('/api/isfr/status', signal);
}

/** GET /api/isfr/current — latest computed composite rate. */
export function fetchIsfrCurrent(
  signal?: AbortSignal,
): Promise<ApiResult<IsfrRateResponse>> {
  return api.get<IsfrRateResponse>('/api/isfr/current', signal);
}

/** GET /api/isfr/history?limit=N — historical rate samples. */
export function fetchIsfrHistory(
  limit = 50,
  signal?: AbortSignal,
): Promise<ApiResult<IsfrRateResponse[]>> {
  return api.get<IsfrRateResponse[]>(
    `/api/isfr/history?limit=${limit}`,
    signal,
  );
}

/** GET /api/isfr/sources — all configured sources with health. */
export function fetchIsfrSources(
  signal?: AbortSignal,
): Promise<ApiResult<IsfrSourceResponse[]>> {
  return api.get<IsfrSourceResponse[]>('/api/isfr/sources', signal);
}

// ── SSE stream URL (for dedicated ISFR-only EventSource) ─────────

/**
 * Returns the URL for the ISFR-filtered SSE stream (F2).
 * Use with SseAdapter or a raw EventSource for dedicated ISFR streaming
 * separate from the main /api/events stream.
 */
export function isfrStreamUrl(): string {
  return `${api.baseUrl}/api/isfr/stream`;
}

// ── Formatting helpers ───────────────────────────────────────────

/**
 * Format a rate in basis points: e.g. "580 bps".
 * Returns "—" for null/undefined.
 */
export function formatBps(bps: number | null | undefined): string {
  if (bps == null) return '\u2014';
  return `${bps.toFixed(0)} bps`;
}

/**
 * Format a basis-point rate as a percentage: e.g. "5.80%".
 * Returns "—" for null/undefined.
 */
export function formatPercent(bps: number | null | undefined): string {
  if (bps == null) return '\u2014';
  return `${(bps / 100).toFixed(2)}%`;
}

/**
 * Format a confidence value (0–10000 bps scale) as a human-readable percentage.
 * Returns "—" for null/undefined.
 */
export function formatConfidence(confidenceBps: number | null | undefined): string {
  if (confidenceBps == null) return '\u2014';
  return `${(confidenceBps / 100).toFixed(1)}%`;
}
