import { useEffect, useState, useRef, useCallback, useMemo, type ReactNode } from 'react';
import './CommandPalette.css';

interface PaletteItem {
  id: string;
  label: string;
  category?: string;
  shortcut?: string;
  icon?: ReactNode;
  action: () => void;
}

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  items: PaletteItem[];
  placeholder?: string;
  className?: string;
}

export default function CommandPalette({
  open,
  onClose,
  items,
  placeholder = 'Type a command...',
  className,
}: CommandPaletteProps) {
  const [visible, setVisible] = useState(false);
  const [closing, setClosing] = useState(false);
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // --- Filtered results with fuzzy (substring) match ---

  const filtered = useMemo(() => {
    if (!query.trim()) return items;
    const q = query.toLowerCase();
    return items.filter((item) => item.label.toLowerCase().includes(q));
  }, [items, query]);

  // --- Group by category ---

  interface GroupedItem {
    type: 'category' | 'item';
    category?: string;
    item?: PaletteItem;
    flatIndex: number;
  }

  const { groups, flatItems } = useMemo(() => {
    const groupMap = new Map<string, PaletteItem[]>();
    const flat: PaletteItem[] = [];

    for (const item of filtered) {
      const cat = item.category ?? '';
      if (!groupMap.has(cat)) groupMap.set(cat, []);
      groupMap.get(cat)!.push(item);
      flat.push(item);
    }

    const result: GroupedItem[] = [];
    let idx = 0;
    for (const [cat, catItems] of groupMap) {
      if (cat) result.push({ type: 'category', category: cat, flatIndex: -1 });
      for (const item of catItems) {
        result.push({ type: 'item', item, flatIndex: idx++ });
      }
    }

    return { groups: result, flatItems: flat };
  }, [filtered]);

  // --- Open / close lifecycle ---

  useEffect(() => {
    if (open) {
      setVisible(true);
      setClosing(false);
      setQuery('');
      setActiveIndex(0);
    } else if (visible) {
      setClosing(true);
      const timer = setTimeout(() => {
        setVisible(false);
        setClosing(false);
      }, 150);
      return () => clearTimeout(timer);
    }
  }, [open, visible]);

  // Focus input on open
  useEffect(() => {
    if (visible && !closing) {
      const timer = setTimeout(() => inputRef.current?.focus(), 50);
      return () => clearTimeout(timer);
    }
  }, [visible, closing]);

  // Reset active index on filter change
  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  // --- Scroll active item into view ---

  useEffect(() => {
    if (!listRef.current) return;
    const active = listRef.current.querySelector('.cmdpal-item--active');
    active?.scrollIntoView({ block: 'nearest' });
  }, [activeIndex]);

  // --- Keyboard navigation ---

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
        return;
      }

      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setActiveIndex((prev) => (prev + 1) % Math.max(flatItems.length, 1));
        return;
      }

      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setActiveIndex((prev) => (prev - 1 + Math.max(flatItems.length, 1)) % Math.max(flatItems.length, 1));
        return;
      }

      if (e.key === 'Enter') {
        e.preventDefault();
        const item = flatItems[activeIndex];
        if (item) {
          item.action();
          onClose();
        }
      }
    },
    [flatItems, activeIndex, onClose],
  );

  // --- Backdrop click ---

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget) onClose();
    },
    [onClose],
  );

  if (!visible) return null;

  const backdropCls = [
    'cmdpal-backdrop',
    !closing && 'cmdpal-backdrop--open',
    closing && 'cmdpal-backdrop--closing',
  ]
    .filter(Boolean)
    .join(' ');

  const containerCls = ['cmdpal-container', className].filter(Boolean).join(' ');

  return (
    <div className={backdropCls} onClick={handleBackdropClick} role="presentation">
      <div className={containerCls} role="listbox" onKeyDown={handleKeyDown}>
        <div className="cmdpal-input-wrap">
          <input
            ref={inputRef}
            className="cmdpal-input"
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={placeholder}
            aria-label="Search commands"
          />
        </div>

        <div className="cmdpal-results" ref={listRef}>
          {flatItems.length === 0 ? (
            <div className="cmdpal-empty">No matches</div>
          ) : (
            groups.map((entry, i) => {
              if (entry.type === 'category') {
                return (
                  <div key={`cat-${entry.category}-${i}`} className="cmdpal-category">
                    {entry.category}
                  </div>
                );
              }

              const item = entry.item!;
              const isActive = entry.flatIndex === activeIndex;

              return (
                <div
                  key={item.id}
                  className={`cmdpal-item${isActive ? ' cmdpal-item--active' : ''}`}
                  role="option"
                  aria-selected={isActive}
                  onClick={() => {
                    item.action();
                    onClose();
                  }}
                  onMouseEnter={() => setActiveIndex(entry.flatIndex)}
                >
                  {item.icon && <span className="cmdpal-item-icon">{item.icon}</span>}
                  <span className="cmdpal-item-label">{item.label}</span>
                  {item.category && <span className="cmdpal-item-category">{item.category}</span>}
                  {item.shortcut && <span className="cmdpal-item-shortcut">{item.shortcut}</span>}
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}
