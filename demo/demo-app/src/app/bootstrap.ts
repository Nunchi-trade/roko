/**
 * Transport wiring — connects SSE, WS, and health polling into DataHub.
 *
 * Call `bootstrapTransport()` ONCE before React render.
 * Returns a cleanup function for teardown.
 *
 * Implementation task: T1.11
 */

import { api } from '../transport/api';
import { SseAdapter } from '../transport/sse';
import { WsAdapter } from '../transport/ws';
import { parseServerEvent } from '../transport/types';
import { useDataHub } from './DataHub';
import { SERVE_URL, WS_BASE } from '../lib/serve-url';

/**
 * Initialize transport layer and wire events into DataHub.
 * Call ONCE before React render. Returns cleanup function.
 */
export function bootstrapTransport(): () => void {
  const hub = useDataHub.getState;
  const set = useDataHub.setState;

  // 1. Probe server health
  api.probe().then((snap) => {
    set({
      serverStatus: snap.reachable ? 'connected' : 'disconnected',
    });
  });

  // 2. Health poll every 30s
  const healthInterval = setInterval(() => {
    api.probe(true).then((snap) => {
      set({
        serverStatus: snap.reachable ? 'connected' : 'disconnected',
      });
    });
  }, 30_000);

  // 3. Connect SSE -> route events to DataHub
  const sse = new SseAdapter({
    url: `${SERVE_URL}/api/events`,
    onEvent: (raw) => {
      const event = parseServerEvent(raw);
      if (event) hub().handleServerEvent(event);
    },
    onStatusChange: (status) => set({ sseStatus: status }),
  });
  sse.connect();

  // 4. Connect WS (workflow frames -- not routed to DataHub directly)
  const ws = new WsAdapter({
    url: `${WS_BASE}/api/workflow/ws`,
    onFrame: () => {
      // WS frames are WorkflowFrames consumed by workflow-api.ts.
      // DataHub does not process them directly.
    },
    onStatusChange: (status) => set({ wsStatus: status }),
  });
  ws.connect();

  // 5. Initial REST fetches
  hub().fetchConfig();
  hub().fetchServerWorkdir();

  // 6. Cleanup function
  return () => {
    clearInterval(healthInterval);
    sse.destroy();
    ws.destroy();
  };
}
