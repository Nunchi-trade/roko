import { useEffect, useRef, useCallback } from 'react';

type EventHandler = (event: unknown) => void;

const KNOWN_SSE_EVENT_TYPES = [
  'workflow_started',
  'phase_transition',
  'workflow_completed',
  'agent_spawned',
  'agent_output',
  'agent_completed',
  'agent_failed',
  'gate_started',
  'gate_passed',
  'gate_failed',
  'feedback_recorded',
  'state_checkpointed',
  'inference_started',
  'inference_completed',
  'inference_failed',
  'agent_trace',
  'task_failed',
  'run_started',
  'run_completed',
  'knowledge_ingested',
  'knowledge_consumed',
  'plan_started',
  'plan_completed',
  'task_started',
  'task_completed',
  'gate_result',
] as const;

export interface EventStreamManager {
  connected: boolean;
  /** Callback invoked whenever the connected state changes. */
  onConnectedChange: ((connected: boolean) => void) | null;
  subscribe(types: string[], handler: EventHandler): () => void;
  destroy(): void;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function normalizeEvent(parsed: Record<string, unknown>, fallbackType?: string): Record<string, unknown> {
  const nested = isRecord(parsed.data) ? parsed.data : {};
  const type = typeof parsed.type === 'string'
    ? parsed.type
    : typeof parsed.kind === 'string'
      ? parsed.kind
      : fallbackType ?? 'unknown';

  return {
    ...nested,
    ...parsed,
    type,
  };
}

/**
 * Singleton EventSource manager. Connects to the SSE endpoint once and
 * dispatches parsed events to subscribers by `type` field. The server sends
 * named SSE events, so wildcard subscribers are backed by known runtime event
 * names plus the default message channel.
 */
const BACKOFF_DELAYS = [1_000, 2_000, 4_000, 8_000, 15_000];
const MAX_RETRIES = 5;

interface SourceEntry {
  source: EventSource | null;
  timer: ReturnType<typeof setTimeout> | undefined;
  retries: number;
  path: string;
  namedEvents: boolean;
}

export function createEventStreamManager(baseUrl: string): EventStreamManager {
  let connected = false;
  let destroyed = false;
  let onConnectedChange: ((connected: boolean) => void) | null = null;

  const sourceEntries = new Map<string, SourceEntry>();
  const handlers = new Map<string, Set<EventHandler>>();
  const connectListeners = new Set<() => void>();
  const listenedTypes = new Set<string>();

  function updateConnected() {
    const prev = connected;
    connected = false;
    for (const entry of sourceEntries.values()) {
      if (entry.source?.readyState === EventSource.OPEN) {
        connected = true;
        break;
      }
    }
    if (connected !== prev) {
      for (const fn of connectListeners) fn();
      if (onConnectedChange) onConnectedChange(connected);
    }
  }

  function dispatch(type: string, event: unknown) {
    const set = handlers.get(type);
    if (set) {
      for (const handler of set) handler(event);
    }
    const wildcard = handlers.get('*');
    if (wildcard) {
      for (const handler of wildcard) handler(event);
    }
  }

  function handleEventData(data: string, fallbackType?: string) {
    try {
      const parsed = JSON.parse(data) as Record<string, unknown>;
      const event = normalizeEvent(parsed, fallbackType);
      const type = typeof event.type === 'string' ? event.type : 'unknown';
      dispatch(type, event);
    } catch {
      // Ignore unparseable events
    }
  }

  function attachNamedListener(source: EventSource, type: string) {
    source.addEventListener(type, (event) => {
      handleEventData((event as MessageEvent).data, type);
    });
  }

  function ensureListening(type: string) {
    if (type === '*') {
      for (const knownType of KNOWN_SSE_EVENT_TYPES) ensureListening(knownType);
      return;
    }
    if (listenedTypes.has(type)) return;
    listenedTypes.add(type);
    for (const entry of sourceEntries.values()) {
      if (entry.source) attachNamedListener(entry.source, type);
    }
  }

  function openSource(entry: SourceEntry) {
    if (destroyed) return;

    // Close previous source if still around
    if (entry.source) {
      entry.source.close();
      entry.source = null;
    }

    const source = new EventSource(`${baseUrl}${entry.path}`);
    entry.source = source;

    source.onopen = () => {
      if (destroyed || entry.source !== source) return;
      entry.retries = 0; // Reset on successful connection
      updateConnected();
    };

    source.onmessage = (e) => {
      if (destroyed || entry.source !== source) return;
      handleEventData(e.data);
    };

    if (entry.namedEvents) {
      for (const type of listenedTypes) attachNamedListener(source, type);
    }

    source.onerror = () => {
      if (destroyed || entry.source !== source) return;
      source.close();
      entry.source = null;
      updateConnected();

      // Independent retry for this source only
      if (entry.retries < MAX_RETRIES) {
        const delay = BACKOFF_DELAYS[Math.min(entry.retries, BACKOFF_DELAYS.length - 1)];
        console.warn(`[SSE] ${entry.path} error, retry ${entry.retries + 1}/${MAX_RETRIES} in ${delay}ms`);
        entry.retries++;
        clearTimeout(entry.timer);
        entry.timer = setTimeout(() => openSource(entry), delay);
      } else {
        console.warn(`[SSE] ${entry.path} max retries reached, stopping reconnect`);
      }
    };
  }

  function initSource(path: string, namedEvents: boolean) {
    const entry: SourceEntry = { source: null, timer: undefined, retries: 0, path, namedEvents };
    sourceEntries.set(path, entry);
    openSource(entry);
  }

  initSource('/api/events', false);
  initSource('/api/workflow/events', true);

  return {
    get connected() {
      return connected;
    },

    get onConnectedChange() {
      return onConnectedChange;
    },
    set onConnectedChange(cb: ((connected: boolean) => void) | null) {
      onConnectedChange = cb;
    },

    subscribe(types: string[], handler: EventHandler): () => void {
      for (const t of types) {
        ensureListening(t);
        let set = handlers.get(t);
        if (!set) {
          set = new Set();
          handlers.set(t, set);
        }
        set.add(handler);
      }
      const wrappedConnect = () => {};
      connectListeners.add(wrappedConnect);

      return () => {
        for (const t of types) {
          const set = handlers.get(t);
          if (set) {
            set.delete(handler);
            if (set.size === 0) handlers.delete(t);
          }
        }
        connectListeners.delete(wrappedConnect);
      };
    },

    destroy() {
      destroyed = true;
      for (const entry of sourceEntries.values()) {
        clearTimeout(entry.timer);
        entry.source?.close();
        entry.source = null;
      }
      sourceEntries.clear();
      handlers.clear();
      connectListeners.clear();
      connected = false;
    },
  };
}

/**
 * Hook that subscribes to specific SSE event types. Must be used inside
 * an `<EventStreamProvider>`.
 *
 * This is a standalone version — prefer `useEventStreamSubscription` from
 * the context module which accesses the shared singleton automatically.
 */
export function useEventStreamSubscription(
  manager: EventStreamManager | null,
  eventTypes: string[],
  handler: (event: unknown) => void,
) {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  const typesKey = eventTypes.join(',');

  const stableHandler = useCallback((event: unknown) => {
    handlerRef.current(event);
  }, []);

  useEffect(() => {
    if (!manager) return;
    const types = typesKey.split(',').filter(Boolean);
    if (types.length === 0) return;
    return manager.subscribe(types, stableHandler);
  }, [manager, typesKey, stableHandler]);
}
