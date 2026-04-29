import {
  type CSSProperties,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react';
import './AnimatedTable.css';

/* ═══════════════════════════════════════════════════════════
   AnimatedTable — reusable animated table wrapper
   ═══════════════════════════════════════════════════════════

   Provides:
   1. Staggered row entrance (fadeSlideUp, 30ms between rows)
   2. Sort arrow rotation on column sort change
   3. Row hover lift + background glow
   4. Cell value flash on data change
   5. Expandable row animation (max-height + opacity)
   6. Animated empty state
   7. Page crossfade (pagination transitions)
   8. Row selection with scale-in checkbox

   Usage: wrap a standard <table> or use the helper subcomponents.
*/

/* ── Sort arrow helper ─────────────────────────────────── */

interface SortArrowProps {
  active: boolean;
  ascending: boolean;
}

export function SortArrow({ active, ascending }: SortArrowProps) {
  if (!active) return null;
  return (
    <span
      className={`atbl-sort-arrow ${ascending ? 'atbl-sort-arrow--asc' : 'atbl-sort-arrow--desc'}`}
      aria-hidden
    >
      ↑
    </span>
  );
}

/* ── AnimatedRow ───────────────────────────────────────── */

interface AnimatedRowProps {
  index: number;
  selected?: boolean;
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
  onClick?: () => void;
  onKeyDown?: (e: React.KeyboardEvent) => void;
  tabIndex?: number;
  role?: string;
}

export function AnimatedRow({
  index,
  selected,
  children,
  className = '',
  style,
  onClick,
  onKeyDown,
  tabIndex,
  role,
}: AnimatedRowProps) {
  return (
    <tr
      className={`atbl-row${selected ? ' atbl-row--selected' : ''} ${className}`.trim()}
      style={{ '--row-i': index, ...style } as CSSProperties}
      onClick={onClick}
      onKeyDown={onKeyDown}
      tabIndex={tabIndex}
      role={role}
    >
      {children}
    </tr>
  );
}

/* ── AnimatedHeaderCell ────────────────────────────────── */

interface AnimatedHeaderCellProps {
  sortKey?: string;
  currentSort?: string;
  ascending?: boolean;
  onSort?: (key: string) => void;
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
}

export function AnimatedHeaderCell({
  sortKey,
  currentSort,
  ascending = true,
  onSort,
  children,
  className = '',
  style,
}: AnimatedHeaderCellProps) {
  const active = sortKey != null && sortKey === currentSort;
  const handleClick = sortKey && onSort ? () => onSort(sortKey) : undefined;
  const handleKeyDown = handleClick
    ? (e: React.KeyboardEvent) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          handleClick();
        }
      }
    : undefined;

  return (
    <th
      className={`atbl-header-cell${active ? ' atbl-header-cell--active' : ''} ${className}`.trim()}
      style={style}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      tabIndex={sortKey ? 0 : undefined}
      role={sortKey ? 'columnheader' : undefined}
      aria-sort={active ? (ascending ? 'ascending' : 'descending') : undefined}
    >
      {children}
      {sortKey && (
        <SortArrow active={active} ascending={ascending ?? true} />
      )}
    </th>
  );
}

/* ── ExpandableDetail ──────────────────────────────────── */

interface ExpandableDetailProps {
  open: boolean;
  colSpan: number;
  children: ReactNode;
}

export function ExpandableDetail({ open, colSpan, children }: ExpandableDetailProps) {
  if (!open) return null;
  return (
    <tr>
      <td colSpan={colSpan} style={{ padding: 0 }}>
        <div className={`atbl-expandable${open ? ' atbl-expandable--open' : ''}`}>
          {children}
        </div>
      </td>
    </tr>
  );
}

/* ── EmptyState ────────────────────────────────────────── */

interface EmptyStateProps {
  colSpan: number;
  message?: string;
  icon?: string;
}

export function TableEmptyState({ colSpan, message = 'No data', icon }: EmptyStateProps) {
  return (
    <tr>
      <td colSpan={colSpan}>
        <div className="atbl-empty">
          {icon && <span className="atbl-empty-icon">{icon}</span>}
          {message}
        </div>
      </td>
    </tr>
  );
}

/* ── SelectionCheckbox ─────────────────────────────────── */

interface SelectionCheckboxProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: string;
}

export function SelectionCheckbox({ checked, onChange, label }: SelectionCheckboxProps) {
  return (
    <button
      className={`atbl-check${checked ? ' atbl-check--checked' : ''}`}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!checked);
      }}
      aria-label={label ?? (checked ? 'Deselect row' : 'Select row')}
      aria-pressed={checked}
      type="button"
    />
  );
}

/* ── useCellChange hook ────────────────────────────────── */

/**
 * Track value changes and return a className for the flash animation.
 * Usage: const flashClass = useCellFlash(value);
 */
export function useCellFlash(value: unknown): string {
  const prevRef = useRef(value);
  const [flashing, setFlashing] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => {
    if (prevRef.current !== value && prevRef.current !== undefined) {
      setFlashing(true);
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setFlashing(false), 800);
    }
    prevRef.current = value;
  }, [value]);

  return flashing ? 'atbl-cell-changed' : '';
}

/* ── usePageTransition hook ────────────────────────────── */

/**
 * Returns a wrapper className that triggers crossfade on page change.
 * Usage: const pageClass = usePageTransition(currentPage);
 */
export function usePageTransition(page: number): string {
  const [transitioning, setTransitioning] = useState(false);
  const prevPage = useRef(page);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  const triggerTransition = useCallback(() => {
    setTransitioning(true);
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setTransitioning(false), 400);
  }, []);

  useEffect(() => {
    if (prevPage.current !== page) {
      triggerTransition();
      prevPage.current = page;
    }
  }, [page, triggerTransition]);

  return transitioning ? 'atbl-page-transition' : '';
}
