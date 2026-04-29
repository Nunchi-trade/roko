import {
  useState,
  useCallback,
  useRef,
  useEffect,
  Fragment,
  type ReactNode,
} from 'react';
import './PaneGroup.css';

/* ── public types ── */

export interface PaneGroupItem {
  id: string;
  /** Content to render */
  children: ReactNode;
  /** Initial size as fraction (0-1), all items must sum to 1.0 */
  initialSize?: number;
  /** Minimum size in px */
  minSize?: number;
}

export interface PaneGroupProps {
  /** Stack direction */
  direction: 'horizontal' | 'vertical';
  /** Pane items to stack */
  items: PaneGroupItem[];
  /** Gap between panes in px (default 2) */
  gap?: number;
  /** localStorage key for persisted sizes */
  persistKey?: string;
  className?: string;
}

/* ── component ── */

export default function PaneGroup({
  direction,
  items,
  gap = 2,
  persistKey,
  className,
}: PaneGroupProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerSize, setContainerSize] = useState(0);
  const [sizes, setSizes] = useState<number[]>([]);
  const initializedRef = useRef(false);

  // Drag state ref (mutable, no re-render on start)
  const dragRef = useRef<{
    index: number;
    startPos: number;
    startSizes: number[];
  } | null>(null);

  /* ── Measure container ── */
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      const s =
        direction === 'horizontal'
          ? entry.contentRect.width
          : entry.contentRect.height;
      setContainerSize(s);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [direction]);

  /* ── Derive initial sizes from container ── */
  useEffect(() => {
    if (containerSize <= 0) return;
    if (initializedRef.current) return;

    // Try localStorage first
    if (persistKey) {
      try {
        const stored = localStorage.getItem(persistKey);
        if (stored) {
          const parsed = JSON.parse(stored) as number[];
          if (Array.isArray(parsed) && parsed.length === items.length) {
            setSizes(parsed);
            initializedRef.current = true;
            return;
          }
        }
      } catch { /* corrupt, derive fresh */ }
    }

    const totalGap = (items.length - 1) * gap;
    const available = containerSize - totalGap;
    const fractions = items.map((it) => it.initialSize ?? 1 / items.length);
    const computed = fractions.map((f, i) =>
      Math.max(f * available, items[i]?.minSize ?? 60),
    );
    setSizes(computed);
    initializedRef.current = true;
  }, [containerSize, items.length, gap, persistKey, items]);

  /* ── Persistence ── */
  const persist = useCallback(
    (s: number[]) => {
      if (persistKey) {
        try {
          localStorage.setItem(persistKey, JSON.stringify(s));
        } catch { /* quota exceeded */ }
      }
    },
    [persistKey],
  );

  /* ── Shared resize handles ── */
  const handlePointerDown = useCallback(
    (handleIndex: number) => (e: React.PointerEvent) => {
      e.preventDefault();
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
      dragRef.current = {
        index: handleIndex,
        startPos: direction === 'horizontal' ? e.clientX : e.clientY,
        startSizes: [...sizes],
      };
    },
    [direction, sizes],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent) => {
      const d = dragRef.current;
      if (!d) return;

      const pos = direction === 'horizontal' ? e.clientX : e.clientY;
      const delta = pos - d.startPos;
      const idx = d.index;
      const minA = items[idx]?.minSize ?? 60;
      const minB = items[idx + 1]?.minSize ?? 60;

      const newA = Math.max(minA, d.startSizes[idx] + delta);
      const newB = Math.max(minB, d.startSizes[idx + 1] - delta);

      setSizes((prev) => {
        const n = [...prev];
        n[idx] = newA;
        n[idx + 1] = newB;
        return n;
      });
    },
    [direction, items],
  );

  const handlePointerUp = useCallback(() => {
    if (dragRef.current) {
      dragRef.current = null;
      persist(sizes);
    }
  }, [sizes, persist]);

  /* ── Render ── */
  const isHorizontal = direction === 'horizontal';

  return (
    <div
      ref={containerRef}
      className={`pane-group pane-group--${direction} ${className ?? ''}`}
      style={{
        display: 'flex',
        flexDirection: isHorizontal ? 'row' : 'column',
        width: '100%',
        height: '100%',
      }}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
    >
      {items.map((item, i) => (
        <Fragment key={item.id}>
          <div
            className="pane-group__pane"
            style={{
              [isHorizontal ? 'width' : 'height']: sizes[i] != null ? `${sizes[i]}px` : 'auto',
              overflow: 'hidden',
              flexShrink: 0,
            }}
          >
            {item.children}
          </div>
          {i < items.length - 1 && (
            <div
              className={`pane-group__handle pane-group__handle--${direction}`}
              style={{ flexShrink: 0, [isHorizontal ? 'width' : 'height']: `${gap}px` }}
              onPointerDown={handlePointerDown(i)}
            />
          )}
        </Fragment>
      ))}
    </div>
  );
}
