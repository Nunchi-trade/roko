/**
 * Shared utilities for ClickableScenario implementations.
 *
 * Provides helpers for multi-pane command targeting, execution,
 * and CommandDef factory functions.
 */
import type { TerminalHandle } from '../hooks/useTerminal';
import type { CommandDef, CommandTarget } from './scenarios';
import { showCmd } from './terminal-session';

// ── Target resolution ───────────────────────────────────────

/**
 * Resolve a CommandTarget to a list of TerminalHandle entries.
 * Returns entries in pane order. Defaults to [entries[0]] if no target.
 */
export function resolveTargetEntries(
  target: CommandTarget | undefined,
  entries: TerminalHandle[],
): TerminalHandle[] {
  if (!target) return entries.slice(0, 1);
  if (target === 'all') return entries;
  if ('pane' in target) {
    const e = entries[target.pane];
    return e ? [e] : [];
  }
  if ('panes' in target) {
    return target.panes.map(i => entries[i]).filter((e): e is TerminalHandle => !!e);
  }
  return entries.slice(0, 1);
}

// ── Command execution ───────────────────────────────────────

export interface ExecCommandOpts {
  timeout?: number;
  customDesc?: string;
  signal?: AbortSignal;
  workspaceDir?: string;
  onGate?: (name: string, status: 'pass' | 'fail') => void;
  onCost?: (cost: string) => void;
  onTokens?: (tokens: string) => void;
}

/**
 * Execute a command on the resolved target pane(s).
 * For multi-pane targets, runs in parallel and aggregates results.
 */
export async function executeCommand(
  entries: TerminalHandle[],
  command: string,
  target: CommandTarget | undefined,
  opts?: ExecCommandOpts,
): Promise<{ ok: boolean; error?: string }> {
  const targets = resolveTargetEntries(target, entries);
  if (targets.length === 0) {
    return { ok: false, error: 'No terminal connected for target' };
  }

  const results = await Promise.all(
    targets.map(handle =>
      showCmd(handle, command, {
        timeout: opts?.timeout ?? 60000,
        customDesc: opts?.customDesc,
        signal: opts?.signal,
        workspaceDir: opts?.workspaceDir,
        onGate: opts?.onGate,
        onCost: opts?.onCost,
        onTokens: opts?.onTokens,
      }),
    ),
  );

  const failed = results.find(r => !r.ok);
  return { ok: !failed, error: failed?.error };
}

// ── CommandDef factory helpers ───────────────────────────────

/** Create a CommandDef targeting a single pane. */
export function cmdForPane(
  pane: number,
  id: string,
  command: string,
  description: string,
  timeout?: number,
): CommandDef {
  return { id, command, description, timeout, target: { pane } };
}

/** Create a CommandDef targeting multiple panes. */
export function cmdForPanes(
  panes: number[],
  id: string,
  command: string,
  description: string,
  timeout?: number,
): CommandDef {
  return { id, command, description, timeout, target: { panes } };
}

/** Create a CommandDef targeting all panes. */
export function cmdForAll(
  id: string,
  command: string,
  description: string,
  timeout?: number,
): CommandDef {
  return { id, command, description, timeout, target: 'all' };
}
