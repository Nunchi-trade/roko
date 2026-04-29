import { useEffect, useRef, useCallback, useState } from 'react';

export interface ShortcutDef {
  keys: string;
  description: string;
  category: string;
  action: () => void;
}

/**
 * Global keyboard shortcut system.
 * Supports single-key (`?`, `Escape`) and sequence shortcuts (`g d`, `g t`).
 * Ignores input when focus is inside <input>, <textarea>, <select>, or [contenteditable].
 */
export function useKeyboardShortcuts(
  shortcuts: ShortcutDef[],
  enabled = true,
) {
  const pendingRef = useRef<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const shortcutsRef = useRef(shortcuts);
  shortcutsRef.current = shortcuts;

  useEffect(() => {
    if (!enabled) return;

    function isEditable(el: EventTarget | null): boolean {
      if (!el || !(el instanceof HTMLElement)) return false;
      const tag = el.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
      if (el.isContentEditable) return true;
      return false;
    }

    function handleKeyDown(e: KeyboardEvent) {
      if (isEditable(e.target)) return;

      const key = e.key;

      // Ctrl+/ shortcut
      if (e.ctrlKey && key === '/') {
        const match = shortcutsRef.current.find((s) => s.keys === 'Ctrl+/');
        if (match) {
          e.preventDefault();
          match.action();
          return;
        }
      }

      // Ignore modifier-only or control-modified keys (except Ctrl+/)
      if (e.ctrlKey || e.metaKey || e.altKey) return;

      // Check for sequence (two-key combos like "g d")
      if (pendingRef.current) {
        const seq = `${pendingRef.current} ${key}`;
        clearTimeout(timerRef.current);
        pendingRef.current = null;
        const match = shortcutsRef.current.find((s) => s.keys === seq);
        if (match) {
          e.preventDefault();
          match.action();
        }
        return;
      }

      // Check if this could be the start of a sequence
      const isSequenceStart = shortcutsRef.current.some(
        (s) => s.keys.startsWith(`${key} `) && s.keys.includes(' '),
      );
      if (isSequenceStart) {
        pendingRef.current = key;
        timerRef.current = setTimeout(() => {
          pendingRef.current = null;
        }, 800);
        return;
      }

      // Single key match
      const match = shortcutsRef.current.find((s) => s.keys === key);
      if (match) {
        e.preventDefault();
        match.action();
      }
    }

    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      clearTimeout(timerRef.current);
    };
  }, [enabled]);
}

/**
 * Hook for managing help overlay open/close state via keyboard.
 */
export function useHelpOverlay() {
  const [open, setOpen] = useState(false);
  const toggle = useCallback(() => setOpen((v) => !v), []);
  const close = useCallback(() => setOpen(false), []);
  return { open, toggle, close };
}
