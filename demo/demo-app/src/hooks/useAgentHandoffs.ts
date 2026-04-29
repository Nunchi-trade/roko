import { useState, useEffect, useMemo } from 'react';
import { useEventStreamContext } from '../contexts/EventStreamContext';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface HandoffEntry {
  id: string;
  from: { name: string; role?: string; status?: 'idle' | 'working' | 'done' };
  to: { name: string; role?: string; status?: 'idle' | 'working' | 'done' };
  status: 'pending' | 'active' | 'done' | 'error';
  label: string;
  taskId?: string;
  timestamp: number;
}

export interface UseAgentHandoffsResult {
  /** All observed handoffs (capped at 20, FIFO). */
  handoffs: HandoffEntry[];
  /** Most recent entry with status === 'active', or null. */
  activeHandoff: HandoffEntry | null;
}

// ---------------------------------------------------------------------------
// SSE event shapes
// ---------------------------------------------------------------------------

interface AgentSpawnedEvent {
  type: 'AgentSpawned';
  agent_name: string;
  role: string;
  task_id: string;
}

interface AgentCompletedEvent {
  type: 'AgentCompleted';
  agent_name: string;
  task_id: string;
  success: boolean;
}

interface TaskAssignedEvent {
  type: 'TaskAssigned';
  task_id: string;
  agent_name: string;
  plan_id: string;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_HANDOFFS = 20;

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Derives agent handoff events from SSE AgentSpawned / TaskAssigned /
 * AgentCompleted events. Each AgentSpawned creates a pending handoff from
 * the orchestrator to the spawned agent. TaskAssigned transitions it to
 * active, and AgentCompleted marks it done or error.
 */
export function useAgentHandoffs(): UseAgentHandoffsResult {
  const { manager } = useEventStreamContext();
  const [handoffs, setHandoffs] = useState<HandoffEntry[]>([]);

  useEffect(() => {
    if (!manager) return;

    const unsub = manager.subscribe(
      ['AgentSpawned', 'AgentCompleted', 'TaskAssigned'],
      (event: unknown) => {
        const e = event as { type: string };

        if (e.type === 'AgentSpawned') {
          const ev = event as AgentSpawnedEvent;
          const entry: HandoffEntry = {
            id: `handoff-${ev.agent_name}-${Date.now()}`,
            from: { name: 'orchestrator', role: 'dispatcher', status: 'working' },
            to: { name: ev.agent_name, role: ev.role, status: 'idle' },
            status: 'pending',
            label: `Spawning ${ev.agent_name}`,
            taskId: ev.task_id,
            timestamp: Date.now(),
          };
          setHandoffs((prev) => {
            const next = [...prev, entry];
            return next.length > MAX_HANDOFFS ? next.slice(-MAX_HANDOFFS) : next;
          });
        }

        if (e.type === 'TaskAssigned') {
          const ev = event as TaskAssignedEvent;
          setHandoffs((prev) =>
            prev.map((h) => {
              if (h.to.name === ev.agent_name && h.status === 'pending') {
                return {
                  ...h,
                  status: 'active' as const,
                  label: ev.task_id,
                  to: { ...h.to, status: 'working' as const },
                };
              }
              return h;
            }),
          );
        }

        if (e.type === 'AgentCompleted') {
          const ev = event as AgentCompletedEvent;
          setHandoffs((prev) =>
            prev.map((h) => {
              if (
                h.to.name === ev.agent_name &&
                (h.status === 'active' || h.status === 'pending')
              ) {
                return {
                  ...h,
                  status: ev.success ? ('done' as const) : ('error' as const),
                  to: { ...h.to, status: 'done' as const },
                  from: { ...h.from, status: 'idle' as const },
                };
              }
              return h;
            }),
          );
        }
      },
    );

    return unsub;
  }, [manager]);

  const activeHandoff = useMemo(() => {
    for (let i = handoffs.length - 1; i >= 0; i--) {
      if (handoffs[i].status === 'active') return handoffs[i];
    }
    return null;
  }, [handoffs]);

  return { handoffs, activeHandoff };
}
