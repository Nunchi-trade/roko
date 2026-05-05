import { createContext, useContext, useEffect, useRef, useState, useCallback } from 'react';
import type { ReactNode } from 'react';
import {
  createEventStreamManager,
  useEventStreamSubscription,
  type EventStreamManager,
} from '../hooks/useEventStream';
import { SERVE_URL } from '../lib/serve-url';

interface EventStreamContextValue {
  connected: boolean;
  manager: EventStreamManager | null;
  subscribe(types: string[], handler: (event: unknown) => void): () => void;
}

const EventStreamContext = createContext<EventStreamContextValue>({
  connected: false,
  manager: null,
  subscribe: () => () => {},
});

export function EventStreamProvider({ children }: { children: ReactNode }) {
  const managerRef = useRef<EventStreamManager | null>(null);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    const mgr = createEventStreamManager(SERVE_URL);
    managerRef.current = mgr;

    // Push-based connected state (replaces 500ms polling interval)
    mgr.onConnectedChange = (c: boolean) => setConnected(c);

    return () => {
      mgr.destroy();
      managerRef.current = null;
    };
  }, []);

  const subscribe = useCallback(
    (types: string[], handler: (event: unknown) => void): (() => void) => {
      if (!managerRef.current) return () => {};
      return managerRef.current.subscribe(types, handler);
    },
    [],
  );

  return (
    <EventStreamContext.Provider
      value={{ connected, manager: managerRef.current, subscribe }}
    >
      {children}
    </EventStreamContext.Provider>
  );
}

/** Access the shared EventStream context. */
export function useEventStreamContext() {
  return useContext(EventStreamContext);
}

/**
 * Convenience hook: subscribe to SSE event types from the shared context.
 * Must be used inside `<EventStreamProvider>`.
 */
export function useContextEventSubscription(
  eventTypes: string[],
  handler: (event: unknown) => void,
) {
  const { manager } = useEventStreamContext();
  useEventStreamSubscription(manager, eventTypes, handler);
}
