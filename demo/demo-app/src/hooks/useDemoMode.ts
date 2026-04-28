import { useEffect, useState, useCallback, useRef } from 'react';

const CYCLE_MS = 45_000;
const VIEW_COUNT = 4;

export function useDemoMode() {
  const [active, setActive] = useState(false);
  const [paused, setPaused] = useState(false);
  const [currentView, setCurrentView] = useState(1);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const toggle = useCallback(() => setActive((a) => !a), []);
  const pause = useCallback(() => setPaused((p) => !p), []);
  const jumpTo = useCallback((view: number) => {
    if (view >= 1 && view <= VIEW_COUNT) setCurrentView(view);
  }, []);

  // Auto-cycle
  useEffect(() => {
    if (timerRef.current) clearInterval(timerRef.current);
    if (active && !paused) {
      timerRef.current = setInterval(() => {
        setCurrentView((v) => (v % VIEW_COUNT) + 1);
      }, CYCLE_MS);
    }
    return () => { if (timerRef.current) clearInterval(timerRef.current); };
  }, [active, paused]);

  // Keyboard
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.key === 'd' || e.key === 'D') toggle();
      else if (e.key === ' ' && active) { e.preventDefault(); pause(); }
      else if (e.key >= '1' && e.key <= '4' && active) jumpTo(parseInt(e.key));
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [active, toggle, pause, jumpTo]);

  return { active, paused, currentView, toggle, jumpTo, pause };
}
