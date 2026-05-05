import { useRef, useCallback } from 'react';

/**
 * Returns a stable trigger function that debounces calls to `fetchFn`.
 * Useful for throttling rapid SSE-triggered refetches.
 */
export function useDebouncedRefetch(
  fetchFn: () => void,
  delayMs: number = 300,
): () => void {
  const timerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const fnRef = useRef(fetchFn);
  fnRef.current = fetchFn;

  return useCallback(() => {
    clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      fnRef.current();
    }, delayMs);
  }, [delayMs]);
}
