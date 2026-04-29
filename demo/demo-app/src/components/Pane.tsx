import type { ReactNode } from 'react';
import FlatIcon, { inferIcon, type FlatIconName } from './FlatIcon';

interface PaneProps {
  title: string;
  icon?: FlatIconName;
  badge?: ReactNode;
  foot?: ReactNode;
  flat?: boolean;
  children: ReactNode;
  className?: string;
  style?: React.CSSProperties;
}

/**
 * Reusable glass panel: head (LED + title + badge) / body (children) / foot.
 * Uses the canonical .pane/.head/.body/.foot classes from rosedust.css.
 */
export default function Pane({ title, icon, badge, foot, flat, children, className, style }: PaneProps) {
  return (
    <div className={`pane${className ? ` ${className}` : ''}`} style={{ marginTop: 0, ...style }}>
      <div className="head">
        <div className="l">
          <FlatIcon name={icon ?? inferIcon(title)} size={14} tone="muted" className="pane-title-icon" />
          <b>{title}</b>
        </div>
        {badge && <div className="r">{badge}</div>}
      </div>
      <div className={`body${flat ? ' flat' : ''}`}>
        {children}
      </div>
      {foot && <div className="foot">{foot}</div>}
    </div>
  );
}
