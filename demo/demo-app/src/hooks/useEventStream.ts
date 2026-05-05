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
export function createEventStreamManager(baseUrl: string): EventStreamManager {
  let sources = new Set<EventSource>();
  let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  let connected = false;
  let destroyed = false;

  const handlers = new Map<string, Set<EventHandler>>();
  const connectListeners = new Set<() => void>();
  const listenedTypes = new Set<string>();

  function notifyConnect() {
    for (const fn of connectListeners) fn();
  }

  function dispatch(type: string, event: unknown) {
    const set = handlers.get(type);
    if (set) {
      for (const handler of set) handler(event);
    }
    // Also dispatch to wildcard subscribers
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
    for (const source of sources) attachNamedListener(source, type);
  }

  function closeSources() {
    for (const source of sources) source.close();
    sources = new Set<EventSource>();
  }

  function scheduleReconnect() {
    clearTimeout(reconnectTimer);
    reconnectTimer = setTimeout(connect, 3_000);
  }

  function openSource(path: string, namedEvents: boolean) {
    if (destroyed) return;

    const source = new EventSource(`${baseUrl}${path}`);
    sources.add(source);

    source.onopen = () => {
      if (destroyed || !sources.has(source)) return;
      connected = true;
      notifyConnect();
    };

    source.onmessage = (e) => {
      if (destroyed || !sources.has(source)) return;
      handleEventData(e.data);
    };
    if (namedEvents) {
      for (const type of listenedTypes) attachNamedListener(source, type);
    }

    source.onerror = () => {
      if (destroyed || !sources.has(source)) {
        source.close();
        return;
      }
      source.close();
      sources.delete(source);
      connected = sources.size > 0;
      notifyConnect();
      scheduleReconnect();
    };
  }

  function connect() {
    if (destroyed) return;
    closeSources();
    connected = false;
    notifyConnect();
    openSource('/api/events', false);
    openSource('/api/workflow/events', true);
  }

  connect();

  return {
    get connected() {
      return connected;
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
      // Also subscribe to connection state changes
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
      clearTimeout(reconnectTimer);
      closeSources();
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
