import {
  useState,
  useCallback,
  useRef,
  useEffect,
  type ReactNode,
} from 'react';
import './ResizablePane.css';

/* ── public types ── */

export interface ResizablePaneProps {
  id: string;
  label?: string;
  icon?: ReactNode;

  // Sizing
  minWidth?: number;
  minHeight?: number;

  // Resize behavior
  resizable?: {
    right?: boolean;
    bottom?: boolean;
  };

  // Header
  showHeader?: boolean;
  headerActions?: ReactNode;
  collapsible?: boolean;
  collapsed?: boolean;
  onCollapse?: (collapsed: boolean) => void;

  // Status
  status?: 'idle' | 'active' | 'loading' | 'error';

  children: ReactNode;
  className?: string;
}

/* ── component ── */

export default function ResizablePane({
  id,
  label,
  icon,
  minWidth = 120,
  minHeight = 80,
  resizable,
  showHeader = true,
  headerActions,
  collapsible = false,
  collapsed: controlledCollapsed,
  onCollapse,
  status = 'idle',
  children,
  className,
}: ResizablePaneProps) {
  const paneRef = useRef<HTMLDivElement>(null);
  const dragging = useRef<'right' | 'bottom' | null>(null);
  const startPos = useRef({ x: 0, y: 0 });
  const startSize = useRef({ w: 0, h: 0 });
  const [dragEdge, setDragEdge] = useState<'right' | 'bottom' | null>(null);

  // Collapse state (uncontrolled fallback)
  const [internalCollapsed, setInternalCollapsed] = useState(false);
  const isControlled = controlledCollapsed !== undefined;
  const isCollapsed = isControlled ? controlledCollapsed : internalCollapsed;

  const toggleCollapse = useCallback(() => {
    const next = !isCollapsed;
    if (isControlled) {
      onCollapse?.(next);
    } else {
      setInternalCollapsed(next);
    }
  }, [isCollapsed, isControlled, onCollapse]);

  /* ── resize via pointer capture (follows SplitView pattern) ── */

  const onPointerDown = useCallback(
    (edge: 'right' | 'bottom') => (e: React.PointerEvent) => {
      e.preventDefault();
      const el = paneRef.current;
      if (!el) return;

      dragging.current = edge;
      setDragEdge(edge);
      startPos.current = { x: e.clientX, y: e.clientY };
      startSize.current = { w: el.offsetWidth, h: el.offsetHeight };
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
    },
    [],
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (!dragging.current || !paneRef.current) return;

      if (dragging.current === 'right') {
        const deltaX = e.clientX - startPos.current.x;
        const newW = Math.max(minWidth, startSize.current.w + deltaX);
        paneRef.current.style.width = `${newW}px`;
      } else {
        const deltaY = e.clientY - startPos.current.y;
        const newH = Math.max(minHeight, startSize.current.h + deltaY);
        paneRef.current.style.height = `${newH}px`;
      }
    },
    [minWidth, minHeight],
  );

  const onPointerUp = useCallback(() => {
    dragging.current = null;
    setDragEdge(null);
  }, []);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      dragging.current = null;
    };
  }, []);

  const rootCls = ['resizable-pane', className].filter(Boolean).join(' ');

  return (
    <div
      ref={paneRef}
      className={rootCls}
      data-pane-id={id}
      style={{ minWidth, minHeight: isCollapsed ? undefined : minHeight }}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
    >
      {/* header */}
      {showHeader && (
        <div className="resizable-pane__header">
          <span className={`resizable-pane__led resizable-pane__led--${status}`} />
          {icon && <span className="resizable-pane__icon">{icon}</span>}
          {label && <span className="resizable-pane__label">{label}</span>}
          <span className="resizable-pane__actions">
            {headerActions}
            {collapsible && (
              <button
                className={[
                  'resizable-pane__collapse-btn',
                  isCollapsed && 'resizable-pane__collapse-btn--collapsed',
                ]
                  .filter(Boolean)
                  .join(' ')}
                onClick={toggleCollapse}
                title={isCollapsed ? 'Expand' : 'Collapse'}
                aria-label={isCollapsed ? 'Expand pane' : 'Collapse pane'}
              >
                {'\u25BE'}
              </button>
            )}
          </span>
        </div>
      )}

      {/* body */}
      <div
        className={[
          'resizable-pane__body',
          isCollapsed && 'resizable-pane__body--collapsed',
        ]
          .filter(Boolean)
          .join(' ')}
      >
        {children}
      </div>

      {/* resize handles */}
      {resizable?.right && (
        <div
          className={[
            'resizable-pane__handle',
            'resizable-pane__handle--right',
            dragEdge === 'right' && 'resizable-pane__handle--dragging',
          ]
            .filter(Boolean)
            .join(' ')}
          onPointerDown={onPointerDown('right')}
        />
      )}
      {resizable?.bottom && (
        <div
          className={[
            'resizable-pane__handle',
            'resizable-pane__handle--bottom',
            dragEdge === 'bottom' && 'resizable-pane__handle--dragging',
          ]
            .filter(Boolean)
            .join(' ')}
          onPointerDown={onPointerDown('bottom')}
        />
      )}
    </div>
  );
}
