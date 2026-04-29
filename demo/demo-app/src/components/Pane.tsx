import { type ReactNode, useState, useRef, useEffect } from 'react';
import FlatIcon, { inferIcon, type FlatIconName } from './FlatIcon';

export type PaneVariant = 'default' | 'accent' | 'success' | 'warning' | 'error';
export type PaneEntrance = 'none' | 'fade' | 'slide-up' | 'scale';

interface PaneProps {
  title: string;
  icon?: FlatIconName;
  /** Right-side header content (badges, buttons, status) */
  badge?: ReactNode;
  /** Alias for badge */
  headerRight?: ReactNode;
  /** Footer content below body */
  foot?: ReactNode;
  /** Alias for foot */
  footer?: ReactNode;
  flat?: boolean;
  children: ReactNode;
  className?: string;
  style?: React.CSSProperties;
  /** Visual variant — adds colored left border accent */
  variant?: PaneVariant;
  /** Entrance animation on mount */
  entrance?: PaneEntrance;
  /** Compact mode — reduced padding */
  compact?: boolean;
  /** Collapsible — click header to toggle body */
  collapsible?: boolean;
  /** Default collapsed state (only when collapsible) */
  defaultCollapsed?: boolean;
}

/**
 * Reusable glass panel: head (LED + title + badge) / body (children) / foot.
 * Uses the canonical .pane/.head/.body/.foot classes from rosedust.css.
 */
export default function Pane({
  title, icon, badge, headerRight, foot, footer, flat, children, className, style,
  variant = 'default', entrance = 'none', compact, collapsible, defaultCollapsed = false,
}: PaneProps) {
  const [collapsed, setCollapsed] = useState(collapsible ? defaultCollapsed : false);
  const [entered, setEntered] = useState(entrance === 'none');
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (entrance !== 'none') {
      // Trigger entrance on next frame so the initial class is applied first
      const id = requestAnimationFrame(() => setEntered(true));
      return () => cancelAnimationFrame(id);
    }
  }, [entrance]);

  const resolvedBadge = badge ?? headerRight;
  const resolvedFoot = foot ?? footer;

  const cls = [
    'pane',
    variant !== 'default' && `pane--${variant}`,
    compact && 'pane--compact',
    collapsible && 'pane--collapsible',
    collapsed && 'pane--collapsed',
    entrance !== 'none' && `pane-enter-${entrance}`,
    entrance !== 'none' && entered && 'pane-entered',
    className,
  ].filter(Boolean).join(' ');

  return (
    <div className={cls} style={{ marginTop: 0, ...style }}>
      <div
        className="head"
        onClick={collapsible ? () => setCollapsed(c => !c) : undefined}
        style={collapsible ? { cursor: 'pointer', userSelect: 'none' } : undefined}
      >
        <div className="l">
          <FlatIcon name={icon ?? inferIcon(title)} size={14} tone="muted" className="pane-title-icon" />
          <b>{title}</b>
          {collapsible && (
            <span className={`pane-chevron${collapsed ? '' : ' pane-chevron--open'}`}>&#9656;</span>
          )}
        </div>
        {resolvedBadge && <div className="r">{resolvedBadge}</div>}
      </div>
      <div
        ref={bodyRef}
        className={`body${flat ? ' flat' : ''}`}
        style={collapsible ? {
          maxHeight: collapsed ? 0 : 'none',
          overflow: collapsed ? 'hidden' : undefined,
          padding: collapsed ? 0 : undefined,
          transition: 'max-height var(--duration-smooth) var(--ease)',
        } : undefined}
      >
        {children}
      </div>
      {resolvedFoot && <div className="foot">{resolvedFoot}</div>}
    </div>
  );
}
