import { useEffect, useRef, useMemo } from 'react';
import { createPortal } from 'react-dom';
import type { ShortcutDef } from '../hooks/useKeyboardShortcuts';
import './HelpOverlay.css';

interface HelpOverlayProps {
  open: boolean;
  onClose: () => void;
  shortcuts: ShortcutDef[];
}

/**
 * Modal overlay displaying all registered keyboard shortcuts,
 * grouped by category. Renders via portal into document.body.
 */
export default function HelpOverlay({ open, onClose, shortcuts }: HelpOverlayProps) {
  const panelRef = useRef<HTMLDivElement>(null);

  // Group shortcuts by category
  const groups = useMemo(() => {
    const map = new Map<string, ShortcutDef[]>();
    for (const s of shortcuts) {
      const cat = s.category || 'General';
      const arr = map.get(cat) ?? [];
      arr.push(s);
      map.set(cat, arr);
    }
    return Array.from(map.entries());
  }, [shortcuts]);

  // Focus trap + Escape to close
  useEffect(() => {
    if (!open) return;

    // Focus the panel on open
    panelRef.current?.focus();

    function handleKey(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
      // Trap tab within the overlay
      if (e.key === 'Tab') {
        const focusable = panelRef.current?.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        );
        if (!focusable || focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    }

    window.addEventListener('keydown', handleKey, true);
    return () => window.removeEventListener('keydown', handleKey, true);
  }, [open, onClose]);

  if (!open) return null;

  function renderKeys(keys: string) {
    // "g d" -> ["g", "d"], "Ctrl+/" -> ["Ctrl", "/"], "Escape" -> ["Esc"]
    const parts = keys.includes('+')
      ? keys.split('+')
      : keys.split(' ');
    return (
      <span className="help-overlay-keys">
        {parts.map((part, i) => (
          <span key={i}>
            <kbd className="help-overlay-kbd">
              {part === 'Escape' ? 'Esc' : part}
            </kbd>
            {i < parts.length - 1 && keys.includes(' ') && (
              <span style={{ color: 'var(--text-ghost)', margin: '0 2px', fontSize: 10 }}>then</span>
            )}
          </span>
        ))}
      </span>
    );
  }

  return createPortal(
    <div
      className="help-overlay-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      role="dialog"
      aria-modal="true"
      aria-label="Keyboard shortcuts"
    >
      <div
        className="help-overlay-panel"
        ref={panelRef}
        tabIndex={-1}
      >
        <div className="help-overlay-header">
          <span className="help-overlay-title">Keyboard Shortcuts</span>
          <button
            className="help-overlay-close"
            onClick={onClose}
            aria-label="Close help overlay"
          >
            Esc
          </button>
        </div>
        <div className="help-overlay-body">
          {groups.map(([category, items]) => (
            <div key={category} className="help-overlay-category">
              <div className="help-overlay-cat-label">{category}</div>
              {items.map((s) => (
                <div key={s.keys} className="help-overlay-row">
                  <span className="help-overlay-desc">{s.description}</span>
                  {renderKeys(s.keys)}
                </div>
              ))}
            </div>
          ))}
        </div>
      </div>
    </div>,
    document.body,
  );
}
