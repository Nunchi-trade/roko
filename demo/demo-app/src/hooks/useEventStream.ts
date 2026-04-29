import { useEffect, useRef, useCallback } from 'react';

type EventHandler = (event: unknown) => void;

export interface EventStreamManager {
  connected: boolean;
  subscribe(types: string[], handler: EventHandler): () => void;
  destroy(): void;
}

/**
 * Singleton EventSource manager. Connects to the SSE endpoint once and
 * dispatches parsed events to subscribers by `type` field.
 */
export function createEventStreamManager(baseUrl: string): EventStreamManager {
  let es: EventSource | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  let connected = false;
  let destroyed = false;

  const handlers = new Map<string, Set<EventHandler>>();
  const connectListeners = new Set<() => void>();

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

  function connect() {
    if (destroyed) return;
    es?.close();

    const source = new EventSource(`${baseUrl}/api/events`);
    es = source;

    source.onopen = () => {
      if (destroyed || es !== source) return;
      connected = true;
      notifyConnect();
    };

    source.onmessage = (e) => {
      if (destroyed || es !== source) return;
      try {
        const parsed = JSON.parse(e.data) as Record<string, unknown>;
        const type = typeof parsed.type === 'string' ? parsed.type : 'unknown';
        dispatch(type, parsed);
      } catch {
        // Ignore unparseable events
      }
    };

    source.onerror = () => {
      if (destroyed || es !== source) {
        source.close();
        return;
      }
      connected = false;
      notifyConnect();
      source.close();
      es = null;
      clearTimeout(reconnectTimer);
      reconnectTimer = setTimeout(connect, 3_000);
    };
  }

  connect();

  return {
    get connected() {
      return connected;
    },

    subscribe(types: string[], handler: EventHandler): () => void {
      for (const t of types) {
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
      if (es) {
        es.close();
        es = null;
      }
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
