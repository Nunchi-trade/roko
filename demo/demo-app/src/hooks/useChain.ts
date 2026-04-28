import { useState, useCallback } from 'react';

/**
 * useChain -- stub hook.
 *
 * The /api/chain/status endpoint returns 400 (chain backend not configured).
 * This hook now returns a static "not connected" state instead of polling.
 */
export function useChain() {
  const [status] = useState<null>(null);
  const [loading] = useState(false);
  const [error] = useState<string | null>('Chain not configured');

  // no-op refresh for callers that may invoke it
  const refresh = useCallback(() => {}, []);

  return { status, blocks: [] as never[], loading, error, refresh };
}
