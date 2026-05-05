/**
 * Frosted glass overlay panel for dashboard scenes.
 *
 * Positioned absolute within a DashboardScene. Corner ornaments for visual flair.
 */
import type { ReactNode, CSSProperties } from 'react';
import './GlassPanel.css';

export type GlassPanelPosition =
  | 'top-left'
  | 'top-right'
  | 'bottom-left'
  | 'bottom-right'
  | 'bottom-center';

interface GlassPanelProps {
  position: GlassPanelPosition;
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
  /** Max width in px. Defaults to 340. */
  maxWidth?: number;
  /** Max height in px. No default. */
  maxHeight?: number;
  /** Hide the corner ornaments. */
  noOrnaments?: boolean;
}

export default function GlassPanel({
  position,
  children,
  className,
  style,
  maxWidth = 340,
  maxHeight,
  noOrnaments,
}: GlassPanelProps) {
  return (
    <div
      className={`glass-panel glass-panel--${position} ${className ?? ''}`}
      style={{ maxWidth, maxHeight, ...style }}
    >
      {!noOrnaments && (
        <>
          <span className="glass-panel__ornament glass-panel__ornament--tl" />
          <span className="glass-panel__ornament glass-panel__ornament--br" />
        </>
      )}
      <div className="glass-panel__content">
        {children}
      </div>
    </div>
  );
}
