import {
  useState,
  useCallback,
  useRef,
  useEffect,
  type ReactNode,
} from 'react';
import './PaneGrid.css';

/* ── public types ── */

export interface PaneGridItem {
  id: string;
  /** Grid area name (e.g., 'main', 'sidebar', 'footer') */
  area: string;
  /** Content to render inside this grid cell */
  children: ReactNode;
  /** Minimum width in px (default 120) */
  minWidth?: number;
  /** Minimum height in px (default 80) */
  minHeight?: number;
}

export interface PaneGridLayout {
  /** CSS grid-template-columns, e.g. '1fr 300px' */
  columns: string;
  /** CSS grid-template-rows, e.g. '1fr 200px' */
  rows: string;
  /** CSS grid-template-areas, e.g. '"main sidebar" "footer footer"' */
  areas: string;
}

export interface PaneGridProps {
  items: PaneGridItem[];
  layout: PaneGridLayout;
  /** localStorage key for persisted column/row sizes (default: none = no persistence) */
  persistKey?: string;
  /** Gap between grid cells in px (default 4) */
  gap?: number;
  className?: string;
}

/* ── helpers ── */

/** Parse a CSS grid template like '1fr 300px 2fr' into proportional pixel values. */
function parseGridTemplate(template: string, totalPx: number): number[] {
  const parts = template.trim().split(/\s+/);
  let fixedTotal = 0;
  let frTotal = 0;

  const parsed = parts.map((p) => {
    if (p.endsWith('fr')) {
      const fr = parseFloat(p) || 1;
      frTotal += fr;
      return { type: 'fr' as const, value: fr };
    }
    const px = parseFloat(p) || 0;
    fixedTotal += px;
    return { type: 'px' as const, value: px };
  });

  const remainingPx = Math.max(0, totalPx - fixedTotal);
  return parsed.map((p) =>
    p.type === 'fr'
      ? frTotal > 0 ? (p.value / frTotal) * remainingPx : remainingPx / parsed.length
      : p.value,
  );
}

/* ── component ── */

