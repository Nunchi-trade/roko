/**
 * DataHub domain types.
 *
 * Centralised type definitions for every domain slice in the DataHub store.
 * Import these instead of scattering interfaces across hooks.
 */

import type { BenchRun, BenchSuite, BenchModel } from '../lib/bench-types';

// ── Re-exports from bench-types (kept canonical there) ──────────────
export type { BenchRun, BenchSuite, BenchModel };

// ── Server / health ─────────────────────────────────────────────────

export type ServerStatus = 'connected' | 'checking' | 'disconnected';

export type StreamStatus =
  | 'idle'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'failed';

export interface HealthSnapshot {
  reachable: boolean;
  checkedAt: number;
}

// ── Config ──────────────────────────────────────────────────────────

export interface RokoConfig {
  [key: string]: unknown;
}

export type Theme = 'dark' | 'light';

// ── Workspace ───────────────────────────────────────────────────────

export interface WorkspaceInfo {
  id: string;
  path: string;
  ready: boolean;
}

// ── Pipeline ────────────────────────────────────────────────────────

export type PipelineStage =
  | 'idle'
  | 'setup'
  | 'idea'
  | 'draft'
  | 'published'
  | 'planning'
  | 'tasks'
  | 'implementing'
  | 'complete'
  | 'failed';

export interface PipelineState {
  stage: PipelineStage;
  planId: string | null;
  activePhase: string | null;
  completed: boolean;
}

// ── Agents ──────────────────────────────────────────────────────────

export interface AgentInfo {
  agentId: string;
  role: string;
  model: string;
  status: 'running' | 'stopped';
}

// ── Episodes / metrics ──────────────────────────────────────────────

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

// ── Knowledge ───────────────────────────────────────────────────────

export interface KnowledgeEntry {
  id: string;
  topic: string;
  content: string;
  tier: number;
  confidence: number;
  createdAt: number;
  updatedAt: number;
  tags: string[];
}

export interface GraphNode {
  id: string;
  label: string;
  type: 'knowledge' | 'agent' | 'episode' | 'plan';
  connections: string[];
  metadata?: Record<string, unknown>;
}

// ── Terminal ────────────────────────────────────────────────────────

export type TerminalStatus = 'connecting' | 'connected' | 'disconnected';

export interface TerminalSession {
  id: string;
  label: string;
  status: TerminalStatus;
  agentId: string | null;
  createdAt: number;
}