export default function PaneGrid({
  items,
  layout,
  persistKey,
  gap = 4,
  className,
}: PaneGridProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [colSizes, setColSizes] = useState<number[]>([]);
  const [rowSizes, setRowSizes] = useState<number[]>([]);
  const [initialized, setInitialized] = useState(false);

  // Drag state refs (mutable, no re-render)
  const dragRef = useRef<{
    axis: 'col' | 'row';
    index: number;
    startPos: number;
    startSizes: number[];
  } | null>(null);

  /* ── Initialize sizes from localStorage or layout spec ── */
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const ro = new ResizeObserver(([entry]) => {
      if (initialized) return;
      const { width, height } = entry.contentRect;
      if (width <= 0 || height <= 0) return;

      // Try restore from localStorage
      if (persistKey) {
        try {
          const stored = localStorage.getItem(persistKey);
          if (stored) {
            const data = JSON.parse(stored) as { cols?: number[]; rows?: number[] };
            const colCount = layout.columns.trim().split(/\s+/).length;
            const rowCount = layout.rows.trim().split(/\s+/).length;
            if (
              Array.isArray(data.cols) && data.cols.length === colCount &&
              Array.isArray(data.rows) && data.rows.length === rowCount
            ) {
              setColSizes(data.cols);
              setRowSizes(data.rows);
              setInitialized(true);
              return;
            }
          }
        } catch { /* corrupt data, derive fresh */ }
      }

      const totalGapH = (layout.columns.trim().split(/\s+/).length - 1) * gap;
      const totalGapV = (layout.rows.trim().split(/\s+/).length - 1) * gap;
      setColSizes(parseGridTemplate(layout.columns, width - totalGapH));
      setRowSizes(parseGridTemplate(layout.rows, height - totalGapV));
      setInitialized(true);
    });

    ro.observe(el);
    return () => ro.disconnect();
  }, [layout.columns, layout.rows, gap, persistKey, initialized]);

  /* ── Persistence ── */
  const persist = useCallback(
    (cols: number[], rows: number[]) => {
      if (persistKey) {
        try {
          localStorage.setItem(persistKey, JSON.stringify({ cols, rows }));
        } catch { /* quota exceeded */ }
      }
    },
    [persistKey],
  );

  /* ── Column resize handlers ── */
  const handleColPointerDown = useCallback(
    (index: number) => (e: React.PointerEvent) => {
      e.preventDefault();
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
      dragRef.current = {
        axis: 'col',
        index,
        startPos: e.clientX,
        startSizes: [...colSizes],
      };
    },
    [colSizes],
  );

  /* ── Row resize handlers ── */
  const handleRowPointerDown = useCallback(
    (index: number) => (e: React.PointerEvent) => {
      e.preventDefault();
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
      dragRef.current = {
        axis: 'row',
        index,
        startPos: e.clientY,
        startSizes: [...rowSizes],
      };
    },
    [rowSizes],
  );

  const handlePointerMove = useCallback(
    (e: React.PointerEvent) => {
      const d = dragRef.current;
      if (!d) return;

      if (d.axis === 'col') {
        const delta = e.clientX - d.startPos;
        const leftIdx = d.index;
        const rightIdx = d.index + 1;
        const leftMin = items.find((_, i) => i === leftIdx)?.minWidth ?? 120;
        const rightMin = items.find((_, i) => i === rightIdx)?.minWidth ?? 120;
        const newLeft = Math.max(leftMin, d.startSizes[leftIdx] + delta);
        const newRight = Math.max(rightMin, d.startSizes[rightIdx] - delta);
        setColSizes((prev) => {
          const n = [...prev];
          n[leftIdx] = newLeft;
          n[rightIdx] = newRight;
          return n;
        });
      } else {
        const delta = e.clientY - d.startPos;
        const topIdx = d.index;
        const bottomIdx = d.index + 1;
        const topMin = items.find((_, i) => i === topIdx)?.minHeight ?? 80;
        const bottomMin = items.find((_, i) => i === bottomIdx)?.minHeight ?? 80;
        const newTop = Math.max(topMin, d.startSizes[topIdx] + delta);
        const newBottom = Math.max(bottomMin, d.startSizes[bottomIdx] - delta);
        setRowSizes((prev) => {
          const n = [...prev];
          n[topIdx] = newTop;
          n[bottomIdx] = newBottom;
          return n;
        });
      }
    },
    [items],
  );

  const handlePointerUp = useCallback(() => {
    if (dragRef.current) {
      dragRef.current = null;
      persist(colSizes, rowSizes);
    }
  }, [colSizes, rowSizes, persist]);

  /* ── Render ── */
  const gridStyle: React.CSSProperties = initialized && colSizes.length > 0
    ? {
        display: 'grid',
        gridTemplateColumns: colSizes.map((s) => `${s}px`).join(' '),
        gridTemplateRows: rowSizes.map((s) => `${s}px`).join(' '),
        gridTemplateAreas: layout.areas,
        gap,
      }
    : {
        display: 'grid',
        gridTemplateColumns: layout.columns,
        gridTemplateRows: layout.rows,
        gridTemplateAreas: layout.areas,
        gap,
      };

  return (
    <div
      ref={containerRef}
      className={`pane-grid ${className ?? ''}`}
      style={gridStyle}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
    >
      {items.map((item) => (
        <div key={item.id} style={{ gridArea: item.area, overflow: 'hidden' }}>
          {item.children}
        </div>
      ))}

      {/* Column resize handles */}
      {colSizes.slice(0, -1).map((_, i) => (
        <div
          key={`col-handle-${i}`}
          className="pane-grid__col-handle"
          style={{
            gridColumn: `${i + 1} / ${i + 2}`,
            gridRow: '1 / -1',
            justifySelf: 'end',
          }}
          onPointerDown={handleColPointerDown(i)}
        />
      ))}

      {/* Row resize handles */}
      {rowSizes.slice(0, -1).map((_, i) => (
        <div
          key={`row-handle-${i}`}
          className="pane-grid__row-handle"
          style={{
            gridRow: `${i + 1} / ${i + 2}`,
            gridColumn: '1 / -1',
            alignSelf: 'end',
          }}
          onPointerDown={handleRowPointerDown(i)}
        />
      ))}
    </div>
  );
}
